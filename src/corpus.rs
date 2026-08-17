//! Deterministic P9A deduplication, decontamination, splitting, and span ordering.

use crate::backend::PROTOTYPE_PROFILE;
use crate::data::Provenance;
use crate::data::source::{
    compact_json_line, is_sha256, join_relative, parse_closed, read_control_file,
    read_stable_document, require_contained_regular_file, require_existing_root,
    require_output_boundary, require_portable_relative_path, sha256,
};
use crate::error::{ProductError, Result};
use crate::parser::{CancellationToken, LexicalToken, parse_python};
use crate::storage::{
    CorpusSplit, GovernedCorpusDocumentV1, TokenCorpusGenerationV1, VerifiedTokenCorpus,
};
use crate::tokenizer::{HashBoundInput, TokenizerSampleDocumentV1};
use rand_chacha::ChaCha12Rng;
use rand_core::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

pub const IMPLEMENTATION_PHASE: &str = "P9A";
pub const PREPARE_CONFIG_SCHEMA: &str = "python-slm-corpus-policy-config-v1";
pub const BENCHMARK_MANIFEST_SCHEMA: &str = "python-slm-benchmark-protection-manifest-v1";
pub const GENERATION_SCHEMA: &str = "python-slm-corpus-policy-generation-v1";
pub const GOVERNED_CORPUS_SCHEMA: &str = "python-slm-governed-corpus-manifest-v2";
pub const GOVERNED_CORPUS_INDEX_SCHEMA: &str = "python-slm-governed-corpus-manifest-v3";
pub const GOVERNED_CORPUS_PART_SCHEMA: &str = "python-slm-governed-corpus-part-v1";

/// Documents per emitted manifest part.
///
/// The governed corpus and tokenizer sample manifests are read back through the
/// 64 MiB control-file bound (`src/data/source/io.rs:8`), which at their measured
/// `561.3` and `463.2` bytes per document capped them near 120,000 and 145,000
/// documents — far short of a production corpus. Unlike the source generations
/// these are emitted rather than supplied, so a list in the configuration cannot
/// help; they are emitted in parts instead, each independently hash-bound and
/// each comfortably inside the bound.
pub(crate) const MANIFEST_PART_DOCUMENTS: usize = 50_000;

/// A part that would not read back is refused rather than published, so the
/// document-count shard size can never silently produce an unreadable artifact.
const MAXIMUM_PART_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestPartRef {
    pub relative_path: String,
    pub sha256: String,
    pub documents: u64,
}

pub(crate) fn require_part_fits(bytes: usize, code: &'static str) -> Result<()> {
    if bytes > MAXIMUM_PART_BYTES {
        return Err(ProductError::integrity(
            code,
            "an emitted manifest part exceeds the readable control-file bound",
        ));
    }
    Ok(())
}
pub const DEDUP_MANIFEST_SCHEMA: &str = "python-slm-dedup-manifest-v1";
pub const DECONTAMINATION_MANIFEST_SCHEMA: &str = "python-slm-decontamination-manifest-v1";
pub const SPLIT_MANIFEST_SCHEMA: &str = "python-slm-split-manifest-v1";
pub const PREPARE_RESULT_SCHEMA: &str = "python-slm-prepare-corpus-result-v1";
pub const SPAN_CONFIG_SCHEMA: &str = "python-slm-span-order-config-v1";
pub const SPAN_MANIFEST_SCHEMA: &str = "python-slm-span-order-manifest-v1";
pub const SPAN_RESULT_SCHEMA: &str = "python-slm-plan-spans-result-v1";

const SOURCE_GENERATION_SCHEMA: &str = "python-slm-source-generation-v4";
pub(crate) const EVALPLUS_REGISTRY_ID: &str = "evalplus-v0.3.1";
pub(crate) const EVALPLUS_REGISTRY_COMMIT: &str = "e5d0ed0bab96280b60b637ec7f15b5e4841b0cb2";

/// The frozen identity of each `DECONTAM-001` asset: release file name, release
/// version, the gzip asset's SHA-256, and the SHA-256 of its decoded bytes.
///
/// These digests were missing, and their absence was a real hole rather than an
/// oversight in degree. `validate_benchmark_manifest` checked only that the two
/// hashes were *shaped* like SHA-256 and never compared them to anything, so a
/// hand-written manifest carrying plausible hex and a single trivial record
/// passed every gate and produced a decontamination manifest that certified
/// almost nothing. Binding them here is what makes the benchmark path mean what
/// it claims.
///
/// Obtained 2026-08-17 by `fetch --discover` against the pinned release assets
/// and cross-checked three ways. `HumanEvalPlus.jsonl.gz` is 925,932 bytes
/// compressed and 7,714,666 decoded across 164 JSONL records;
/// `MbppPlus.jsonl.gz` is 336,032 and 2,592,369 across 378. The compressed counts
/// match the GitHub release metadata exactly, and 164 and 378 are the canonical
/// HumanEval+ and MBPP+ task counts â€” which is the cheapest independent signal
/// that these are the full assets rather than the `-Mini`, `-NoExtreme`, or
/// `-OriginFmt` variants published in the same releases. `DECONTAM-001` specifies
/// the full assets.
///
/// Only the identities this validator actually checks are carried in code. The
/// byte and task counts stay in this comment until the importer that consumes
/// them exists, rather than sitting in the struct unread.
const EVALPLUS_ASSETS: [EvalPlusAsset; 2] = [
    EvalPlusAsset {
        dataset: "humanevalplus",
        release_asset: "HumanEvalPlus.jsonl.gz",
        release_version: "v0.1.10",
        asset_sha256: "272720b90ac375502c8ed23cd791c2a93dfb22a911641a494da74a426c09f101",
        decoded_sha256: "42526ec0e7d5f3ee0b06d6ced98f8c8bae3d76519151bfb3d36f79010645bd7f",
    },
    EvalPlusAsset {
        dataset: "mbppplus",
        release_asset: "MbppPlus.jsonl.gz",
        release_version: "v0.2.0",
        asset_sha256: "af43697e8791c4c149bdfd6b489d8b5412507551ac20e28a439f650b8225db63",
        decoded_sha256: "b54e762755248ca411b523c917fa9f93c07b5ff2966bf60b3917b853926a3dad",
    },
];

pub(crate) struct EvalPlusAsset {
    pub dataset: &'static str,
    pub release_asset: &'static str,
    pub release_version: &'static str,
    pub asset_sha256: &'static str,
    pub decoded_sha256: &'static str,
}

pub(crate) fn evalplus_asset(dataset: &str) -> Option<&'static EvalPlusAsset> {
    EVALPLUS_ASSETS
        .iter()
        .find(|asset| asset.dataset == dataset)
}
const SHINGLE_BASE_DOMAIN: &[u8] = b"python-slm/shingle-base/v1\0";
const COEFFICIENT_DOMAIN: &[u8] = b"python-slm/minhash-coeff/v1\0";
const LSH_DOMAIN: &[u8] = b"python-slm/lsh-band/v1\0";
const SHORT_DOMAIN: &[u8] = b"short/v1\0";
const COMPONENT_DOMAIN: &[u8] = b"python-slm/component/v1\0";
const SPLIT_DOMAIN: &[u8] = b"python-slm/split/v1\0";
const SPAN_DOMAIN: &[u8] = b"python-slm/span-order/v1\0";
const PRIME: u64 = (1_u64 << 61) - 1;
const MINHASH_COMPONENTS: usize = 256;
const LSH_BANDS: usize = 32;
const LSH_ROWS: usize = 8;
const PROTECTED_SPAN_TOKENS: usize = 50;
const TARGET_SPAN: u64 = 2_048;
static PARTIAL_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusPolicyConfigV1 {
    pub schema: String,
    pub profile: String,
    /// One entry per P4 generation, in the operator's order.
    ///
    /// A single generation cannot reach production scale: `prepare-corpus` reads
    /// the whole generation manifest through the 64 MiB control-file bound
    /// (`src/data/source/io.rs:8`), and an accepted outcome carrying its
    /// `governed_source_metadata` block runs to roughly 2.4 KB, which caps one
    /// manifest near 28,000 documents against the 1.5 to 3 million the frozen
    /// target needs. Composing many generations keeps each one independently
    /// verifiable and under that bound rather than raising it.
    pub source_generations: Vec<SourceGenerationInput>,
    pub benchmark_manifest: HashBoundInput,
    pub benchmark_content_root: PathBuf,
    pub output_root: PathBuf,
    pub limits: CorpusPolicyLimits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceGenerationInput {
    pub manifest: HashBoundInput,
    /// The generation's own root; `content_path` values are relative to it, so
    /// each generation keeps its own and they are never interchangeable.
    pub content_root: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusPolicyLimits {
    pub maximum_documents: u64,
    pub maximum_total_canonical_bytes: u64,
    pub maximum_total_shingles: u64,
    pub maximum_candidate_pairs: u64,
    pub maximum_benchmark_records: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkProtectionManifestV1 {
    pub schema: String,
    pub registry_id: String,
    pub registry_commit: String,
    pub assets: Vec<BenchmarkAssetV1>,
    pub records: Vec<BenchmarkRecordV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkAssetV1 {
    pub dataset: String,
    pub release_asset: String,
    pub release_version: String,
    pub asset_sha256: String,
    pub decoded_sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkContentKind {
    PythonModule,
    PythonFragment,
    CanonicalJson,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkRecordV1 {
    pub dataset: String,
    pub task_id: String,
    pub json_pointer: String,
    pub role: String,
    pub content_kind: BenchmarkContentKind,
    pub relative_path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceGenerationV4 {
    schema: String,
    profile: String,
    adapter_namespace: String,
    source_snapshot_id: String,
    source_manifest_sha256: String,
    authorization: Value,
    license_policy: String,
    generated_marker_policy: String,
    parser_status: String,
    parser_bundle: Value,
    policy_status: String,
    sensitive_policy: Value,
    governed_source_policy: Value,
    governance_status: String,
    removal_snapshots: Vec<Value>,
    outcomes: Vec<SourceOutcomeV4>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceOutcomeV4 {
    source_id: String,
    repository_group_id: String,
    provider_record_id: String,
    status: String,
    reasons: Vec<String>,
    raw_sha256: Option<String>,
    raw_bytes: u64,
    canonical_decoded_sha256: Option<String>,
    canonical_decoded_bytes: Option<u64>,
    bom_removed: bool,
    license_expression: String,
    provenance: Provenance,
    parser_result_path: Option<String>,
    parser_result_sha256: Option<String>,
    policy_result_path: Option<String>,
    policy_result_sha256: Option<String>,
    content_path: Option<String>,
    governed_source_metadata: Value,
}

#[derive(Clone, Debug)]
struct PreparedDocument {
    source_id: String,
    repository_group_id: String,
    curated_sha256_raw: String,
    canonical_sha256: String,
    canonical_bytes: u64,
    content: Vec<u8>,
    provenance: Provenance,
    comment_bytes: u64,
    /// The encoded form is a one-to-one map of the lexical tokens, so it carries
    /// the token count too. The lexical tokens themselves are deliberately not
    /// retained: every later use needed only their number, and holding a `String`
    /// kind and a `Vec<u8>` text per token — two heap allocations each, across
    /// millions of tokens — was the single largest resident cost in this stage.
    encoded_tokens: Vec<Vec<u8>>,
    /// The MinHash signature is retained; the shingle set it was built from is
    /// not. A shingle concatenates five encoded tokens, so the set costs roughly
    /// five times the token bytes and dominated this stage's residency, yet it is
    /// wholly derived from `encoded_tokens` and is consulted only for the exact
    /// Jaccard of an LSH candidate pair. Rebuilding it for those few pairs costs
    /// far less than holding it for every document.
    signature: [u64; MINHASH_COMPONENTS],
}

#[derive(Clone, Debug)]
struct ProtectedBenchmark {
    identity: String,
    kind: BenchmarkContentKind,
    content: Vec<u8>,
    canonical_sha256: String,
    encoded_tokens: Vec<Vec<u8>>,
    shingles: BTreeSet<Vec<u8>>,
    signature: Option<[u64; MINHASH_COMPONENTS]>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedCorpusManifestV2 {
    pub schema: String,
    pub source_generation_manifest_sha256: String,
    pub split_manifest_sha256: String,
    pub tokenizer_sample_manifest_sha256: String,
    pub documents: Vec<GovernedCorpusDocumentV1>,
}

/// The sharded form: the same header, with the documents in hash-bound parts
/// beside it rather than inline. The file keeps its name and its own digest, so
/// every configuration and every downstream binding equality is unchanged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedCorpusManifestV3 {
    pub schema: String,
    pub source_generation_manifest_sha256: String,
    pub split_manifest_sha256: String,
    pub tokenizer_sample_manifest_sha256: String,
    pub parts: Vec<ManifestPartRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedCorpusPartV1 {
    pub schema: String,
    pub part: u64,
    pub documents: Vec<GovernedCorpusDocumentV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DedupManifestV1 {
    pub schema: String,
    pub lexical_policy: String,
    pub exact_hash_algorithm: String,
    pub minhash_components: u64,
    pub lsh_bands: u64,
    pub lsh_rows: u64,
    pub jaccard_threshold: String,
    pub input_documents: u64,
    pub candidate_pairs: u64,
    pub exact_duplicate_edges: u64,
    pub near_duplicate_edges: u64,
    pub clusters: Vec<DuplicateClusterV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DuplicateClusterV1 {
    pub cluster_id: String,
    pub representative_source_id: String,
    pub member_source_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecontaminationManifestV1 {
    pub schema: String,
    pub benchmark_manifest_sha256: String,
    pub registry_id: String,
    pub registry_commit: String,
    pub protected_records: u64,
    pub rejected_clusters: u64,
    pub rejected_documents: Vec<DecontaminationOutcomeV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecontaminationOutcomeV1 {
    pub source_id: String,
    pub benchmark_identities: Vec<String>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SplitManifestV1 {
    pub schema: String,
    pub algorithm: String,
    pub components: Vec<SplitComponentV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SplitComponentV1 {
    pub component_id: String,
    pub member_source_ids: Vec<String>,
    pub representative_source_ids: Vec<String>,
    pub bucket: u64,
    pub split: CorpusSplit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactBinding {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusPolicyGenerationV1 {
    pub schema: String,
    pub profile: String,
    pub qualification_status: String,
    pub source_generation_manifest_sha256: String,
    pub benchmark_manifest_sha256: String,
    pub dedup_manifest: ArtifactBinding,
    pub decontamination_manifest: ArtifactBinding,
    pub split_manifest: ArtifactBinding,
    pub tokenizer_sample_manifest: ArtifactBinding,
    pub governed_corpus_manifest: ArtifactBinding,
    pub representative_documents: u64,
    pub excluded_documents: u64,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PrepareResult {
    schema: &'static str,
    status: &'static str,
    qualification_status: &'static str,
    profile: &'static str,
    generation_manifest_sha256: String,
    representative_documents: u64,
    excluded_documents: u64,
    output_created: bool,
    receipts_written: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpanOrderConfigV1 {
    pub schema: String,
    pub profile: String,
    pub token_corpus_root: PathBuf,
    pub token_corpus_manifest_sha256: String,
    pub decision_ledger_path: PathBuf,
    pub decision_ledger_sha256: String,
    pub output_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpanOrderManifestV1 {
    pub schema: String,
    pub profile: String,
    pub qualification_status: String,
    pub algorithm: String,
    pub rng: String,
    pub contract_decisions_sha256: String,
    pub corpus_manifest_sha256: String,
    pub seed_sha256: String,
    pub target_span: u64,
    pub training_prefix_ids: u64,
    pub valid_targets: u64,
    pub complete_span_count: u64,
    pub partial_span: Option<PartialSpanV1>,
    pub ordered_spans: Vec<SpanDescriptorV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialSpanV1 {
    pub first_target_offset: u64,
    pub valid_targets: u64,
    pub runtime_padding_targets: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpanDescriptorV1 {
    pub order: u64,
    pub first_target_offset: u64,
    pub valid_targets: u64,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct SpanResult {
    schema: &'static str,
    status: &'static str,
    qualification_status: &'static str,
    profile: &'static str,
    span_manifest_sha256: String,
    complete_span_count: u64,
    partial_span_targets: u64,
    output_created: bool,
    receipts_written: bool,
}

pub fn prepare(config_path: &Path) -> Result<Value> {
    crate::platform::require_portable_data_host()?;
    prepare_portable(config_path)
}

pub fn plan_spans(config_path: &Path) -> Result<Value> {
    crate::platform::require_portable_data_host()?;
    plan_spans_portable(config_path)
}

fn prepare_portable(config_path: &Path) -> Result<Value> {
    let config_bytes = read_control_file(config_path, None, "CORPUS_CONFIG_READ_FAILED")?;
    let config: CorpusPolicyConfigV1 = parse_closed(&config_bytes, "CORPUS_CONFIG_INVALID")?;
    validate_prepare_config(&config)?;
    let mut sources = Vec::with_capacity(config.source_generations.len());
    for generation in &config.source_generations {
        let source_bytes = read_control_file(
            &generation.manifest.path,
            Some(&generation.manifest.sha256),
            "SOURCE_GENERATION_READ_FAILED",
        )?;
        let source: SourceGenerationV4 = parse_closed(&source_bytes, "SOURCE_GENERATION_INVALID")?;
        validate_source_generation(&source)?;
        let root = require_existing_root(&generation.content_root, "SOURCE_CONTENT_ROOT_INVALID")?;
        sources.push((source, root));
    }
    let source_generation_sha256 = combined_source_generation_sha256(&config.source_generations);
    let benchmark_bytes = read_control_file(
        &config.benchmark_manifest.path,
        Some(&config.benchmark_manifest.sha256),
        "BENCHMARK_MANIFEST_READ_FAILED",
    )?;
    let benchmark: BenchmarkProtectionManifestV1 =
        parse_closed(&benchmark_bytes, "BENCHMARK_MANIFEST_INVALID")?;
    validate_benchmark_manifest(&benchmark, &config)?;
    let benchmark_root =
        require_existing_root(&config.benchmark_content_root, "BENCHMARK_ROOT_INVALID")?;
    for (_, root) in &sources {
        require_output_boundary(&config.output_root, root)?;
    }
    require_output_boundary(&config.output_root, &benchmark_root)?;

    let cancellation = CancellationToken::default();
    let documents = load_documents(&sources, &config, &cancellation)?;
    let protected = load_benchmark(&benchmark, &benchmark_root, &cancellation)?;
    let (dedup, duplicate_groups) = deduplicate(&documents, config.limits.maximum_candidate_pairs)?;
    let (decontamination, rejected_clusters) = decontaminate(
        &documents,
        &duplicate_groups,
        &protected,
        &config.benchmark_manifest.sha256,
    )?;
    let (split_manifest, assignments) =
        assign_splits(&documents, &duplicate_groups, &rejected_clusters)?;

    let mut generation = PartialCorpusGeneration::create(&config.output_root)?;
    generation.create_directory("documents")?;
    let mut governed_documents = Vec::new();
    let mut sample_documents = Vec::new();
    for assignment in &assignments {
        let document = &documents[assignment.document_index];
        let relative = format!("documents/{}.py", document.source_id);
        generation.write_file(Path::new(&relative), &document.content)?;
        governed_documents.push(GovernedCorpusDocumentV1 {
            component_id: assignment.component_id.clone(),
            repository_group_id: document.repository_group_id.clone(),
            source_id: document.source_id.clone(),
            curated_sha256_raw: document.curated_sha256_raw.clone(),
            canonical_sha256: document.canonical_sha256.clone(),
            canonical_bytes: document.canonical_bytes,
            relative_path: relative.clone(),
            split: assignment.split,
        });
        if assignment.split == CorpusSplit::Train {
            sample_documents.push(TokenizerSampleDocumentV1 {
                repository_group_id: document.repository_group_id.clone(),
                source_id: document.source_id.clone(),
                curated_sha256_raw: document.curated_sha256_raw.clone(),
                canonical_sha256: document.canonical_sha256.clone(),
                canonical_bytes: document.canonical_bytes,
                relative_path: relative,
            });
        }
    }
    sample_documents.sort_by(sample_document_identity_order);
    let mut sample_parts = Vec::new();
    for (index, chunk) in sample_documents.chunks(MANIFEST_PART_DOCUMENTS).enumerate() {
        let part = crate::tokenizer::TokenizerSamplePartV1 {
            schema: crate::tokenizer::SAMPLE_PART_SCHEMA.to_owned(),
            part: index as u64,
            documents: chunk.to_vec(),
        };
        let bytes = compact_json_line(&part, "TOKENIZER_SAMPLE_SERIALIZATION_FAILED")?;
        require_part_fits(bytes.len(), "TOKENIZER_SAMPLE_SERIALIZATION_FAILED")?;
        let relative_path = format!("tokenizer-sample-part-{index:05}.json");
        generation.write_file(Path::new(&relative_path), &bytes)?;
        sample_parts.push(ManifestPartRef {
            relative_path,
            sha256: sha256(&bytes),
            documents: chunk.len() as u64,
        });
    }
    let tokenizer_sample = crate::tokenizer::TokenizerSampleManifestV2 {
        schema: crate::tokenizer::SAMPLE_MANIFEST_INDEX_SCHEMA.to_owned(),
        source_generation_manifest_sha256: source_generation_sha256.clone(),
        parts: sample_parts,
    };
    let sample_bytes =
        compact_json_line(&tokenizer_sample, "TOKENIZER_SAMPLE_SERIALIZATION_FAILED")?;
    let sample_sha256 = sha256(&sample_bytes);
    let dedup_bytes = compact_json_line(&dedup, "DEDUP_MANIFEST_SERIALIZATION_FAILED")?;
    let decontamination_bytes = compact_json_line(
        &decontamination,
        "DECONTAMINATION_MANIFEST_SERIALIZATION_FAILED",
    )?;
    let split_bytes = compact_json_line(&split_manifest, "SPLIT_MANIFEST_SERIALIZATION_FAILED")?;
    let split_sha256 = sha256(&split_bytes);
    governed_documents.sort_by(governed_document_order);
    let mut governed_parts = Vec::new();
    for (index, chunk) in governed_documents
        .chunks(MANIFEST_PART_DOCUMENTS)
        .enumerate()
    {
        let part = GovernedCorpusPartV1 {
            schema: GOVERNED_CORPUS_PART_SCHEMA.to_owned(),
            part: index as u64,
            documents: chunk.to_vec(),
        };
        let bytes = compact_json_line(&part, "GOVERNED_CORPUS_SERIALIZATION_FAILED")?;
        require_part_fits(bytes.len(), "GOVERNED_CORPUS_SERIALIZATION_FAILED")?;
        let relative_path = format!("governed-corpus-part-{index:05}.json");
        generation.write_file(Path::new(&relative_path), &bytes)?;
        governed_parts.push(ManifestPartRef {
            relative_path,
            sha256: sha256(&bytes),
            documents: chunk.len() as u64,
        });
    }
    let governed = GovernedCorpusManifestV3 {
        schema: GOVERNED_CORPUS_INDEX_SCHEMA.to_owned(),
        source_generation_manifest_sha256: source_generation_sha256.clone(),
        split_manifest_sha256: split_sha256,
        tokenizer_sample_manifest_sha256: sample_sha256,
        parts: governed_parts,
    };
    let governed_bytes = compact_json_line(&governed, "GOVERNED_CORPUS_SERIALIZATION_FAILED")?;
    for (path, bytes) in [
        ("dedup-manifest.json", dedup_bytes.as_slice()),
        (
            "decontamination-manifest.json",
            decontamination_bytes.as_slice(),
        ),
        ("split-manifest.json", split_bytes.as_slice()),
        ("tokenizer-sample-manifest.json", sample_bytes.as_slice()),
        ("governed-corpus-manifest.json", governed_bytes.as_slice()),
    ] {
        generation.write_file(Path::new(path), bytes)?;
    }
    let excluded_documents = rejected_clusters.iter().try_fold(0_u64, |total, cluster| {
        total
            .checked_add(duplicate_groups[*cluster].len() as u64)
            .ok_or_else(accounting_overflow)
    })?;
    let manifest = CorpusPolicyGenerationV1 {
        schema: GENERATION_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        source_generation_manifest_sha256: source_generation_sha256,
        benchmark_manifest_sha256: config.benchmark_manifest.sha256,
        dedup_manifest: artifact_binding("dedup-manifest.json", &dedup_bytes),
        decontamination_manifest: artifact_binding(
            "decontamination-manifest.json",
            &decontamination_bytes,
        ),
        split_manifest: artifact_binding("split-manifest.json", &split_bytes),
        tokenizer_sample_manifest: artifact_binding(
            "tokenizer-sample-manifest.json",
            &sample_bytes,
        ),
        governed_corpus_manifest: artifact_binding(
            "governed-corpus-manifest.json",
            &governed_bytes,
        ),
        representative_documents: assignments.len() as u64,
        excluded_documents,
    };
    let manifest_bytes = compact_json_line(&manifest, "CORPUS_GENERATION_SERIALIZATION_FAILED")?;
    let generation_manifest_sha256 = sha256(&manifest_bytes);
    generation.write_file(Path::new("manifest.json"), &manifest_bytes)?;
    generation.publish()?;
    serde_json::to_value(PrepareResult {
        schema: PREPARE_RESULT_SCHEMA,
        status: "CORPUS_POLICY_MATERIALIZED",
        qualification_status: "SKIPPED",
        profile: PROTOTYPE_PROFILE,
        generation_manifest_sha256,
        representative_documents: manifest.representative_documents,
        excluded_documents,
        output_created: true,
        receipts_written: false,
    })
    .map_err(|_| {
        ProductError::internal(
            "RESULT_SERIALIZATION_FAILED",
            "could not serialize the corpus policy result",
        )
    })
}
fn validate_prepare_config(config: &CorpusPolicyConfigV1) -> Result<()> {
    if config.schema != PREPARE_CONFIG_SCHEMA {
        return Err(ProductError::usage(
            "CONFIG_SCHEMA_UNSUPPORTED",
            "the corpus policy configuration schema is unsupported",
        ));
    }
    if config.profile != PROTOTYPE_PROFILE {
        return Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "the requested profile is designed but not implemented",
        ));
    }
    if config.source_generations.is_empty() {
        return Err(ProductError::usage(
            "CONFIG_INPUT_INVALID",
            "at least one source generation is required",
        ));
    }
    let mut manifests = BTreeSet::new();
    let mut roots = BTreeSet::new();
    for generation in &config.source_generations {
        if !generation.manifest.path.is_absolute()
            || !is_sha256(&generation.manifest.sha256)
            || !generation.content_root.is_absolute()
        {
            return Err(ProductError::usage(
                "CONFIG_INPUT_INVALID",
                "hash-bound corpus inputs require absolute paths and lowercase SHA-256",
            ));
        }
        // Naming one generation twice would double every document it holds, and
        // the duplicate would be indistinguishable from a genuine one downstream.
        if !manifests.insert(generation.manifest.path.clone())
            || !roots.insert(generation.content_root.clone())
        {
            return Err(ProductError::usage(
                "CONFIG_SOURCE_GENERATION_DUPLICATE",
                "a source generation manifest or content root is named twice",
            ));
        }
    }
    if !config.benchmark_manifest.path.is_absolute()
        || !is_sha256(&config.benchmark_manifest.sha256)
    {
        return Err(ProductError::usage(
            "CONFIG_INPUT_INVALID",
            "hash-bound corpus inputs require absolute paths and lowercase SHA-256",
        ));
    }
    if !config.benchmark_content_root.is_absolute()
        || !config.output_root.is_absolute()
        || config.limits.maximum_documents == 0
        || config.limits.maximum_total_canonical_bytes == 0
        || config.limits.maximum_total_shingles == 0
        || config.limits.maximum_candidate_pairs == 0
        || config.limits.maximum_benchmark_records == 0
    {
        return Err(ProductError::usage(
            "CONFIG_LIMITS_INVALID",
            "corpus roots must be absolute and every explicit capacity must be positive",
        ));
    }
    Ok(())
}

fn validate_source_generation(source: &SourceGenerationV4) -> Result<()> {
    if source.schema != SOURCE_GENERATION_SCHEMA
        || source.profile != PROTOTYPE_PROFILE
        || source.parser_status != "COMPLETE"
        || source.policy_status != "COMPLETE"
        || !is_sha256(&source.source_manifest_sha256)
    {
        return Err(ProductError::integrity(
            "SOURCE_GENERATION_INVALID",
            "the source generation is not a complete supported v4 generation",
        ));
    }
    let _ = (
        &source.adapter_namespace,
        &source.source_snapshot_id,
        &source.authorization,
        &source.license_policy,
        &source.generated_marker_policy,
        &source.parser_bundle,
        &source.sensitive_policy,
        &source.governed_source_policy,
        &source.governance_status,
        &source.removal_snapshots,
    );
    Ok(())
}

fn validate_benchmark_manifest(
    benchmark: &BenchmarkProtectionManifestV1,
    config: &CorpusPolicyConfigV1,
) -> Result<()> {
    if benchmark.schema != BENCHMARK_MANIFEST_SCHEMA
        || benchmark.registry_id != EVALPLUS_REGISTRY_ID
        || benchmark.registry_commit != EVALPLUS_REGISTRY_COMMIT
        || benchmark.assets.len() != 2
        || benchmark.records.is_empty()
        || benchmark.records.len() as u64 > config.limits.maximum_benchmark_records
    {
        return Err(ProductError::integrity(
            "BENCHMARK_MANIFEST_INVALID",
            "the benchmark manifest does not bind the frozen EvalPlus registry",
        ));
    }
    let mut assets = BTreeSet::new();
    for asset in &benchmark.assets {
        let expected = evalplus_asset(&asset.dataset).ok_or_else(|| {
            ProductError::integrity(
                "BENCHMARK_ASSET_INVALID",
                "the benchmark manifest contains an unknown dataset",
            )
        })?;
        if asset.release_asset != expected.release_asset
            || asset.release_version != expected.release_version
            || !is_sha256(&asset.asset_sha256)
            || !is_sha256(&asset.decoded_sha256)
            || !assets.insert(asset.dataset.as_str())
        {
            return Err(ProductError::integrity(
                "BENCHMARK_ASSET_INVALID",
                "the benchmark asset identity is malformed or duplicated",
            ));
        }
        // The digests are compared against the frozen values, not merely checked
        // for shape. Without this a plausible-looking manifest protecting nothing
        // passes every other gate here.
        if asset.asset_sha256 != expected.asset_sha256
            || asset.decoded_sha256 != expected.decoded_sha256
        {
            return Err(ProductError::integrity(
                "BENCHMARK_ASSET_DIGEST_MISMATCH",
                "a benchmark asset digest differs from the frozen DECONTAM-001 identity",
            ));
        }
    }
    let mut identities = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for record in &benchmark.records {
        require_portable_relative_path(&record.relative_path, "BENCHMARK_PATH_INVALID")?;
        if record.dataset.is_empty()
            || record.task_id.is_empty()
            || record.json_pointer.is_empty()
            || record.role.is_empty()
            || record.bytes == 0
            || !is_sha256(&record.sha256)
            || !assets.contains(record.dataset.as_str())
            || !identities.insert(benchmark_identity(record))
            || !paths.insert(record.relative_path.as_str())
        {
            return Err(ProductError::integrity(
                "BENCHMARK_RECORD_INVALID",
                "a benchmark protection record is malformed or duplicated",
            ));
        }
    }
    Ok(())
}

/// The identity of the composed source generations.
///
/// Downstream this stays one value — `tokenize` binds the sample manifest and the
/// tokenizer artifact to it by equality (`src/storage.rs:631-633`) — so the
/// generalization is a domain-separated digest over the per-generation digests in
/// configuration order, not a list. Order is significant because it is the order
/// documents are loaded in, and a domain tag keeps a one-element combination
/// distinguishable from the bare digest it combines.
fn combined_source_generation_sha256(generations: &[SourceGenerationInput]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"python-slm/source-generation-set/v1\0");
    hasher.update((generations.len() as u64).to_le_bytes());
    for generation in generations {
        hasher.update(generation.manifest.sha256.as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn load_documents(
    sources: &[(SourceGenerationV4, PathBuf)],
    config: &CorpusPolicyConfigV1,
    cancellation: &CancellationToken,
) -> Result<Vec<PreparedDocument>> {
    // Every capacity applies to the composed corpus, not to one generation.
    let accepted = sources
        .iter()
        .flat_map(|(source, root)| {
            source
                .outcomes
                .iter()
                .filter(|outcome| outcome.status == "POLICY_ACCEPTED")
                .map(move |outcome| (outcome, root.as_path()))
        })
        .collect::<Vec<_>>();
    if accepted.is_empty() || accepted.len() as u64 > config.limits.maximum_documents {
        return Err(ProductError::gate(
            "CORPUS_DOCUMENT_CAPACITY_INVALID",
            "the accepted document count is empty or exceeds explicit capacity",
        ));
    }
    let mut documents = Vec::with_capacity(accepted.len());
    let mut source_ids = BTreeSet::new();
    let mut total_bytes = 0_u64;
    let mut total_shingles = 0_u64;
    for (outcome, root) in accepted {
        validate_source_outcome(outcome)?;
        if !source_ids.insert(outcome.source_id.as_str()) {
            return Err(ProductError::integrity(
                "SOURCE_ID_DUPLICATE",
                "the source generation repeats an accepted source identity",
            ));
        }
        let relative = outcome.content_path.as_deref().ok_or_else(|| {
            ProductError::integrity(
                "SOURCE_CONTENT_MISSING",
                "an accepted source outcome has no content path",
            )
        })?;
        require_portable_relative_path(relative, "SOURCE_CONTENT_PATH_INVALID")?;
        let path = join_relative(root, relative)?;
        let metadata = require_contained_regular_file(root, &path)?;
        let canonical_bytes = outcome.canonical_decoded_bytes.ok_or_else(|| {
            ProductError::integrity(
                "SOURCE_CONTENT_IDENTITY_MISSING",
                "an accepted source outcome has no canonical byte count",
            )
        })?;
        if metadata.len() != canonical_bytes {
            return Err(ProductError::integrity(
                "SOURCE_CONTENT_LENGTH_MISMATCH",
                "an accepted source document length differs from its identity",
            ));
        }
        let content = read_stable_document(&path, &metadata)?;
        let canonical_sha256 = outcome.canonical_decoded_sha256.clone().ok_or_else(|| {
            ProductError::integrity(
                "SOURCE_CONTENT_IDENTITY_MISSING",
                "an accepted source outcome has no canonical hash",
            )
        })?;
        if sha256(&content) != canonical_sha256 {
            return Err(ProductError::integrity(
                "SOURCE_CONTENT_HASH_MISMATCH",
                "an accepted source document hash differs from its identity",
            ));
        }
        let parsed = parse_python(&content, cancellation)?;
        if parsed.result.status != "PARSER_ACCEPTED" {
            return Err(ProductError::integrity(
                "SOURCE_PARSER_DRIFT",
                "an accepted source document no longer passes the pinned parser",
            ));
        }
        let encoded_tokens = parsed
            .lexical_tokens
            .iter()
            .map(encode_token)
            .collect::<Result<Vec<_>>>()?;
        let shingles = shingle_set(&encoded_tokens)?;
        total_bytes = total_bytes
            .checked_add(canonical_bytes)
            .ok_or_else(accounting_overflow)?;
        total_shingles = total_shingles
            .checked_add(shingles.len() as u64)
            .ok_or_else(accounting_overflow)?;
        if total_bytes > config.limits.maximum_total_canonical_bytes
            || total_shingles > config.limits.maximum_total_shingles
        {
            return Err(ProductError::gate(
                "CORPUS_CAPACITY_EXCEEDED",
                "accepted source bytes or lexical shingles exceed capacity",
            ));
        }
        documents.push(PreparedDocument {
            source_id: outcome.source_id.clone(),
            repository_group_id: outcome.repository_group_id.clone(),
            curated_sha256_raw: outcome.raw_sha256.clone().expect("validated"),
            canonical_sha256,
            canonical_bytes,
            content,
            provenance: outcome.provenance.clone(),
            comment_bytes: parsed.result.comment_bytes,
            signature: minhash_signature(&shingles),
            encoded_tokens,
        });
        // The shingles have served their purpose; the signature carries forward.
        drop(shingles);
    }
    documents.sort_by(|left, right| left.source_id.as_bytes().cmp(right.source_id.as_bytes()));
    Ok(documents)
}

fn validate_source_outcome(outcome: &SourceOutcomeV4) -> Result<()> {
    let _ = (
        &outcome.provider_record_id,
        &outcome.reasons,
        outcome.raw_bytes,
        outcome.bom_removed,
        &outcome.license_expression,
        &outcome.parser_result_path,
        &outcome.parser_result_sha256,
        &outcome.policy_result_path,
        &outcome.policy_result_sha256,
        &outcome.governed_source_metadata,
    );
    if !is_sha256(&outcome.source_id)
        || !is_sha256(&outcome.repository_group_id)
        || !outcome.raw_sha256.as_deref().is_some_and(is_sha256)
        || !outcome
            .canonical_decoded_sha256
            .as_deref()
            .is_some_and(is_sha256)
        || outcome.canonical_decoded_bytes.is_none()
        || outcome.provenance.origin_url.is_empty()
        || outcome.provenance.revision.is_empty()
        || outcome.provenance.source_path.is_empty()
    {
        return Err(ProductError::integrity(
            "SOURCE_OUTCOME_INVALID",
            "an accepted source outcome has incomplete identity or provenance",
        ));
    }
    Ok(())
}

fn load_benchmark(
    manifest: &BenchmarkProtectionManifestV1,
    root: &Path,
    cancellation: &CancellationToken,
) -> Result<Vec<ProtectedBenchmark>> {
    let mut records = manifest.records.iter().collect::<Vec<_>>();
    records.sort_by(|left, right| {
        (
            left.dataset.as_bytes(),
            left.task_id.as_bytes(),
            left.json_pointer.as_bytes(),
            left.role.as_bytes(),
        )
            .cmp(&(
                right.dataset.as_bytes(),
                right.task_id.as_bytes(),
                right.json_pointer.as_bytes(),
                right.role.as_bytes(),
            ))
    });
    let mut protected = Vec::with_capacity(records.len());
    for record in records {
        let path = join_relative(root, &record.relative_path)?;
        let metadata = require_contained_regular_file(root, &path)?;
        if metadata.len() != record.bytes {
            return Err(ProductError::integrity(
                "BENCHMARK_RECORD_LENGTH_MISMATCH",
                "a benchmark record length differs from its manifest",
            ));
        }
        let content = read_stable_document(&path, &metadata)?;
        let canonical_sha256 = sha256(&content);
        if canonical_sha256 != record.sha256 {
            return Err(ProductError::integrity(
                "BENCHMARK_RECORD_HASH_MISMATCH",
                "a benchmark record hash differs from its manifest",
            ));
        }
        let encoded_tokens = match record.content_kind {
            BenchmarkContentKind::PythonModule => {
                let parsed = parse_python(&content, cancellation)?;
                if parsed.result.status != "PARSER_ACCEPTED" {
                    return Err(ProductError::integrity(
                        "BENCHMARK_PYTHON_INVALID",
                        "a protected Python module does not pass the pinned parser",
                    ));
                }
                parsed
                    .lexical_tokens
                    .iter()
                    .map(encode_token)
                    .collect::<Result<Vec<_>>>()?
            }
            BenchmarkContentKind::PythonFragment => fragment_tokens(&content, cancellation)?,
            BenchmarkContentKind::CanonicalJson => Vec::new(),
        };
        let shingles = shingle_set(&encoded_tokens)?;
        let signature = (!encoded_tokens.is_empty()).then(|| minhash_signature(&shingles));
        protected.push(ProtectedBenchmark {
            identity: benchmark_identity(record),
            kind: record.content_kind,
            content,
            canonical_sha256,
            encoded_tokens,
            shingles,
            signature,
        });
    }
    Ok(protected)
}

fn fragment_tokens(fragment: &[u8], cancellation: &CancellationToken) -> Result<Vec<Vec<u8>>> {
    let normalized = normalize_newlines(fragment);
    let prefix = b"def __evalplus_fragment__():\n";
    let mut wrapped = prefix.to_vec();
    for line in normalized.split_inclusive(|byte| *byte == b'\n') {
        wrapped.extend_from_slice(b"    ");
        wrapped.extend_from_slice(line);
    }
    if !normalized.ends_with(b"\n") {
        wrapped.push(b'\n');
    }
    let parsed = parse_python(&wrapped, cancellation)?;
    if parsed.result.status != "PARSER_ACCEPTED" {
        return Err(ProductError::integrity(
            "BENCHMARK_FRAGMENT_INVALID",
            "a wrapped benchmark fragment does not pass the pinned parser",
        ));
    }
    let mut wrapper_tokens =
        parse_python(b"def __evalplus_fragment__():\n    pass\n", cancellation)?
            .lexical_tokens
            .into_iter()
            .map(|token| (token.kind, token.text))
            .collect::<Vec<_>>();
    if wrapper_tokens.pop() != Some(("pass".to_owned(), b"pass".to_vec())) {
        return Err(ProductError::integrity(
            "BENCHMARK_FRAGMENT_WRAPPER_DRIFT",
            "the pinned parser changed the benchmark wrapper sentinel",
        ));
    }
    let mut tokens = parsed.lexical_tokens;
    if tokens.len() < wrapper_tokens.len()
        || !tokens
            .iter()
            .take(wrapper_tokens.len())
            .map(|token| (&token.kind, &token.text))
            .eq(wrapper_tokens.iter().map(|(kind, text)| (kind, text)))
    {
        return Err(ProductError::integrity(
            "BENCHMARK_FRAGMENT_WRAPPER_DRIFT",
            "the pinned parser changed the benchmark wrapper token prefix",
        ));
    }
    tokens.drain(..wrapper_tokens.len());
    tokens.iter().map(encode_token).collect()
}

/// `DECONTAM-001`: CRLF and CR become LF, without trimming or adding bytes.
/// Shared with the importer so one frozen rule has one implementation.
pub(crate) fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' {
            normalized.push(b'\n');
            index += if bytes.get(index + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
        } else {
            normalized.push(bytes[index]);
            index += 1;
        }
    }
    normalized
}
fn deduplicate(
    documents: &[PreparedDocument],
    maximum_candidate_pairs: u64,
) -> Result<(DedupManifestV1, Vec<Vec<usize>>)> {
    let mut union = UnionFind::new(documents.len());
    let mut exact_groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, document) in documents.iter().enumerate() {
        exact_groups
            .entry(&document.canonical_sha256)
            .or_default()
            .push(index);
    }
    let mut exact_edges = 0_u64;
    for group in exact_groups.values() {
        for pair in group.windows(2) {
            union.union(pair[0], pair[1]);
            exact_edges += 1;
        }
    }

    let mut band_index: BTreeMap<[u8; 32], Vec<usize>> = BTreeMap::new();
    let mut candidates = BTreeSet::new();
    for (index, document) in documents.iter().enumerate() {
        for key in lsh_keys(&document.signature) {
            if let Some(previous) = band_index.get(&key) {
                for candidate in previous {
                    let pair = if *candidate < index {
                        (*candidate, index)
                    } else {
                        (index, *candidate)
                    };
                    candidates.insert(pair);
                }
            }
            band_index.entry(key).or_default().push(index);
        }
    }
    if candidates.len() as u64 > maximum_candidate_pairs {
        return Err(ProductError::gate(
            "DEDUP_CANDIDATE_CAPACITY_EXCEEDED",
            "LSH candidate pairs exceed the explicit configured capacity",
        ));
    }
    let mut near_edges = 0_u64;
    // Shingles are rebuilt for the pairs that need an exact Jaccard rather than
    // held for every document. `candidates` is an ordered set, so every pair
    // sharing a left document is contiguous and that side is built once.
    let mut cached_left: Option<usize> = None;
    let mut left_shingles = BTreeSet::new();
    for (left, right) in &candidates {
        if documents[*left].canonical_sha256 == documents[*right].canonical_sha256 {
            continue;
        }
        if cached_left != Some(*left) {
            left_shingles = shingle_set(&documents[*left].encoded_tokens)?;
            cached_left = Some(*left);
        }
        let right_shingles = shingle_set(&documents[*right].encoded_tokens)?;
        if jaccard_exceeds(&left_shingles, &right_shingles) {
            union.union(*left, *right);
            near_edges += 1;
        }
    }
    let mut grouped: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..documents.len() {
        grouped.entry(union.find(index)).or_default().push(index);
    }
    let mut groups = grouped.into_values().collect::<Vec<_>>();
    for group in &mut groups {
        group.sort_by(|left, right| {
            documents[*left]
                .source_id
                .as_bytes()
                .cmp(documents[*right].source_id.as_bytes())
        });
    }
    groups.sort_by(|left, right| {
        documents[left[0]]
            .source_id
            .as_bytes()
            .cmp(documents[right[0]].source_id.as_bytes())
    });
    let mut clusters = Vec::with_capacity(groups.len());
    for group in &groups {
        let representative = representative(group, documents);
        let members = group
            .iter()
            .map(|index| documents[*index].source_id.clone())
            .collect::<Vec<_>>();
        clusters.push(DuplicateClusterV1 {
            cluster_id: component_id(&members)?,
            representative_source_id: documents[representative].source_id.clone(),
            member_source_ids: members,
        });
    }
    Ok((
        DedupManifestV1 {
            schema: DEDUP_MANIFEST_SCHEMA.to_owned(),
            lexical_policy: "DEDUP-001".to_owned(),
            exact_hash_algorithm: "sha256-canonical-bytes-v1".to_owned(),
            minhash_components: MINHASH_COMPONENTS as u64,
            lsh_bands: LSH_BANDS as u64,
            lsh_rows: LSH_ROWS as u64,
            jaccard_threshold: "strictly-greater-than-0.85".to_owned(),
            input_documents: documents.len() as u64,
            candidate_pairs: candidates.len() as u64,
            exact_duplicate_edges: exact_edges,
            near_duplicate_edges: near_edges,
            clusters,
        },
        groups,
    ))
}

fn representative(group: &[usize], documents: &[PreparedDocument]) -> usize {
    *group
        .iter()
        .min_by(|left, right| representative_order(&documents[**left], &documents[**right]))
        .expect("dedup groups are nonempty")
}

fn representative_order(left: &PreparedDocument, right: &PreparedDocument) -> Ordering {
    provenance_complete(right)
        .cmp(&provenance_complete(left))
        .then_with(|| {
            (left.comment_bytes as u128 * right.canonical_bytes as u128)
                .cmp(&(right.comment_bytes as u128 * left.canonical_bytes as u128))
        })
        // The encoded tokens are one per lexical token, so this is the same
        // higher-token-count tie-break DEDUP-001 specifies.
        .then_with(|| right.encoded_tokens.len().cmp(&left.encoded_tokens.len()))
        .then_with(|| left.source_id.as_bytes().cmp(right.source_id.as_bytes()))
}

fn provenance_complete(document: &PreparedDocument) -> bool {
    !document.provenance.origin_url.is_empty()
        && !document.provenance.revision.is_empty()
        && !document.provenance.source_path.is_empty()
}

fn decontaminate(
    documents: &[PreparedDocument],
    duplicate_groups: &[Vec<usize>],
    protected: &[ProtectedBenchmark],
    benchmark_manifest_sha256: &str,
) -> Result<(DecontaminationManifestV1, BTreeSet<usize>)> {
    let mut exact = BTreeMap::new();
    let mut bands: BTreeMap<[u8; 32], Vec<usize>> = BTreeMap::new();
    let mut spans: BTreeMap<[u8; 32], Vec<usize>> = BTreeMap::new();
    let mut short_sequences: BTreeMap<Vec<Vec<u8>>, Vec<usize>> = BTreeMap::new();
    let mut canonical_json = Vec::new();
    for (index, record) in protected.iter().enumerate() {
        match record.kind {
            BenchmarkContentKind::CanonicalJson => canonical_json.push(index),
            _ => {
                exact
                    .entry(record.canonical_sha256.as_str())
                    .or_insert_with(Vec::new)
                    .push(index);
                if let Some(signature) = &record.signature {
                    for key in lsh_keys(signature) {
                        bands.entry(key).or_default().push(index);
                    }
                }
                if record.encoded_tokens.len() < PROTECTED_SPAN_TOKENS {
                    short_sequences
                        .entry(record.encoded_tokens.clone())
                        .or_default()
                        .push(index);
                } else {
                    for window in record.encoded_tokens.windows(PROTECTED_SPAN_TOKENS) {
                        spans.entry(sequence_hash(window)).or_default().push(index);
                    }
                }
            }
        }
    }

    let mut outcomes = Vec::new();
    let mut matched_documents = BTreeSet::new();
    for (document_index, document) in documents.iter().enumerate() {
        let mut matches: BTreeMap<usize, BTreeSet<&'static str>> = BTreeMap::new();
        if let Some(indices) = exact.get(document.canonical_sha256.as_str()) {
            for index in indices {
                matches.entry(*index).or_default().insert("EXACT_SOURCE");
            }
        }
        for index in &canonical_json {
            let bytes = &protected[*index].content;
            if !bytes.is_empty()
                && document
                    .content
                    .windows(bytes.len())
                    .any(|part| part == bytes)
            {
                matches
                    .entry(*index)
                    .or_default()
                    .insert("CANONICAL_JSON_EXACT");
            }
        }
        let mut benchmark_candidates = BTreeSet::new();
        for key in lsh_keys(&document.signature) {
            if let Some(indices) = bands.get(&key) {
                benchmark_candidates.extend(indices.iter().copied());
            }
        }
        // Built once per document, and only when the bands actually proposed a
        // benchmark candidate to compare against.
        if !benchmark_candidates.is_empty() {
            let document_shingles = shingle_set(&document.encoded_tokens)?;
            for index in benchmark_candidates {
                if jaccard_exceeds(&document_shingles, &protected[index].shingles) {
                    matches.entry(index).or_default().insert("LEXICAL_JACCARD");
                }
            }
        }
        if let Some(indices) = short_sequences.get(&document.encoded_tokens) {
            for index in indices {
                matches
                    .entry(*index)
                    .or_default()
                    .insert("PROTECTED_COMPLETE_SEQUENCE");
            }
        }
        if document.encoded_tokens.len() >= PROTECTED_SPAN_TOKENS {
            for window in document.encoded_tokens.windows(PROTECTED_SPAN_TOKENS) {
                if let Some(indices) = spans.get(&sequence_hash(window)) {
                    for index in indices {
                        if !protected[*index]
                            .encoded_tokens
                            .windows(PROTECTED_SPAN_TOKENS)
                            .any(|protected_window| protected_window == window)
                        {
                            continue;
                        }
                        matches
                            .entry(*index)
                            .or_default()
                            .insert("PROTECTED_50_TOKEN_SPAN");
                    }
                }
            }
        }
        if !matches.is_empty() {
            matched_documents.insert(document_index);
            let benchmark_identities = matches
                .keys()
                .map(|index| protected[*index].identity.clone())
                .collect::<Vec<_>>();
            let reasons = matches
                .values()
                .flat_map(|reasons| reasons.iter().copied())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(str::to_owned)
                .collect();
            outcomes.push(DecontaminationOutcomeV1 {
                source_id: document.source_id.clone(),
                benchmark_identities,
                reasons,
            });
        }
    }
    outcomes.sort_by(|left, right| left.source_id.as_bytes().cmp(right.source_id.as_bytes()));
    let mut rejected_clusters = BTreeSet::new();
    for (cluster, group) in duplicate_groups.iter().enumerate() {
        if group.iter().any(|index| matched_documents.contains(index)) {
            rejected_clusters.insert(cluster);
        }
    }
    Ok((
        DecontaminationManifestV1 {
            schema: DECONTAMINATION_MANIFEST_SCHEMA.to_owned(),
            benchmark_manifest_sha256: benchmark_manifest_sha256.to_owned(),
            registry_id: EVALPLUS_REGISTRY_ID.to_owned(),
            registry_commit: EVALPLUS_REGISTRY_COMMIT.to_owned(),
            protected_records: protected.len() as u64,
            rejected_clusters: rejected_clusters.len() as u64,
            rejected_documents: outcomes,
        },
        rejected_clusters,
    ))
}

#[derive(Clone, Debug)]
struct RepresentativeAssignment {
    document_index: usize,
    component_id: String,
    split: CorpusSplit,
}

fn assign_splits(
    documents: &[PreparedDocument],
    duplicate_groups: &[Vec<usize>],
    rejected_clusters: &BTreeSet<usize>,
) -> Result<(SplitManifestV1, Vec<RepresentativeAssignment>)> {
    let mut union = UnionFind::new(documents.len());
    let mut repository_first: BTreeMap<&str, usize> = BTreeMap::new();
    for (cluster, group) in duplicate_groups.iter().enumerate() {
        if rejected_clusters.contains(&cluster) {
            continue;
        }
        for pair in group.windows(2) {
            union.union(pair[0], pair[1]);
        }
        for index in group {
            if let Some(first) =
                repository_first.get(documents[*index].repository_group_id.as_str())
            {
                union.union(*first, *index);
            } else {
                repository_first.insert(&documents[*index].repository_group_id, *index);
            }
        }
    }
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (cluster, group) in duplicate_groups.iter().enumerate() {
        if !rejected_clusters.contains(&cluster) {
            for index in group {
                components
                    .entry(union.find(*index))
                    .or_default()
                    .push(*index);
            }
        }
    }
    let mut split_components = Vec::new();
    let mut assignments = Vec::new();
    for mut members in components.into_values() {
        members.sort_by(|left, right| {
            documents[*left]
                .source_id
                .as_bytes()
                .cmp(documents[*right].source_id.as_bytes())
        });
        members.dedup();
        let member_ids = members
            .iter()
            .map(|index| documents[*index].source_id.clone())
            .collect::<Vec<_>>();
        let component_id = component_id(&member_ids)?;
        let bucket = split_bucket(&component_id)?;
        let split = split_for_bucket(bucket);
        let member_set = members.iter().copied().collect::<BTreeSet<_>>();
        let mut representatives = Vec::new();
        for group in duplicate_groups {
            if group.iter().any(|index| member_set.contains(index)) {
                let representative = representative(group, documents);
                representatives.push(representative);
            }
        }
        representatives.sort_by(|left, right| {
            documents[*left]
                .source_id
                .as_bytes()
                .cmp(documents[*right].source_id.as_bytes())
        });
        representatives.dedup();
        let representative_source_ids = representatives
            .iter()
            .map(|index| documents[*index].source_id.clone())
            .collect::<Vec<_>>();
        for document_index in representatives {
            assignments.push(RepresentativeAssignment {
                document_index,
                component_id: component_id.clone(),
                split,
            });
        }
        split_components.push(SplitComponentV1 {
            component_id,
            member_source_ids: member_ids,
            representative_source_ids,
            bucket,
            split,
        });
    }
    split_components.sort_by(|left, right| {
        left.component_id
            .as_bytes()
            .cmp(right.component_id.as_bytes())
    });
    assignments.sort_by(|left, right| {
        (
            left.split,
            left.component_id.as_bytes(),
            documents[left.document_index].source_id.as_bytes(),
        )
            .cmp(&(
                right.split,
                right.component_id.as_bytes(),
                documents[right.document_index].source_id.as_bytes(),
            ))
    });
    Ok((
        SplitManifestV1 {
            schema: SPLIT_MANIFEST_SCHEMA.to_owned(),
            algorithm: "SPLIT-001".to_owned(),
            components: split_components,
        },
        assignments,
    ))
}
fn encode_token(token: &LexicalToken) -> Result<Vec<u8>> {
    let kind = token.kind.as_bytes();
    let kind_length = u64::try_from(kind.len()).map_err(|_| accounting_overflow())?;
    let text_length = u64::try_from(token.text.len()).map_err(|_| accounting_overflow())?;
    let mut encoded = Vec::with_capacity(16 + kind.len() + token.text.len());
    encoded.extend_from_slice(&kind_length.to_le_bytes());
    encoded.extend_from_slice(kind);
    encoded.extend_from_slice(&text_length.to_le_bytes());
    encoded.extend_from_slice(&token.text);
    Ok(encoded)
}

fn shingle_set(tokens: &[Vec<u8>]) -> Result<BTreeSet<Vec<u8>>> {
    let mut shingles = BTreeSet::new();
    if tokens.len() < 5 {
        let mut short = SHORT_DOMAIN.to_vec();
        short.extend_from_slice(
            &u64::try_from(tokens.len())
                .map_err(|_| accounting_overflow())?
                .to_le_bytes(),
        );
        for token in tokens {
            short.extend_from_slice(token);
        }
        shingles.insert(short);
    } else {
        for window in tokens.windows(5) {
            let capacity = window.iter().try_fold(0_usize, |total, token| {
                total
                    .checked_add(token.len())
                    .ok_or_else(accounting_overflow)
            })?;
            let mut shingle = Vec::with_capacity(capacity);
            for token in window {
                shingle.extend_from_slice(token);
            }
            shingles.insert(shingle);
        }
    }
    Ok(shingles)
}

fn minhash_signature(shingles: &BTreeSet<Vec<u8>>) -> [u64; MINHASH_COMPONENTS] {
    let bases = shingles
        .iter()
        .map(|shingle| {
            let mut hasher = Sha256::new();
            hasher.update(SHINGLE_BASE_DOMAIN);
            hasher.update((shingle.len() as u64).to_le_bytes());
            hasher.update(shingle);
            let digest: [u8; 32] = hasher.finalize().into();
            u64::from_le_bytes(digest[..8].try_into().expect("fixed slice")) % PRIME
        })
        .collect::<Vec<_>>();
    std::array::from_fn(|index| {
        let mut hasher = Sha256::new();
        hasher.update(COEFFICIENT_DOMAIN);
        hasher.update((index as u32).to_le_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let a =
            1 + (u64::from_le_bytes(digest[..8].try_into().expect("fixed slice")) % (PRIME - 1));
        let b = u64::from_le_bytes(digest[8..16].try_into().expect("fixed slice")) % PRIME;
        bases
            .iter()
            .map(|base| ((a as u128 * *base as u128 + b as u128) % PRIME as u128) as u64)
            .min()
            .expect("short documents have one shingle")
    })
}

fn lsh_keys(signature: &[u64; MINHASH_COMPONENTS]) -> [[u8; 32]; LSH_BANDS] {
    std::array::from_fn(|band| {
        let mut hasher = Sha256::new();
        hasher.update(LSH_DOMAIN);
        hasher.update((band as u32).to_le_bytes());
        for value in &signature[band * LSH_ROWS..(band + 1) * LSH_ROWS] {
            hasher.update(value.to_le_bytes());
        }
        hasher.finalize().into()
    })
}

fn jaccard_exceeds(left: &BTreeSet<Vec<u8>>, right: &BTreeSet<Vec<u8>>) -> bool {
    let intersection = left.intersection(right).count() as u128;
    let union = left.len() as u128 + right.len() as u128 - intersection;
    intersection * 100 > union * 85
}

fn sequence_hash(tokens: &[Vec<u8>]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"python-slm/protected-token-span/v1\0");
    for token in tokens {
        hasher.update(token);
    }
    hasher.finalize().into()
}

fn component_id(member_ids: &[String]) -> Result<String> {
    let mut sorted = member_ids.iter().map(String::as_str).collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    if sorted.is_empty() || sorted.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ProductError::integrity(
            "COMPONENT_MEMBERS_INVALID",
            "component source identities must be nonempty and unique",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(COMPONENT_DOMAIN);
    for member in sorted {
        update_lp(&mut hasher, member.as_bytes())?;
    }
    Ok(hex::encode(hasher.finalize()))
}

fn split_bucket(component_id: &str) -> Result<u64> {
    let raw = hex::decode(component_id).map_err(|_| {
        ProductError::integrity(
            "COMPONENT_ID_INVALID",
            "a component identity is not lowercase SHA-256",
        )
    })?;
    if raw.len() != 32 || !is_sha256(component_id) {
        return Err(ProductError::integrity(
            "COMPONENT_ID_INVALID",
            "a component identity is not lowercase SHA-256",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(SPLIT_DOMAIN);
    hasher.update(&raw);
    let digest: [u8; 32] = hasher.finalize().into();
    Ok(u64::from_be_bytes(digest[..8].try_into().expect("fixed slice")) % 10_000)
}

fn split_for_bucket(bucket: u64) -> CorpusSplit {
    match bucket {
        0..=9_799 => CorpusSplit::Train,
        9_800..=9_899 => CorpusSplit::Validation,
        _ => CorpusSplit::Test,
    }
}

fn update_lp(hasher: &mut Sha256, bytes: &[u8]) -> Result<()> {
    let length = u64::try_from(bytes.len()).map_err(|_| accounting_overflow())?;
    hasher.update(length.to_le_bytes());
    hasher.update(bytes);
    Ok(())
}

fn benchmark_identity(record: &BenchmarkRecordV1) -> String {
    format!(
        "{}:{}:{}:{}",
        record.dataset, record.task_id, record.json_pointer, record.role
    )
}

fn sample_document_identity_order(
    left: &TokenizerSampleDocumentV1,
    right: &TokenizerSampleDocumentV1,
) -> Ordering {
    (
        left.repository_group_id.as_bytes(),
        left.source_id.as_bytes(),
        left.curated_sha256_raw.as_bytes(),
    )
        .cmp(&(
            right.repository_group_id.as_bytes(),
            right.source_id.as_bytes(),
            right.curated_sha256_raw.as_bytes(),
        ))
}

fn governed_document_order(
    left: &GovernedCorpusDocumentV1,
    right: &GovernedCorpusDocumentV1,
) -> Ordering {
    (
        left.split,
        left.component_id.as_bytes(),
        left.repository_group_id.as_bytes(),
        left.source_id.as_bytes(),
        left.curated_sha256_raw.as_bytes(),
    )
        .cmp(&(
            right.split,
            right.component_id.as_bytes(),
            right.repository_group_id.as_bytes(),
            right.source_id.as_bytes(),
            right.curated_sha256_raw.as_bytes(),
        ))
}

fn artifact_binding(path: &str, bytes: &[u8]) -> ArtifactBinding {
    ArtifactBinding {
        path: path.to_owned(),
        sha256: sha256(bytes),
        bytes: bytes.len() as u64,
    }
}

fn accounting_overflow() -> ProductError {
    ProductError::integrity(
        "CORPUS_ACCOUNTING_OVERFLOW",
        "corpus policy arithmetic overflowed",
    )
}

#[derive(Clone, Debug)]
struct UnionFind {
    parents: Vec<usize>,
    ranks: Vec<u8>,
}

impl UnionFind {
    fn new(length: usize) -> Self {
        Self {
            parents: (0..length).collect(),
            ranks: vec![0; length],
        }
    }

    fn find(&mut self, value: usize) -> usize {
        if self.parents[value] != value {
            self.parents[value] = self.find(self.parents[value]);
        }
        self.parents[value]
    }

    fn union(&mut self, left: usize, right: usize) {
        let left = self.find(left);
        let right = self.find(right);
        if left == right {
            return;
        }
        match self.ranks[left].cmp(&self.ranks[right]) {
            Ordering::Less => self.parents[left] = right,
            Ordering::Greater => self.parents[right] = left,
            Ordering::Equal => {
                self.parents[right] = left;
                self.ranks[left] += 1;
            }
        }
    }
}
fn plan_spans_portable(config_path: &Path) -> Result<Value> {
    let config_bytes = read_control_file(config_path, None, "SPAN_CONFIG_READ_FAILED")?;
    let config: SpanOrderConfigV1 = parse_closed(&config_bytes, "SPAN_CONFIG_INVALID")?;
    validate_span_config(&config)?;
    let corpus_root =
        require_existing_root(&config.token_corpus_root, "TOKEN_GENERATION_ROOT_INVALID")?;
    require_output_boundary(&config.output_path, &corpus_root)?;
    let corpus_manifest_bytes = read_control_file(
        &corpus_root.join("manifest.json"),
        Some(&config.token_corpus_manifest_sha256),
        "TOKEN_MANIFEST_READ_FAILED",
    )?;
    let _: TokenCorpusGenerationV1 =
        parse_closed(&corpus_manifest_bytes, "TOKEN_MANIFEST_INVALID")?;
    let corpus = VerifiedTokenCorpus::open(&corpus_root)?;
    let decision_bytes = read_control_file(
        &config.decision_ledger_path,
        Some(&config.decision_ledger_sha256),
        "DECISION_LEDGER_READ_FAILED",
    )?;
    let decisions = frozen_decision_range(&decision_bytes)?;
    let decisions_sha256 = sha256(decisions);
    let manifest = build_span_order(
        corpus.manifest.accounting.training_prefix_ids,
        &decisions_sha256,
        &config.token_corpus_manifest_sha256,
    )?;
    let manifest_bytes = compact_json_line(&manifest, "SPAN_MANIFEST_SERIALIZATION_FAILED")?;
    write_new_atomic(&config.output_path, &manifest_bytes)?;
    serde_json::to_value(SpanResult {
        schema: SPAN_RESULT_SCHEMA,
        status: "SPAN_ORDER_MATERIALIZED",
        qualification_status: "SKIPPED",
        profile: PROTOTYPE_PROFILE,
        span_manifest_sha256: sha256(&manifest_bytes),
        complete_span_count: manifest.complete_span_count,
        partial_span_targets: manifest
            .partial_span
            .as_ref()
            .map_or(0, |span| span.valid_targets),
        output_created: true,
        receipts_written: false,
    })
    .map_err(|_| {
        ProductError::internal(
            "RESULT_SERIALIZATION_FAILED",
            "could not serialize the span-order result",
        )
    })
}

fn validate_span_config(config: &SpanOrderConfigV1) -> Result<()> {
    if config.schema != SPAN_CONFIG_SCHEMA {
        return Err(ProductError::usage(
            "CONFIG_SCHEMA_UNSUPPORTED",
            "the span-order configuration schema is unsupported",
        ));
    }
    if config.profile != PROTOTYPE_PROFILE {
        return Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "the requested profile is designed but not implemented",
        ));
    }
    if !config.token_corpus_root.is_absolute()
        || !config.decision_ledger_path.is_absolute()
        || !config.output_path.is_absolute()
        || !is_sha256(&config.token_corpus_manifest_sha256)
        || !is_sha256(&config.decision_ledger_sha256)
    {
        return Err(ProductError::usage(
            "SPAN_CONFIG_INVALID",
            "span-order paths must be absolute and hashes must be lowercase SHA-256",
        ));
    }
    Ok(())
}

pub fn build_span_order(
    training_prefix_ids: u64,
    contract_decisions_sha256: &str,
    corpus_manifest_sha256: &str,
) -> Result<SpanOrderManifestV1> {
    if training_prefix_ids < 2
        || !is_sha256(contract_decisions_sha256)
        || !is_sha256(corpus_manifest_sha256)
    {
        return Err(ProductError::integrity(
            "SPAN_OPERANDS_INVALID",
            "span ordering requires a nonempty prefix and two lowercase SHA-256 operands",
        ));
    }
    let valid_targets = training_prefix_ids.saturating_sub(1);
    let complete_span_count = valid_targets / TARGET_SPAN;
    let partial_targets = valid_targets % TARGET_SPAN;
    let mut seed_hasher = Sha256::new();
    seed_hasher.update(SPAN_DOMAIN);
    seed_hasher.update(decode_sha256(contract_decisions_sha256)?);
    seed_hasher.update(decode_sha256(corpus_manifest_sha256)?);
    let seed: [u8; 32] = seed_hasher.finalize().into();
    let mut rng = ChaCha12Rng::from_seed(seed);
    let mut offsets = (0..complete_span_count)
        .map(|index| index * TARGET_SPAN)
        .collect::<Vec<_>>();
    for i in (1..offsets.len()).rev() {
        let range = i as u64 + 1;
        let threshold = 0_u64.wrapping_sub(range) % range;
        let x = loop {
            let value = rng.next_u64();
            if value >= threshold {
                break value;
            }
        };
        offsets.swap(i, (x % range) as usize);
    }
    let mut ordered_spans = offsets
        .into_iter()
        .enumerate()
        .map(|(order, first_target_offset)| SpanDescriptorV1 {
            order: order as u64,
            first_target_offset,
            valid_targets: TARGET_SPAN,
        })
        .collect::<Vec<_>>();
    let partial_span = (partial_targets > 0).then(|| PartialSpanV1 {
        first_target_offset: complete_span_count * TARGET_SPAN,
        valid_targets: partial_targets,
        runtime_padding_targets: TARGET_SPAN - partial_targets,
    });
    if let Some(partial) = &partial_span {
        ordered_spans.push(SpanDescriptorV1 {
            order: ordered_spans.len() as u64,
            first_target_offset: partial.first_target_offset,
            valid_targets: partial.valid_targets,
        });
    }
    validate_span_order(valid_targets, complete_span_count, &ordered_spans)?;
    Ok(SpanOrderManifestV1 {
        schema: SPAN_MANIFEST_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        algorithm: "SPAN-001".to_owned(),
        rng: "rand_chacha-0.10.0/ChaCha12Rng".to_owned(),
        contract_decisions_sha256: contract_decisions_sha256.to_owned(),
        corpus_manifest_sha256: corpus_manifest_sha256.to_owned(),
        seed_sha256: hex::encode(seed),
        target_span: TARGET_SPAN,
        training_prefix_ids,
        valid_targets,
        complete_span_count,
        partial_span,
        ordered_spans,
    })
}

fn validate_span_order(
    valid_targets: u64,
    complete_span_count: u64,
    spans: &[SpanDescriptorV1],
) -> Result<()> {
    let mut complete = BTreeSet::new();
    let mut observed_targets = 0_u64;
    for (order, span) in spans.iter().enumerate() {
        if span.order != order as u64
            || span.first_target_offset >= valid_targets
            || span.valid_targets == 0
            || span.valid_targets > TARGET_SPAN
        {
            return Err(ProductError::integrity(
                "SPAN_ORDER_INVALID",
                "a span descriptor is out of range or out of order",
            ));
        }
        observed_targets = observed_targets
            .checked_add(span.valid_targets)
            .ok_or_else(accounting_overflow)?;
        if span.valid_targets == TARGET_SPAN {
            if span.first_target_offset % TARGET_SPAN != 0
                || !complete.insert(span.first_target_offset)
            {
                return Err(ProductError::integrity(
                    "SPAN_ORDER_INVALID",
                    "a complete span is duplicated or misaligned",
                ));
            }
        } else if order + 1 != spans.len()
            || span.first_target_offset != complete_span_count * TARGET_SPAN
        {
            return Err(ProductError::integrity(
                "SPAN_ORDER_INVALID",
                "the partial span is not the exact final descriptor",
            ));
        }
    }
    let expected = (0..complete_span_count)
        .map(|index| index * TARGET_SPAN)
        .collect::<BTreeSet<_>>();
    if complete != expected || observed_targets != valid_targets {
        return Err(ProductError::integrity(
            "SPAN_ORDER_INVALID",
            "the span order omits, duplicates, or overcounts training targets",
        ));
    }
    Ok(())
}

fn frozen_decision_range(bytes: &[u8]) -> Result<&[u8]> {
    let start_marker = b"## Frozen Decision Ledger\n";
    let end_marker = b"\n## Deferred Qualification Facts";
    let start = find_bytes(bytes, start_marker).ok_or_else(|| {
        ProductError::integrity(
            "DECISION_LEDGER_RANGE_MISSING",
            "the frozen decision-ledger start marker is missing or not LF-normalized",
        )
    })?;
    let end = find_bytes(&bytes[start..], end_marker)
        .map(|offset| start + offset)
        .ok_or_else(|| {
            ProductError::integrity(
                "DECISION_LEDGER_RANGE_MISSING",
                "the frozen decision-ledger end marker is missing or not LF-normalized",
            )
        })?;
    Ok(&bytes[start..end])
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn decode_sha256(value: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(value)
        .map_err(|_| ProductError::integrity("SHA256_INVALID", "a SHA-256 operand is malformed"))?;
    bytes
        .try_into()
        .map_err(|_| ProductError::integrity("SHA256_INVALID", "a SHA-256 operand is malformed"))
}

struct PartialCorpusGeneration {
    partial_path: PathBuf,
    final_path: PathBuf,
    published: bool,
}

impl PartialCorpusGeneration {
    fn create(final_path: &Path) -> Result<Self> {
        if final_path.exists() {
            return Err(ProductError::integrity(
                "OUTPUT_ALREADY_EXISTS",
                "the create-new corpus policy output already exists",
            ));
        }
        let parent = final_path.parent().ok_or_else(|| {
            ProductError::usage(
                "OUTPUT_PARENT_INVALID",
                "the corpus policy output has no parent",
            )
        })?;
        let name = final_path.file_name().ok_or_else(|| {
            ProductError::usage(
                "OUTPUT_NAME_INVALID",
                "the corpus policy output has no final component",
            )
        })?;
        for _ in 0..128 {
            let sequence = PARTIAL_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
            let partial_path = parent.join(format!(
                ".{}.partial-{sequence:016x}",
                name.to_string_lossy()
            ));
            match fs::create_dir(&partial_path) {
                Ok(()) => {
                    return Ok(Self {
                        partial_path,
                        final_path: final_path.to_path_buf(),
                        published: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    return Err(ProductError::environment(
                        "OUTPUT_CREATE_FAILED",
                        "could not create the private corpus policy generation",
                    ));
                }
            }
        }
        Err(ProductError::environment(
            "OUTPUT_CREATE_FAILED",
            "could not allocate a unique corpus policy generation",
        ))
    }

    fn create_directory(&mut self, relative: &str) -> Result<()> {
        require_portable_relative_path(relative, "OUTPUT_PATH_INVALID")?;
        fs::create_dir(self.partial_path.join(relative)).map_err(|_| {
            ProductError::environment(
                "OUTPUT_DIRECTORY_CREATE_FAILED",
                "could not create a corpus policy generation directory",
            )
        })
    }

    fn write_file(&mut self, relative: &Path, bytes: &[u8]) -> Result<()> {
        let relative = relative.to_str().ok_or_else(|| {
            ProductError::integrity(
                "OUTPUT_PATH_INVALID",
                "a corpus output path is not portable UTF-8",
            )
        })?;
        require_portable_relative_path(relative, "OUTPUT_PATH_INVALID")?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.partial_path.join(relative))
            .map_err(|_| {
                ProductError::environment(
                    "OUTPUT_FILE_CREATE_FAILED",
                    "could not create an immutable corpus policy file",
                )
            })?;
        file.write_all(bytes).map_err(|_| {
            ProductError::environment(
                "OUTPUT_FILE_WRITE_FAILED",
                "could not write a corpus policy file",
            )
        })?;
        file.sync_all().map_err(|_| {
            ProductError::environment(
                "OUTPUT_FILE_SYNC_FAILED",
                "could not sync a corpus policy file",
            )
        })
    }

    fn publish(&mut self) -> Result<()> {
        if self.final_path.exists() {
            return Err(ProductError::integrity(
                "OUTPUT_ALREADY_EXISTS",
                "the corpus policy output appeared before publication",
            ));
        }
        publish_create_new_directory(&self.partial_path, &self.final_path)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for PartialCorpusGeneration {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.partial_path);
        }
    }
}
fn publish_create_new_directory(from: &Path, to: &Path) -> Result<()> {
    crate::platform::publish_create_new(from, to)
}
fn write_new_atomic(output: &Path, bytes: &[u8]) -> Result<()> {
    if output.exists() {
        return Err(ProductError::integrity(
            "OUTPUT_ALREADY_EXISTS",
            "the create-new span-order output already exists",
        ));
    }
    let parent = output.parent().ok_or_else(|| {
        ProductError::usage("OUTPUT_PARENT_INVALID", "the span output has no parent")
    })?;
    let name = output.file_name().ok_or_else(|| {
        ProductError::usage("OUTPUT_NAME_INVALID", "the span output has no file name")
    })?;
    let sequence = PARTIAL_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    let temporary = parent.join(format!(
        ".{}.partial-{sequence:016x}",
        name.to_string_lossy()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| {
                ProductError::environment(
                    "OUTPUT_FILE_CREATE_FAILED",
                    "could not create the private span-order file",
                )
            })?;
        file.write_all(bytes).map_err(|_| {
            ProductError::environment(
                "OUTPUT_FILE_WRITE_FAILED",
                "could not write the span-order file",
            )
        })?;
        file.sync_all().map_err(|_| {
            ProductError::environment(
                "OUTPUT_FILE_SYNC_FAILED",
                "could not sync the span-order file",
            )
        })?;
        if output.exists() {
            return Err(ProductError::integrity(
                "OUTPUT_ALREADY_EXISTS",
                "the span-order output appeared before publication",
            ));
        }
        publish_create_new_file(&temporary, output)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn publish_create_new_file(from: &Path, to: &Path) -> Result<()> {
    crate::platform::publish_create_new(from, to)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn hash(label: &str) -> String {
        sha256(label.as_bytes())
    }

    /// The frozen `DECONTAM-001` table is transcribed by hand from a discovery
    /// run, so a mistyped digest is the obvious failure mode. This checks the
    /// shape of every field rather than trusting the transcription, and it fails
    /// at test time instead of at the point where a corpus is being decontaminated.
    #[test]
    fn the_frozen_evalplus_table_is_well_formed() {
        assert_eq!(EVALPLUS_ASSETS.len(), 2);
        let mut datasets = BTreeSet::new();
        for asset in &EVALPLUS_ASSETS {
            assert!(
                datasets.insert(asset.dataset),
                "duplicate dataset {}",
                asset.dataset
            );
            assert!(
                is_sha256(asset.asset_sha256),
                "{} asset digest is not lowercase SHA-256",
                asset.dataset
            );
            assert!(
                is_sha256(asset.decoded_sha256),
                "{} decoded digest is not lowercase SHA-256",
                asset.dataset
            );
            assert_ne!(asset.asset_sha256, asset.decoded_sha256);
            assert!(asset.release_asset.ends_with(".jsonl.gz"));
            assert!(asset.release_version.starts_with('v'));
        }
        // The two datasets DECONTAM-001 names, and nothing else.
        assert!(evalplus_asset("humanevalplus").is_some());
        assert!(evalplus_asset("mbppplus").is_some());
        assert!(evalplus_asset("nonexistent").is_none());
        // A near-miss on the dataset key must not resolve to a real asset.
        assert!(evalplus_asset("HumanEvalPlus").is_none());
    }

    fn document(label: &str, repository: &str, source: &[u8]) -> PreparedDocument {
        let parsed = parse_python(source, &CancellationToken::default()).unwrap();
        assert_eq!(parsed.result.status, "PARSER_ACCEPTED");
        let encoded_tokens = parsed
            .lexical_tokens
            .iter()
            .map(encode_token)
            .collect::<Result<Vec<_>>>()
            .unwrap();
        let shingles = shingle_set(&encoded_tokens).unwrap();
        PreparedDocument {
            source_id: hash(label),
            repository_group_id: hash(repository),
            curated_sha256_raw: sha256(source),
            canonical_sha256: sha256(source),
            canonical_bytes: source.len() as u64,
            content: source.to_vec(),
            provenance: Provenance {
                origin_url: format!("https://example.invalid/{label}"),
                revision: "r1".to_owned(),
                source_path: format!("{label}.py"),
            },
            comment_bytes: parsed.result.comment_bytes,
            signature: minhash_signature(&shingles),
            encoded_tokens,
        }
    }
    /// A document whose shingles derive from `tokens` distinct encoded tokens, so
    /// two of them share `min(a, b) - 4` shingles out of `max(a, b) - 4`.
    fn synthetic_document(label: &str, tokens: u16) -> PreparedDocument {
        let encoded_tokens = (0..tokens)
            .map(|value| value.to_le_bytes().to_vec())
            .collect::<Vec<_>>();
        let shingles = shingle_set(&encoded_tokens).expect("synthetic shingles");
        PreparedDocument {
            source_id: hash(label),
            repository_group_id: hash(&format!("repo-{label}")),
            curated_sha256_raw: hash(&format!("raw-{label}")),
            canonical_sha256: hash(&format!("canonical-{label}")),
            canonical_bytes: 1,
            content: label.as_bytes().to_vec(),
            provenance: Provenance {
                origin_url: format!("https://example.invalid/{label}"),
                revision: "r1".to_owned(),
                source_path: format!("{label}.py"),
            },
            comment_bytes: 0,
            signature: minhash_signature(&shingles),
            encoded_tokens,
        }
    }

    #[test]
    fn jaccard_threshold_is_strictly_above_point_eighty_five() {
        let left = (0_u8..20).map(|value| vec![value]).collect::<BTreeSet<_>>();
        let equal = (0_u8..17).map(|value| vec![value]).collect::<BTreeSet<_>>();
        let above = (0_u8..18).map(|value| vec![value]).collect::<BTreeSet<_>>();
        assert!(!jaccard_exceeds(&left, &equal));
        assert!(jaccard_exceeds(&left, &above));
    }

    #[test]
    fn lsh_candidates_are_confirmed_by_exact_jaccard() {
        // 100 shingles against 90 shared ones: a Jaccard of exactly 0.9, which is
        // above the 0.85 threshold and must survive rebuilding the sets on demand.
        let documents = vec![
            synthetic_document("left", 104),
            synthetic_document("right", 94),
        ];
        let (manifest, groups) = deduplicate(&documents, 100).unwrap();
        assert_eq!(manifest.near_duplicate_edges, 1);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }
    #[test]
    fn exact_dedup_is_order_invariant_and_retains_all_members() {
        let first = document("first", "repo-a", b"def answer():\n    return 42\n");
        let second = document("second", "repo-b", b"def answer():\n    return 42\n");
        let third = document("third", "repo-c", b"def other():\n    return 7\n");
        let ordered = vec![first.clone(), second.clone(), third.clone()];
        let reversed = vec![third, second, first];
        let (left, _) = deduplicate(&ordered, 100).unwrap();
        let (right, _) = deduplicate(&reversed, 100).unwrap();
        assert_eq!(left.clusters, right.clusters);
        assert_eq!(left.exact_duplicate_edges, 1);
        assert_eq!(left.clusters[0].member_source_ids.len(), 2);
    }

    #[test]
    fn decontamination_rejects_the_entire_duplicate_cluster() {
        let first = document("first", "repo-a", b"def answer():\n    return 42\n");
        let second = document("second", "repo-b", b"def answer():\n    return 42\n");
        let documents = vec![first, second];
        let (_, groups) = deduplicate(&documents, 100).unwrap();
        let protected = vec![ProtectedBenchmark {
            identity: "humanevalplus:0:/prompt:prompt".to_owned(),
            kind: BenchmarkContentKind::PythonModule,
            content: documents[0].content.clone(),
            canonical_sha256: documents[0].canonical_sha256.clone(),
            encoded_tokens: documents[0].encoded_tokens.clone(),
            // Protected records keep their shingles: there are only a few thousand
            // of them, and they are compared against every document.
            shingles: shingle_set(&documents[0].encoded_tokens).unwrap(),
            signature: Some(documents[0].signature),
        }];
        let (manifest, rejected) =
            decontaminate(&documents, &groups, &protected, &hash("benchmark")).unwrap();
        assert_eq!(rejected.len(), 1);
        assert_eq!(manifest.rejected_documents.len(), 2);
        assert!(
            manifest.rejected_documents[0]
                .reasons
                .contains(&"EXACT_SOURCE".to_owned())
        );
    }

    #[test]
    fn repository_groups_never_cross_split_components() {
        let documents = vec![
            document("first", "shared", b"def first():\n    return 1\n"),
            document("second", "shared", b"def second():\n    return 2\n"),
            document("third", "other", b"def third():\n    return 3\n"),
        ];
        let (_, groups) = deduplicate(&documents, 100).unwrap();
        let (manifest, assignments) = assign_splits(&documents, &groups, &BTreeSet::new()).unwrap();
        let shared = assignments
            .iter()
            .filter(|assignment| {
                documents[assignment.document_index].repository_group_id == hash("shared")
            })
            .collect::<Vec<_>>();
        assert_eq!(shared.len(), 2);
        assert_eq!(shared[0].component_id, shared[1].component_id);
        assert_eq!(shared[0].split, shared[1].split);
        assert_eq!(manifest.components.len(), 2);
    }

    #[test]
    fn fragment_wrapper_removes_only_wrapper_tokens() {
        let fragment = b"assert value == 7\n";
        let tokens = fragment_tokens(fragment, &CancellationToken::default()).unwrap();
        let module = document("wrapped", "repo", b"def holder():\n    assert value == 7\n");
        assert!(
            module
                .encoded_tokens
                .windows(tokens.len())
                .any(|window| window == tokens)
        );
    }

    #[test]
    fn span_order_is_stable_complete_and_partial_last() {
        let decisions = hash("decisions");
        let corpus = hash("corpus");
        let first = build_span_order(10_242, &decisions, &corpus).unwrap();
        let second = build_span_order(10_242, &decisions, &corpus).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.complete_span_count, 5);
        assert_eq!(first.partial_span.as_ref().unwrap().valid_targets, 1);
        assert_eq!(first.ordered_spans.last().unwrap().valid_targets, 1);
        let offsets = first
            .ordered_spans
            .iter()
            .filter(|span| span.valid_targets == TARGET_SPAN)
            .map(|span| span.first_target_offset)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            offsets,
            [0, 2_048, 4_096, 6_144, 8_192]
                .into_iter()
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn production_span_arithmetic_preserves_every_complete_span_once() {
        let manifest =
            build_span_order(2_000_000_001, &hash("decisions"), &hash("corpus")).unwrap();
        assert_eq!(manifest.valid_targets, 2_000_000_000);
        assert_eq!(manifest.complete_span_count, 976_562);
        assert_eq!(
            manifest.partial_span,
            Some(PartialSpanV1 {
                first_target_offset: 1_999_998_976,
                valid_targets: 1_024,
                runtime_padding_targets: 1_024,
            })
        );
        assert_eq!(manifest.ordered_spans.len(), 976_563);
        assert_eq!(
            manifest.ordered_spans.last().unwrap().first_target_offset,
            1_999_998_976
        );
    }
}
