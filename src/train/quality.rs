//! Provider-neutral P16A quality evaluation and deterministic replay.

use super::profile::PrototypeTrainingDefaultsV1;
use crate::backend::PROTOTYPE_PROFILE;
use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

pub const IMPLEMENTATION_PHASE: &str = "P16A";
pub const IMPLEMENTATION_RESULT_SCHEMA: &str =
    "python-slm-quality-evaluation-implementation-result-v1";
pub const EVALUATION_RESULT_SCHEMA: &str = "python-slm-quality-evaluation-result-v1";
pub const QUALITY_PACK_SCHEMA: &str = "python-slm-quality-pack-v1";
pub const EXECUTION_SURFACE: &str = "provider-neutral-held-out-quality-evaluator";
pub const UNIGRAM_ALGORITHM: &str = "add-one-smoothed-unigram-nll-v1";
pub const LOSS_ALGORITHM: &str = "ordered-valid-target-nll-sum-f64-v1";
pub const PERPLEXITY_ALGORITHM: &str = "exp-aggregate-loss-f64-v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LossChunkV1 {
    pub first_target: u64,
    pub valid_targets: u64,
    pub negative_log_likelihood_sum: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateMetricsV1 {
    pub evaluated_targets: u64,
    pub aggregate_loss: f64,
    pub aggregate_perplexity: f64,
    pub aggregate_loss_f64_le_hex: String,
    pub aggregate_perplexity_f64_le_hex: String,
    pub ordered_chunks_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationSettingsV1 {
    pub algorithm: String,
    pub maximum_new_tokens: u64,
    pub temperature_milli: u64,
    pub top_k: u64,
    pub sampling: bool,
    pub seed: u64,
}

impl GenerationSettingsV1 {
    pub fn deterministic_default() -> Self {
        Self {
            algorithm: "greedy-token-id-v1".to_owned(),
            maximum_new_tokens: 64,
            temperature_milli: 0,
            top_k: 1,
            sampling: false,
            seed: 0,
        }
    }

    fn validate(&self) -> Result<()> {
        if self != &Self::deterministic_default() {
            return Err(ProductError::integrity(
                "P16A_GENERATION_SETTINGS_INVALID",
                "quality replay settings differ from the deterministic P16A policy",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PromptCaseV1 {
    pub prompt_id: String,
    pub prompt_token_ids: Vec<u16>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QualityPackV1 {
    pub schema: String,
    pub profile: String,
    pub held_out_manifest_sha256: String,
    pub held_out_targets: u64,
    pub vocabulary_size: u64,
    pub initialized_checkpoint_sha256: String,
    pub unigram_artifact_sha256: String,
    pub generation_settings: GenerationSettingsV1,
    pub prompts: Vec<PromptCaseV1>,
}

impl QualityPackV1 {
    pub fn validate(&self) -> Result<()> {
        self.generation_settings.validate()?;
        if self.schema != QUALITY_PACK_SCHEMA
            || self.profile != PROTOTYPE_PROFILE
            || !is_sha256(&self.held_out_manifest_sha256)
            || !is_sha256(&self.initialized_checkpoint_sha256)
            || !is_sha256(&self.unigram_artifact_sha256)
            || self.held_out_targets == 0
            || self.vocabulary_size < 2
            || self.vocabulary_size > u16::MAX as u64 + 1
            || self.prompts.is_empty()
        {
            return Err(ProductError::integrity(
                "P16A_QUALITY_PACK_INVALID",
                "the quality pack has an invalid identity, target count, vocabulary, or prompt set",
            ));
        }
        let mut previous: Option<&str> = None;
        let mut identities = BTreeSet::new();
        for prompt in &self.prompts {
            if !portable_id(&prompt.prompt_id)
                || prompt.prompt_token_ids.is_empty()
                || prompt.prompt_token_ids.len() > 4_096
                || prompt
                    .prompt_token_ids
                    .iter()
                    .any(|token| u64::from(*token) >= self.vocabulary_size)
                || !identities.insert(prompt.prompt_id.as_str())
                || previous.is_some_and(|value| value >= prompt.prompt_id.as_str())
            {
                return Err(ProductError::integrity(
                    "P16A_PROMPT_PACK_INVALID",
                    "quality prompts are empty, unordered, duplicated, oversized, or outside the vocabulary",
                ));
            }
            previous = Some(&prompt.prompt_id);
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String> {
        self.validate()?;
        canonical_sha256(self, "P16A_QUALITY_PACK_SERIALIZE_FAILED")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnigramBaselineInputV1 {
    pub training_token_counts: Vec<u64>,
    pub held_out_token_counts: Vec<u64>,
}

impl UnigramBaselineInputV1 {
    pub fn sha256(&self) -> Result<String> {
        canonical_sha256(self, "P16A_UNIGRAM_SERIALIZE_FAILED")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PromptReplayV1 {
    pub prompt_id: String,
    pub prompt_sha256: String,
    pub generated_token_ids: Vec<u16>,
    pub generated_sha256: String,
    pub replay_count: u64,
    pub deterministic: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QualityEvaluationResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub profile: String,
    pub quality_pack_sha256: String,
    pub held_out_manifest_sha256: String,
    pub final_checkpoint_sha256: String,
    pub unigram_artifact_sha256: String,
    pub loss_algorithm: String,
    pub perplexity_algorithm: String,
    pub unigram_algorithm: String,
    pub initialized: AggregateMetricsV1,
    pub unigram: AggregateMetricsV1,
    pub final_checkpoint: AggregateMetricsV1,
    pub final_loss_below_initialized: bool,
    pub final_loss_below_unigram: bool,
    pub prompt_replays: Vec<PromptReplayV1>,
    pub deterministic_outputs: bool,
    pub backend_state_unchanged: bool,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QualityEvaluationImplementationResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub profile: String,
    pub execution_status: String,
    pub execution_surface: String,
    pub configuration_sha256: String,
    pub implementation_sha256: String,
    pub loss_algorithm: String,
    pub perplexity_algorithm: String,
    pub unigram_algorithm: String,
    pub deterministic_generation_settings: GenerationSettingsV1,
    pub claims: QualityClaimStatusV1,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QualityClaimStatusV1 {
    pub quality_pack_frozen_before_p15: String,
    pub final_checkpoint_reloaded: String,
    pub initialized_baseline: String,
    pub unigram_baseline: String,
    pub final_held_out_metrics: String,
    pub qualitative_outputs: String,
    pub prototype_quality: String,
}

pub trait QualityEvaluationBackend {
    fn state_sha256(&self) -> Result<String>;
    fn evaluate_held_out(&mut self, held_out_manifest_sha256: &str) -> Result<Vec<LossChunkV1>>;
    fn generate(
        &mut self,
        prompt_token_ids: &[u16],
        settings: &GenerationSettingsV1,
    ) -> Result<Vec<u16>>;
}

pub fn aggregate_metrics(chunks: &[LossChunkV1]) -> Result<AggregateMetricsV1> {
    if chunks.is_empty() {
        return Err(ProductError::gate(
            "P16A_EVALUATION_EMPTY",
            "held-out evaluation returned no valid-target loss chunks",
        ));
    }
    let mut expected_first = 0_u64;
    let mut sum = 0.0_f64;
    let mut compensation = 0.0_f64;
    for chunk in chunks {
        if chunk.first_target != expected_first
            || chunk.valid_targets == 0
            || !chunk.negative_log_likelihood_sum.is_finite()
            || chunk.negative_log_likelihood_sum < 0.0
        {
            return Err(ProductError::gate(
                "P16A_EVALUATION_CHUNK_INVALID",
                "held-out loss chunks are unordered, empty, negative, or nonfinite",
            ));
        }
        expected_first = expected_first
            .checked_add(chunk.valid_targets)
            .ok_or_else(accounting_overflow)?;
        let corrected = chunk.negative_log_likelihood_sum - compensation;
        let next = sum + corrected;
        compensation = (next - sum) - corrected;
        sum = next;
    }
    let aggregate_loss = sum / expected_first as f64;
    let aggregate_perplexity = aggregate_loss.exp();
    if !aggregate_loss.is_finite() || !aggregate_perplexity.is_finite() {
        return Err(ProductError::gate(
            "P16A_METRIC_NONFINITE",
            "aggregate held-out loss or perplexity is not finite",
        ));
    }
    Ok(AggregateMetricsV1 {
        evaluated_targets: expected_first,
        aggregate_loss,
        aggregate_perplexity,
        aggregate_loss_f64_le_hex: hex::encode(aggregate_loss.to_le_bytes()),
        aggregate_perplexity_f64_le_hex: hex::encode(aggregate_perplexity.to_le_bytes()),
        ordered_chunks_sha256: canonical_sha256(chunks, "P16A_LOSS_CHUNKS_SERIALIZE_FAILED")?,
    })
}

pub fn unigram_baseline(
    input: &UnigramBaselineInputV1,
    expected_vocabulary_size: u64,
) -> Result<AggregateMetricsV1> {
    if input.training_token_counts.len() as u64 != expected_vocabulary_size
        || input.held_out_token_counts.len() as u64 != expected_vocabulary_size
    {
        return Err(ProductError::integrity(
            "P16A_UNIGRAM_VOCABULARY_MISMATCH",
            "unigram count vectors do not match the frozen vocabulary size",
        ));
    }
    let training_targets = input
        .training_token_counts
        .iter()
        .try_fold(0_u64, |sum, value| sum.checked_add(*value))
        .ok_or_else(accounting_overflow)?;
    let held_out_targets = input
        .held_out_token_counts
        .iter()
        .try_fold(0_u64, |sum, value| sum.checked_add(*value))
        .ok_or_else(accounting_overflow)?;
    if training_targets == 0 || held_out_targets == 0 {
        return Err(ProductError::integrity(
            "P16A_UNIGRAM_COUNTS_EMPTY",
            "unigram training and held-out count vectors must both contain targets",
        ));
    }
    let denominator = training_targets
        .checked_add(expected_vocabulary_size)
        .ok_or_else(accounting_overflow)? as f64;
    let mut sum = 0.0_f64;
    let mut compensation = 0.0_f64;
    for (training, held_out) in input
        .training_token_counts
        .iter()
        .zip(&input.held_out_token_counts)
    {
        if *held_out == 0 {
            continue;
        }
        let probability = (*training as f64 + 1.0) / denominator;
        let contribution = -(*held_out as f64) * probability.ln();
        let corrected = contribution - compensation;
        let next = sum + corrected;
        compensation = (next - sum) - corrected;
        sum = next;
    }
    aggregate_metrics(&[LossChunkV1 {
        first_target: 0,
        valid_targets: held_out_targets,
        negative_log_likelihood_sum: sum,
    }])
}

pub fn evaluate_quality<I: QualityEvaluationBackend, F: QualityEvaluationBackend>(
    pack: &QualityPackV1,
    initialized: &mut I,
    final_checkpoint: &mut F,
    final_checkpoint_sha256: &str,
    unigram: &UnigramBaselineInputV1,
) -> Result<QualityEvaluationResultV1> {
    pack.validate()?;
    if !is_sha256(final_checkpoint_sha256) {
        return Err(ProductError::integrity(
            "P16A_FINAL_CHECKPOINT_IDENTITY_INVALID",
            "the final checkpoint identity is not a lowercase SHA-256",
        ));
    }
    let unigram_artifact_sha256 = unigram.sha256()?;
    if unigram_artifact_sha256 != pack.unigram_artifact_sha256 {
        return Err(ProductError::integrity(
            "P16A_UNIGRAM_ARTIFACT_MISMATCH",
            "the unigram counts do not match the frozen quality-pack identity",
        ));
    }
    let initialized_before = initialized.state_sha256()?;
    if initialized_before != pack.initialized_checkpoint_sha256 {
        return Err(ProductError::integrity(
            "P16A_INITIALIZED_CHECKPOINT_MISMATCH",
            "the initialized backend does not match the frozen quality-pack identity",
        ));
    }
    let initialized_metrics =
        aggregate_metrics(&initialized.evaluate_held_out(&pack.held_out_manifest_sha256)?)?;
    if initialized.state_sha256()? != initialized_before {
        return Err(ProductError::gate(
            "P16A_EVALUATION_MUTATED_STATE",
            "initialized-model evaluation changed backend state",
        ));
    }

    let final_before = final_checkpoint.state_sha256()?;
    if final_before != final_checkpoint_sha256 {
        return Err(ProductError::integrity(
            "P16A_FINAL_CHECKPOINT_MISMATCH",
            "the final backend does not match the requested checkpoint identity",
        ));
    }
    let final_metrics =
        aggregate_metrics(&final_checkpoint.evaluate_held_out(&pack.held_out_manifest_sha256)?)?;
    if final_checkpoint.state_sha256()? != final_before {
        return Err(ProductError::gate(
            "P16A_EVALUATION_MUTATED_STATE",
            "final held-out evaluation changed backend state",
        ));
    }
    let unigram_metrics = unigram_baseline(unigram, pack.vocabulary_size)?;
    if initialized_metrics.evaluated_targets != pack.held_out_targets
        || final_metrics.evaluated_targets != pack.held_out_targets
        || unigram_metrics.evaluated_targets != pack.held_out_targets
    {
        return Err(ProductError::integrity(
            "P16A_HELD_OUT_TARGET_MISMATCH",
            "initialized, unigram, and final metrics do not cover the identical frozen target set",
        ));
    }
    let below_initialized = final_metrics.aggregate_loss < initialized_metrics.aggregate_loss;
    let below_unigram = final_metrics.aggregate_loss < unigram_metrics.aggregate_loss;
    if !below_initialized || !below_unigram {
        return Err(ProductError::gate(
            "P16A_FINAL_LOSS_NOT_IMPROVED",
            "final aggregate held-out loss is not strictly below both frozen baselines",
        ));
    }

    let mut prompt_replays = Vec::with_capacity(pack.prompts.len());
    for prompt in &pack.prompts {
        let first =
            final_checkpoint.generate(&prompt.prompt_token_ids, &pack.generation_settings)?;
        if final_checkpoint.state_sha256()? != final_before {
            return Err(ProductError::gate(
                "P16A_EVALUATION_MUTATED_STATE",
                "the first prompt replay changed backend state",
            ));
        }
        let second =
            final_checkpoint.generate(&prompt.prompt_token_ids, &pack.generation_settings)?;
        if final_checkpoint.state_sha256()? != final_before {
            return Err(ProductError::gate(
                "P16A_EVALUATION_MUTATED_STATE",
                "the second prompt replay changed backend state",
            ));
        }
        if first.is_empty()
            || first.len() as u64 > pack.generation_settings.maximum_new_tokens
            || first != second
            || first
                .iter()
                .any(|token| u64::from(*token) >= pack.vocabulary_size)
        {
            return Err(ProductError::gate(
                "P16A_GENERATION_NONDETERMINISTIC",
                "prompt replay was empty, oversized, out of vocabulary, or byte-inexact",
            ));
        }
        prompt_replays.push(PromptReplayV1 {
            prompt_id: prompt.prompt_id.clone(),
            prompt_sha256: token_ids_sha256(&prompt.prompt_token_ids),
            generated_sha256: token_ids_sha256(&first),
            generated_token_ids: first,
            replay_count: 2,
            deterministic: true,
        });
    }
    if final_checkpoint.state_sha256()? != final_before {
        return Err(ProductError::gate(
            "P16A_EVALUATION_MUTATED_STATE",
            "final evaluation or generation changed backend state",
        ));
    }
    Ok(QualityEvaluationResultV1 {
        schema: EVALUATION_RESULT_SCHEMA.to_owned(),
        status: "QUALITY_EVALUATED".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        quality_pack_sha256: pack.sha256()?,
        held_out_manifest_sha256: pack.held_out_manifest_sha256.clone(),
        final_checkpoint_sha256: final_checkpoint_sha256.to_owned(),
        unigram_artifact_sha256,
        loss_algorithm: LOSS_ALGORITHM.to_owned(),
        perplexity_algorithm: PERPLEXITY_ALGORITHM.to_owned(),
        unigram_algorithm: UNIGRAM_ALGORITHM.to_owned(),
        initialized: initialized_metrics,
        unigram: unigram_metrics,
        final_checkpoint: final_metrics,
        final_loss_below_initialized: below_initialized,
        final_loss_below_unigram: below_unigram,
        prompt_replays,
        deterministic_outputs: true,
        backend_state_unchanged: true,
        limitations: vec![
            "automated-metrics-and-replay-only".to_owned(),
            "not-owner-quality-approval".to_owned(),
            "not-hardware-qualification".to_owned(),
            "not-performance-or-sla-evidence".to_owned(),
            "not-portability-acceptance".to_owned(),
        ],
    })
}

pub fn build_implementation_result(
    configuration: PrototypeTrainingDefaultsV1,
) -> Result<QualityEvaluationImplementationResultV1> {
    configuration.validate()?;
    Ok(QualityEvaluationImplementationResultV1 {
        schema: IMPLEMENTATION_RESULT_SCHEMA.to_owned(),
        status: "IMPLEMENTATION_READY".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        execution_status: "NOT_RUN".to_owned(),
        execution_surface: EXECUTION_SURFACE.to_owned(),
        configuration_sha256: configuration.sha256()?,
        implementation_sha256: hex::encode(Sha256::digest(include_bytes!("quality.rs"))),
        loss_algorithm: LOSS_ALGORITHM.to_owned(),
        perplexity_algorithm: PERPLEXITY_ALGORITHM.to_owned(),
        unigram_algorithm: UNIGRAM_ALGORITHM.to_owned(),
        deterministic_generation_settings: GenerationSettingsV1::deterministic_default(),
        claims: QualityClaimStatusV1 {
            quality_pack_frozen_before_p15: "UNVERIFIED".to_owned(),
            final_checkpoint_reloaded: "UNVERIFIED".to_owned(),
            initialized_baseline: "UNVERIFIED".to_owned(),
            unigram_baseline: "UNVERIFIED".to_owned(),
            final_held_out_metrics: "UNVERIFIED".to_owned(),
            qualitative_outputs: "UNVERIFIED".to_owned(),
            prototype_quality: "UNVERIFIED".to_owned(),
        },
        limitations: vec![
            "implementation-readiness-only".to_owned(),
            "no-final-checkpoint-was-evaluated".to_owned(),
            "no-quality-pack-freeze-is-claimed".to_owned(),
            "not-owner-quality-approval".to_owned(),
            "not-portability-unlock".to_owned(),
        ],
    })
}

pub fn quality_evaluation(config_path: &Path) -> Result<Value> {
    #[cfg(not(windows))]
    {
        let _ = config_path;
        return Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "prototype quality evaluation is implemented only for native Windows",
        ));
    }
    #[cfg(windows)]
    {
        let bytes = super::profile::read_control_file(config_path, "P16A_CONFIG_READ_FAILED")?;
        let configuration = super::profile::parse_default_configuration(&bytes)?;
        serde_json::to_value(build_implementation_result(configuration)?).map_err(|_| {
            ProductError::internal(
                "P16A_RESULT_SERIALIZE_FAILED",
                "could not serialize the closed P16A implementation result",
            )
        })
    }
}

fn canonical_sha256<T: Serialize + ?Sized>(value: &T, code: &'static str) -> Result<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| ProductError::internal(code, "could not serialize canonical P16A bytes"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn token_ids_sha256(token_ids: &[u16]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"python-slm/p16a/token-ids/v1\0");
    hasher.update((token_ids.len() as u64).to_le_bytes());
    for token in token_ids {
        hasher.update(token.to_le_bytes());
    }
    hex::encode(hasher.finalize())
}

fn portable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn accounting_overflow() -> ProductError {
    ProductError::integrity(
        "P16A_ACCOUNTING_OVERFLOW",
        "quality target or unigram accounting overflowed",
    )
}
