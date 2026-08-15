#![cfg(windows)]

use base64::{Engine as _, engine::general_purpose::STANDARD};
use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::data::{
    ADAPTER_NAMESPACE, AUTHORIZATION_SCHEME, CURATE_CONFIG_SCHEMA, CurateConfigV1, HashBoundPath,
    IngestionLimits, MaterializedSourceManifestV1, ParserFacts, ParserPolicyDecision, Provenance,
    REMOVAL_MANIFEST_SCHEMA, RemovalManifestV1, SOURCE_MANIFEST_SCHEMA, SourceAuthorization,
    SourceDocument, evaluate_parser_policy,
};
use rust_llm_pretrain::parser::{
    CancellationToken, PARSER_RESULT_SCHEMA, identity_manifest, parse_python,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const REMOVAL_AUTHORITY: &str = "https://example.invalid/stack-v2/removals";

#[derive(Deserialize)]
struct Compatibility {
    cases: Vec<CompatibilityCase>,
}

#[derive(Deserialize)]
struct CompatibilityCase {
    source_base64: String,
    expected_cst_sexp: String,
    comment_ranges: Vec<[usize; 2]>,
    expected_lexical_tokens: Vec<CompatibilityToken>,
}

#[derive(Deserialize)]
struct CompatibilityToken {
    kind: String,
    text_hex: String,
}

#[test]
fn locked_parser_identity_matches_package_sources() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lock = fs::read_to_string(repository.join("Cargo.lock")).unwrap();
    let manifest = identity_manifest().unwrap();
    assert_eq!(manifest.identity.packages.len(), 3);
    for package in &manifest.identity.packages {
        let record = format!(
            "name = \"{}\"\nversion = \"{}\"\n",
            package.name, package.version
        );
        let offset = lock.find(&record).unwrap();
        let tail = &lock[offset..];
        assert!(
            tail.lines()
                .take_while(|line| !line.is_empty())
                .any(|line| line == format!("checksum = \"{}\"", package.crates_io_checksum))
        );
    }
    for source in &manifest.identity.sources {
        let package_root = registry_package(&source.package);
        let bytes = fs::read(package_root.join(&source.path)).unwrap();
        assert_eq!(bytes.len() as u64, source.bytes);
        assert_eq!(sha256(&bytes), source.sha256);
    }
    let compatibility = fs::read(repository.join(&manifest.identity.compatibility_path)).unwrap();
    assert_eq!(
        sha256(&compatibility),
        manifest.identity.compatibility_sha256
    );
}

#[test]
fn frozen_compatibility_corpus_matches_parser_facts() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let compatibility: Compatibility = serde_json::from_slice(
        &fs::read(
            repository.join("docs/schemas/portable-v2/tree-sitter-python-compatibility-v1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for case in compatibility.cases {
        let source = STANDARD.decode(case.source_base64).unwrap();
        let first = parse_python(&source, &CancellationToken::default()).unwrap();
        let second = parse_python(&source, &CancellationToken::default()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.result.schema, PARSER_RESULT_SCHEMA);
        assert_eq!(first.result.status, "PARSER_ACCEPTED");
        let expected_sexp_sha = sha256(case.expected_cst_sexp.as_bytes());
        assert_eq!(
            first.result.cst_sexp_sha256.as_deref(),
            Some(expected_sexp_sha.as_str())
        );
        assert_eq!(
            first
                .result
                .comment_ranges
                .iter()
                .map(|range| [range.start, range.end])
                .collect::<Vec<_>>(),
            case.comment_ranges
        );
        assert_eq!(
            first.lexical_tokens.len(),
            case.expected_lexical_tokens.len()
        );
        for (actual, expected) in first
            .lexical_tokens
            .iter()
            .zip(case.expected_lexical_tokens)
        {
            assert_eq!(actual.kind, expected.kind);
            assert_eq!(hex::encode(&actual.text), expected.text_hex);
        }
    }
}

#[test]
fn syntax_comments_and_resource_facts_are_deterministic() {
    let source = br##"module_doc = "# text in string"
from __future__ import annotations
# standalone
def f(value: int) -> str:
    data = f"# value={value}"
    match value:
        case 1:
            return data  # inline
        case _:
            return "other"
"##;
    let parsed = parse_python(source, &CancellationToken::default()).unwrap();
    assert_eq!(parsed.result.status, "PARSER_ACCEPTED");
    assert_eq!(parsed.result.root_kind.as_deref(), Some("module"));
    assert_eq!(parsed.result.root_start_byte, Some(0));
    assert_eq!(parsed.result.root_end_byte, Some(source.len() as u64));
    assert_eq!(parsed.result.has_error, Some(false));
    assert_eq!(parsed.result.work_budget, 4_096 + 64 * source.len() as u64);
    assert_eq!(parsed.result.node_limit, 1 + 2 * source.len() as u64);
    assert_eq!(parsed.result.comment_ranges.len(), 2);
    assert!(parsed.result.maximum_depth.unwrap() <= 4_096);

    for invalid in [
        b"try:\n    pass\nexcept Exception, error:\n    pass\n".as_slice(),
        b"def broken(:\n    pass\n".as_slice(),
    ] {
        let rejected = parse_python(invalid, &CancellationToken::default()).unwrap();
        assert_eq!(rejected.result.status, "REJECTED");
        assert!(rejected.result.reasons.contains(&"PYTHON_SYNTAX_REJECTED"));
    }
}

#[test]
fn real_comment_facts_drive_generated_marker_header_boundary() {
    let inside = generated_marker_source(0);
    let outside = generated_marker_source(8_192);
    let inside = parse_python(&inside, &CancellationToken::default()).unwrap();
    let outside = parse_python(&outside, &CancellationToken::default()).unwrap();
    assert_eq!(inside.result.status, "PARSER_ACCEPTED");
    assert_eq!(outside.result.status, "PARSER_ACCEPTED");
    assert_eq!(inside.result.comment_ranges.len(), 1);
    assert_eq!(outside.result.comment_ranges.len(), 1);
    assert_eq!(
        evaluate_parser_policy(
            &generated_marker_source(0),
            &ParserFacts {
                dialect_accepted: true,
                comment_ranges: inside.result.comment_ranges,
            },
        )
        .unwrap(),
        ParserPolicyDecision::Rejected("GENERATED_MARKER")
    );
    assert_eq!(
        evaluate_parser_policy(
            &generated_marker_source(8_192),
            &ParserFacts {
                dialect_accepted: true,
                comment_ranges: outside.result.comment_ranges,
            },
        )
        .unwrap(),
        ParserPolicyDecision::Accepted
    );
}

#[test]
fn cancellation_is_fatal_and_does_not_leak_between_documents() {
    let cancelled = CancellationToken::default();
    cancelled.cancel();
    let error = parse_python(b"value = 1\n", &cancelled).unwrap_err();
    assert_eq!(error.code, "PARSER_CANCELLED");

    let fresh = parse_python(b"value = 1\n", &CancellationToken::default()).unwrap();
    assert_eq!(fresh.result.status, "PARSER_ACCEPTED");
}

#[test]
fn curate_publishes_only_parser_accepted_sources_and_closed_facts() {
    let root = tempfile::tempdir().unwrap();
    let content_root = root.path().join("content");
    fs::create_dir(&content_root).unwrap();
    let accepted = padded(b"# ordinary comment\nvalue = 1\n".to_vec());
    let rejected = padded(b"def broken(:\n    return 1\n".to_vec());
    fs::write(content_root.join("accepted.py"), &accepted).unwrap();
    fs::write(content_root.join("rejected.py"), &rejected).unwrap();

    let source_manifest_path = root.path().join("source.json");
    let source_manifest = MaterializedSourceManifestV1 {
        schema: SOURCE_MANIFEST_SCHEMA.to_owned(),
        adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
        source_snapshot_id: "snapshot-p5-1".to_owned(),
        authorization: SourceAuthorization {
            scheme: AUTHORIZATION_SCHEME.to_owned(),
            authority_url: "https://huggingface.co/datasets/bigcode/the-stack-v2".to_owned(),
            authorization_id: "owner-authorized-p5".to_owned(),
        },
        required_removal_authorities: vec![REMOVAL_AUTHORITY.to_owned()],
        documents: vec![
            source_document("record-accepted", "accepted.py", &accepted),
            source_document("record-rejected", "rejected.py", &rejected),
        ],
    };
    write_json(&source_manifest_path, &source_manifest);
    let removal_path = root.path().join("removal.json");
    let removal_bytes = write_json(
        &removal_path,
        &RemovalManifestV1 {
            schema: REMOVAL_MANIFEST_SCHEMA.to_owned(),
            adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
            authority_url: REMOVAL_AUTHORITY.to_owned(),
            provider_snapshot_id: "removals-p5-1".to_owned(),
            provider_order: 1,
            publication_time_utc: "2026-08-15T00:00:00Z".to_owned(),
            retrieval_time_utc: "2026-08-15T00:01:00Z".to_owned(),
            removed_provider_record_ids: vec![],
            removed_provider_repository_ids: vec![],
        },
    );
    let output = root.path().join("generation");
    let config_path = root.path().join("config.json");
    write_json(
        &config_path,
        &CurateConfigV1 {
            schema: CURATE_CONFIG_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            source_manifest: source_manifest_path,
            content_root,
            removal_manifests: vec![HashBoundPath {
                path: removal_path,
                sha256: sha256(&removal_bytes),
            }],
            output_root: output.clone(),
            limits: IngestionLimits {
                maximum_documents: 2,
                maximum_total_raw_bytes: 1_000_000,
            },
        },
    );

    let result = Command::new(env!("CARGO_BIN_EXE_python-slm"))
        .args(["curate", "--config"])
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty());
    let result_json: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result_json["schema"], "python-slm-curate-result-v3");
    assert_eq!(result_json["parser_status"], "COMPLETE");
    assert_eq!(result_json["parser_accepted_count"], 1);
    assert_eq!(result_json["policy_accepted_count"], 1);
    assert_eq!(result_json["quarantined_count"], 0);
    assert_eq!(result_json["rejected_count"], 1);

    let manifest_bytes = fs::read(output.join("manifest.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["schema"], "python-slm-source-generation-v3");
    assert_eq!(manifest["parser_status"], "COMPLETE");
    assert_eq!(manifest["policy_status"], "COMPLETE");
    assert_eq!(fs::read_dir(output.join("documents")).unwrap().count(), 1);
    assert_eq!(fs::read_dir(output.join("parser")).unwrap().count(), 2);
    assert_eq!(fs::read_dir(output.join("policy")).unwrap().count(), 1);
    assert!(
        !String::from_utf8(manifest_bytes)
            .unwrap()
            .contains(&root.path().display().to_string())
    );
    assert!(!output.join("receipts").exists());
}

fn generated_marker_source(marker_start: usize) -> Vec<u8> {
    let mut source = vec![b'x'; marker_start];
    source.extend_from_slice(b"# generated by tool\n");
    source
}

fn registry_package(package: &str) -> PathBuf {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("USERPROFILE").unwrap()).join(".cargo"));
    for index in fs::read_dir(cargo_home.join("registry/src")).unwrap() {
        let candidate = index.unwrap().path().join(package);
        if candidate.is_dir() {
            return candidate;
        }
    }
    panic!("locked package source not found: {package}");
}

fn source_document(record_id: &str, relative_path: &str, bytes: &[u8]) -> SourceDocument {
    SourceDocument {
        provider_record_id: record_id.to_owned(),
        provider_repository_id: Some("repository-p5".to_owned()),
        stable_provenance_origin_namespace: String::new(),
        relative_path: relative_path.to_owned(),
        expected_raw_sha256: sha256(bytes),
        expected_raw_bytes: bytes.len() as u64,
        dialect: "python3".to_owned(),
        license_expression: "MIT".to_owned(),
        provenance: Provenance {
            origin_url: format!("https://archive.softwareheritage.org/{record_id}"),
            revision: format!("revision-{record_id}"),
            source_path: relative_path.to_owned(),
        },
    }
}

fn padded(mut bytes: Vec<u8>) -> Vec<u8> {
    while bytes.len() < 120 {
        bytes.extend_from_slice(b"value = 1\n");
    }
    bytes
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    fs::write(path, &bytes).unwrap();
    bytes
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
