//! Deterministic byte-level BPE training and source encoding.

use crate::backend::PROTOTYPE_PROFILE;
use crate::data::source::{
    compact_json_line, is_sha256, join_relative, parse_closed, read_control_file,
    read_stable_document, require_contained_regular_file, require_existing_root,
    require_output_boundary, require_portable_relative_path, sha256,
};
use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const IMPLEMENTATION_PHASE: &str = "P7";
pub const TRAIN_CONFIG_SCHEMA: &str = "python-slm-tokenizer-train-config-v1";
pub const SAMPLE_MANIFEST_SCHEMA: &str = "python-slm-tokenizer-sample-manifest-v1";
pub const ARTIFACT_SCHEMA: &str = "python-slm-byte-bpe-tokenizer-v1";
pub const TRAIN_RESULT_SCHEMA: &str = "python-slm-tokenizer-train-result-v1";
pub const VOCABULARY_SIZE: u32 = 32_000;
pub const MAX_TOKEN_ID: u32 = VOCABULARY_SIZE - 1;
pub const MIN_MERGE_FREQUENCY: u64 = 2;
pub const REPOSITORY_BYTE_CAP: u64 = 10_000_000;
pub const GLOBAL_SAMPLE_BYTE_CAP: u64 = 2_000_000_000;
pub const QUALIFIED_SAMPLE_MINIMUM: u64 = 1_999_000_000;
pub const PAD_ID: u32 = 0;
pub const BOS_ID: u32 = 1;
pub const EOS_ID: u32 = 2;
pub const UNK_ID: u32 = 3;
pub const FIRST_BYTE_ID: u32 = 4;
const BASE_VOCABULARY_SIZE: u32 = FIRST_BYTE_ID + 256;
const NONE: u32 = u32::MAX;
const SAMPLE_DOMAIN: &[u8] = b"python-slm/tokenizer-sample/v1\0";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenizerTrainConfigV1 {
    pub schema: String,
    pub profile: String,
    pub sample_manifest: HashBoundInput,
    pub content_root: PathBuf,
    pub output_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HashBoundInput {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenizerSampleManifestV1 {
    pub schema: String,
    pub source_generation_manifest_sha256: String,
    pub documents: Vec<TokenizerSampleDocumentV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenizerSampleDocumentV1 {
    pub repository_group_id: String,
    pub source_id: String,
    pub curated_sha256_raw: String,
    pub canonical_sha256: String,
    pub canonical_bytes: u64,
    pub relative_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenizerArtifactV1 {
    pub schema: String,
    pub profile: String,
    pub algorithm: String,
    pub vocabulary_size: u32,
    pub maximum_token_id: u32,
    pub special_tokens: SpecialTokenIds,
    pub byte_alphabet: ByteAlphabet,
    pub normalization: String,
    pub minimum_merge_frequency: u64,
    pub tie_breaker: String,
    pub sample: SampleBinding,
    pub merges: Vec<MergeRule>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpecialTokenIds {
    pub pad: u32,
    pub bos: u32,
    pub eos: u32,
    pub unk: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ByteAlphabet {
    pub first_id: u32,
    pub count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SampleBinding {
    pub manifest_sha256: String,
    pub source_generation_manifest_sha256: String,
    pub selected_document_count: u64,
    pub selected_bytes: u64,
    pub skipped_document_count: u64,
    pub skipped_bytes: u64,
    pub repository_byte_cap: u64,
    pub global_byte_cap: u64,
    pub qualified_minimum_bytes: u64,
    pub qualified_range_satisfied: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MergeRule {
    pub id: u32,
    pub left: u32,
    pub right: u32,
    pub training_frequency: u64,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct TrainResult {
    schema: &'static str,
    status: &'static str,
    qualification_status: &'static str,
    profile: &'static str,
    artifact_sha256: String,
    vocabulary_size: u32,
    selected_document_count: u64,
    selected_bytes: u64,
    qualified_sample_range_satisfied: bool,
    output_created: bool,
    receipts_written: bool,
}

#[derive(Clone, Debug)]
pub struct ByteBpeTokenizer {
    artifact: TokenizerArtifactV1,
    merge_ranks: HashMap<(u32, u32), u32>,
}

impl ByteBpeTokenizer {
    pub fn from_artifact_bytes(bytes: &[u8]) -> Result<Self> {
        let artifact: TokenizerArtifactV1 = parse_closed(bytes, "TOKENIZER_ARTIFACT_INVALID")?;
        validate_artifact(&artifact, true)?;
        Self::from_validated_artifact(artifact)
    }

    pub fn artifact(&self) -> &TokenizerArtifactV1 {
        &self.artifact
    }

    /// Encode bytes without recognizing or injecting special-token spellings.
    pub fn encode(&self, bytes: &[u8]) -> Vec<u32> {
        if bytes.is_empty() {
            return Vec::new();
        }
        let mut tokens = bytes
            .iter()
            .map(|byte| FIRST_BYTE_ID + u32::from(*byte))
            .collect::<Vec<_>>();
        let mut previous = (0..tokens.len())
            .map(|index| index.checked_sub(1).map_or(NONE, |value| value as u32))
            .collect::<Vec<_>>();
        let mut next = (0..tokens.len())
            .map(|index| {
                if index + 1 == tokens.len() {
                    NONE
                } else {
                    (index + 1) as u32
                }
            })
            .collect::<Vec<_>>();
        let mut candidates = BinaryHeap::new();
        for index in 0..tokens.len().saturating_sub(1) {
            push_encode_candidate(
                index as u32,
                &tokens,
                &next,
                &self.merge_ranks,
                &mut candidates,
            );
        }
        while let Some(Reverse((rank, left_index, right_index))) = candidates.pop() {
            let left = left_index as usize;
            let right = right_index as usize;
            if tokens[left] == NONE || tokens[right] == NONE || next[left] != right_index {
                continue;
            }
            if self
                .merge_ranks
                .get(&(tokens[left], tokens[right]))
                .copied()
                != Some(rank)
            {
                continue;
            }
            let before = previous[left];
            let after = next[right];
            tokens[left] = rank;
            tokens[right] = NONE;
            next[left] = after;
            if after != NONE {
                previous[after as usize] = left_index;
            }
            if before != NONE {
                push_encode_candidate(before, &tokens, &next, &self.merge_ranks, &mut candidates);
            }
            push_encode_candidate(
                left_index,
                &tokens,
                &next,
                &self.merge_ranks,
                &mut candidates,
            );
        }
        tokens.into_iter().filter(|token| *token != NONE).collect()
    }

    pub fn decode_source(&self, token_ids: &[u32]) -> Result<Vec<u8>> {
        let mut output = Vec::new();
        let mut stack = token_ids.iter().rev().copied().collect::<Vec<_>>();
        while let Some(id) = stack.pop() {
            if id < FIRST_BYTE_ID {
                return Err(ProductError::integrity(
                    "SPECIAL_TOKEN_IN_SOURCE",
                    "source decoding does not accept boundary or alignment tokens",
                ));
            }
            if id < BASE_VOCABULARY_SIZE {
                output.push((id - FIRST_BYTE_ID) as u8);
                if output.len() > 1_000_000 {
                    return Err(ProductError::gate(
                        "TOKENIZER_DECODE_LIMIT_EXCEEDED",
                        "decoded source exceeds the canonical document byte limit",
                    ));
                }
                continue;
            }
            if id >= self.artifact.vocabulary_size {
                return Err(ProductError::integrity(
                    "TOKEN_ID_OUT_OF_RANGE",
                    "a token ID exceeds the tokenizer vocabulary",
                ));
            }
            let rule = self
                .artifact
                .merges
                .get((id - BASE_VOCABULARY_SIZE) as usize)
                .ok_or_else(|| {
                    ProductError::integrity(
                        "TOKEN_ID_OUT_OF_RANGE",
                        "a token ID has no corresponding merge rule",
                    )
                })?;
            stack.push(rule.right);
            stack.push(rule.left);
            if stack.len() > 1_000_000 {
                return Err(ProductError::gate(
                    "TOKENIZER_DECODE_LIMIT_EXCEEDED",
                    "token expansion exceeds the canonical document byte limit",
                ));
            }
        }
        Ok(output)
    }

    fn from_validated_artifact(artifact: TokenizerArtifactV1) -> Result<Self> {
        let mut merge_ranks = HashMap::with_capacity(artifact.merges.len());
        for rule in &artifact.merges {
            if merge_ranks
                .insert((rule.left, rule.right), rule.id)
                .is_some()
            {
                return Err(ProductError::integrity(
                    "TOKENIZER_MERGE_DUPLICATE",
                    "the tokenizer artifact repeats a merge pair",
                ));
            }
        }
        Ok(Self {
            artifact,
            merge_ranks,
        })
    }
}

pub fn train_tokenizer(config_path: &Path) -> Result<Value> {
    #[cfg(not(windows))]
    {
        let _ = config_path;
        Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "train-tokenizer is implemented only for the prototype Windows host",
        ))
    }
    #[cfg(windows)]
    {
        train_tokenizer_windows(config_path)
    }
}

#[cfg(windows)]
fn train_tokenizer_windows(config_path: &Path) -> Result<Value> {
    let config_bytes = read_control_file(config_path, None, "TOKENIZER_CONFIG_READ_FAILED")?;
    let config: TokenizerTrainConfigV1 = parse_closed(&config_bytes, "TOKENIZER_CONFIG_INVALID")?;
    validate_config(&config)?;
    let manifest_bytes = read_control_file(
        &config.sample_manifest.path,
        Some(&config.sample_manifest.sha256),
        "TOKENIZER_SAMPLE_READ_FAILED",
    )?;
    let manifest: TokenizerSampleManifestV1 =
        parse_closed(&manifest_bytes, "TOKENIZER_SAMPLE_INVALID")?;
    validate_manifest(&manifest)?;
    let content_root =
        require_existing_root(&config.content_root, "TOKENIZER_CONTENT_ROOT_INVALID")?;
    require_output_boundary(&config.output_path, &content_root)?;
    let selected = select_documents(&manifest.documents)?;
    let corpus = read_selected_documents(&selected.documents, &content_root)?;
    let rules = train_rules(&corpus, VOCABULARY_SIZE)?;
    let sample = SampleBinding {
        manifest_sha256: sha256(&manifest_bytes),
        source_generation_manifest_sha256: manifest.source_generation_manifest_sha256,
        selected_document_count: selected.documents.len() as u64,
        selected_bytes: selected.selected_bytes,
        skipped_document_count: selected.skipped_document_count,
        skipped_bytes: selected.skipped_bytes,
        repository_byte_cap: REPOSITORY_BYTE_CAP,
        global_byte_cap: GLOBAL_SAMPLE_BYTE_CAP,
        qualified_minimum_bytes: QUALIFIED_SAMPLE_MINIMUM,
        qualified_range_satisfied: selected.selected_bytes >= QUALIFIED_SAMPLE_MINIMUM,
    };
    let artifact = artifact_from_rules(rules, sample.clone());
    validate_artifact(&artifact, true)?;
    let artifact_bytes = compact_json_line(&artifact, "TOKENIZER_ARTIFACT_SERIALIZATION_FAILED")?;
    let reloaded = ByteBpeTokenizer::from_artifact_bytes(&artifact_bytes)?;
    if reloaded.artifact() != &artifact {
        return Err(ProductError::internal(
            "TOKENIZER_RELOAD_MISMATCH",
            "the serialized tokenizer did not reload identically",
        ));
    }
    publish_new_file(&config.output_path, &artifact_bytes)?;
    serde_json::to_value(TrainResult {
        schema: TRAIN_RESULT_SCHEMA,
        status: "TOKENIZER_TRAINED",
        qualification_status: "SKIPPED",
        profile: PROTOTYPE_PROFILE,
        artifact_sha256: sha256(&artifact_bytes),
        vocabulary_size: VOCABULARY_SIZE,
        selected_document_count: sample.selected_document_count,
        selected_bytes: sample.selected_bytes,
        qualified_sample_range_satisfied: sample.qualified_range_satisfied,
        output_created: true,
        receipts_written: false,
    })
    .map_err(|_| {
        ProductError::internal(
            "TOKENIZER_RESULT_SERIALIZATION_FAILED",
            "could not serialize the tokenizer training result",
        )
    })
}

fn validate_config(config: &TokenizerTrainConfigV1) -> Result<()> {
    if config.schema != TRAIN_CONFIG_SCHEMA {
        return Err(ProductError::usage(
            "TOKENIZER_CONFIG_SCHEMA_UNSUPPORTED",
            "the tokenizer configuration schema is unsupported",
        ));
    }
    if config.profile != PROTOTYPE_PROFILE {
        return Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "the requested tokenizer profile is designed but not implemented",
        ));
    }
    if !config.sample_manifest.path.is_absolute()
        || !config.content_root.is_absolute()
        || !config.output_path.is_absolute()
        || !is_sha256(&config.sample_manifest.sha256)
    {
        return Err(ProductError::usage(
            "TOKENIZER_CONFIG_PATH_INVALID",
            "tokenizer paths must be absolute and the sample manifest must be hash-bound",
        ));
    }
    Ok(())
}

fn validate_manifest(manifest: &TokenizerSampleManifestV1) -> Result<()> {
    if manifest.schema != SAMPLE_MANIFEST_SCHEMA
        || !is_sha256(&manifest.source_generation_manifest_sha256)
        || manifest.documents.is_empty()
    {
        return Err(ProductError::integrity(
            "TOKENIZER_SAMPLE_INVALID",
            "the tokenizer sample manifest violates its closed identity contract",
        ));
    }
    let mut source_ids = BTreeSet::new();
    for document in &manifest.documents {
        if !is_sha256(&document.repository_group_id)
            || !is_sha256(&document.source_id)
            || !is_sha256(&document.curated_sha256_raw)
            || !is_sha256(&document.canonical_sha256)
            || document.canonical_bytes == 0
        {
            return Err(ProductError::integrity(
                "TOKENIZER_DOCUMENT_IDENTITY_INVALID",
                "a tokenizer sample document has an invalid identity or byte count",
            ));
        }
        require_portable_relative_path(&document.relative_path, "TOKENIZER_DOCUMENT_PATH_INVALID")?;
        if !source_ids.insert(document.source_id.clone()) {
            return Err(ProductError::integrity(
                "TOKENIZER_DOCUMENT_DUPLICATE",
                "the tokenizer sample repeats a source identity",
            ));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct SelectedDocuments {
    documents: Vec<TokenizerSampleDocumentV1>,
    selected_bytes: u64,
    skipped_document_count: u64,
    skipped_bytes: u64,
}

fn select_documents(documents: &[TokenizerSampleDocumentV1]) -> Result<SelectedDocuments> {
    let mut ranked = documents
        .iter()
        .map(|document| Ok((sample_rank(document)?, document.clone())))
        .collect::<Result<Vec<_>>>()?;
    ranked.sort_by(|left, right| {
        left.0.cmp(&right.0).then_with(|| {
            left.1
                .source_id
                .as_bytes()
                .cmp(right.1.source_id.as_bytes())
        })
    });
    let mut repository_bytes = BTreeMap::<String, u64>::new();
    let mut selected_bytes = 0_u64;
    let mut selected = Vec::new();
    let mut skipped_document_count = 0_u64;
    let mut skipped_bytes = 0_u64;
    for (_, document) in ranked {
        let repository_total = repository_bytes
            .get(&document.repository_group_id)
            .copied()
            .unwrap_or(0);
        let next_repository = repository_total.checked_add(document.canonical_bytes);
        let next_global = selected_bytes.checked_add(document.canonical_bytes);
        if next_repository.is_none_or(|value| value > REPOSITORY_BYTE_CAP)
            || next_global.is_none_or(|value| value > GLOBAL_SAMPLE_BYTE_CAP)
        {
            skipped_document_count += 1;
            skipped_bytes = skipped_bytes
                .checked_add(document.canonical_bytes)
                .ok_or_else(|| {
                    ProductError::integrity(
                        "TOKENIZER_SAMPLE_BYTE_OVERFLOW",
                        "skipped tokenizer sample bytes overflowed",
                    )
                })?;
            continue;
        }
        let next_repository = next_repository.expect("checked above");
        let next_global = next_global.expect("checked above");
        repository_bytes.insert(document.repository_group_id.clone(), next_repository);
        selected_bytes = next_global;
        selected.push(document);
    }
    if selected.is_empty() {
        return Err(ProductError::gate(
            "TOKENIZER_SAMPLE_EMPTY",
            "no whole document fits the tokenizer sample caps",
        ));
    }
    Ok(SelectedDocuments {
        documents: selected,
        selected_bytes,
        skipped_document_count,
        skipped_bytes,
    })
}

fn sample_rank(document: &TokenizerSampleDocumentV1) -> Result<[u8; 32]> {
    let raw_hash = hex::decode(&document.curated_sha256_raw).map_err(|_| {
        ProductError::integrity(
            "TOKENIZER_DOCUMENT_IDENTITY_INVALID",
            "a curated document hash is not lowercase SHA-256",
        )
    })?;
    let mut digest = Sha256::new();
    digest.update(SAMPLE_DOMAIN);
    update_length_prefixed(&mut digest, document.repository_group_id.as_bytes());
    update_length_prefixed(&mut digest, document.source_id.as_bytes());
    digest.update(raw_hash);
    Ok(digest.finalize().into())
}

fn update_length_prefixed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value);
}

fn read_selected_documents(
    documents: &[TokenizerSampleDocumentV1],
    content_root: &Path,
) -> Result<Vec<Vec<u8>>> {
    let mut corpus = Vec::with_capacity(documents.len());
    for document in documents {
        let path = join_relative(content_root, &document.relative_path)?;
        let metadata = require_contained_regular_file(content_root, &path)?;
        if metadata.len() != document.canonical_bytes {
            return Err(ProductError::integrity(
                "TOKENIZER_DOCUMENT_LENGTH_MISMATCH",
                "a tokenizer document length differs from its manifest identity",
            ));
        }
        let bytes = read_stable_document(&path, &metadata)?;
        if sha256(&bytes) != document.canonical_sha256 {
            return Err(ProductError::integrity(
                "TOKENIZER_DOCUMENT_HASH_MISMATCH",
                "a tokenizer document hash differs from its manifest identity",
            ));
        }
        corpus.push(bytes);
    }
    Ok(corpus)
}

fn train_rules(corpus: &[Vec<u8>], target_vocabulary_size: u32) -> Result<Vec<MergeRule>> {
    if target_vocabulary_size < BASE_VOCABULARY_SIZE {
        return Err(ProductError::internal(
            "TOKENIZER_TARGET_INVALID",
            "the tokenizer target vocabulary is smaller than the byte alphabet",
        ));
    }
    let node_count = corpus.iter().try_fold(0_usize, |sum, document| {
        sum.checked_add(document.len()).ok_or_else(|| {
            ProductError::gate(
                "TOKENIZER_SAMPLE_TOO_LARGE",
                "the tokenizer sample exceeds addressable memory",
            )
        })
    })?;
    if node_count == 0 || node_count > (u32::MAX - 1) as usize {
        return Err(ProductError::gate(
            "TOKENIZER_SAMPLE_TOO_LARGE",
            "the tokenizer sample is empty or exceeds the fixed node index",
        ));
    }
    let mut tokens = Vec::with_capacity(node_count);
    let mut previous = Vec::with_capacity(node_count);
    let mut next = Vec::with_capacity(node_count);
    let mut counts = HashMap::<(u32, u32), u64>::new();
    let mut occurrences = HashMap::<(u32, u32), Vec<u32>>::new();
    for document in corpus {
        let start = tokens.len();
        for (offset, byte) in document.iter().enumerate() {
            let index = tokens.len();
            tokens.push(FIRST_BYTE_ID + u32::from(*byte));
            previous.push(if offset == 0 {
                NONE
            } else {
                (index - 1) as u32
            });
            next.push(if offset + 1 == document.len() {
                NONE
            } else {
                (index + 1) as u32
            });
        }
        for index in start..tokens.len().saturating_sub(1) {
            if next[index] == NONE {
                continue;
            }
            let pair = (tokens[index], tokens[next[index] as usize]);
            *counts.entry(pair).or_default() += 1;
            occurrences.entry(pair).or_default().push(index as u32);
        }
    }
    let mut ranking = counts
        .iter()
        .map(|(pair, count)| (Reverse(*count), pair.0, pair.1))
        .collect::<BTreeSet<_>>();
    let merge_count = (target_vocabulary_size - BASE_VOCABULARY_SIZE) as usize;
    let mut rules = Vec::with_capacity(merge_count);
    for offset in 0..merge_count {
        let Some((Reverse(frequency), left, right)) = ranking.first().copied() else {
            return Err(ProductError::gate(
                "TOKENIZER_VOCABULARY_UNDERFILLED",
                "the sample exhausted merge pairs before reaching the fixed vocabulary",
            ));
        };
        if frequency < MIN_MERGE_FREQUENCY {
            return Err(ProductError::gate(
                "TOKENIZER_VOCABULARY_UNDERFILLED",
                "the sample cannot reach the fixed vocabulary at minimum merge frequency two",
            ));
        }
        let pair = (left, right);
        let new_id = BASE_VOCABULARY_SIZE + offset as u32;
        let mut starts = occurrences.remove(&pair).unwrap_or_default();
        starts.sort_unstable();
        starts.dedup();
        for left_index in starts {
            let left_usize = left_index as usize;
            if tokens[left_usize] != left {
                continue;
            }
            let right_index = next[left_usize];
            if right_index == NONE || tokens[right_index as usize] != right {
                continue;
            }
            let right_usize = right_index as usize;
            let before = previous[left_usize];
            let after = next[right_usize];
            if before != NONE {
                adjust_count(
                    (tokens[before as usize], left),
                    -1,
                    &mut counts,
                    &mut ranking,
                )?;
            }
            adjust_count(pair, -1, &mut counts, &mut ranking)?;
            if after != NONE {
                adjust_count(
                    (right, tokens[after as usize]),
                    -1,
                    &mut counts,
                    &mut ranking,
                )?;
            }
            tokens[left_usize] = new_id;
            tokens[right_usize] = NONE;
            next[left_usize] = after;
            if after != NONE {
                previous[after as usize] = left_index;
            }
            if before != NONE {
                add_occurrence(
                    (tokens[before as usize], new_id),
                    before,
                    &mut counts,
                    &mut occurrences,
                    &mut ranking,
                )?;
            }
            if after != NONE {
                add_occurrence(
                    (new_id, tokens[after as usize]),
                    left_index,
                    &mut counts,
                    &mut occurrences,
                    &mut ranking,
                )?;
            }
        }
        if counts.contains_key(&pair) {
            return Err(ProductError::internal(
                "TOKENIZER_PAIR_ACCOUNTING_FAILED",
                "incremental BPE pair accounting diverged from the selected frequency",
            ));
        }
        rules.push(MergeRule {
            id: new_id,
            left,
            right,
            training_frequency: frequency,
        });
    }
    Ok(rules)
}

fn adjust_count(
    pair: (u32, u32),
    delta: i8,
    counts: &mut HashMap<(u32, u32), u64>,
    ranking: &mut BTreeSet<(Reverse<u64>, u32, u32)>,
) -> Result<()> {
    let old = counts.get(&pair).copied().unwrap_or(0);
    if old > 0 {
        ranking.remove(&(Reverse(old), pair.0, pair.1));
    }
    let new = if delta > 0 {
        old.checked_add(delta as u64)
    } else {
        old.checked_sub(delta.unsigned_abs() as u64)
    }
    .ok_or_else(|| {
        ProductError::internal(
            "TOKENIZER_PAIR_ACCOUNTING_FAILED",
            "incremental BPE pair counts overflowed or underflowed",
        )
    })?;
    if new == 0 {
        counts.remove(&pair);
    } else {
        counts.insert(pair, new);
        ranking.insert((Reverse(new), pair.0, pair.1));
    }
    Ok(())
}

fn add_occurrence(
    pair: (u32, u32),
    start: u32,
    counts: &mut HashMap<(u32, u32), u64>,
    occurrences: &mut HashMap<(u32, u32), Vec<u32>>,
    ranking: &mut BTreeSet<(Reverse<u64>, u32, u32)>,
) -> Result<()> {
    adjust_count(pair, 1, counts, ranking)?;
    occurrences.entry(pair).or_default().push(start);
    Ok(())
}

fn artifact_from_rules(rules: Vec<MergeRule>, sample: SampleBinding) -> TokenizerArtifactV1 {
    let vocabulary_size = BASE_VOCABULARY_SIZE + rules.len() as u32;
    TokenizerArtifactV1 {
        schema: ARTIFACT_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        algorithm: "deterministic-byte-bpe-v1".to_owned(),
        vocabulary_size,
        maximum_token_id: vocabulary_size - 1,
        special_tokens: SpecialTokenIds {
            pad: PAD_ID,
            bos: BOS_ID,
            eos: EOS_ID,
            unk: UNK_ID,
        },
        byte_alphabet: ByteAlphabet {
            first_id: FIRST_BYTE_ID,
            count: 256,
        },
        normalization: "none-byte-preserving-v1".to_owned(),
        minimum_merge_frequency: MIN_MERGE_FREQUENCY,
        tie_breaker: "highest-frequency-then-lowest-token-id-pair-v1".to_owned(),
        sample,
        merges: rules,
    }
}

fn validate_artifact(artifact: &TokenizerArtifactV1, require_canonical_size: bool) -> Result<()> {
    let expected_size = BASE_VOCABULARY_SIZE + artifact.merges.len() as u32;
    if artifact.schema != ARTIFACT_SCHEMA
        || artifact.profile != PROTOTYPE_PROFILE
        || artifact.algorithm != "deterministic-byte-bpe-v1"
        || artifact.vocabulary_size != expected_size
        || artifact.maximum_token_id + 1 != artifact.vocabulary_size
        || (require_canonical_size && artifact.vocabulary_size != VOCABULARY_SIZE)
        || artifact.special_tokens
            != (SpecialTokenIds {
                pad: PAD_ID,
                bos: BOS_ID,
                eos: EOS_ID,
                unk: UNK_ID,
            })
        || artifact.byte_alphabet
            != (ByteAlphabet {
                first_id: FIRST_BYTE_ID,
                count: 256,
            })
        || artifact.normalization != "none-byte-preserving-v1"
        || artifact.minimum_merge_frequency != MIN_MERGE_FREQUENCY
        || artifact.tie_breaker != "highest-frequency-then-lowest-token-id-pair-v1"
        || !is_sha256(&artifact.sample.manifest_sha256)
        || !is_sha256(&artifact.sample.source_generation_manifest_sha256)
        || artifact.sample.selected_document_count == 0
        || artifact.sample.selected_bytes == 0
        || artifact.sample.repository_byte_cap != REPOSITORY_BYTE_CAP
        || artifact.sample.global_byte_cap != GLOBAL_SAMPLE_BYTE_CAP
        || artifact.sample.qualified_minimum_bytes != QUALIFIED_SAMPLE_MINIMUM
        || artifact.sample.selected_bytes > GLOBAL_SAMPLE_BYTE_CAP
        || artifact.sample.qualified_range_satisfied
            != (artifact.sample.selected_bytes >= QUALIFIED_SAMPLE_MINIMUM)
    {
        return Err(ProductError::integrity(
            "TOKENIZER_ARTIFACT_INVALID",
            "the tokenizer artifact violates its fixed top-level contract",
        ));
    }
    let mut seen_pairs = BTreeSet::new();
    for (offset, rule) in artifact.merges.iter().enumerate() {
        let expected_id = BASE_VOCABULARY_SIZE + offset as u32;
        if rule.id != expected_id
            || rule.left >= rule.id
            || rule.right >= rule.id
            || rule.training_frequency < MIN_MERGE_FREQUENCY
            || !seen_pairs.insert((rule.left, rule.right))
        {
            return Err(ProductError::integrity(
                "TOKENIZER_MERGE_INVALID",
                "a tokenizer merge is non-contiguous, forward-referencing, or duplicated",
            ));
        }
    }
    Ok(())
}

fn push_encode_candidate(
    start: u32,
    tokens: &[u32],
    next: &[u32],
    ranks: &HashMap<(u32, u32), u32>,
    heap: &mut BinaryHeap<Reverse<(u32, u32, u32)>>,
) {
    let end = next[start as usize];
    if end == NONE {
        return;
    }
    if let Some(rank) = ranks.get(&(tokens[start as usize], tokens[end as usize])) {
        heap.push(Reverse((*rank, start, end)));
    }
}

#[cfg(windows)]
fn publish_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        ProductError::usage("TOKENIZER_OUTPUT_INVALID", "the output path has no parent")
    })?;
    let leaf = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            ProductError::usage(
                "TOKENIZER_OUTPUT_INVALID",
                "the output path has no portable file name",
            )
        })?;
    let temporary = (0..64)
        .find_map(|_| {
            let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".{leaf}.p7-partial-{}-{sequence}",
                std::process::id()
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => Some(Ok((candidate, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(_) => Some(Err(ProductError::environment(
                    "TOKENIZER_OUTPUT_CREATE_FAILED",
                    "could not create an owned tokenizer artifact temporary",
                ))),
            }
        })
        .transpose()?
        .ok_or_else(|| {
            ProductError::environment(
                "TOKENIZER_OUTPUT_CREATE_FAILED",
                "could not allocate an owned tokenizer artifact temporary",
            )
        })?;
    let (temporary_path, mut file) = temporary;
    let result = (|| {
        file.write_all(bytes).map_err(|_| {
            ProductError::environment(
                "TOKENIZER_OUTPUT_WRITE_FAILED",
                "could not write the tokenizer artifact",
            )
        })?;
        file.sync_all().map_err(|_| {
            ProductError::environment(
                "TOKENIZER_OUTPUT_SYNC_FAILED",
                "could not sync the tokenizer artifact",
            )
        })?;
        drop(file);
        if path.exists() {
            return Err(ProductError::integrity(
                "TOKENIZER_OUTPUT_EXISTS",
                "the create-new tokenizer artifact already exists",
            ));
        }
        move_file_write_through(&temporary_path, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(windows)]
fn move_file_write_through(from: &Path, to: &Path) -> Result<()> {
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
    // SAFETY: both UTF-16 buffers are NUL-terminated and remain live for the call.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(ProductError::environment(
            "TOKENIZER_OUTPUT_PUBLISH_FAILED",
            "could not publish the create-new tokenizer artifact",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_binding() -> SampleBinding {
        SampleBinding {
            manifest_sha256: "11".repeat(32),
            source_generation_manifest_sha256: "22".repeat(32),
            selected_document_count: 2,
            selected_bytes: 24,
            skipped_document_count: 0,
            skipped_bytes: 0,
            repository_byte_cap: REPOSITORY_BYTE_CAP,
            global_byte_cap: GLOBAL_SAMPLE_BYTE_CAP,
            qualified_minimum_bytes: QUALIFIED_SAMPLE_MINIMUM,
            qualified_range_satisfied: false,
        }
    }

    fn train_small(corpus: &[Vec<u8>], vocabulary_size: u32) -> ByteBpeTokenizer {
        let rules = train_rules(corpus, vocabulary_size).unwrap();
        let artifact = artifact_from_rules(rules, sample_binding());
        validate_artifact(&artifact, false).unwrap();
        ByteBpeTokenizer::from_validated_artifact(artifact).unwrap()
    }

    #[test]
    fn canonical_ids_and_byte_round_trip_are_exact() {
        let corpus = vec![b"banana bandana\n".repeat(4), b"banana bandana\n".repeat(4)];
        let tokenizer = train_small(&corpus, BASE_VOCABULARY_SIZE + 8);
        assert_eq!(tokenizer.artifact.special_tokens.pad, 0);
        assert_eq!(tokenizer.artifact.special_tokens.bos, 1);
        assert_eq!(tokenizer.artifact.special_tokens.eos, 2);
        assert_eq!(tokenizer.artifact.special_tokens.unk, 3);
        let source = b"<s>\0banana\r\n\xf0\x9f\xa6\x80";
        let encoded = tokenizer.encode(source);
        assert!(encoded.iter().all(|id| *id >= FIRST_BYTE_ID));
        assert_eq!(tokenizer.decode_source(&encoded).unwrap(), source);
        assert_eq!(
            tokenizer.decode_source(&[BOS_ID]).unwrap_err().code,
            "SPECIAL_TOKEN_IN_SOURCE"
        );
    }

    #[test]
    fn training_and_serialization_are_stable() {
        let corpus = vec![b"abcabcabcabc".to_vec(), b"xyzxyzxyzxyz".to_vec()];
        let left = train_rules(&corpus, BASE_VOCABULARY_SIZE + 6).unwrap();
        let right = train_rules(&corpus, BASE_VOCABULARY_SIZE + 6).unwrap();
        assert_eq!(left, right);
        let left_bytes = compact_json_line(
            &artifact_from_rules(left, sample_binding()),
            "TEST_SERIALIZE",
        )
        .unwrap();
        let right_bytes = compact_json_line(
            &artifact_from_rules(right, sample_binding()),
            "TEST_SERIALIZE",
        )
        .unwrap();
        assert_eq!(left_bytes, right_bytes);
    }

    #[test]
    fn tie_breaking_prefers_lowest_token_id_pair() {
        let rules = train_rules(&[b"ababcdcd".to_vec()], BASE_VOCABULARY_SIZE + 1).unwrap();
        assert_eq!(rules[0].training_frequency, 2);
        assert_eq!(rules[0].left, FIRST_BYTE_ID + u32::from(b'a'));
        assert_eq!(rules[0].right, FIRST_BYTE_ID + u32::from(b'b'));
    }

    #[test]
    fn documents_do_not_create_cross_boundary_pairs() {
        let error =
            train_rules(&[b"a".to_vec(), b"b".to_vec()], BASE_VOCABULARY_SIZE + 1).unwrap_err();
        assert_eq!(error.code, "TOKENIZER_VOCABULARY_UNDERFILLED");
    }

    #[test]
    fn minimum_frequency_is_enforced() {
        let error = train_rules(&[b"abcdef".to_vec()], BASE_VOCABULARY_SIZE + 1).unwrap_err();
        assert_eq!(error.code, "TOKENIZER_VOCABULARY_UNDERFILLED");
    }

    fn document(repository: u8, source: u8, bytes: u64) -> TokenizerSampleDocumentV1 {
        TokenizerSampleDocumentV1 {
            repository_group_id: format!("{repository:02x}").repeat(32),
            source_id: format!("{source:02x}").repeat(32),
            curated_sha256_raw: format!("{:02x}", source.wrapping_add(1)).repeat(32),
            canonical_sha256: format!("{:02x}", source.wrapping_add(2)).repeat(32),
            canonical_bytes: bytes,
            relative_path: format!("{source:02x}.py"),
        }
    }

    #[test]
    fn ranking_and_caps_skip_whole_documents_and_continue() {
        let oversized = document(1, 1, REPOSITORY_BYTE_CAP + 1);
        let fitting = document(2, 2, 128);
        let selected = select_documents(&[oversized, fitting.clone()]).unwrap();
        assert_eq!(selected.documents, vec![fitting]);
        assert_eq!(selected.selected_bytes, 128);
        assert_eq!(selected.skipped_document_count, 1);
        assert_eq!(selected.skipped_bytes, REPOSITORY_BYTE_CAP + 1);
    }

    #[test]
    fn canonical_vocabulary_serializes_reloads_and_preserves_exact_bytes() {
        let mut state = 0x4d59_5df4_d0f3_3173_u64;
        let document = (0..40_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect::<Vec<_>>();
        let rules = train_rules(&[document.clone(), document.clone()], VOCABULARY_SIZE).unwrap();
        assert_eq!(
            rules.len(),
            (VOCABULARY_SIZE - BASE_VOCABULARY_SIZE) as usize
        );
        assert_eq!(rules.last().unwrap().id, MAX_TOKEN_ID);
        let artifact = artifact_from_rules(rules, sample_binding());
        let bytes = compact_json_line(&artifact, "TEST_SERIALIZE").unwrap();
        let reloaded = ByteBpeTokenizer::from_artifact_bytes(&bytes).unwrap();
        assert_eq!(reloaded.artifact(), &artifact);
        let probe = &document[123..9123];
        let token_ids = reloaded.encode(probe);
        assert!(
            token_ids
                .iter()
                .all(|id| *id >= FIRST_BYTE_ID && *id <= MAX_TOKEN_ID)
        );
        assert_eq!(reloaded.decode_source(&token_ids).unwrap(), probe);
    }

    #[test]
    fn exponential_merge_expansion_is_bounded() {
        let mut rules = Vec::new();
        for id in BASE_VOCABULARY_SIZE..VOCABULARY_SIZE {
            let left = if id == BASE_VOCABULARY_SIZE {
                FIRST_BYTE_ID
            } else {
                id - 1
            };
            rules.push(MergeRule {
                id,
                left,
                right: left,
                training_frequency: 2,
            });
        }
        let artifact = artifact_from_rules(rules, sample_binding());
        let bytes = compact_json_line(&artifact, "TEST_SERIALIZE").unwrap();
        let tokenizer = ByteBpeTokenizer::from_artifact_bytes(&bytes).unwrap();
        let error = tokenizer.decode_source(&[MAX_TOKEN_ID]).unwrap_err();
        assert_eq!(error.code, "TOKENIZER_DECODE_LIMIT_EXCEEDED");
    }

    #[test]
    fn malformed_forward_referencing_artifact_is_rejected() {
        let rules = train_rules(
            &[b"aaaaaaaa".to_vec(), b"aaaaaaaa".to_vec()],
            BASE_VOCABULARY_SIZE + 1,
        )
        .unwrap();
        let mut artifact = artifact_from_rules(rules, sample_binding());
        artifact.merges[0].left = artifact.merges[0].id;
        assert_eq!(
            validate_artifact(&artifact, false).unwrap_err().code,
            "TOKENIZER_MERGE_INVALID"
        );
    }
}
