//! The `DECONTAM-001` EvalPlus importer.
//!
//! `docs/rebuild-contract.md:159` specifies this extraction completely and
//! assigns it to P6A, but the rebuilt Phase 6A became Adversarial Filter Cases
//! and the importer was never carried into the rebuild plan — `README.md` states
//! the exclusion outright. Its absence is why `prepare-corpus` could only ever be
//! pointed at a hand-authored benchmark manifest.
//!
//! Nothing here executes Python. Classification uses the same pinned tree-sitter
//! boundary the corpus policy uses, and the fragment wrapper is the contract's
//! own, so a record that cannot stand alone is still lexically comparable.

pub mod strict_json;

use crate::data::source::{compact_json_line, join_relative, parse_closed, sha256};
use crate::error::{ProductError, Result};
use crate::parser::{CancellationToken, parse_python};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use strict_json::{Json, Parser as JsonParser};

pub const IMPLEMENTATION_PHASE: &str = "E3";
pub const IMPORT_CONFIG_SCHEMA: &str = "python-slm-benchmark-import-config-v1";
pub const IMPORT_RESULT_SCHEMA: &str = "python-slm-benchmark-import-result-v1";

/// Decoded assets are a few megabytes; this bounds a hostile or corrupt gzip
/// stream from expanding without limit.
const MAXIMUM_DECODED_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkImportConfigV1 {
    pub schema: String,
    pub profile: String,
    /// Directory holding the two acquired `.jsonl.gz` assets, normally a `fetch`
    /// generation root.
    pub asset_root: PathBuf,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkImportResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub profile: String,
    pub registry_id: String,
    pub registry_commit: String,
    pub configuration_sha256: String,
    pub manifest_sha256: String,
    pub protected_records: u64,
    pub python_module_records: u64,
    pub python_fragment_records: u64,
    pub canonical_json_records: u64,
    pub skipped_unrepresentable_values: u64,
    pub output_created: bool,
    pub receipts_written: bool,
    pub limitations: Vec<String>,
}

impl BenchmarkImportConfigV1 {
    pub fn validate(&self) -> Result<()> {
        if self.schema != IMPORT_CONFIG_SCHEMA {
            return Err(ProductError::usage(
                "BENCHMARK_IMPORT_CONFIG_INVALID",
                "the import configuration is not the closed import schema",
            ));
        }
        if self.profile != crate::backend::PROTOTYPE_PROFILE {
            return Err(ProductError::gate(
                "DEFERRED_POST_P16",
                "only the prototype profile is implemented",
            ));
        }
        if !self.asset_root.is_absolute() || !self.output_root.is_absolute() {
            return Err(ProductError::usage(
                "BENCHMARK_IMPORT_PATH_NOT_ABSOLUTE",
                "every import path must be absolute",
            ));
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String> {
        Ok(sha256(&compact_json_line(
            self,
            "BENCHMARK_IMPORT_SERIALIZE_FAILED",
        )?))
    }
}

/// One extracted protection record, before it becomes a manifest entry.
struct Extracted {
    dataset: &'static str,
    task_id: String,
    json_pointer: String,
    role: &'static str,
    kind: crate::corpus::BenchmarkContentKind,
    bytes: Vec<u8>,
}

/// Whether a record stands alone as pinned Python.
///
/// This is the classification `DECONTAM-001` requires: prompts and full solutions
/// must parse standalone, and a contract or test fragment that does not is
/// compared through the contract's wrapper instead.
fn parses_standalone(bytes: &[u8], cancellation: &CancellationToken) -> Result<StandaloneVerdict> {
    let parsed = parse_python(bytes, cancellation)?;
    Ok(StandaloneVerdict {
        accepted: parsed.result.status == "PARSER_ACCEPTED",
        // Carried so a refusal can say why. A bare "does not parse" is unusable
        // when the input is a 20-line function that plainly is valid Python.
        why: format!(
            "status={} reasons={:?} policy={} policy_reason={:?}",
            parsed.result.status,
            parsed.result.reasons,
            parsed.result.policy_status,
            parsed.result.policy_reason
        ),
    })
}

struct StandaloneVerdict {
    accepted: bool,
    why: String,
}

fn decode_asset(path: &Path, expected_asset: &str, expected_decoded: &str) -> Result<Vec<u8>> {
    let compressed = std::fs::read(path).map_err(|_| {
        ProductError::environment(
            "BENCHMARK_ASSET_READ_FAILED",
            format!("could not read the benchmark asset at {}", path.display()),
        )
    })?;
    // The acquired bytes are verified before anything is decoded, which is the
    // order `DECONTAM-001` specifies.
    if sha256(&compressed) != expected_asset {
        return Err(ProductError::integrity(
            "BENCHMARK_ASSET_DIGEST_MISMATCH",
            "an asset does not match its frozen DECONTAM-001 digest",
        ));
    }
    let decoder = GzDecoder::new(compressed.as_slice());
    let mut decoded = Vec::new();
    decoder
        .take(MAXIMUM_DECODED_BYTES)
        .read_to_end(&mut decoded)
        .map_err(|error| {
            ProductError::integrity(
                "BENCHMARK_ASSET_DECODE_FAILED",
                format!("strict gzip decoding failed: {error}"),
            )
        })?;
    if decoded.len() as u64 >= MAXIMUM_DECODED_BYTES {
        return Err(ProductError::integrity(
            "BENCHMARK_ASSET_DECODE_FAILED",
            "the decoded asset exceeded its bound",
        ));
    }
    if sha256(&decoded) != expected_decoded {
        return Err(ProductError::integrity(
            "BENCHMARK_ASSET_DIGEST_MISMATCH",
            "a decoded asset does not match its frozen DECONTAM-001 digest",
        ));
    }
    Ok(decoded)
}

/// Extract every protection record from one decoded asset, in the frozen order.
fn extract_dataset(
    dataset: &'static str,
    decoded: &[u8],
    cancellation: &CancellationToken,
    unrepresentable: &mut u64,
) -> Result<Vec<Extracted>> {
    let text = std::str::from_utf8(decoded).map_err(|_| {
        ProductError::integrity(
            "BENCHMARK_ASSET_DECODE_FAILED",
            "a decoded asset is not strict UTF-8",
        )
    })?;

    let mut records = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        let value = JsonParser::parse_one(line.as_bytes())?;
        let task_id = value
            .get("task_id")
            .and_then(Json::as_str)
            .ok_or_else(|| {
                ProductError::integrity(
                    "BENCHMARK_RECORD_INVALID",
                    "a benchmark record has no string task_id",
                )
            })?
            .to_owned();
        records.push((task_id, value));
    }

    // `DECONTAM-001`: sort by raw UTF-8 task_id.
    records.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));

    let mut extracted = Vec::new();
    for (task_id, value) in records {
        let mut push_text = |pointer: String,
                             role: &'static str,
                             text: &str,
                             must_stand_alone: bool|
         -> Result<()> {
            // The shared frozen normalizer, so one rule has one implementation.
            let bytes = crate::corpus::normalize_newlines(text.as_bytes());
            let verdict = parses_standalone(&bytes, cancellation)?;
            let standalone = verdict.accepted;
            if must_stand_alone && !standalone {
                return Err(ProductError::integrity(
                    "BENCHMARK_PYTHON_INVALID",
                    format!(
                        "{task_id} {pointer} does not parse as standalone Python: {}",
                        verdict.why
                    ),
                ));
            }
            extracted.push(Extracted {
                dataset,
                task_id: task_id.clone(),
                json_pointer: pointer,
                role,
                kind: if standalone {
                    crate::corpus::BenchmarkContentKind::PythonModule
                } else {
                    crate::corpus::BenchmarkContentKind::PythonFragment
                },
                bytes,
            });
            Ok(())
        };

        // The frozen extraction order.
        let prompt = value.get("prompt").and_then(Json::as_str).ok_or_else(|| {
            ProductError::integrity(
                "BENCHMARK_RECORD_INVALID",
                "a benchmark record has no string prompt",
            )
        })?;
        push_text("/prompt".to_owned(), "prompt", prompt, true)?;

        // The full solution is the prompt concatenated with the canonical solution.
        if let Some(solution) = value.get("canonical_solution").and_then(Json::as_str) {
            let full = format!("{prompt}{solution}");
            push_text(
                "/prompt||/canonical_solution".to_owned(),
                "full_solution",
                &full,
                true,
            )?;
        }
        if let Some(contract) = value.get("contract").and_then(Json::as_str) {
            push_text("/contract".to_owned(), "contract", contract, false)?;
        }
        if let Some(test) = value.get("test").and_then(Json::as_str) {
            push_text("/test".to_owned(), "test", test, false)?;
        }
        for (field, role) in [
            ("test_list", "test_list"),
            ("challenge_test_list", "challenge_test_list"),
        ] {
            if let Some(Json::Array(items)) = value.get(field) {
                for (index, item) in items.iter().enumerate() {
                    if let Some(text) = item.as_str() {
                        push_text(format!("/{field}/{index}"), role, text, false)?;
                    }
                }
            }
        }

        // `/base_input` and `/plus_input` are protected as RFC 8785 canonical
        // bytes for exact matching, not lexically.
        for field in ["base_input", "plus_input"] {
            let Some(item) = value.get(field) else {
                continue;
            };
            match strict_json::canonicalize(item) {
                Ok(bytes) => extracted.push(Extracted {
                    dataset,
                    task_id: task_id.clone(),
                    json_pointer: format!("/{field}"),
                    role: field,
                    kind: crate::corpus::BenchmarkContentKind::CanonicalJson,
                    bytes,
                }),
                Err(error) if error.code == "BENCHMARK_NUMBER_NOT_REPRESENTABLE" => {
                    // RFC 8785 has no canonical form for a literal that is not
                    // exactly an IEEE-754 double, and these payloads contain
                    // integers far beyond that. Emitting the nearest double would
                    // silently rewrite the value into something that can never
                    // match a source document, so the record is counted and
                    // skipped rather than fabricated.
                    *unrepresentable += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(extracted)
}

pub fn import_benchmark(config_path: &Path) -> Result<serde_json::Value> {
    crate::platform::require_portable_data_host()?;
    let config_bytes = crate::data::source::read_control_file(
        config_path,
        None,
        "BENCHMARK_IMPORT_CONFIG_READ_FAILED",
    )?;
    let config: BenchmarkImportConfigV1 =
        parse_closed(&config_bytes, "BENCHMARK_IMPORT_CONFIG_INVALID")?;
    config.validate()?;

    let cancellation = CancellationToken::default();
    let mut unrepresentable = 0_u64;
    let mut extracted = Vec::new();
    let mut assets = Vec::new();
    for dataset in ["humanevalplus", "mbppplus"] {
        let frozen = crate::corpus::evalplus_asset(dataset).expect("frozen dataset");
        let path = join_relative(&config.asset_root, frozen.release_asset)?;
        let decoded = decode_asset(&path, frozen.asset_sha256, frozen.decoded_sha256)?;
        extracted.extend(extract_dataset(
            dataset,
            &decoded,
            &cancellation,
            &mut unrepresentable,
        )?);
        assets.push(crate::corpus::BenchmarkAssetV1 {
            dataset: dataset.to_owned(),
            release_asset: frozen.release_asset.to_owned(),
            release_version: frozen.release_version.to_owned(),
            asset_sha256: frozen.asset_sha256.to_owned(),
            decoded_sha256: frozen.decoded_sha256.to_owned(),
        });
    }

    let mut generation = crate::acquire::PartialTree::create(&config.output_root)?;
    let mut records = Vec::with_capacity(extracted.len());
    let (mut modules, mut fragments, mut canonical) = (0_u64, 0_u64, 0_u64);
    for item in &extracted {
        match item.kind {
            crate::corpus::BenchmarkContentKind::PythonModule => modules += 1,
            crate::corpus::BenchmarkContentKind::PythonFragment => fragments += 1,
            crate::corpus::BenchmarkContentKind::CanonicalJson => canonical += 1,
        }
        // A content-addressed path keeps the tree flat and collision-free while
        // the manifest carries the human-meaningful identity.
        let identity = format!(
            "{}:{}:{}:{}",
            item.dataset, item.task_id, item.json_pointer, item.role
        );
        let relative_path = format!("records/{}.bin", sha256(identity.as_bytes()));
        generation.write(&relative_path, &item.bytes)?;
        records.push(crate::corpus::BenchmarkRecordV1 {
            dataset: item.dataset.to_owned(),
            task_id: item.task_id.clone(),
            json_pointer: item.json_pointer.clone(),
            role: item.role.to_owned(),
            content_kind: item.kind,
            relative_path,
            sha256: sha256(&item.bytes),
            bytes: item.bytes.len() as u64,
        });
    }

    let manifest = crate::corpus::BenchmarkProtectionManifestV1 {
        schema: crate::corpus::BENCHMARK_MANIFEST_SCHEMA.to_owned(),
        registry_id: crate::corpus::EVALPLUS_REGISTRY_ID.to_owned(),
        registry_commit: crate::corpus::EVALPLUS_REGISTRY_COMMIT.to_owned(),
        assets,
        records,
    };
    let manifest_bytes = compact_json_line(&manifest, "BENCHMARK_MANIFEST_SERIALIZE_FAILED")?;
    generation.write("manifest.json", &manifest_bytes)?;
    generation.publish()?;

    let result = BenchmarkImportResultV1 {
        schema: IMPORT_RESULT_SCHEMA.to_owned(),
        status: "BENCHMARK_IMPORTED".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        profile: config.profile.clone(),
        registry_id: crate::corpus::EVALPLUS_REGISTRY_ID.to_owned(),
        registry_commit: crate::corpus::EVALPLUS_REGISTRY_COMMIT.to_owned(),
        configuration_sha256: config.sha256()?,
        manifest_sha256: sha256(&manifest_bytes),
        protected_records: manifest.records.len() as u64,
        python_module_records: modules,
        python_fragment_records: fragments,
        canonical_json_records: canonical,
        skipped_unrepresentable_values: unrepresentable,
        output_created: true,
        receipts_written: false,
        limitations: vec![
            "import-is-not-benchmark-license-review".to_owned(),
            "no-python-is-executed-at-any-point".to_owned(),
        ],
    };
    serde_json::to_value(result).map_err(|_| {
        ProductError::internal(
            "BENCHMARK_IMPORT_RESULT_SERIALIZE_FAILED",
            "could not serialize the closed import result",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::normalize_newlines;

    /// What the pinned parser does with leading and trailing blank lines.
    ///
    /// `parse_python` requires the module node to span the whole input
    /// (`src/parser.rs:280-283`), and tree-sitter treats leading newlines as
    /// extras outside that span. Real benchmark prompts begin with blank lines,
    /// so this boundary decides whether the importer can take them at face value.
    #[test]
    fn the_parser_boundary_for_surrounding_blank_lines_is_explicit() {
        let cancellation = CancellationToken::default();
        let body: &[u8] = b"def f(x: int) -> int:\n    \"\"\"doc\"\"\"\n";
        assert!(
            parses_standalone(body, &cancellation).unwrap().accepted,
            "a plain function with a docstring body must parse"
        );

        // Surrounding insignificant whitespace must never decide syntax. The
        // leading cases all failed before `src/parser.rs` was corrected, which is
        // how a real benchmark prompt beginning with two blank lines exposed it.
        for (label, source) in [
            ("leading LF", b"\ndef f():\n    pass\n".to_vec()),
            ("leading two LF", b"\n\ndef f():\n    pass\n".to_vec()),
            ("leading spaces", b"   \ndef f():\n    pass\n".to_vec()),
            ("leading tab line", b"\t\ndef f():\n    pass\n".to_vec()),
            ("leading comment", b"# c\ndef f():\n    pass\n".to_vec()),
            ("trailing LF", b"def f():\n    pass\n\n".to_vec()),
            ("trailing spaces", b"def f():\n    pass\n   ".to_vec()),
            ("plain", b"def f():\n    pass\n".to_vec()),
        ] {
            let verdict = parses_standalone(&source, &cancellation).unwrap();
            assert!(verdict.accepted, "{label} must parse: {}", verdict.why);
        }

        // Genuinely broken Python still fails, so the correction did not simply
        // widen the gate.
        for (label, source) in [
            ("unclosed paren", b"def f(:\n    pass\n".to_vec()),
            ("double equals assignment", b"x = = 1\n".to_vec()),
            ("stray text", b"def f():\n    pass\n%%%\n".to_vec()),
        ] {
            let verdict = parses_standalone(&source, &cancellation).unwrap();
            assert!(!verdict.accepted, "{label} must be rejected");
        }
    }

    /// The importer and the corpus policy share one normalizer, and this is the
    /// property `DECONTAM-001` states: CRLF and CR become LF, nothing is trimmed
    /// and nothing is added.
    #[test]
    fn newline_normalization_neither_trims_nor_adds() {
        let normalize =
            |text: &str| String::from_utf8(normalize_newlines(text.as_bytes())).unwrap();
        assert_eq!(normalize("a\r\nb\rc\nd"), "a\nb\nc\nd");
        // Leading and trailing whitespace survives exactly.
        assert_eq!(normalize("  a\r\n  "), "  a\n  ");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("\r\n"), "\n");
        // A lone CR at the end is one LF, not a dropped byte.
        assert_eq!(normalize("a\r"), "a\n");
        // Byte count never grows.
        assert!(normalize_newlines(b"a\r\nb").len() <= 4);
    }
}
