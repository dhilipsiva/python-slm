use super::{
    CANONICAL_MODEL_ID, CPU_ORACLE_FIXTURE_ID, CpuOracleResult, canonical_parameter_specs,
    cpu_oracle_fixture, parameter_layout_sha256,
};
use crate::backend::{BURN_CUBECL_CUDA, ProviderIdentity};
use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const ACCELERATOR_MODEL_SCHEMA: &str = "python-slm-accelerator-model-result-v2";
pub const ACCELERATOR_OBSERVATION_SCHEMA: &str = "python-slm-accelerator-model-observation-v1";
pub const ACCELERATOR_PLAN_SCHEMA: &str = "python-slm-accelerator-model-plan-v1";
pub const P10_MODEL_SEMANTICS: &str = "pre-norm-gqa-rope-swiglu-causal-cross-entropy-v1";

const EXECUTION_STAGES: [&str; 8] = [
    "load-p9b-bf16-parameters",
    "embedding-and-block-forward",
    "fused-valid-target-cross-entropy",
    "autodiff-backward",
    "ordered-fp32-gradient-readback",
    "explicit-device-synchronization",
    "reverse-order-resource-release",
    "final-device-synchronization",
];
pub fn accelerator_execution_stages() -> Vec<String> {
    EXECUTION_STAGES.map(str::to_owned).to_vec()
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcceleratorExecutionPlan {
    pub schema: &'static str,
    pub model_identity: &'static str,
    pub parameter_layout_sha256: String,
    pub parameter_count: u64,
    pub parameter_artifact_count: usize,
    pub parameter_storage: &'static str,
    pub activation_storage: &'static str,
    pub accumulation: &'static str,
    pub gradient_storage: &'static str,
    pub attention: &'static str,
    pub loss: &'static str,
    pub activation_lifetime: &'static str,
    pub cancellation_boundaries: Vec<&'static str>,
    pub cleanup_order: &'static str,
}

pub fn accelerator_execution_plan() -> Result<AcceleratorExecutionPlan> {
    let specs = canonical_parameter_specs()?;
    Ok(AcceleratorExecutionPlan {
        schema: ACCELERATOR_PLAN_SCHEMA,
        model_identity: CANONICAL_MODEL_ID,
        parameter_layout_sha256: parameter_layout_sha256(&specs),
        parameter_count: specs.iter().map(|spec| spec.elements).sum(),
        parameter_artifact_count: specs.len(),
        parameter_storage: "bf16",
        activation_storage: "bf16",
        accumulation: "fp32",
        gradient_storage: "fp32",
        attention: "block-scoped-gqa-causal",
        loss: "valid-target-logsumexp-cross-entropy",
        activation_lifetime: "block-scoped-release-after-last-use",
        cancellation_boundaries: vec![
            "before-parameter-load",
            "after-parameter-load",
            "after-forward",
            "after-fused-loss",
            "after-backward",
            "after-readback",
        ],
        cleanup_order: "reverse-acquisition-then-final-sync",
    })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcceleratorModelObservation {
    pub schema: String,
    pub backend: String,
    pub provider: ProviderIdentity,
    pub device_ordinal: usize,
    pub fixture_id: String,
    pub model_semantics: String,
    pub parameter_layout_sha256: String,
    pub input_token_ids: Vec<usize>,
    pub target_token_ids: Vec<usize>,
    pub logits_bf16_le_hex: String,
    pub loss_f32_le_hex: String,
    pub gradient_f32_le_hex: String,
    pub gradient_sha256: String,
    pub stages_completed: Vec<String>,
    pub synchronized: bool,
    pub owned_resources_released: bool,
}

/// The `PRECISION-002` gradient bound. These values are the already-frozen,
/// provider-independent numbers from
/// `docs/schemas/P2/python-slm-backend-qualification-policy-v1.schema.json`; they
/// predate the E1A measurements, so the gate is predeclared rather than tuned to
/// its own result. The same bound applies to every candidate and every provider.
pub const GRADIENT_RELATIVE_L2_MAX: f64 = 0.03;
pub const GRADIENT_COSINE_MIN: f64 = 0.999;
pub const GRADIENT_ENVELOPE_ABSOLUTE_FLOOR: f64 = 0.0078125;
pub const GRADIENT_ENVELOPE_REFERENCE_MULTIPLIER: f64 = 0.03125;

/// Measured gradient agreement between a device observation and the oracle.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GradientConformance {
    pub relative_l2: f64,
    pub cosine_similarity: f64,
    pub envelope_violations: u64,
    pub nonfinite_values: u64,
    pub relative_l2_max: f64,
    pub cosine_min: f64,
    pub envelope_absolute_floor: f64,
    pub envelope_reference_multiplier: f64,
    pub within_bound: bool,
}

/// Compare device gradient bytes against the oracle under `PRECISION-002`.
///
/// Bit equality is unattainable across differing transcendental libraries, so the
/// gate is a predeclared bound. Determinism is unaffected: repeated execution and
/// resume remain byte-identical checks elsewhere.
pub fn gradient_conformance(device: &[u8], reference: &[u8]) -> GradientConformance {
    let mut squared_error = 0.0_f64;
    let mut squared_reference = 0.0_f64;
    let mut squared_device = 0.0_f64;
    let mut dot = 0.0_f64;
    let mut envelope_violations = 0_u64;
    let mut nonfinite_values = 0_u64;

    for (device_chunk, reference_chunk) in device.chunks_exact(4).zip(reference.chunks_exact(4)) {
        let device_value =
            f32::from_le_bytes(device_chunk.try_into().expect("four-byte gradient element"));
        let reference_value = f32::from_le_bytes(
            reference_chunk
                .try_into()
                .expect("four-byte gradient element"),
        );
        if !device_value.is_finite() {
            nonfinite_values += 1;
            continue;
        }
        let device_value = f64::from(device_value);
        let reference_value = f64::from(reference_value);
        let error = (device_value - reference_value).abs();
        let allowance = GRADIENT_ENVELOPE_ABSOLUTE_FLOOR
            + GRADIENT_ENVELOPE_REFERENCE_MULTIPLIER * reference_value.abs();
        if error > allowance {
            envelope_violations += 1;
        }
        squared_error += error * error;
        squared_reference += reference_value * reference_value;
        squared_device += device_value * device_value;
        dot += device_value * reference_value;
    }

    let relative_l2 = if squared_reference == 0.0 {
        squared_error.sqrt()
    } else {
        squared_error.sqrt() / squared_reference.sqrt()
    };
    let denominator = squared_device.sqrt() * squared_reference.sqrt();
    let cosine_similarity = if denominator == 0.0 {
        1.0
    } else {
        dot / denominator
    };
    let within_bound = nonfinite_values == 0
        && envelope_violations == 0
        && relative_l2.is_finite()
        && relative_l2 <= GRADIENT_RELATIVE_L2_MAX
        && cosine_similarity >= GRADIENT_COSINE_MIN;

    GradientConformance {
        relative_l2,
        cosine_similarity,
        envelope_violations,
        nonfinite_values,
        relative_l2_max: GRADIENT_RELATIVE_L2_MAX,
        cosine_min: GRADIENT_COSINE_MIN,
        envelope_absolute_floor: GRADIENT_ENVELOPE_ABSOLUTE_FLOOR,
        envelope_reference_multiplier: GRADIENT_ENVELOPE_REFERENCE_MULTIPLIER,
        within_bound,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcceleratorParityChecks {
    pub model_semantics_exact: bool,
    pub parameter_layout_exact: bool,
    pub inputs_exact: bool,
    pub logits_exact: bool,
    pub loss_exact: bool,
    pub gradient_within_bound: bool,
    pub gradient_conformance: GradientConformance,
    pub execution_stages_exact: bool,
    pub synchronized: bool,
    pub cleanup_complete: bool,
    pub repeated_execution_exact: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcceleratorModelResult {
    pub schema: &'static str,
    pub status: &'static str,
    pub qualification_status: &'static str,
    pub support_level: &'static str,
    pub model_identity: &'static str,
    pub backend: &'static str,
    pub provider: ProviderIdentity,
    pub fixture_id: &'static str,
    pub plan: AcceleratorExecutionPlan,
    pub expected_logits_sha256: String,
    pub expected_loss_sha256: String,
    pub expected_gradient_sha256: String,
    pub observation_sha256: String,
    pub checks: AcceleratorParityChecks,
    pub receipts_written: bool,
    pub limitations: Vec<&'static str>,
}

#[derive(Clone, Debug, Default)]
pub struct AcceleratorCancellation {
    cancelled: Arc<AtomicBool>,
}

impl AcceleratorCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn require_active(&self, boundary: &'static str) -> Result<()> {
        if self.is_cancelled() {
            return Err(ProductError::gate(
                "P10_EXECUTION_CANCELLED",
                format!("accelerator execution was cancelled at {boundary}"),
            ));
        }
        Ok(())
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn phase_code(prefix: &'static str, suffix: &str) -> String {
    format!("{prefix}_{suffix}")
}

fn decode_exact_hex(
    value: &str,
    expected_bytes: usize,
    code: impl Into<String>,
) -> Result<Vec<u8>> {
    let code = code.into();
    let bytes = hex::decode(value).map_err(|_| {
        ProductError::integrity(
            code.clone(),
            "an accelerator result is not lowercase hexadecimal",
        )
    })?;
    if value != hex::encode(&bytes) || bytes.len() != expected_bytes {
        return Err(ProductError::integrity(
            code,
            "an accelerator result has the wrong canonical byte length or spelling",
        ));
    }
    Ok(bytes)
}

fn observation_sha256(
    observation: &AcceleratorModelObservation,
    prefix: &'static str,
) -> Result<String> {
    let bytes = serde_json::to_vec(observation).map_err(|error| {
        ProductError::internal(
            phase_code(prefix, "OBSERVATION_SERIALIZATION_FAILED"),
            format!("could not serialize the closed accelerator observation: {error}"),
        )
    })?;
    Ok(sha256_hex(&bytes))
}

fn validate_single_observation(
    observation: &AcceleratorModelObservation,
    oracle: &CpuOracleResult,
    plan: &AcceleratorExecutionPlan,
    expected_backend: &str,
    expected_provider: ProviderIdentity,
    prefix: &'static str,
) -> Result<AcceleratorParityChecks> {
    if observation.schema != ACCELERATOR_OBSERVATION_SCHEMA
        || observation.backend != expected_backend
        || observation.provider != expected_provider
        || observation.fixture_id != CPU_ORACLE_FIXTURE_ID
    {
        return Err(ProductError::integrity(
            phase_code(prefix, "OBSERVATION_IDENTITY_MISMATCH"),
            format!(
                "the accelerator observation is not the closed {expected_backend} parity fixture"
            ),
        ));
    }
    let logits = decode_exact_hex(
        &observation.logits_bf16_le_hex,
        oracle.logits_bf16_le_hex.len() / 2,
        phase_code(prefix, "LOGITS_INVALID"),
    )?;
    let loss = decode_exact_hex(
        &observation.loss_f32_le_hex,
        oracle.loss_f32_le_hex.len() / 2,
        phase_code(prefix, "LOSS_INVALID"),
    )?;
    let gradient = decode_exact_hex(
        &observation.gradient_f32_le_hex,
        oracle.gradient_f32_le_hex.len() / 2,
        phase_code(prefix, "GRADIENT_INVALID"),
    )?;
    if sha256_hex(&gradient) != observation.gradient_sha256 {
        return Err(ProductError::integrity(
            phase_code(prefix, "GRADIENT_HASH_MISMATCH"),
            "the accelerator gradient bytes do not match their declared digest",
        ));
    }
    let expected_stages = EXECUTION_STAGES.map(str::to_owned).to_vec();
    // PRECISION-002: the forward is compared byte for byte; gradients are compared
    // against the frozen provider-independent bound, because differing transcendental
    // libraries make bit equality unattainable. See ADR 0001.
    let conformance = gradient_conformance(
        &gradient,
        &hex::decode(&oracle.gradient_f32_le_hex).unwrap(),
    );
    let checks = AcceleratorParityChecks {
        model_semantics_exact: observation.model_semantics == P10_MODEL_SEMANTICS,
        parameter_layout_exact: observation.parameter_layout_sha256 == plan.parameter_layout_sha256,
        inputs_exact: observation.input_token_ids == oracle.input_token_ids
            && observation.target_token_ids == oracle.target_token_ids,
        logits_exact: logits == hex::decode(&oracle.logits_bf16_le_hex).unwrap(),
        loss_exact: loss == hex::decode(&oracle.loss_f32_le_hex).unwrap(),
        gradient_within_bound: conformance.within_bound,
        gradient_conformance: conformance,
        execution_stages_exact: observation.stages_completed == expected_stages,
        synchronized: observation.synchronized,
        cleanup_complete: observation.owned_resources_released,
        repeated_execution_exact: false,
    };
    if !checks.model_semantics_exact
        || !checks.parameter_layout_exact
        || !checks.inputs_exact
        || !checks.logits_exact
        || !checks.loss_exact
        || !checks.gradient_within_bound
        || !checks.execution_stages_exact
        || !checks.synchronized
        || !checks.cleanup_complete
    {
        return Err(ProductError::gate(
            phase_code(prefix, "ACCELERATOR_PARITY_FAILED"),
            "the accelerator forward, fused loss, bounded gradient conformance, stage order, or cleanup differs from the CPU oracle",
        ));
    }
    Ok(checks)
}

pub fn validate_repeated_accelerator_execution(
    first: &AcceleratorModelObservation,
    second: &AcceleratorModelObservation,
) -> Result<AcceleratorModelResult> {
    let oracle = cpu_oracle_fixture();
    let plan = accelerator_execution_plan()?;
    let mut checks = validate_single_observation(
        first,
        &oracle,
        &plan,
        BURN_CUBECL_CUDA,
        ProviderIdentity::Cuda,
        "P10",
    )?;
    validate_single_observation(
        second,
        &oracle,
        &plan,
        BURN_CUBECL_CUDA,
        ProviderIdentity::Cuda,
        "P10",
    )?;
    if first != second {
        return Err(ProductError::gate(
            "P10_REPEATED_EXECUTION_DRIFT",
            "two accelerator executions did not produce byte-identical observations",
        ));
    }
    checks.repeated_execution_exact = true;
    let expected_logits = hex::decode(&oracle.logits_bf16_le_hex).unwrap();
    let expected_loss = hex::decode(&oracle.loss_f32_le_hex).unwrap();
    Ok(AcceleratorModelResult {
        schema: ACCELERATOR_MODEL_SCHEMA,
        status: "PARITY_OK",
        qualification_status: "SKIPPED",
        support_level: "implemented",
        model_identity: CANONICAL_MODEL_ID,
        backend: BURN_CUBECL_CUDA,
        provider: ProviderIdentity::Cuda,
        fixture_id: CPU_ORACLE_FIXTURE_ID,
        plan,
        expected_logits_sha256: sha256_hex(&expected_logits),
        expected_loss_sha256: sha256_hex(&expected_loss),
        expected_gradient_sha256: oracle.gradient_sha256,
        observation_sha256: observation_sha256(first, "P10")?,
        checks,
        receipts_written: false,
        limitations: vec![
            "no_hardware_qualification_claim",
            "no_full_model_vram_fit_claim",
            "no_optimizer_or_resume_claim",
            "no_throughput_or_sla_claim",
            "no_cross_provider_claim",
        ],
    })
}

pub const PROVIDER_PARITY_RESULT_SCHEMA: &str = "python-slm-provider-parity-result-v2";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderParityResult {
    pub schema: &'static str,
    pub status: &'static str,
    pub qualification_status: &'static str,
    pub support_level: &'static str,
    pub model_identity: &'static str,
    pub backend: &'static str,
    pub provider: ProviderIdentity,
    pub fixture_id: &'static str,
    pub plan: AcceleratorExecutionPlan,
    pub expected_logits_sha256: String,
    pub expected_loss_sha256: String,
    pub expected_gradient_sha256: String,
    pub observation_sha256: String,
    pub checks: AcceleratorParityChecks,
    pub receipts_written: bool,
    pub limitations: Vec<&'static str>,
}

pub fn validate_repeated_provider_execution(
    provider: ProviderIdentity,
    first: &AcceleratorModelObservation,
    second: &AcceleratorModelObservation,
) -> Result<ProviderParityResult> {
    let backend = crate::backend::provider_backend_name(provider);
    let oracle = cpu_oracle_fixture();
    let plan = accelerator_execution_plan()?;
    let mut checks = validate_single_observation(first, &oracle, &plan, backend, provider, "P18")?;
    validate_single_observation(second, &oracle, &plan, backend, provider, "P18")?;
    if first != second {
        return Err(ProductError::gate(
            "P18_REPEATED_EXECUTION_DRIFT",
            "two provider executions did not produce byte-identical observations",
        ));
    }
    checks.repeated_execution_exact = true;
    let expected_logits = hex::decode(&oracle.logits_bf16_le_hex).unwrap();
    let expected_loss = hex::decode(&oracle.loss_f32_le_hex).unwrap();
    Ok(ProviderParityResult {
        schema: PROVIDER_PARITY_RESULT_SCHEMA,
        status: "PARITY_OK",
        qualification_status: "SKIPPED",
        support_level: "implemented",
        model_identity: CANONICAL_MODEL_ID,
        backend,
        provider,
        fixture_id: CPU_ORACLE_FIXTURE_ID,
        plan,
        expected_logits_sha256: sha256_hex(&expected_logits),
        expected_loss_sha256: sha256_hex(&expected_loss),
        expected_gradient_sha256: oracle.gradient_sha256,
        observation_sha256: observation_sha256(first, "P18")?,
        checks,
        receipts_written: false,
        limitations: vec![
            "no_hardware_qualification_claim",
            "no_full_model_vram_fit_claim",
            "no_optimizer_or_resume_claim",
            "no_throughput_or_sla_claim",
            "no_performance_equivalence_claim",
            "no_cross_provider_checkpoint_migration_claim",
            "no_two_billion_target_run_claim",
        ],
    })
}

#[cfg(any(
    feature = "cuda",
    all(feature = "rocm", target_os = "linux"),
    all(feature = "metal", target_os = "macos")
))]
pub(crate) mod full_model;

#[cfg(feature = "cuda")]
mod cuda;

#[cfg(feature = "cuda")]
pub use cuda::{
    run_burn_cubecl_cuda_fixture_observation, run_burn_cubecl_cuda_model_parity,
    run_burn_cubecl_cuda_provider_parity,
};

#[cfg(all(feature = "rocm", target_os = "linux"))]
mod rocm;

#[cfg(all(feature = "rocm", target_os = "linux"))]
pub use rocm::run_burn_cubecl_rocm_model_parity;

#[cfg(all(feature = "metal", target_os = "macos"))]
mod metal;

#[cfg(all(feature = "metal", target_os = "macos"))]
pub use metal::run_burn_cubecl_metal_model_parity;

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_observation() -> AcceleratorModelObservation {
        let oracle = cpu_oracle_fixture();
        AcceleratorModelObservation {
            schema: ACCELERATOR_OBSERVATION_SCHEMA.to_owned(),
            backend: BURN_CUBECL_CUDA.to_owned(),
            provider: ProviderIdentity::Cuda,
            device_ordinal: 0,
            fixture_id: CPU_ORACLE_FIXTURE_ID.to_owned(),
            model_semantics: P10_MODEL_SEMANTICS.to_owned(),
            parameter_layout_sha256: accelerator_execution_plan()
                .unwrap()
                .parameter_layout_sha256,
            input_token_ids: oracle.input_token_ids,
            target_token_ids: oracle.target_token_ids,
            logits_bf16_le_hex: oracle.logits_bf16_le_hex,
            loss_f32_le_hex: oracle.loss_f32_le_hex,
            gradient_f32_le_hex: oracle.gradient_f32_le_hex,
            gradient_sha256: oracle.gradient_sha256,
            stages_completed: EXECUTION_STAGES.map(str::to_owned).to_vec(),
            synchronized: true,
            owned_resources_released: true,
        }
    }

    #[test]
    fn canonical_execution_plan_is_closed_and_memory_conscious() {
        let plan = accelerator_execution_plan().unwrap();
        assert_eq!(plan.parameter_count, 135_285_504);
        assert_eq!(plan.parameter_artifact_count, 111);
        assert_eq!(
            plan.parameter_layout_sha256,
            "3a0116a9e046f58bf5a93170328e3b6f6fa7d930391a4cefdbf545e74b827007"
        );
        assert_eq!(
            plan.activation_lifetime,
            "block-scoped-release-after-last-use"
        );
    }

    #[test]
    fn exact_repeated_observations_pass_without_qualification_claims() {
        let observation = passing_observation();
        let result = validate_repeated_accelerator_execution(&observation, &observation).unwrap();
        assert_eq!(result.status, "PARITY_OK");
        assert_eq!(result.qualification_status, "SKIPPED");
        assert!(result.checks.gradient_within_bound);
        assert_eq!(result.checks.gradient_conformance.relative_l2, 0.0);
        assert!(result.checks.repeated_execution_exact);
        assert!(!result.receipts_written);
    }

    /// Under `PRECISION-002` a one-ULP gradient difference is inside the frozen
    /// bound by design, but determinism is unrelaxed: two executions that differ at
    /// all still fail closed on repeated-execution drift.
    #[test]
    fn one_bit_gradient_drift_still_fails_closed_on_determinism() {
        let first = passing_observation();
        let mut second = first.clone();
        let mut gradient = hex::decode(&second.gradient_f32_le_hex).unwrap();
        gradient[0] ^= 1;
        second.gradient_f32_le_hex = hex::encode(&gradient);
        second.gradient_sha256 = sha256_hex(&gradient);
        assert_eq!(
            validate_repeated_accelerator_execution(&first, &second)
                .unwrap_err()
                .code,
            "P10_REPEATED_EXECUTION_DRIFT"
        );
    }

    /// A gradient far enough from the oracle to leave the frozen bound must still
    /// fail the parity gate, even when both executions agree with each other.
    #[test]
    fn out_of_bound_gradient_fails_the_parity_gate() {
        let mut observation = passing_observation();
        let gradient = hex::decode(&observation.gradient_f32_le_hex).unwrap();
        let corrupted = gradient
            .chunks_exact(4)
            .flat_map(|chunk| {
                let value = f32::from_le_bytes(chunk.try_into().unwrap());
                (value * 4.0 + 1.0).to_le_bytes()
            })
            .collect::<Vec<_>>();
        observation.gradient_f32_le_hex = hex::encode(&corrupted);
        observation.gradient_sha256 = sha256_hex(&corrupted);
        assert_eq!(
            validate_repeated_accelerator_execution(&observation, &observation)
                .unwrap_err()
                .code,
            "P10_ACCELERATOR_PARITY_FAILED"
        );
    }

    /// Non-finite gradients are rejected regardless of magnitude.
    #[test]
    fn nonfinite_gradients_fail_the_parity_gate() {
        let mut observation = passing_observation();
        let mut gradient = hex::decode(&observation.gradient_f32_le_hex).unwrap();
        gradient[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        observation.gradient_f32_le_hex = hex::encode(&gradient);
        observation.gradient_sha256 = sha256_hex(&gradient);
        assert_eq!(
            validate_repeated_accelerator_execution(&observation, &observation)
                .unwrap_err()
                .code,
            "P10_ACCELERATOR_PARITY_FAILED"
        );
    }

    #[test]
    fn stage_cleanup_and_repeatability_drift_fail_closed() {
        let first = passing_observation();
        let mut incomplete = first.clone();
        incomplete.owned_resources_released = false;
        assert_eq!(
            validate_repeated_accelerator_execution(&incomplete, &incomplete)
                .unwrap_err()
                .code,
            "P10_ACCELERATOR_PARITY_FAILED"
        );
        let mut different_device = first.clone();
        different_device.device_ordinal = 1;
        assert_eq!(
            validate_repeated_accelerator_execution(&first, &different_device)
                .unwrap_err()
                .code,
            "P10_REPEATED_EXECUTION_DRIFT"
        );
    }

    #[test]
    fn cancellation_is_monotonic_and_typed() {
        let cancellation = AcceleratorCancellation::default();
        cancellation
            .require_active("before-parameter-load")
            .unwrap();
        cancellation.cancel();
        assert!(cancellation.is_cancelled());
        assert_eq!(
            cancellation
                .require_active("after-forward")
                .unwrap_err()
                .code,
            "P10_EXECUTION_CANCELLED"
        );
    }

    #[test]
    fn fixture_parameters_are_shared_with_p9b_in_param_order() {
        let parameters = super::super::cpu_oracle_fixture_parameters();
        assert_eq!(parameters.len(), 12);
        assert_eq!(parameters[0].name, "tok_embeddings.weight");
        assert_eq!(parameters[11].name, "lm_head.weight");
        assert_eq!(
            parameters
                .iter()
                .map(|parameter| parameter.values_bf16_bits.len())
                .sum::<usize>(),
            140
        );
    }

    #[test]
    fn canonical_model_preset_remains_the_plan_identity() {
        assert_eq!(
            super::super::ModelPreset::Gqa135mV1.identity(),
            CANONICAL_MODEL_ID
        );
    }
}
