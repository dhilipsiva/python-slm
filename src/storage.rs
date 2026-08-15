//! Deterministic, immutable token-corpus materialization and verified reading.

use crate::backend::PROTOTYPE_PROFILE;
use crate::data::source::{
    compact_json_line, is_sha256, join_relative, parse_closed, read_control_file,
    read_stable_document, require_contained_regular_file, require_existing_root,
    require_output_boundary, require_portable_relative_path, sha256,
};
use crate::error::{ProductError, Result};
use crate::tokenizer::{
    ARTIFACT_SCHEMA, ByteBpeTokenizer, EOS_ID, HashBoundInput, MAX_TOKEN_ID,
    SAMPLE_MANIFEST_SCHEMA, TokenizerSampleManifestV1,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const IMPLEMENTATION_PHASE: &str = "P8";
pub const HASH_ALGORITHM: &str = "sha256";
pub const CONFIG_SCHEMA: &str = "python-slm-token-materialize-config-v1";
pub const CORPUS_MANIFEST_SCHEMA: &str = "python-slm-governed-corpus-manifest-v1";
pub const GENERATION_SCHEMA: &str = "python-slm-token-corpus-generation-v1";
pub const DOCUMENT_INDEX_SCHEMA: &str = "python-slm-token-document-index-v1";
pub const SEQUENCE_INDEX_SCHEMA: &str = "python-slm-token-sequence-index-v1";
pub const RESULT_SCHEMA: &str = "python-slm-tokenize-result-v1";
pub const TRAINING_PREFIX_IDS: u64 = 2_000_000_001;
pub const TRAINING_TARGETS: u64 = 2_000_000_000;
pub const SEQUENCE_TARGETS: u64 = 2_048;
const MAX_SHARD_IDS: u64 = 67_108_864;
static PARTIAL_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenMaterializeConfigV1 {
    pub schema: String,
    pub profile: String,
    pub corpus_manifest: HashBoundInput,
    pub content_root: PathBuf,
    pub tokenizer_sample_manifest: HashBoundInput,
    pub tokenizer_artifact: HashBoundInput,
    pub output_root: PathBuf,
    pub limits: MaterializationLimits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializationLimits {
    pub maximum_documents: u64,
    pub maximum_total_canonical_bytes: u64,
    pub maximum_total_stored_ids: u64,
    pub shard_maximum_ids: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedCorpusManifestV1 {
    pub schema: String,
    pub source_generation_manifest_sha256: String,
    pub split_manifest_sha256: String,
    pub tokenizer_sample_manifest_sha256: String,
    pub tokenizer_artifact_sha256: String,
    pub documents: Vec<GovernedCorpusDocumentV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedCorpusDocumentV1 {
    pub component_id: String,
    pub repository_group_id: String,
    pub source_id: String,
    pub curated_sha256_raw: String,
    pub canonical_sha256: String,
    pub canonical_bytes: u64,
    pub relative_path: String,
    pub split: CorpusSplit,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CorpusSplit {
    Train,
    Validation,
    Test,
}

impl CorpusSplit {
    fn name(self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Validation => "validation",
            Self::Test => "test",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenShardRef {
    pub path: String,
    pub split: CorpusSplit,
    pub sequence: u64,
    pub first_id: u64,
    pub ids: u64,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SplitAccounting {
    pub split: CorpusSplit,
    pub materialized_documents: u64,
    pub source_bytes: u64,
    pub source_token_ids: u64,
    pub eos_ids: u64,
    pub stored_ids: u64,
    pub valid_targets: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusAccounting {
    pub total_input_documents: u64,
    pub total_input_canonical_bytes: u64,
    pub total_materialized_documents: u64,
    pub total_stored_ids: u64,
    pub training_prefix_ids: u64,
    pub training_valid_targets: u64,
    pub training_unused_tail_ids: u64,
    pub unmaterialized_training_documents: u64,
    pub unmaterialized_training_bytes: u64,
    pub training_target_satisfied: bool,
    pub stored_pad_ids: u64,
    pub stored_bos_ids: u64,
    pub stored_unk_ids: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenCorpusGenerationV1 {
    pub schema: String,
    pub profile: String,
    pub qualification_status: String,
    pub hash_algorithm: String,
    pub token_encoding: String,
    pub corpus_manifest: ArtifactRef,
    pub tokenizer_sample_manifest: ArtifactRef,
    pub tokenizer_artifact: ArtifactRef,
    pub source_generation_manifest_sha256: String,
    pub split_manifest_sha256: String,
    pub document_index: ArtifactRef,
    pub sequence_index: ArtifactRef,
    pub shards: Vec<TokenShardRef>,
    pub streams: Vec<SplitAccounting>,
    pub accounting: CorpusAccounting,
    pub limits: MaterializationLimits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenDocumentIndexV1 {
    pub schema: String,
    pub entries: Vec<TokenDocumentEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenDocumentEntry {
    pub split: CorpusSplit,
    pub order: u64,
    pub component_id: String,
    pub repository_group_id: String,
    pub source_id: String,
    pub curated_sha256_raw: String,
    pub canonical_sha256: String,
    pub canonical_bytes: u64,
    pub first_id: u64,
    pub source_token_ids: u64,
    pub stored_ids: u64,
    pub eos_id: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenSequenceIndexV1 {
    pub schema: String,
    pub target_span: u64,
    pub entries: Vec<TokenSequenceEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenSequenceEntry {
    pub split: CorpusSplit,
    pub sequence: u64,
    pub first_id: u64,
    pub logical_ids: u64,
    pub valid_targets: u64,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct TokenizeResult {
    schema: &'static str,
    status: &'static str,
    qualification_status: &'static str,
    profile: &'static str,
    generation_manifest_sha256: String,
    input_documents: u64,
    materialized_documents: u64,
    stored_ids: u64,
    training_prefix_ids: u64,
    training_target_satisfied: bool,
    output_created: bool,
    receipts_written: bool,
}

pub fn tokenize(config_path: &Path) -> Result<Value> {
    #[cfg(not(windows))]
    {
        let _ = config_path;
        Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "tokenize is implemented only for the prototype Windows host",
        ))
    }
    #[cfg(windows)]
    {
        tokenize_windows(config_path)
    }
}

#[cfg(windows)]
fn tokenize_windows(config_path: &Path) -> Result<Value> {
    let config_bytes = read_control_file(config_path, None, "TOKEN_CONFIG_READ_FAILED")?;
    let config: TokenMaterializeConfigV1 = parse_closed(&config_bytes, "TOKEN_CONFIG_INVALID")?;
    validate_config(&config)?;

    let corpus_bytes = read_control_file(
        &config.corpus_manifest.path,
        Some(&config.corpus_manifest.sha256),
        "CORPUS_MANIFEST_READ_FAILED",
    )?;
    let corpus = parse_governed_corpus(&corpus_bytes, &config)?;
    validate_corpus(&corpus, &config)?;

    let sample_bytes = read_control_file(
        &config.tokenizer_sample_manifest.path,
        Some(&config.tokenizer_sample_manifest.sha256),
        "TOKENIZER_SAMPLE_READ_FAILED",
    )?;
    let sample: TokenizerSampleManifestV1 =
        parse_closed(&sample_bytes, "TOKENIZER_SAMPLE_INVALID")?;
    let artifact_bytes = read_control_file(
        &config.tokenizer_artifact.path,
        Some(&config.tokenizer_artifact.sha256),
        "TOKENIZER_ARTIFACT_READ_FAILED",
    )?;
    let tokenizer = ByteBpeTokenizer::from_artifact_bytes(&artifact_bytes)?;
    validate_bindings(&corpus, &sample, &tokenizer, &config)?;

    let content_root = require_existing_root(&config.content_root, "CONTENT_ROOT_INVALID")?;
    require_output_boundary(&config.output_root, &content_root)?;
    let mut generation = PartialTokenGeneration::create(&config.output_root)?;
    for directory in ["inputs", "indexes", "shards"] {
        generation.create_directory(directory)?;
    }
    generation.write_file(Path::new("inputs/corpus-manifest.json"), &corpus_bytes)?;
    generation.write_file(
        Path::new("inputs/tokenizer-sample-manifest.json"),
        &sample_bytes,
    )?;
    generation.write_file(Path::new("inputs/tokenizer.json"), &artifact_bytes)?;

    let mut documents = corpus.documents.clone();
    documents.sort_by(|left, right| {
        left.split.cmp(&right.split).then_with(|| {
            (
                left.component_id.as_bytes(),
                left.repository_group_id.as_bytes(),
                left.source_id.as_bytes(),
                left.curated_sha256_raw.as_bytes(),
            )
                .cmp(&(
                    right.component_id.as_bytes(),
                    right.repository_group_id.as_bytes(),
                    right.source_id.as_bytes(),
                    right.curated_sha256_raw.as_bytes(),
                ))
        })
    });

    let mut writers = BTreeMap::new();
    for split in [
        CorpusSplit::Train,
        CorpusSplit::Validation,
        CorpusSplit::Test,
    ] {
        writers.insert(
            split,
            ShardWriter::new(split, config.limits.shard_maximum_ids),
        );
    }
    let mut document_entries = Vec::new();
    let mut total_stored_ids = 0_u64;
    let mut unmaterialized_training_documents = 0_u64;
    let mut unmaterialized_training_bytes = 0_u64;
    let mut train_closed = false;

    for document in &documents {
        if document.split == CorpusSplit::Train && train_closed {
            unmaterialized_training_documents += 1;
            unmaterialized_training_bytes = unmaterialized_training_bytes
                .checked_add(document.canonical_bytes)
                .ok_or_else(accounting_overflow)?;
            continue;
        }
        let path = join_relative(&content_root, &document.relative_path)?;
        let metadata = require_contained_regular_file(&content_root, &path)?;
        if metadata.len() != document.canonical_bytes {
            return Err(ProductError::integrity(
                "CORPUS_DOCUMENT_LENGTH_MISMATCH",
                "a corpus document length differs from its declared identity",
            ));
        }
        let bytes = read_stable_document(&path, &metadata)?;
        if sha256(&bytes) != document.canonical_sha256 {
            return Err(ProductError::integrity(
                "CORPUS_DOCUMENT_HASH_MISMATCH",
                "a corpus document hash differs from its declared identity",
            ));
        }
        let encoded = tokenizer.encode(&bytes);
        if encoded.is_empty()
            || encoded.iter().any(|id| *id <= 3 || *id > MAX_TOKEN_ID)
            || tokenizer.decode_source(&encoded)? != bytes
        {
            return Err(ProductError::integrity(
                "TOKEN_ROUND_TRIP_FAILED",
                "a governed document did not round-trip through the tokenizer",
            ));
        }
        let stored = encoded.len() as u64 + 1;
        total_stored_ids = total_stored_ids
            .checked_add(stored)
            .ok_or_else(accounting_overflow)?;
        if total_stored_ids > config.limits.maximum_total_stored_ids {
            return Err(ProductError::gate(
                "TOKEN_CAPACITY_EXCEEDED",
                "materialized token IDs exceed the explicit configured capacity",
            ));
        }
        let writer = writers.get_mut(&document.split).expect("all splits exist");
        let first_id = writer.stored_ids;
        let order = writer.documents;
        writer.append(&encoded, &mut generation)?;
        writer.append(&[EOS_ID], &mut generation)?;
        writer.documents += 1;
        writer.source_bytes += document.canonical_bytes;
        writer.source_token_ids += encoded.len() as u64;
        document_entries.push(TokenDocumentEntry {
            split: document.split,
            order,
            component_id: document.component_id.clone(),
            repository_group_id: document.repository_group_id.clone(),
            source_id: document.source_id.clone(),
            curated_sha256_raw: document.curated_sha256_raw.clone(),
            canonical_sha256: document.canonical_sha256.clone(),
            canonical_bytes: document.canonical_bytes,
            first_id,
            source_token_ids: encoded.len() as u64,
            stored_ids: stored,
            eos_id: EOS_ID,
        });
        if document.split == CorpusSplit::Train && writer.stored_ids >= TRAINING_PREFIX_IDS {
            train_closed = true;
        }
    }

    let mut shards = Vec::new();
    let mut streams = Vec::new();
    for split in [
        CorpusSplit::Train,
        CorpusSplit::Validation,
        CorpusSplit::Test,
    ] {
        let mut writer = writers.remove(&split).expect("all splits exist");
        writer.finish(&mut generation)?;
        shards.extend(writer.shards);
        streams.push(SplitAccounting {
            split,
            materialized_documents: writer.documents,
            source_bytes: writer.source_bytes,
            source_token_ids: writer.source_token_ids,
            eos_ids: writer.documents,
            stored_ids: writer.stored_ids,
            valid_targets: writer.stored_ids.saturating_sub(1),
        });
    }
    let train_ids = streams
        .iter()
        .find(|stream| stream.split == CorpusSplit::Train)
        .expect("train stream exists")
        .stored_ids;
    let training_target_satisfied = train_ids >= TRAINING_PREFIX_IDS;
    let training_prefix_ids = train_ids.min(TRAINING_PREFIX_IDS);
    let training_unused_tail_ids = train_ids - training_prefix_ids;
    let training_valid_targets = training_prefix_ids.saturating_sub(1);

    let document_index = TokenDocumentIndexV1 {
        schema: DOCUMENT_INDEX_SCHEMA.to_owned(),
        entries: document_entries,
    };
    let document_index_bytes =
        compact_json_line(&document_index, "DOCUMENT_INDEX_SERIALIZATION_FAILED")?;
    generation.write_file(Path::new("indexes/documents.json"), &document_index_bytes)?;
    let sequence_index = build_sequence_index(&streams, training_prefix_ids);
    let sequence_index_bytes =
        compact_json_line(&sequence_index, "SEQUENCE_INDEX_SERIALIZATION_FAILED")?;
    generation.write_file(Path::new("indexes/sequences.json"), &sequence_index_bytes)?;

    let total_input_canonical_bytes =
        corpus.documents.iter().try_fold(0_u64, |total, document| {
            total
                .checked_add(document.canonical_bytes)
                .ok_or_else(accounting_overflow)
        })?;
    let accounting = CorpusAccounting {
        total_input_documents: corpus.documents.len() as u64,
        total_input_canonical_bytes,
        total_materialized_documents: document_index.entries.len() as u64,
        total_stored_ids,
        training_prefix_ids,
        training_valid_targets,
        training_unused_tail_ids,
        unmaterialized_training_documents,
        unmaterialized_training_bytes,
        training_target_satisfied,
        stored_pad_ids: 0,
        stored_bos_ids: 0,
        stored_unk_ids: 0,
    };
    let manifest = TokenCorpusGenerationV1 {
        schema: GENERATION_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        hash_algorithm: HASH_ALGORITHM.to_owned(),
        token_encoding: "u16le-v1".to_owned(),
        corpus_manifest: artifact_ref("inputs/corpus-manifest.json", &corpus_bytes),
        tokenizer_sample_manifest: artifact_ref(
            "inputs/tokenizer-sample-manifest.json",
            &sample_bytes,
        ),
        tokenizer_artifact: artifact_ref("inputs/tokenizer.json", &artifact_bytes),
        source_generation_manifest_sha256: corpus.source_generation_manifest_sha256,
        split_manifest_sha256: corpus.split_manifest_sha256,
        document_index: artifact_ref("indexes/documents.json", &document_index_bytes),
        sequence_index: artifact_ref("indexes/sequences.json", &sequence_index_bytes),
        shards,
        streams,
        accounting,
        limits: config.limits,
    };
    validate_manifest_relations(&manifest, &document_index, &sequence_index)?;
    let manifest_bytes = compact_json_line(&manifest, "TOKEN_MANIFEST_SERIALIZATION_FAILED")?;
    let generation_manifest_sha256 = sha256(&manifest_bytes);
    generation.write_file(Path::new("manifest.json"), &manifest_bytes)?;
    generation.publish()?;

    serde_json::to_value(TokenizeResult {
        schema: RESULT_SCHEMA,
        status: "TOKEN_CORPUS_MATERIALIZED",
        qualification_status: "SKIPPED",
        profile: PROTOTYPE_PROFILE,
        generation_manifest_sha256,
        input_documents: manifest.accounting.total_input_documents,
        materialized_documents: manifest.accounting.total_materialized_documents,
        stored_ids: manifest.accounting.total_stored_ids,
        training_prefix_ids: manifest.accounting.training_prefix_ids,
        training_target_satisfied: manifest.accounting.training_target_satisfied,
        output_created: true,
        receipts_written: false,
    })
    .map_err(|_| {
        ProductError::internal(
            "RESULT_SERIALIZATION_FAILED",
            "could not serialize the token materialization result",
        )
    })
}

fn validate_config(config: &TokenMaterializeConfigV1) -> Result<()> {
    if config.schema != CONFIG_SCHEMA {
        return Err(ProductError::usage(
            "CONFIG_SCHEMA_UNSUPPORTED",
            "the token materialization configuration schema is unsupported",
        ));
    }
    if config.profile != PROTOTYPE_PROFILE {
        return Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "the requested profile is designed but not implemented",
        ));
    }
    for input in [
        &config.corpus_manifest,
        &config.tokenizer_sample_manifest,
        &config.tokenizer_artifact,
    ] {
        if !input.path.is_absolute() || !is_sha256(&input.sha256) {
            return Err(ProductError::usage(
                "HASH_BOUND_INPUT_INVALID",
                "token materialization inputs must be absolute and hash-bound",
            ));
        }
    }
    if !config.content_root.is_absolute()
        || !config.output_root.is_absolute()
        || config.limits.maximum_documents == 0
        || config.limits.maximum_total_canonical_bytes == 0
        || config.limits.maximum_total_stored_ids == 0
        || config.limits.shard_maximum_ids == 0
        || config.limits.shard_maximum_ids > MAX_SHARD_IDS
    {
        return Err(ProductError::usage(
            "TOKEN_MATERIALIZATION_LIMIT_INVALID",
            "paths must be absolute and every explicit capacity must be positive and bounded",
        ));
    }
    Ok(())
}
fn parse_governed_corpus(
    bytes: &[u8],
    config: &TokenMaterializeConfigV1,
) -> Result<GovernedCorpusManifestV1> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| {
        ProductError::integrity(
            "CORPUS_MANIFEST_INVALID",
            "the governed corpus manifest is malformed",
        )
    })?;
    match value.get("schema").and_then(Value::as_str) {
        Some(CORPUS_MANIFEST_SCHEMA) => parse_closed(bytes, "CORPUS_MANIFEST_INVALID"),
        Some(crate::corpus::GOVERNED_CORPUS_SCHEMA) => {
            let corpus: crate::corpus::GovernedCorpusManifestV2 =
                parse_closed(bytes, "CORPUS_MANIFEST_INVALID")?;
            Ok(GovernedCorpusManifestV1 {
                schema: CORPUS_MANIFEST_SCHEMA.to_owned(),
                source_generation_manifest_sha256: corpus.source_generation_manifest_sha256,
                split_manifest_sha256: corpus.split_manifest_sha256,
                tokenizer_sample_manifest_sha256: corpus.tokenizer_sample_manifest_sha256,
                tokenizer_artifact_sha256: config.tokenizer_artifact.sha256.clone(),
                documents: corpus.documents,
            })
        }
        _ => Err(ProductError::integrity(
            "CORPUS_MANIFEST_INVALID",
            "the governed corpus manifest schema is unsupported",
        )),
    }
}

fn validate_corpus(
    corpus: &GovernedCorpusManifestV1,
    config: &TokenMaterializeConfigV1,
) -> Result<()> {
    if corpus.schema != CORPUS_MANIFEST_SCHEMA
        || !is_sha256(&corpus.source_generation_manifest_sha256)
        || !is_sha256(&corpus.split_manifest_sha256)
        || !is_sha256(&corpus.tokenizer_sample_manifest_sha256)
        || !is_sha256(&corpus.tokenizer_artifact_sha256)
        || corpus.documents.is_empty()
        || corpus.documents.len() as u64 > config.limits.maximum_documents
    {
        return Err(ProductError::integrity(
            "CORPUS_MANIFEST_INVALID",
            "the governed corpus manifest violates its closed identity or document bounds",
        ));
    }
    let mut sources = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut total_bytes = 0_u64;
    for document in &corpus.documents {
        require_portable_relative_path(&document.relative_path, "CORPUS_PATH_INVALID")?;
        if !is_sha256(&document.component_id)
            || !is_sha256(&document.repository_group_id)
            || !is_sha256(&document.source_id)
            || !is_sha256(&document.curated_sha256_raw)
            || !is_sha256(&document.canonical_sha256)
            || document.canonical_bytes == 0
        {
            return Err(ProductError::integrity(
                "CORPUS_DOCUMENT_IDENTITY_INVALID",
                "a governed corpus document has an invalid identity or length",
            ));
        }
        if !sources.insert(document.source_id.clone())
            || !paths.insert(document.relative_path.clone())
        {
            return Err(ProductError::integrity(
                "CORPUS_DOCUMENT_DUPLICATE",
                "the governed corpus repeats a source identity or content path",
            ));
        }
        total_bytes = total_bytes
            .checked_add(document.canonical_bytes)
            .ok_or_else(accounting_overflow)?;
        if total_bytes > config.limits.maximum_total_canonical_bytes {
            return Err(ProductError::gate(
                "CORPUS_BYTE_CAPACITY_EXCEEDED",
                "declared canonical bytes exceed the explicit configured capacity",
            ));
        }
    }
    Ok(())
}

fn validate_bindings(
    corpus: &GovernedCorpusManifestV1,
    sample: &TokenizerSampleManifestV1,
    tokenizer: &ByteBpeTokenizer,
    config: &TokenMaterializeConfigV1,
) -> Result<()> {
    let artifact = tokenizer.artifact();
    if sample.schema != SAMPLE_MANIFEST_SCHEMA
        || artifact.schema != ARTIFACT_SCHEMA
        || corpus.tokenizer_sample_manifest_sha256 != config.tokenizer_sample_manifest.sha256
        || corpus.tokenizer_artifact_sha256 != config.tokenizer_artifact.sha256
        || artifact.sample.manifest_sha256 != config.tokenizer_sample_manifest.sha256
        || sample.source_generation_manifest_sha256 != corpus.source_generation_manifest_sha256
        || artifact.sample.source_generation_manifest_sha256
            != corpus.source_generation_manifest_sha256
    {
        return Err(ProductError::integrity(
            "TOKEN_INPUT_BINDING_MISMATCH",
            "the corpus, tokenizer sample, tokenizer artifact, and source generation disagree",
        ));
    }
    Ok(())
}

fn artifact_ref(path: &str, bytes: &[u8]) -> ArtifactRef {
    ArtifactRef {
        path: path.to_owned(),
        sha256: sha256(bytes),
        bytes: bytes.len() as u64,
    }
}

fn accounting_overflow() -> ProductError {
    ProductError::integrity(
        "TOKEN_ACCOUNTING_OVERFLOW",
        "token corpus byte or ID accounting overflowed",
    )
}

fn build_sequence_index(
    streams: &[SplitAccounting],
    training_prefix_ids: u64,
) -> TokenSequenceIndexV1 {
    let mut entries = Vec::new();
    for stream in streams {
        let usable_ids = if stream.split == CorpusSplit::Train {
            training_prefix_ids
        } else {
            stream.stored_ids
        };
        let mut first_id = 0_u64;
        let mut sequence = 0_u64;
        while first_id + 1 < usable_ids {
            let valid_targets = (usable_ids - first_id - 1).min(SEQUENCE_TARGETS);
            entries.push(TokenSequenceEntry {
                split: stream.split,
                sequence,
                first_id,
                logical_ids: valid_targets + 1,
                valid_targets,
            });
            first_id += valid_targets;
            sequence += 1;
        }
    }
    TokenSequenceIndexV1 {
        schema: SEQUENCE_INDEX_SCHEMA.to_owned(),
        target_span: SEQUENCE_TARGETS,
        entries,
    }
}

fn validate_manifest_relations(
    manifest: &TokenCorpusGenerationV1,
    documents: &TokenDocumentIndexV1,
    sequences: &TokenSequenceIndexV1,
) -> Result<()> {
    let shard_ids = manifest.shards.iter().try_fold(0_u64, |total, shard| {
        total.checked_add(shard.ids).ok_or_else(accounting_overflow)
    })?;
    let stream_ids = manifest.streams.iter().try_fold(0_u64, |total, stream| {
        total
            .checked_add(stream.stored_ids)
            .ok_or_else(accounting_overflow)
    })?;
    let stream_source_bytes = manifest.streams.iter().try_fold(0_u64, |total, stream| {
        total
            .checked_add(stream.source_bytes)
            .ok_or_else(accounting_overflow)
    })?;
    let stream_splits = manifest
        .streams
        .iter()
        .map(|stream| stream.split)
        .collect::<BTreeSet<_>>();
    let train = manifest
        .streams
        .iter()
        .find(|stream| stream.split == CorpusSplit::Train);
    let streams_valid = manifest.streams.len() == 3
        && stream_splits.len() == 3
        && manifest.streams.iter().all(|stream| {
            stream.source_token_ids.checked_add(stream.eos_ids) == Some(stream.stored_ids)
                && stream.eos_ids == stream.materialized_documents
                && stream.valid_targets == stream.stored_ids.saturating_sub(1)
        });
    if manifest.schema != GENERATION_SCHEMA
        || manifest.profile != PROTOTYPE_PROFILE
        || manifest.qualification_status != "SKIPPED"
        || manifest.hash_algorithm != HASH_ALGORITHM
        || manifest.token_encoding != "u16le-v1"
        || documents.schema != DOCUMENT_INDEX_SCHEMA
        || sequences.schema != SEQUENCE_INDEX_SCHEMA
        || sequences.target_span != SEQUENCE_TARGETS
        || !streams_valid
        || shard_ids != manifest.accounting.total_stored_ids
        || stream_ids != manifest.accounting.total_stored_ids
        || manifest.accounting.total_stored_ids > manifest.limits.maximum_total_stored_ids
        || manifest.accounting.total_input_documents > manifest.limits.maximum_documents
        || manifest.accounting.total_input_canonical_bytes
            > manifest.limits.maximum_total_canonical_bytes
        || manifest
            .accounting
            .total_materialized_documents
            .checked_add(manifest.accounting.unmaterialized_training_documents)
            != Some(manifest.accounting.total_input_documents)
        || stream_source_bytes.checked_add(manifest.accounting.unmaterialized_training_bytes)
            != Some(manifest.accounting.total_input_canonical_bytes)
        || train.is_none_or(|stream| {
            manifest.accounting.training_prefix_ids != stream.stored_ids.min(TRAINING_PREFIX_IDS)
                || stream
                    .stored_ids
                    .checked_sub(manifest.accounting.training_prefix_ids)
                    != Some(manifest.accounting.training_unused_tail_ids)
        })
        || documents.entries.len() as u64 != manifest.accounting.total_materialized_documents
        || manifest.accounting.stored_pad_ids != 0
        || manifest.accounting.stored_bos_ids != 0
        || manifest.accounting.stored_unk_ids != 0
        || manifest.accounting.training_valid_targets
            != manifest.accounting.training_prefix_ids.saturating_sub(1)
        || manifest.accounting.training_target_satisfied
            != (manifest.accounting.training_prefix_ids == TRAINING_PREFIX_IDS)
    {
        return Err(ProductError::integrity(
            "TOKEN_MANIFEST_RELATION_INVALID",
            "token corpus manifest accounting or fixed constants disagree",
        ));
    }
    Ok(())
}

struct ShardWriter {
    split: CorpusSplit,
    maximum_ids: u64,
    buffer: Vec<u16>,
    stored_ids: u64,
    flushed_ids: u64,
    documents: u64,
    source_bytes: u64,
    source_token_ids: u64,
    shards: Vec<TokenShardRef>,
}

impl ShardWriter {
    fn new(split: CorpusSplit, maximum_ids: u64) -> Self {
        Self {
            split,
            maximum_ids,
            buffer: Vec::with_capacity(maximum_ids.min(1_048_576) as usize),
            stored_ids: 0,
            flushed_ids: 0,
            documents: 0,
            source_bytes: 0,
            source_token_ids: 0,
            shards: Vec::new(),
        }
    }

    fn append(&mut self, ids: &[u32], generation: &mut PartialTokenGeneration) -> Result<()> {
        for id in ids {
            let value = u16::try_from(*id).map_err(|_| {
                ProductError::integrity(
                    "TOKEN_ID_OUT_OF_RANGE",
                    "a token ID does not fit the immutable u16 representation",
                )
            })?;
            self.buffer.push(value);
            self.stored_ids += 1;
            if self.buffer.len() as u64 == self.maximum_ids {
                self.flush(generation)?;
            }
        }
        Ok(())
    }

    fn flush(&mut self, generation: &mut PartialTokenGeneration) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let sequence = self.shards.len() as u64;
        let relative = format!("shards/{}-{sequence:08}.u16le", self.split.name());
        let mut bytes = Vec::with_capacity(self.buffer.len() * 2);
        for id in &self.buffer {
            bytes.extend_from_slice(&id.to_le_bytes());
        }
        generation.write_file(Path::new(&relative), &bytes)?;
        self.shards.push(TokenShardRef {
            path: relative,
            split: self.split,
            sequence,
            first_id: self.flushed_ids,
            ids: self.buffer.len() as u64,
            bytes: bytes.len() as u64,
            sha256: sha256(&bytes),
        });
        self.flushed_ids += self.buffer.len() as u64;
        self.buffer.clear();
        Ok(())
    }

    fn finish(&mut self, generation: &mut PartialTokenGeneration) -> Result<()> {
        self.flush(generation)?;
        if self.flushed_ids != self.stored_ids {
            return Err(ProductError::internal(
                "TOKEN_SHARD_ACCOUNTING_FAILED",
                "flushed shard IDs disagree with the encoded stream",
            ));
        }
        Ok(())
    }
}

struct PartialTokenGeneration {
    final_path: PathBuf,
    partial_path: PathBuf,
    published: bool,
}

impl PartialTokenGeneration {
    fn create(final_path: &Path) -> Result<Self> {
        let parent = final_path.parent().ok_or_else(|| {
            ProductError::usage("OUTPUT_ROOT_INVALID", "the output root has no parent")
        })?;
        let leaf = final_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                ProductError::usage(
                    "OUTPUT_ROOT_INVALID",
                    "the output root has no portable final component",
                )
            })?;
        for _ in 0..64 {
            let sequence = PARTIAL_COUNTER.fetch_add(1, Ordering::Relaxed);
            let partial_path = parent.join(format!(
                ".{leaf}.p8-partial-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&partial_path) {
                Ok(()) => {
                    return Ok(Self {
                        final_path: final_path.to_owned(),
                        partial_path,
                        published: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    return Err(ProductError::environment(
                        "OUTPUT_PARTIAL_CREATE_FAILED",
                        "could not create a unique token-generation partial directory",
                    ));
                }
            }
        }
        Err(ProductError::environment(
            "OUTPUT_PARTIAL_CREATE_FAILED",
            "could not allocate a unique token-generation partial directory",
        ))
    }

    fn create_directory(&mut self, relative: &str) -> Result<()> {
        require_portable_relative_path(relative, "OUTPUT_PATH_INVALID")?;
        fs::create_dir(self.partial_path.join(relative)).map_err(|_| {
            ProductError::environment(
                "OUTPUT_DIRECTORY_CREATE_FAILED",
                "could not create a token-generation directory",
            )
        })
    }

    fn write_file(&mut self, relative: &Path, bytes: &[u8]) -> Result<()> {
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(ProductError::internal(
                "OUTPUT_PATH_INVALID",
                "an internal token-generation path is not contained",
            ));
        }
        let path = self.partial_path.join(relative);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| {
                ProductError::environment(
                    "OUTPUT_FILE_CREATE_FAILED",
                    "could not create an immutable token-generation file",
                )
            })?;
        file.write_all(bytes).map_err(|_| {
            ProductError::environment(
                "OUTPUT_FILE_WRITE_FAILED",
                "could not write a token-generation file",
            )
        })?;
        file.sync_all().map_err(|_| {
            ProductError::environment(
                "OUTPUT_FILE_SYNC_FAILED",
                "could not sync a token-generation file",
            )
        })
    }

    fn publish(&mut self) -> Result<()> {
        if self.final_path.exists() {
            return Err(ProductError::integrity(
                "OUTPUT_ALREADY_EXISTS",
                "the create-new token generation appeared before publication",
            ));
        }
        #[cfg(windows)]
        publish_directory_windows(&self.partial_path, &self.final_path)?;
        #[cfg(not(windows))]
        fs::rename(&self.partial_path, &self.final_path).map_err(|_| {
            ProductError::environment(
                "OUTPUT_PUBLISH_FAILED",
                "could not publish the token generation",
            )
        })?;
        self.published = true;
        Ok(())
    }
}

impl Drop for PartialTokenGeneration {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.partial_path);
        }
    }
}

#[cfg(windows)]
fn publish_directory_windows(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let from = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let to = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both UTF-16 buffers are NUL-terminated and live for this call.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(ProductError::environment(
            "OUTPUT_PUBLISH_FAILED",
            "could not atomically publish the create-new token generation",
        ));
    }
    Ok(())
}

/// A fully hash-verified token generation. Sequence reads revalidate backing shards.
pub struct VerifiedTokenCorpus {
    root: PathBuf,
    pub manifest: TokenCorpusGenerationV1,
    pub documents: TokenDocumentIndexV1,
    pub sequences: TokenSequenceIndexV1,
}

impl VerifiedTokenCorpus {
    pub fn open(root: &Path) -> Result<Self> {
        let root = require_existing_root(root, "TOKEN_GENERATION_ROOT_INVALID")?;
        let manifest_bytes = read_generation_file(&root, "manifest.json", None)?;
        let manifest: TokenCorpusGenerationV1 =
            parse_closed(&manifest_bytes, "TOKEN_MANIFEST_INVALID")?;
        let document_bytes = read_generation_file(
            &root,
            &manifest.document_index.path,
            Some(&manifest.document_index),
        )?;
        let documents: TokenDocumentIndexV1 =
            parse_closed(&document_bytes, "DOCUMENT_INDEX_INVALID")?;
        let sequence_bytes = read_generation_file(
            &root,
            &manifest.sequence_index.path,
            Some(&manifest.sequence_index),
        )?;
        let sequences: TokenSequenceIndexV1 =
            parse_closed(&sequence_bytes, "SEQUENCE_INDEX_INVALID")?;
        validate_manifest_relations(&manifest, &documents, &sequences)?;
        for reference in [
            &manifest.corpus_manifest,
            &manifest.tokenizer_sample_manifest,
            &manifest.tokenizer_artifact,
        ] {
            read_generation_file(&root, &reference.path, Some(reference))?;
        }
        let mut eos_positions: BTreeMap<CorpusSplit, BTreeSet<u64>> = BTreeMap::new();
        for split in [
            CorpusSplit::Train,
            CorpusSplit::Validation,
            CorpusSplit::Test,
        ] {
            eos_positions.insert(split, BTreeSet::new());
        }
        for document in &documents.entries {
            let eos_position = document
                .first_id
                .checked_add(document.source_token_ids)
                .ok_or_else(accounting_overflow)?;
            eos_positions
                .get_mut(&document.split)
                .expect("all splits exist")
                .insert(eos_position);
        }
        for shard in &manifest.shards {
            let reference = ArtifactRef {
                path: shard.path.clone(),
                sha256: shard.sha256.clone(),
                bytes: shard.bytes,
            };
            let bytes = read_generation_file(&root, &shard.path, Some(&reference))?;
            validate_shard_bytes(shard, &bytes)?;
            validate_shard_eos(
                shard,
                &bytes,
                eos_positions.get(&shard.split).expect("all splits exist"),
            )?;
        }
        validate_indexes(&manifest, &documents, &sequences)?;
        Ok(Self {
            root,
            manifest,
            documents,
            sequences,
        })
    }

    pub fn read_sequence(&self, split: CorpusSplit, sequence: u64) -> Result<Vec<u16>> {
        let entry = self
            .sequences
            .entries
            .iter()
            .find(|entry| entry.split == split && entry.sequence == sequence)
            .ok_or_else(|| {
                ProductError::usage(
                    "TOKEN_SEQUENCE_NOT_FOUND",
                    "the requested token sequence is not present",
                )
            })?;
        self.read_range(split, entry.first_id, entry.logical_ids)
    }

    pub fn read_document_tokens(&self, source_id: &str) -> Result<Vec<u16>> {
        let entry = self
            .documents
            .entries
            .iter()
            .find(|entry| entry.source_id == source_id)
            .ok_or_else(|| {
                ProductError::usage(
                    "TOKEN_DOCUMENT_NOT_FOUND",
                    "the requested tokenized document is not present",
                )
            })?;
        let tokens = self.read_range(entry.split, entry.first_id, entry.stored_ids)?;
        if tokens.last().copied() != Some(EOS_ID as u16) {
            return Err(ProductError::integrity(
                "TOKEN_DOCUMENT_EOS_MISSING",
                "the requested tokenized document is not terminated by exactly one EOS",
            ));
        }
        Ok(tokens)
    }

    fn read_range(&self, split: CorpusSplit, first_id: u64, ids: u64) -> Result<Vec<u16>> {
        let end = first_id.checked_add(ids).ok_or_else(accounting_overflow)?;
        let capacity = usize::try_from(ids).map_err(|_| {
            ProductError::gate(
                "TOKEN_SEQUENCE_TOO_LARGE",
                "the requested sequence cannot fit the host address space",
            )
        })?;
        let mut output = Vec::with_capacity(capacity);
        for shard in self
            .manifest
            .shards
            .iter()
            .filter(|shard| shard.split == split)
        {
            let shard_end = shard
                .first_id
                .checked_add(shard.ids)
                .ok_or_else(accounting_overflow)?;
            let overlap_start = first_id.max(shard.first_id);
            let overlap_end = end.min(shard_end);
            if overlap_start >= overlap_end {
                continue;
            }
            let reference = ArtifactRef {
                path: shard.path.clone(),
                sha256: shard.sha256.clone(),
                bytes: shard.bytes,
            };
            let bytes = read_generation_file(&self.root, &shard.path, Some(&reference))?;
            validate_shard_bytes(shard, &bytes)?;
            let start = ((overlap_start - shard.first_id) * 2) as usize;
            let finish = ((overlap_end - shard.first_id) * 2) as usize;
            output.extend(
                bytes[start..finish]
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]])),
            );
        }
        if output.len() as u64 != ids {
            return Err(ProductError::integrity(
                "TOKEN_SEQUENCE_INCOMPLETE",
                "the verified shards do not cover the requested token sequence",
            ));
        }
        Ok(output)
    }
}

fn read_generation_file(
    root: &Path,
    relative: &str,
    expected: Option<&ArtifactRef>,
) -> Result<Vec<u8>> {
    require_portable_relative_path(relative, "TOKEN_ARTIFACT_PATH_INVALID")?;
    let path = join_relative(root, relative)?;
    let metadata = require_contained_regular_file(root, &path)?;
    let bytes = read_stable_document(&path, &metadata)?;
    if expected.is_some_and(|reference| {
        reference.path != relative
            || reference.bytes != bytes.len() as u64
            || reference.sha256 != sha256(&bytes)
    }) {
        return Err(ProductError::integrity(
            "TOKEN_ARTIFACT_IDENTITY_MISMATCH",
            "a token-generation file differs from its manifest identity",
        ));
    }
    Ok(bytes)
}

fn validate_shard_bytes(shard: &TokenShardRef, bytes: &[u8]) -> Result<()> {
    if shard.ids.checked_mul(2) != Some(shard.bytes)
        || bytes.len() as u64 != shard.bytes
        || !bytes.len().is_multiple_of(2)
        || bytes.chunks_exact(2).any(|pair| {
            let id = u16::from_le_bytes([pair[0], pair[1]]) as u32;
            id > MAX_TOKEN_ID || matches!(id, 0 | 1 | 3)
        })
    {
        return Err(ProductError::integrity(
            "TOKEN_SHARD_INVALID",
            "a token shard violates its u16 length or token-ID contract",
        ));
    }
    Ok(())
}

fn validate_shard_eos(
    shard: &TokenShardRef,
    bytes: &[u8],
    eos_positions: &BTreeSet<u64>,
) -> Result<()> {
    for (offset, pair) in bytes.chunks_exact(2).enumerate() {
        let position = shard
            .first_id
            .checked_add(offset as u64)
            .ok_or_else(accounting_overflow)?;
        let id = u16::from_le_bytes([pair[0], pair[1]]) as u32;
        if (id == EOS_ID) != eos_positions.contains(&position) {
            return Err(ProductError::integrity(
                "TOKEN_DOCUMENT_BOUNDARY_INVALID",
                "EOS tokens do not exactly match the immutable document index",
            ));
        }
    }
    Ok(())
}

fn validate_indexes(
    manifest: &TokenCorpusGenerationV1,
    documents: &TokenDocumentIndexV1,
    sequences: &TokenSequenceIndexV1,
) -> Result<()> {
    for split in [
        CorpusSplit::Train,
        CorpusSplit::Validation,
        CorpusSplit::Test,
    ] {
        let stream = manifest
            .streams
            .iter()
            .find(|stream| stream.split == split)
            .ok_or_else(|| {
                ProductError::integrity(
                    "TOKEN_STREAM_MISSING",
                    "the token generation omits a required split stream",
                )
            })?;
        let mut expected_first = 0_u64;
        for (sequence, shard) in manifest
            .shards
            .iter()
            .filter(|shard| shard.split == split)
            .enumerate()
        {
            if shard.sequence != sequence as u64 || shard.first_id != expected_first {
                return Err(ProductError::integrity(
                    "TOKEN_SHARD_ORDER_INVALID",
                    "token shards are not contiguous and deterministically sequenced",
                ));
            }
            expected_first = expected_first
                .checked_add(shard.ids)
                .ok_or_else(accounting_overflow)?;
        }
        if expected_first != stream.stored_ids {
            return Err(ProductError::integrity(
                "TOKEN_SHARD_COVERAGE_INVALID",
                "token shard coverage differs from split accounting",
            ));
        }

        let mut expected_document_first = 0_u64;
        let mut indexed_source_bytes = 0_u64;
        let mut previous_key: Option<(String, String, String, String)> = None;
        for (order, document) in documents
            .entries
            .iter()
            .filter(|document| document.split == split)
            .enumerate()
        {
            let key = (
                document.component_id.clone(),
                document.repository_group_id.clone(),
                document.source_id.clone(),
                document.curated_sha256_raw.clone(),
            );
            if document.order != order as u64
                || document.first_id != expected_document_first
                || document.source_token_ids.checked_add(1) != Some(document.stored_ids)
                || document.eos_id != EOS_ID
                || previous_key
                    .as_ref()
                    .is_some_and(|previous| previous >= &key)
            {
                return Err(ProductError::integrity(
                    "TOKEN_DOCUMENT_INDEX_INVALID",
                    "token document entries are not ordered, contiguous, or EOS-terminated",
                ));
            }
            previous_key = Some(key);
            expected_document_first = expected_document_first
                .checked_add(document.stored_ids)
                .ok_or_else(accounting_overflow)?;
            indexed_source_bytes = indexed_source_bytes
                .checked_add(document.canonical_bytes)
                .ok_or_else(accounting_overflow)?;
        }
        if expected_document_first != stream.stored_ids
            || indexed_source_bytes != stream.source_bytes
        {
            return Err(ProductError::integrity(
                "TOKEN_DOCUMENT_COVERAGE_INVALID",
                "token document coverage differs from split accounting",
            ));
        }

        let usable_ids = if split == CorpusSplit::Train {
            manifest.accounting.training_prefix_ids
        } else {
            stream.stored_ids
        };
        let mut expected_first = 0_u64;
        for (expected_sequence, entry) in sequences
            .entries
            .iter()
            .filter(|entry| entry.split == split)
            .enumerate()
        {
            if entry.sequence != expected_sequence as u64
                || entry.first_id != expected_first
                || entry.logical_ids != entry.valid_targets + 1
                || entry.valid_targets == 0
                || entry.valid_targets > SEQUENCE_TARGETS
                || entry
                    .first_id
                    .checked_add(entry.logical_ids)
                    .is_none_or(|end| end > usable_ids)
            {
                return Err(ProductError::integrity(
                    "TOKEN_SEQUENCE_INDEX_INVALID",
                    "token sequence entries violate the fixed non-wrapping span contract",
                ));
            }
            expected_first = expected_first
                .checked_add(entry.valid_targets)
                .ok_or_else(accounting_overflow)?;
        }
        if expected_first != usable_ids.saturating_sub(1) {
            return Err(ProductError::integrity(
                "TOKEN_SEQUENCE_COVERAGE_INVALID",
                "token sequence targets do not exactly cover the usable stream",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_index_uses_2049_ids_for_2048_targets_without_wrap() {
        let streams = vec![SplitAccounting {
            split: CorpusSplit::Train,
            materialized_documents: 1,
            source_bytes: 1,
            source_token_ids: 4_100,
            eos_ids: 1,
            stored_ids: 4_101,
            valid_targets: 4_100,
        }];
        let index = build_sequence_index(&streams, 4_101);
        assert_eq!(index.entries.len(), 3);
        assert_eq!(index.entries[0].logical_ids, 2_049);
        assert_eq!(index.entries[1].first_id, 2_048);
        assert_eq!(index.entries[2].first_id, 4_096);
        assert_eq!(index.entries[2].valid_targets, 4);
    }

    #[test]
    fn interrupted_partial_generation_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("generation");
        let partial;
        {
            let mut generation = PartialTokenGeneration::create(&output).unwrap();
            partial = generation.partial_path.clone();
            generation.create_directory("shards").unwrap();
            generation
                .write_file(Path::new("shards/train-00000000.u16le"), &[4, 0, 2, 0])
                .unwrap();
        }
        assert!(!partial.exists());
        assert!(!output.exists());
    }

    #[test]
    fn duplicate_source_or_path_is_rejected() {
        let hash = "a".repeat(64);
        let document = GovernedCorpusDocumentV1 {
            component_id: hash.clone(),
            repository_group_id: hash.clone(),
            source_id: hash.clone(),
            curated_sha256_raw: hash.clone(),
            canonical_sha256: hash.clone(),
            canonical_bytes: 100,
            relative_path: "documents/a.py".to_owned(),
            split: CorpusSplit::Train,
        };
        let corpus = GovernedCorpusManifestV1 {
            schema: CORPUS_MANIFEST_SCHEMA.to_owned(),
            source_generation_manifest_sha256: hash.clone(),
            split_manifest_sha256: hash.clone(),
            tokenizer_sample_manifest_sha256: hash.clone(),
            tokenizer_artifact_sha256: hash,
            documents: vec![document.clone(), document],
        };
        let config = TokenMaterializeConfigV1 {
            schema: CONFIG_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            corpus_manifest: HashBoundInput {
                path: PathBuf::from("C:/corpus.json"),
                sha256: "b".repeat(64),
            },
            content_root: PathBuf::from("C:/content"),
            tokenizer_sample_manifest: HashBoundInput {
                path: PathBuf::from("C:/sample.json"),
                sha256: "c".repeat(64),
            },
            tokenizer_artifact: HashBoundInput {
                path: PathBuf::from("C:/tokenizer.json"),
                sha256: "d".repeat(64),
            },
            output_root: PathBuf::from("C:/output"),
            limits: MaterializationLimits {
                maximum_documents: 2,
                maximum_total_canonical_bytes: 1_000,
                maximum_total_stored_ids: 1_000,
                shard_maximum_ids: 100,
            },
        };
        assert_eq!(
            validate_corpus(&corpus, &config).unwrap_err().code,
            "CORPUS_DOCUMENT_DUPLICATE"
        );
    }
    #[test]
    fn p9a_v2_manifest_is_normalized_with_the_explicit_tokenizer_binding() {
        let hash = "a".repeat(64);
        let corpus = crate::corpus::GovernedCorpusManifestV2 {
            schema: crate::corpus::GOVERNED_CORPUS_SCHEMA.to_owned(),
            source_generation_manifest_sha256: hash.clone(),
            split_manifest_sha256: "b".repeat(64),
            tokenizer_sample_manifest_sha256: "c".repeat(64),
            documents: vec![GovernedCorpusDocumentV1 {
                component_id: hash.clone(),
                repository_group_id: hash.clone(),
                source_id: hash.clone(),
                curated_sha256_raw: hash.clone(),
                canonical_sha256: hash,
                canonical_bytes: 100,
                relative_path: "documents/a.py".to_owned(),
                split: CorpusSplit::Train,
            }],
        };
        let config = TokenMaterializeConfigV1 {
            schema: CONFIG_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            corpus_manifest: HashBoundInput {
                path: PathBuf::from("C:/corpus.json"),
                sha256: "d".repeat(64),
            },
            content_root: PathBuf::from("C:/content"),
            tokenizer_sample_manifest: HashBoundInput {
                path: PathBuf::from("C:/sample.json"),
                sha256: "c".repeat(64),
            },
            tokenizer_artifact: HashBoundInput {
                path: PathBuf::from("C:/tokenizer.json"),
                sha256: "e".repeat(64),
            },
            output_root: PathBuf::from("C:/output"),
            limits: MaterializationLimits {
                maximum_documents: 1,
                maximum_total_canonical_bytes: 1_000,
                maximum_total_stored_ids: 1_000,
                shard_maximum_ids: 100,
            },
        };
        let bytes = compact_json_line(&corpus, "TEST").unwrap();
        let normalized = parse_governed_corpus(&bytes, &config).unwrap();
        assert_eq!(normalized.schema, CORPUS_MANIFEST_SCHEMA);
        assert_eq!(
            normalized.tokenizer_artifact_sha256,
            config.tokenizer_artifact.sha256
        );
        assert_eq!(normalized.documents, corpus.documents);
    }
}
