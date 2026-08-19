//! Closed prototype defaults and non-publishing profiling diagnostics.

use super::trainer::{
    EVALUATION_TARGETS, EVENT_INTERVAL_TARGETS, RETENTION_ANCHORS, TARGETS_PER_FULL_UPDATE,
    TOTAL_TARGETS,
};
use crate::backend::{BURN_CUBECL_CUDA, PROTOTYPE_PROFILE};
use crate::error::{ProductError, Result};
use crate::model::{CANONICAL_MODEL_ID, COMPATIBILITY_ALLOCATION_BYTES};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;
#[cfg(windows)]
use std::{
    fs::{self, OpenOptions},
    io::Read,
};

pub const IMPLEMENTATION_PHASE: &str = "P14";
pub const DEFAULT_CONFIG_SCHEMA: &str = "python-slm-prototype-training-defaults-v1";
pub const DIAGNOSTICS_SCHEMA: &str = "python-slm-prototype-profile-diagnostics-v1";
pub const RESULT_SCHEMA: &str = "python-slm-prototype-profile-result-v1";
pub const DEFAULT_CONFIG_BYTES: &[u8] = include_bytes!("prototype-windows-5090-v1.defaults.json");

/// Sequences per accelerator dispatch.
///
/// Sixteen fits and was chosen when fitting was the question. Measured over a
/// long dispatch sequence rather than a warm burst, it is the wrong answer:
/// sixteen sustains `9,482` targets per second and swings `72` percent between
/// windows, because it peaks at `32,079` MiB of `32,607` and the allocator
/// thrashes against the remaining `530`. Eight sustains `14,944` and varies by
/// `1.6` percent, at `19,130` MiB. So the narrower dispatch is `1.58x` faster in
/// the only regime a run ever occupies, and the earlier claim that it cost `7.6`
/// percent came from measuring five dispatches immediately after warmup.
///
/// The optimizer update is unchanged at `65,536` targets — halving the dispatch
/// doubles the accumulation steps and nothing downstream of the update boundary
/// moves. Owner-approved 2026-08-19.
pub const MICRO_BATCH_SEQUENCES: u64 = 8;
pub const GRADIENT_ACCUMULATION_STEPS: u64 = 4;
pub const LOADER_BUFFER_SPANS: u64 = 32;
pub const TRANSFER_IN_FLIGHT: u64 = 8;
pub const RETAIN_LATEST_CHECKPOINTS: u64 = 2;
pub const ADMISSION_SECONDS: u64 = 25_920;
pub const COMPLETION_SLA_SECONDS: u64 = 28_800;
pub const ACTUAL_ELAPSED_LIMIT_NS: u64 = 28_800_000_000_000;
const MAX_CONTROL_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrototypeTrainingDefaultsV1 {
    pub schema: String,
    pub profile: String,
    pub support_level: String,
    pub qualification_status: String,
    pub model_identity: String,
    pub batch: BatchDefaults,
    pub loader: LoaderDefaults,
    pub checkpoint: CheckpointDefaults,
    pub evaluation: EvaluationDefaults,
    pub backend: BackendDefaults,
    pub sla: SlaDesignTargets,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BatchDefaults {
    pub sequence_targets: u64,
    pub micro_batch_sequences: u64,
    pub micro_batch_targets: u64,
    pub gradient_accumulation_steps: u64,
    pub optimizer_update_targets: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoaderDefaults {
    pub host_buffer_spans: u64,
    pub maximum_in_flight_transfers: u64,
    pub transfer_memory: String,
    pub preserve_span_order: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointDefaults {
    pub interval_targets: u64,
    pub completed_update_boundary_only: bool,
    pub retain_latest: u64,
    pub retention_anchors: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationDefaults {
    pub before_first_update: bool,
    pub interval_targets: u64,
    pub evaluated_targets: u64,
    pub mutate_training_state: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackendDefaults {
    pub provider: String,
    pub backend: String,
    pub selection: String,
    pub parameter_and_activation_storage: String,
    pub sensitive_accumulation: String,
    pub compatibility_allocation_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SlaDesignTargets {
    pub admission_seconds: u64,
    pub completion_seconds: u64,
    pub actual_elapsed_limit_ns: u64,
    pub claim_status: String,
}

impl PrototypeTrainingDefaultsV1 {
    pub fn canonical() -> Self {
        let sequence_targets = crate::storage::SEQUENCE_TARGETS;
        let micro_batch_targets = sequence_targets * MICRO_BATCH_SEQUENCES;
        Self {
            schema: DEFAULT_CONFIG_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            support_level: "implemented".to_owned(),
            qualification_status: "SKIPPED".to_owned(),
            model_identity: CANONICAL_MODEL_ID.to_owned(),
            batch: BatchDefaults {
                sequence_targets,
                micro_batch_sequences: MICRO_BATCH_SEQUENCES,
                micro_batch_targets,
                gradient_accumulation_steps: GRADIENT_ACCUMULATION_STEPS,
                optimizer_update_targets: TARGETS_PER_FULL_UPDATE,
            },
            loader: LoaderDefaults {
                host_buffer_spans: LOADER_BUFFER_SPANS,
                maximum_in_flight_transfers: TRANSFER_IN_FLIGHT,
                transfer_memory: "cuda-page-locked".to_owned(),
                preserve_span_order: true,
            },
            checkpoint: CheckpointDefaults {
                interval_targets: EVENT_INTERVAL_TARGETS,
                completed_update_boundary_only: true,
                retain_latest: RETAIN_LATEST_CHECKPOINTS,
                retention_anchors: RETENTION_ANCHORS.to_vec(),
            },
            evaluation: EvaluationDefaults {
                before_first_update: true,
                interval_targets: EVENT_INTERVAL_TARGETS,
                evaluated_targets: EVALUATION_TARGETS,
                mutate_training_state: false,
            },
            backend: BackendDefaults {
                provider: "cuda".to_owned(),
                backend: BURN_CUBECL_CUDA.to_owned(),
                selection: "explicit".to_owned(),
                parameter_and_activation_storage: "bf16".to_owned(),
                sensitive_accumulation: "fp32".to_owned(),
                compatibility_allocation_bytes: COMPATIBILITY_ALLOCATION_BYTES,
            },
            sla: SlaDesignTargets {
                admission_seconds: ADMISSION_SECONDS,
                completion_seconds: COMPLETION_SLA_SECONDS,
                actual_elapsed_limit_ns: ACTUAL_ELAPSED_LIMIT_NS,
                claim_status: "DESIGN_TARGET_UNVERIFIED".to_owned(),
            },
        }
    }

    pub fn validate(&self) -> Result<()> {
        let micro_batch_targets = self
            .batch
            .sequence_targets
            .checked_mul(self.batch.micro_batch_sequences)
            .ok_or_else(accounting_overflow)?;
        let update_targets = micro_batch_targets
            .checked_mul(self.batch.gradient_accumulation_steps)
            .ok_or_else(accounting_overflow)?;
        if self != &Self::canonical()
            || micro_batch_targets != self.batch.micro_batch_targets
            || update_targets != self.batch.optimizer_update_targets
            || self.loader.host_buffer_spans < self.batch.micro_batch_sequences
            || self.loader.maximum_in_flight_transfers > self.loader.host_buffer_spans
        {
            return Err(ProductError::integrity(
                "P14_DEFAULT_CONFIG_MISMATCH",
                "the prototype configuration differs from the selected P14 defaults",
            ));
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| {
            ProductError::internal(
                "P14_CONFIG_SERIALIZE_FAILED",
                "could not serialize the closed P14 configuration",
            )
        })?;
        Ok(sha256(&bytes))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDiagnosticsV1 {
    pub schema: String,
    pub profile: String,
    pub configuration_sha256: String,
    pub observations: Vec<ProfileObservationV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileObservationV1 {
    pub sample_id: String,
    pub valid_targets: u64,
    pub synchronized_elapsed_ns: u64,
    pub loader_wait_ns: u64,
    pub evaluation_ns: u64,
    pub checkpoint_ns: u64,
    pub peak_accelerator_bytes: Option<u64>,
    pub synchronized: bool,
    pub cleanup_complete: bool,
}

impl ProfileDiagnosticsV1 {
    pub fn validate(&self, configuration_sha256: &str) -> Result<()> {
        if self.schema != DIAGNOSTICS_SCHEMA
            || self.profile != PROTOTYPE_PROFILE
            || self.configuration_sha256 != configuration_sha256
        {
            return Err(ProductError::integrity(
                "P14_DIAGNOSTIC_IDENTITY_MISMATCH",
                "profiling diagnostics do not bind the selected prototype configuration",
            ));
        }
        let mut prior: Option<&str> = None;
        let mut identities = BTreeSet::new();
        for observation in &self.observations {
            if !portable_sample_id(&observation.sample_id)
                || !identities.insert(observation.sample_id.as_str())
                || prior.is_some_and(|value| value >= observation.sample_id.as_str())
                || observation.valid_targets == 0
                || observation.valid_targets > TOTAL_TARGETS
                || observation.synchronized_elapsed_ns == 0
                || observation.loader_wait_ns > observation.synchronized_elapsed_ns
                || observation.evaluation_ns > observation.synchronized_elapsed_ns
                || observation.checkpoint_ns > observation.synchronized_elapsed_ns
            {
                return Err(ProductError::integrity(
                    "P14_DIAGNOSTIC_INVALID",
                    "profiling observations are duplicated, unordered, empty, or arithmetically invalid",
                ));
            }
            prior = Some(&observation.sample_id);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub performance_status: String,
    pub profile: String,
    pub configuration: PrototypeTrainingDefaultsV1,
    pub configuration_sha256: String,
    pub diagnostics: ProfileDiagnosticsSummary,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDiagnosticsSummary {
    pub status: String,
    pub sample_count: u64,
    pub minimum_milli_targets_per_second: Option<u64>,
    pub maximum_milli_targets_per_second: Option<u64>,
    pub synchronized_samples: u64,
    pub cleanup_complete_samples: u64,
    pub maximum_peak_accelerator_bytes: Option<u64>,
    pub observations: Vec<ProfileObservationV1>,
}

pub fn parse_default_configuration(bytes: &[u8]) -> Result<PrototypeTrainingDefaultsV1> {
    let configuration =
        serde_json::from_slice::<PrototypeTrainingDefaultsV1>(bytes).map_err(|_| {
            ProductError::usage(
                "P14_CONFIG_INVALID",
                "the prototype configuration is not a closed P14 defaults object",
            )
        })?;
    configuration.validate()?;
    Ok(configuration)
}

pub fn parse_diagnostics(bytes: &[u8], configuration_sha256: &str) -> Result<ProfileDiagnosticsV1> {
    let diagnostics = serde_json::from_slice::<ProfileDiagnosticsV1>(bytes).map_err(|_| {
        ProductError::usage(
            "P14_DIAGNOSTICS_INVALID",
            "the profiling input is not a closed P14 diagnostics object",
        )
    })?;
    diagnostics.validate(configuration_sha256)?;
    Ok(diagnostics)
}

pub fn build_result(
    configuration: PrototypeTrainingDefaultsV1,
    diagnostics: Option<ProfileDiagnosticsV1>,
) -> Result<ProfileResultV1> {
    let configuration_sha256 = configuration.sha256()?;
    let observations = diagnostics
        .map(|value| {
            value.validate(&configuration_sha256)?;
            Ok(value.observations)
        })
        .transpose()?
        .unwrap_or_default();
    let rates = observations
        .iter()
        .map(milli_targets_per_second)
        .collect::<Result<Vec<_>>>()?;
    let minimum_rate = rates.iter().copied().min();
    let maximum_rate = rates.iter().copied().max();
    let maximum_peak = observations
        .iter()
        .filter_map(|value| value.peak_accelerator_bytes)
        .max();
    let synchronized_samples = observations
        .iter()
        .filter(|value| value.synchronized)
        .count();
    let cleanup_samples = observations
        .iter()
        .filter(|value| value.cleanup_complete)
        .count();
    let sample_count = observations.len() as u64;
    Ok(ProfileResultV1 {
        schema: RESULT_SCHEMA.to_owned(),
        status: "DEFAULT_CONFIGURED".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        performance_status: "UNVERIFIED".to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        configuration,
        configuration_sha256,
        diagnostics: ProfileDiagnosticsSummary {
            status: if observations.is_empty() {
                "UNAVAILABLE".to_owned()
            } else {
                "OBSERVED_UNVERIFIED".to_owned()
            },
            sample_count,
            minimum_milli_targets_per_second: minimum_rate,
            maximum_milli_targets_per_second: maximum_rate,
            synchronized_samples: synchronized_samples as u64,
            cleanup_complete_samples: cleanup_samples as u64,
            maximum_peak_accelerator_bytes: maximum_peak,
            observations,
        },
        limitations: vec![
            "not-hardware-qualification".to_owned(),
            "not-performance-admission".to_owned(),
            "not-sla-verification".to_owned(),
            "not-full-run-evidence".to_owned(),
            "diagnostics-never-retune-correctness-or-contract-constants".to_owned(),
        ],
    })
}

pub fn profile(config_path: &Path, diagnostics_path: Option<&Path>) -> Result<Value> {
    #[cfg(not(windows))]
    {
        let _ = (config_path, diagnostics_path);
        return Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "prototype profiling is implemented only for native Windows",
        ));
    }
    #[cfg(windows)]
    {
        let config_bytes = read_control_file(config_path, "P14_CONFIG_READ_FAILED")?;
        let configuration = parse_default_configuration(&config_bytes)?;
        let configuration_sha256 = configuration.sha256()?;
        let diagnostics = diagnostics_path
            .map(|path| {
                read_control_file(path, "P14_DIAGNOSTICS_READ_FAILED")
                    .and_then(|bytes| parse_diagnostics(&bytes, &configuration_sha256))
            })
            .transpose()?;
        serde_json::to_value(build_result(configuration, diagnostics)?).map_err(|_| {
            ProductError::internal(
                "P14_RESULT_SERIALIZE_FAILED",
                "could not serialize the closed P14 result",
            )
        })
    }
}

fn milli_targets_per_second(observation: &ProfileObservationV1) -> Result<u64> {
    let numerator = u128::from(observation.valid_targets)
        .checked_mul(1_000_000_000_000)
        .ok_or_else(accounting_overflow)?;
    u64::try_from(numerator / u128::from(observation.synchronized_elapsed_ns))
        .map_err(|_| accounting_overflow())
}

fn portable_sample_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(windows)]
pub(super) fn read_control_file(path: &Path, code: &'static str) -> Result<Vec<u8>> {
    if !path.is_absolute() {
        return Err(ProductError::usage(
            code,
            "the P14 control path must be absolute",
        ));
    }
    let before = fs::symlink_metadata(path)
        .map_err(|_| ProductError::environment(code, "could not inspect a P14 control file"))?;
    if !before.is_file() || is_reparse(&before) || before.len() > MAX_CONTROL_BYTES {
        return Err(ProductError::integrity(
            code,
            "a P14 control file is not regular, is a reparse point, or exceeds its bound",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
    options.share_mode(FILE_SHARE_READ);
    let mut file = options
        .open(path)
        .map_err(|_| ProductError::environment(code, "could not open a P14 control file"))?;
    let bound = file
        .metadata()
        .map_err(|_| ProductError::environment(code, "could not bind P14 control metadata"))?;
    if fingerprint(&before) != fingerprint(&bound) {
        return Err(ProductError::integrity(
            code,
            "a P14 control file changed before reading",
        ));
    }
    let mut bytes = Vec::with_capacity(bound.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|_| ProductError::environment(code, "could not read a P14 control file"))?;
    let after = file
        .metadata()
        .map_err(|_| ProductError::environment(code, "could not recheck P14 control metadata"))?;
    if fingerprint(&bound) != fingerprint(&after) || after.len() != bytes.len() as u64 {
        return Err(ProductError::integrity(
            code,
            "a P14 control file changed while reading",
        ));
    }
    Ok(bytes)
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileFingerprint {
    len: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
}

#[cfg(windows)]
fn fingerprint(metadata: &fs::Metadata) -> FileFingerprint {
    FileFingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        created: metadata.created().ok(),
    }
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn accounting_overflow() -> ProductError {
    ProductError::integrity(
        "P14_ACCOUNTING_OVERFLOW",
        "prototype profiling arithmetic overflowed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_defaults_are_exactly_canonical() {
        let parsed = parse_default_configuration(DEFAULT_CONFIG_BYTES).unwrap();
        assert_eq!(parsed, PrototypeTrainingDefaultsV1::canonical());
        assert_eq!(
            parsed.batch.micro_batch_targets * parsed.batch.gradient_accumulation_steps,
            TARGETS_PER_FULL_UPDATE
        );
    }

    #[test]
    fn diagnostics_are_observations_not_admission() {
        let configuration = PrototypeTrainingDefaultsV1::canonical();
        let digest = configuration.sha256().unwrap();
        let diagnostics = ProfileDiagnosticsV1 {
            schema: DIAGNOSTICS_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            configuration_sha256: digest,
            observations: vec![ProfileObservationV1 {
                sample_id: "sample-01".to_owned(),
                valid_targets: 65_536,
                synchronized_elapsed_ns: 1_000_000_000,
                loader_wait_ns: 10,
                evaluation_ns: 0,
                checkpoint_ns: 0,
                peak_accelerator_bytes: Some(3_000_000_000),
                synchronized: true,
                cleanup_complete: true,
            }],
        };
        let result = build_result(configuration, Some(diagnostics)).unwrap();
        assert_eq!(result.performance_status, "UNVERIFIED");
        assert_eq!(
            result.diagnostics.minimum_milli_targets_per_second,
            Some(65_536_000)
        );
        assert_eq!(result.configuration.sla.admission_seconds, 25_920);
        assert_eq!(result.configuration.sla.completion_seconds, 28_800);
    }
}
