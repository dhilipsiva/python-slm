use serde::{Deserialize, Serialize};
use std::fmt;

pub const PROTOTYPE_PROFILE: &str = "prototype-windows-5090-v1";
pub use crate::model::COMPATIBILITY_ALLOCATION_BYTES;
pub const BURN_CUBECL_CUDA: &str = "burn-cubecl-cuda";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderIdentity {
    Cuda,
    Rocm,
    Metal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendRequestKind {
    Auto,
    BurnCubeclCuda,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackendRequest {
    pub profile: String,
    pub provider: ProviderIdentity,
    pub backend: BackendRequestKind,
    pub device_uuid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackendIdentity {
    pub backend: String,
    pub provider: ProviderIdentity,
    pub framework: String,
    pub framework_version: String,
    pub runtime_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StableDeviceSelection {
    pub uuid: String,
    pub model: String,
    pub compute_capability: String,
    pub total_vram_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "backend", rename_all = "kebab-case")]
pub enum RuntimeBackend {
    BurnCubeclCuda { device_uuid: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionDiagnostics {
    pub correctness_passed: bool,
    pub exact_gradient_bytes_passed: bool,
    pub compatibility_memory_passed: bool,
    pub launch_and_synchronization_passed: bool,
    pub cleanup_passed: bool,
    pub p1b_diagnostic_result_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackendSelection {
    pub request: BackendRequest,
    pub selected: RuntimeBackend,
    pub identity: BackendIdentity,
    pub capability: BackendCapability,
    pub device: StableDeviceSelection,
    pub diagnostics: SelectionDiagnostics,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackendCapability {
    pub backend: String,
    pub provider: ProviderIdentity,
    pub support_level: String,
    pub framework: String,
    pub autodiff: bool,
    pub bf16: bool,
    pub exact_gradient_bytes: bool,
    pub compatibility_allocation_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateResult {
    pub capability: BackendCapability,
    pub compiled: bool,
    pub implemented: bool,
    pub passed: bool,
    pub failure_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackendFixtureDiagnostics {
    pub schema: String,
    pub status: String,
    pub backend: String,
    pub provider: ProviderIdentity,
    pub device_ordinal: usize,
    pub forward_values_f32: Vec<f32>,
    pub forward_exact: bool,
    pub gradient_a_f32_le_hex: String,
    pub gradient_b_f32_le_hex: String,
    pub gradient_bytes_exact: bool,
    pub finite: bool,
    pub allocation_bytes: u64,
    pub allocation_touched: bool,
    pub sentinel_first: f32,
    pub sentinel_last: f32,
    pub synchronized: bool,
    pub owned_resources_released: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendSelectionError {
    pub code: &'static str,
    pub message: String,
}

impl fmt::Display for BackendSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BackendSelectionError {}

pub fn burn_cubecl_cuda_capability() -> BackendCapability {
    BackendCapability {
        backend: BURN_CUBECL_CUDA.to_owned(),
        provider: ProviderIdentity::Cuda,
        support_level: "implemented".to_owned(),
        framework: "burn-cubecl".to_owned(),
        autodiff: true,
        bf16: true,
        exact_gradient_bytes: true,
        compatibility_allocation_bytes: COMPATIBILITY_ALLOCATION_BYTES,
    }
}

pub fn select_candidate<'a>(
    request: &BackendRequest,
    candidates: &'a [CandidateResult],
) -> Result<&'a CandidateResult, BackendSelectionError> {
    if request.profile != PROTOTYPE_PROFILE {
        return Err(BackendSelectionError {
            code: "DEFERRED_POST_P16",
            message: format!("profile {} is not implemented", request.profile),
        });
    }
    if request.provider != ProviderIdentity::Cuda {
        return Err(BackendSelectionError {
            code: "DEFERRED_POST_P16",
            message: format!("provider {:?} is deferred", request.provider),
        });
    }

    let eligible = candidates
        .iter()
        .filter(|candidate| {
            candidate.capability.provider == request.provider
                && candidate.compiled
                && candidate.implemented
                && candidate.passed
        })
        .collect::<Vec<_>>();

    match request.backend {
        BackendRequestKind::Auto => match eligible.as_slice() {
            [candidate] => Ok(*candidate),
            [] => Err(BackendSelectionError {
                code: "P2_BACKEND_NOT_AVAILABLE",
                message: "no compiled implemented CUDA candidate passed every required check"
                    .to_owned(),
            }),
            _ => Err(BackendSelectionError {
                code: "P2_BACKEND_AMBIGUOUS",
                message: "multiple CUDA candidates passed; automatic ranking is forbidden"
                    .to_owned(),
            }),
        },
        BackendRequestKind::BurnCubeclCuda => candidates
            .iter()
            .find(|candidate| candidate.capability.backend == BURN_CUBECL_CUDA)
            .filter(|candidate| {
                candidate.capability.provider == request.provider
                    && candidate.compiled
                    && candidate.implemented
                    && candidate.passed
            })
            .ok_or_else(|| BackendSelectionError {
                code: "P2_BACKEND_NOT_AVAILABLE",
                message: "explicit burn-cubecl-cuda selection failed without fallback".to_owned(),
            }),
    }
}

#[cfg(feature = "cuda")]
pub fn run_burn_cubecl_cuda_fixture() -> anyhow::Result<BackendFixtureDiagnostics> {
    use anyhow::{Context, ensure};
    use burn::{
        backend::{Autodiff, Cuda},
        tensor::{FloatDType, Tensor, TensorData, backend::Backend},
    };
    use half::bf16;

    type Gpu = Autodiff<Cuda<bf16, i32>>;
    let device = burn::backend::cuda::CudaDevice::new(0);

    let a = Tensor::<Gpu, 2>::from_data(
        TensorData::new(vec![1.0_f32, 2.0, 3.0, 4.0], [2, 2]),
        &device,
    )
    .require_grad();
    let b = Tensor::<Gpu, 2>::from_data(
        TensorData::new(vec![5.0_f32, 6.0, 7.0, 8.0], [2, 2]),
        &device,
    )
    .require_grad();
    let forward = a.clone().matmul(b.clone());
    let forward_values = forward
        .clone()
        .cast(FloatDType::F32)
        .try_into_data()
        .context("P2_FORWARD_READ_FAILED")?
        .to_vec::<f32>()
        .context("P2_FORWARD_DTYPE_INVALID")?;
    let expected_forward = [19.0_f32, 22.0, 43.0, 50.0];
    ensure!(forward_values == expected_forward, "P2_FORWARD_MISMATCH");
    ensure!(
        forward_values.iter().all(|value| value.is_finite()),
        "P2_NONFINITE_RESULT"
    );

    let loss = forward.cast(FloatDType::F32).powf_scalar(2.0).mean();
    let gradients = loss.backward();
    let gradient_a = a
        .grad(&gradients)
        .context("P2_GRADIENT_A_MISSING")?
        .cast(FloatDType::F32)
        .try_into_data()
        .context("P2_GRADIENT_A_READ_FAILED")?
        .to_vec::<f32>()
        .context("P2_GRADIENT_A_DTYPE_INVALID")?;
    let gradient_b = b
        .grad(&gradients)
        .context("P2_GRADIENT_B_MISSING")?
        .cast(FloatDType::F32)
        .try_into_data()
        .context("P2_GRADIENT_B_READ_FAILED")?
        .to_vec::<f32>()
        .context("P2_GRADIENT_B_DTYPE_INVALID")?;
    let expected_a = [124.5_f32, 145.0, 282.5, 329.0];
    let expected_b = [74.0_f32, 86.0, 105.0, 122.0];
    ensure!(
        gradient_a == expected_a && gradient_b == expected_b,
        "P2_GRADIENT_BYTES_MISMATCH"
    );
    ensure!(
        gradient_a
            .iter()
            .chain(&gradient_b)
            .all(|value| value.is_finite()),
        "P2_NONFINITE_GRADIENT"
    );

    let elements = usize::try_from(COMPATIBILITY_ALLOCATION_BYTES / 2)
        .context("P2_ALLOCATION_SIZE_OVERFLOW")?;
    let allocation = Tensor::<Gpu, 1>::zeros([elements], &device).add_scalar(7.0_f32);
    let first = allocation
        .clone()
        .slice(0..1)
        .cast(FloatDType::F32)
        .try_into_data()
        .context("P2_SENTINEL_FIRST_READ_FAILED")?
        .to_vec::<f32>()
        .context("P2_SENTINEL_FIRST_DTYPE_INVALID")?[0];
    let last = allocation
        .clone()
        .slice(elements - 1..elements)
        .cast(FloatDType::F32)
        .try_into_data()
        .context("P2_SENTINEL_LAST_READ_FAILED")?
        .to_vec::<f32>()
        .context("P2_SENTINEL_LAST_DTYPE_INVALID")?[0];
    ensure!(first == 7.0 && last == 7.0, "P2_SENTINEL_MISMATCH");
    Gpu::sync(&device).context("P2_SYNCHRONIZATION_FAILED")?;

    let diagnostics = BackendFixtureDiagnostics {
        schema: "python-slm-p2-burn-cubecl-cuda-fixture-v1".to_owned(),
        status: "PASS".to_owned(),
        backend: BURN_CUBECL_CUDA.to_owned(),
        provider: ProviderIdentity::Cuda,
        device_ordinal: 0,
        forward_values_f32: forward_values,
        forward_exact: true,
        gradient_a_f32_le_hex: f32_le_hex(&gradient_a),
        gradient_b_f32_le_hex: f32_le_hex(&gradient_b),
        gradient_bytes_exact: true,
        finite: true,
        allocation_bytes: COMPATIBILITY_ALLOCATION_BYTES,
        allocation_touched: true,
        sentinel_first: first,
        sentinel_last: last,
        synchronized: true,
        owned_resources_released: true,
    };
    drop(allocation);
    Gpu::sync(&device).context("P2_FINAL_SYNCHRONIZATION_FAILED")?;
    Ok(diagnostics)
}

#[cfg(feature = "cuda")]
fn f32_le_hex(values: &[f32]) -> String {
    let bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, passed: bool) -> CandidateResult {
        let mut capability = burn_cubecl_cuda_capability();
        capability.backend = name.to_owned();
        CandidateResult {
            capability,
            compiled: true,
            implemented: true,
            passed,
            failure_code: (!passed).then(|| "FAILED".to_owned()),
        }
    }

    fn request(kind: BackendRequestKind) -> BackendRequest {
        BackendRequest {
            profile: PROTOTYPE_PROFILE.to_owned(),
            provider: ProviderIdentity::Cuda,
            backend: kind,
            device_uuid: None,
        }
    }

    #[test]
    fn runtime_backend_serialization_does_not_leak_framework_types() {
        let value = serde_json::to_value(RuntimeBackend::BurnCubeclCuda {
            device_uuid: "GPU-00000000-0000-0000-0000-000000000001".to_owned(),
        })
        .unwrap();
        assert_eq!(value["backend"], "burn-cubecl-cuda");
        assert_eq!(value.as_object().unwrap().len(), 2);
    }

    #[test]
    fn auto_selects_the_sole_passing_candidate() {
        let values = vec![candidate(BURN_CUBECL_CUDA, true)];
        assert_eq!(
            select_candidate(&request(BackendRequestKind::Auto), &values)
                .unwrap()
                .capability
                .backend,
            BURN_CUBECL_CUDA
        );
    }

    #[test]
    fn auto_rejects_zero_and_multiple_passing_candidates() {
        let none = vec![candidate(BURN_CUBECL_CUDA, false)];
        assert_eq!(
            select_candidate(&request(BackendRequestKind::Auto), &none)
                .unwrap_err()
                .code,
            "P2_BACKEND_NOT_AVAILABLE"
        );
        let many = vec![
            candidate(BURN_CUBECL_CUDA, true),
            candidate("future-cuda", true),
        ];
        assert_eq!(
            select_candidate(&request(BackendRequestKind::Auto), &many)
                .unwrap_err()
                .code,
            "P2_BACKEND_AMBIGUOUS"
        );
    }

    #[test]
    fn explicit_selection_never_falls_back() {
        let values = vec![
            candidate(BURN_CUBECL_CUDA, false),
            candidate("future-cuda", true),
        ];
        assert_eq!(
            select_candidate(&request(BackendRequestKind::BurnCubeclCuda), &values)
                .unwrap_err()
                .code,
            "P2_BACKEND_NOT_AVAILABLE"
        );
    }

    #[test]
    fn deferred_provider_is_rejected_before_candidate_policy() {
        let mut value = request(BackendRequestKind::Auto);
        value.provider = ProviderIdentity::Rocm;
        assert_eq!(
            select_candidate(&value, &[]).unwrap_err().code,
            "DEFERRED_POST_P16"
        );
    }
}
