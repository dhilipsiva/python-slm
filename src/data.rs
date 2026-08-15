//! Deterministic, zero-Python document source and policy boundary.

mod policy;
mod publication;
mod source;

use crate::backend::PROTOTYPE_PROFILE;
use crate::error::{ProductError, Result};
use crate::parser::{
    CancellationToken, ParserBundleBinding, PythonParserResult, parse_python, parser_bundle_binding,
};
use publication::PartialGeneration;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use source::{PreparedDocument, SelectedRemovalSnapshot};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use policy::{ByteRange, ParserFacts, ParserPolicyDecision, evaluate_parser_policy};
pub use source::{repository_group_id, source_id};

pub const IMPLEMENTATION_PHASE: &str = "P5";
pub const CURATE_CONFIG_SCHEMA: &str = "python-slm-curate-config-v1";
pub const SOURCE_MANIFEST_SCHEMA: &str = "python-slm-materialized-source-manifest-v1";
pub const REMOVAL_MANIFEST_SCHEMA: &str = "python-slm-removal-manifest-v1";
pub const GENERATION_SCHEMA: &str = "python-slm-source-generation-v2";
pub const CURATE_RESULT_SCHEMA: &str = "python-slm-curate-result-v2";
pub const ADAPTER_NAMESPACE: &str = "stack-v2-swh-materialized-v1";
pub const AUTHORIZATION_SCHEME: &str = "materialized-source-authorization-v1";
pub const LICENSE_POLICY: &str = "permissive-v1";
pub const GENERATED_POLICY: &str = "generated-v1";
pub const MIN_CANONICAL_BYTES: usize = 100;
pub const MAX_CANONICAL_BYTES: usize = 1_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CurateConfigV1 {
    pub schema: String,
    pub profile: String,
    pub source_manifest: PathBuf,
    pub content_root: PathBuf,
    pub removal_manifests: Vec<HashBoundPath>,
    pub output_root: PathBuf,
    pub limits: IngestionLimits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HashBoundPath {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IngestionLimits {
    pub maximum_documents: u64,
    pub maximum_total_raw_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedSourceManifestV1 {
    pub schema: String,
    pub adapter_namespace: String,
    pub source_snapshot_id: String,
    pub authorization: SourceAuthorization,
    pub required_removal_authorities: Vec<String>,
    pub documents: Vec<SourceDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAuthorization {
    pub scheme: String,
    pub authority_url: String,
    pub authorization_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDocument {
    pub provider_record_id: String,
    pub provider_repository_id: Option<String>,
    pub stable_provenance_origin_namespace: String,
    pub relative_path: String,
    pub expected_raw_sha256: String,
    pub expected_raw_bytes: u64,
    pub dialect: String,
    pub license_expression: String,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub origin_url: String,
    pub revision: String,
    pub source_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemovalManifestV1 {
    pub schema: String,
    pub adapter_namespace: String,
    pub authority_url: String,
    pub provider_snapshot_id: String,
    pub provider_order: u64,
    pub publication_time_utc: String,
    pub retrieval_time_utc: String,
    pub removed_provider_record_ids: Vec<String>,
    pub removed_provider_repository_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct GenerationManifest {
    schema: &'static str,
    profile: &'static str,
    adapter_namespace: String,
    source_snapshot_id: String,
    source_manifest_sha256: String,
    authorization: SourceAuthorization,
    license_policy: &'static str,
    generated_marker_policy: &'static str,
    parser_status: &'static str,
    parser_bundle: ParserBundleBinding,
    removal_snapshots: Vec<RemovalSnapshotBinding>,
    outcomes: Vec<DocumentOutcome>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct RemovalSnapshotBinding {
    authority_url: String,
    provider_snapshot_id: String,
    provider_order: u64,
    manifest_sha256: String,
    publication_time_utc: String,
    retrieval_time_utc: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DocumentOutcome {
    source_id: String,
    repository_group_id: String,
    provider_record_id: String,
    status: &'static str,
    reasons: Vec<&'static str>,
    raw_sha256: Option<String>,
    raw_bytes: u64,
    canonical_decoded_sha256: Option<String>,
    canonical_decoded_bytes: Option<u64>,
    bom_removed: bool,
    license_expression: String,
    provenance: Provenance,
    parser_result_path: Option<String>,
    parser_result_sha256: Option<String>,
    content_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct CurateResult {
    schema: &'static str,
    status: &'static str,
    qualification_status: &'static str,
    profile: &'static str,
    generation_manifest_sha256: String,
    document_count: u64,
    parser_accepted_count: u64,
    rejected_count: u64,
    parser_status: &'static str,
    output_created: bool,
    receipts_written: bool,
}

pub fn curate(config_path: &Path) -> Result<Value> {
    #[cfg(not(windows))]
    {
        let _ = config_path;
        Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "curate is implemented only for the prototype Windows host",
        ))
    }
    #[cfg(windows)]
    {
        curate_windows(config_path)
    }
}

#[cfg(windows)]
fn curate_windows(config_path: &Path) -> Result<Value> {
    let config_bytes = source::read_control_file(config_path, None, "CONFIG_READ_FAILED")?;
    let config: CurateConfigV1 = source::parse_closed(&config_bytes, "CONFIG_INVALID")?;
    source::validate_config(&config)?;

    let source_bytes =
        source::read_control_file(&config.source_manifest, None, "SOURCE_MANIFEST_READ_FAILED")?;
    let source_manifest: MaterializedSourceManifestV1 =
        source::parse_closed(&source_bytes, "SOURCE_MANIFEST_INVALID")?;
    let source_manifest_sha256 = source::canonical_source_manifest_sha256(&source_manifest)?;
    source::validate_source_manifest(&source_manifest, &config.limits)?;

    let removals = source::load_removal_snapshots(&config, &source_manifest)?;
    let prepared = source::prepare_documents(&source_manifest)?;
    let content_root = source::require_existing_root(&config.content_root, "CONTENT_ROOT_INVALID")?;
    source::require_output_boundary(&config.output_root, &content_root)?;
    let parser_bundle = parser_bundle_binding()?;
    let cancellation = CancellationToken::default();

    let mut generation = PartialGeneration::create(&config.output_root)?;
    generation.create_documents_directory()?;
    generation.create_parser_directory()?;
    let mut outcomes = Vec::with_capacity(prepared.len());
    for document in prepared {
        outcomes.push(process_document(
            &document,
            &content_root,
            &removals,
            &cancellation,
            &mut generation,
        )?);
    }

    let removal_snapshots = removals
        .values()
        .map(|snapshot| RemovalSnapshotBinding {
            authority_url: snapshot.manifest.authority_url.clone(),
            provider_snapshot_id: snapshot.manifest.provider_snapshot_id.clone(),
            provider_order: snapshot.manifest.provider_order,
            manifest_sha256: snapshot.sha256.clone(),
            publication_time_utc: snapshot.manifest.publication_time_utc.clone(),
            retrieval_time_utc: snapshot.manifest.retrieval_time_utc.clone(),
        })
        .collect::<Vec<_>>();
    let manifest = GenerationManifest {
        schema: GENERATION_SCHEMA,
        profile: PROTOTYPE_PROFILE,
        adapter_namespace: source_manifest.adapter_namespace,
        source_snapshot_id: source_manifest.source_snapshot_id,
        source_manifest_sha256,
        authorization: source_manifest.authorization,
        license_policy: LICENSE_POLICY,
        generated_marker_policy: GENERATED_POLICY,
        parser_status: "COMPLETE",
        parser_bundle,
        removal_snapshots,
        outcomes,
    };
    let manifest_bytes = source::compact_json_line(&manifest, "GENERATION_SERIALIZATION_FAILED")?;
    let generation_manifest_sha256 = source::sha256(&manifest_bytes);
    generation.write_file(Path::new("manifest.json"), &manifest_bytes)?;
    generation.publish()?;

    let parser_accepted_count = manifest
        .outcomes
        .iter()
        .filter(|outcome| outcome.status == "PARSER_ACCEPTED")
        .count() as u64;
    let rejected_count = manifest.outcomes.len() as u64 - parser_accepted_count;
    serde_json::to_value(CurateResult {
        schema: CURATE_RESULT_SCHEMA,
        status: "SOURCE_MATERIALIZED",
        qualification_status: "SKIPPED",
        profile: PROTOTYPE_PROFILE,
        generation_manifest_sha256,
        document_count: manifest.outcomes.len() as u64,
        parser_accepted_count,
        rejected_count,
        parser_status: "COMPLETE",
        output_created: true,
        receipts_written: false,
    })
    .map_err(|_| {
        ProductError::internal(
            "RESULT_SERIALIZATION_FAILED",
            "could not serialize the curate result",
        )
    })
}

fn process_document(
    prepared: &PreparedDocument,
    content_root: &Path,
    removals: &BTreeMap<String, SelectedRemovalSnapshot>,
    cancellation: &CancellationToken,
    generation: &mut PartialGeneration,
) -> Result<DocumentOutcome> {
    let document = &prepared.document;
    let path = source::join_relative(content_root, &document.relative_path)?;
    let metadata = source::require_contained_regular_file(content_root, &path)?;
    if metadata.len() != document.expected_raw_bytes {
        return Err(ProductError::integrity(
            "DOCUMENT_LENGTH_MISMATCH",
            "a document length differs from its declared immutable identity",
        ));
    }

    let removed = removals.values().any(|snapshot| {
        snapshot
            .removed_records
            .contains(&document.provider_record_id)
            || document
                .provider_repository_id
                .as_ref()
                .is_some_and(|id| snapshot.removed_repositories.contains(id))
    });
    let mut reasons = Vec::new();
    if document.dialect != "python3" {
        reasons.push("DIALECT_UNSUPPORTED");
    }
    if !policy::license_allowed(&document.license_expression) {
        reasons.push("LICENSE_NOT_ALLOWED");
    }
    if removed {
        reasons.push("REMOVED_BY_AUTHORITY");
    }

    let mut raw_sha256 = None;
    let mut decoded_sha256 = None;
    let mut decoded_bytes = None;
    let mut bom_removed = false;
    let mut canonical = None;
    if document.expected_raw_bytes > (MAX_CANONICAL_BYTES + policy::UTF8_BOM.len()) as u64 {
        reasons.push("SIZE_OUT_OF_RANGE");
    } else {
        let bytes = source::read_stable_document(&path, &metadata)?;
        let observed_hash = source::sha256(&bytes);
        if observed_hash != document.expected_raw_sha256 {
            return Err(ProductError::integrity(
                "DOCUMENT_HASH_MISMATCH",
                "a document hash differs from its declared immutable identity",
            ));
        }
        raw_sha256 = Some(observed_hash);
        match policy::decode_python(&bytes) {
            Ok(decoded) => {
                bom_removed = decoded.bom_removed;
                decoded_bytes = Some(decoded.bytes.len() as u64);
                decoded_sha256 = Some(source::sha256(&decoded.bytes));
                if !policy::canonical_size_allowed(decoded.bytes.len()) {
                    reasons.push("SIZE_OUT_OF_RANGE");
                } else {
                    canonical = Some(decoded.bytes);
                }
            }
            Err(reason) => reasons.push(reason),
        }
    }

    let mut parser_result: Option<PythonParserResult> = None;
    if reasons.is_empty() {
        let bytes = canonical.as_deref().ok_or_else(|| {
            ProductError::internal(
                "PARSER_CONTENT_MISSING",
                "a parser-eligible document has no canonical bytes",
            )
        })?;
        let mut parsed = parse_python(bytes, cancellation)?;
        if parsed.result.status == "PARSER_ACCEPTED" {
            let facts = ParserFacts {
                dialect_accepted: true,
                comment_ranges: parsed.result.comment_ranges.clone(),
            };
            match evaluate_parser_policy(bytes, &facts)? {
                ParserPolicyDecision::Accepted => parsed.result.apply_policy(true, None),
                ParserPolicyDecision::Rejected(reason) => {
                    parsed.result.apply_policy(false, Some(reason));
                }
            }
        }
        reasons.extend(parsed.result.reasons.iter().copied());
        parser_result = Some(parsed.result);
    }

    let status = if parser_result
        .as_ref()
        .is_some_and(|result| result.status == "PARSER_ACCEPTED")
    {
        "PARSER_ACCEPTED"
    } else {
        "REJECTED"
    };
    let (parser_result_path, parser_result_sha256) = if let Some(result) = &parser_result {
        let relative = format!("parser/{}.json", prepared.source_id);
        let bytes = source::compact_json_line(result, "PARSER_RESULT_SERIALIZATION_FAILED")?;
        let hash = source::sha256(&bytes);
        generation.write_file(Path::new(&relative), &bytes)?;
        (Some(relative), Some(hash))
    } else {
        (None, None)
    };
    let content_path = if status == "PARSER_ACCEPTED" {
        let relative = format!("documents/{}.py", prepared.source_id);
        let bytes = canonical.as_deref().ok_or_else(|| {
            ProductError::internal(
                "ACCEPTED_CONTENT_MISSING",
                "a parser-accepted document has no canonical bytes",
            )
        })?;
        generation.write_file(Path::new(&relative), bytes)?;
        Some(relative)
    } else {
        None
    };
    Ok(DocumentOutcome {
        source_id: prepared.source_id.clone(),
        repository_group_id: prepared.repository_group_id.clone(),
        provider_record_id: document.provider_record_id.clone(),
        status,
        reasons,
        raw_sha256,
        raw_bytes: document.expected_raw_bytes,
        canonical_decoded_sha256: decoded_sha256,
        canonical_decoded_bytes: decoded_bytes,
        bom_removed,
        license_expression: document.license_expression.clone(),
        provenance: document.provenance.clone(),
        parser_result_path,
        parser_result_sha256,
        content_path,
    })
}
