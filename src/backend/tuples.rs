//! Closed P18 accelerator-provider tuple lanes behind the provider-neutral interface.

use super::{PROTOTYPE_PROFILE, ProviderIdentity, provider_backend_name};
use crate::error::{ProductError, Result};
use crate::platform::HostPlatform;
use serde::Serialize;

pub const PROVIDER_ADAPTER_MATRIX_SCHEMA: &str = "python-slm-provider-adapter-matrix-v1";
pub const DISCRETE_STAGING_MEMORY_PATH: &str = "page-locked-staging-async-ring-v1";
pub const UNIFIED_MEMORY_PATH: &str = "unified-shared-access-v1";

pub const P18_WINDOWS_NVIDIA_CUDA_REGRESSION: &str = "p18-windows-x86-64-nvidia-cuda-regression-v1";
pub const P18_LINUX_NVIDIA_CUDA: &str = "p18-linux-x86-64-nvidia-cuda-v1";
pub const P18_LINUX_AMD_ROCM: &str = "p18-linux-x86-64-amd-rocm-hip-v1";
pub const P18_MACOS_APPLE_METAL: &str = "p18-macos-arm64-apple-silicon-metal-v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcceleratorTupleLane {
    pub tuple_id: &'static str,
    pub host: HostPlatform,
    pub provider: ProviderIdentity,
    pub backend: &'static str,
    pub memory_path: &'static str,
    pub regression_lane: bool,
    pub support_level: &'static str,
    pub execution_status: &'static str,
    pub qualification_status: &'static str,
}

const fn lane(
    tuple_id: &'static str,
    host: HostPlatform,
    provider: ProviderIdentity,
    memory_path: &'static str,
    regression_lane: bool,
) -> AcceleratorTupleLane {
    AcceleratorTupleLane {
        tuple_id,
        host,
        provider,
        backend: provider_backend_name(provider),
        memory_path,
        regression_lane,
        support_level: "implemented",
        execution_status: "UNVERIFIED",
        qualification_status: "SKIPPED",
    }
}

pub const fn mandatory_tuple_lanes() -> [AcceleratorTupleLane; 4] {
    [
        lane(
            P18_WINDOWS_NVIDIA_CUDA_REGRESSION,
            HostPlatform::WindowsX86_64,
            ProviderIdentity::Cuda,
            DISCRETE_STAGING_MEMORY_PATH,
            true,
        ),
        lane(
            P18_LINUX_NVIDIA_CUDA,
            HostPlatform::LinuxX86_64,
            ProviderIdentity::Cuda,
            DISCRETE_STAGING_MEMORY_PATH,
            false,
        ),
        lane(
            P18_LINUX_AMD_ROCM,
            HostPlatform::LinuxX86_64,
            ProviderIdentity::Rocm,
            DISCRETE_STAGING_MEMORY_PATH,
            false,
        ),
        lane(
            P18_MACOS_APPLE_METAL,
            HostPlatform::MacosAppleSilicon,
            ProviderIdentity::Metal,
            UNIFIED_MEMORY_PATH,
            false,
        ),
    ]
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderAdapterMatrix {
    pub schema: &'static str,
    pub lanes: Vec<AcceleratorTupleLane>,
    pub deferred_selection_code: &'static str,
    pub limitations: Vec<&'static str>,
}

pub fn provider_adapter_matrix() -> ProviderAdapterMatrix {
    ProviderAdapterMatrix {
        schema: PROVIDER_ADAPTER_MATRIX_SCHEMA,
        lanes: mandatory_tuple_lanes().to_vec(),
        deferred_selection_code: "DEFERRED_POST_P16",
        limitations: vec![
            "no_tuple_qualification_claim",
            "no_unlisted_tuple_claim",
            "no_performance_equivalence_claim",
            "no_cross_provider_checkpoint_migration_claim",
            "no_two_billion_target_run_claim",
        ],
    }
}

pub fn implemented_profile_provider(profile: &str) -> Option<ProviderIdentity> {
    match profile {
        PROTOTYPE_PROFILE | P18_WINDOWS_NVIDIA_CUDA_REGRESSION | P18_LINUX_NVIDIA_CUDA => {
            Some(ProviderIdentity::Cuda)
        }
        P18_LINUX_AMD_ROCM => Some(ProviderIdentity::Rocm),
        P18_MACOS_APPLE_METAL => Some(ProviderIdentity::Metal),
        _ => None,
    }
}

pub fn is_implemented_training_profile(profile: &str) -> bool {
    implemented_profile_provider(profile).is_some()
}

pub fn require_implemented_tuple(
    host: HostPlatform,
    provider: ProviderIdentity,
) -> Result<AcceleratorTupleLane> {
    mandatory_tuple_lanes()
        .into_iter()
        .find(|lane| lane.host == host && lane.provider == provider)
        .ok_or_else(|| {
            ProductError::gate(
                "DEFERRED_POST_P16",
                format!("{host:?}/{provider:?} has no implemented accelerator provider adapter"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_mandatory_lanes_are_closed_and_skipped() {
        let matrix = provider_adapter_matrix();
        assert_eq!(matrix.schema, PROVIDER_ADAPTER_MATRIX_SCHEMA);
        assert_eq!(matrix.lanes.len(), 4);
        assert_eq!(
            matrix
                .lanes
                .iter()
                .filter(|lane| lane.regression_lane)
                .count(),
            1
        );
        for lane in &matrix.lanes {
            assert_eq!(lane.support_level, "implemented");
            assert_eq!(lane.execution_status, "UNVERIFIED");
            assert_eq!(lane.qualification_status, "SKIPPED");
            assert_eq!(lane.backend, provider_backend_name(lane.provider));
        }
        assert_eq!(matrix.lanes[3].memory_path, UNIFIED_MEMORY_PATH);
    }

    #[test]
    fn unlisted_tuples_stay_deferred_with_the_stable_code() {
        for (host, provider) in [
            (HostPlatform::WindowsX86_64, ProviderIdentity::Rocm),
            (HostPlatform::WindowsX86_64, ProviderIdentity::Metal),
            (HostPlatform::LinuxX86_64, ProviderIdentity::Metal),
            (HostPlatform::MacosAppleSilicon, ProviderIdentity::Cuda),
            (HostPlatform::MacosAppleSilicon, ProviderIdentity::Rocm),
        ] {
            assert_eq!(
                require_implemented_tuple(host, provider).unwrap_err().code,
                "DEFERRED_POST_P16"
            );
        }
    }

    #[test]
    fn implemented_profiles_map_to_their_exact_provider() {
        assert_eq!(
            implemented_profile_provider(PROTOTYPE_PROFILE),
            Some(ProviderIdentity::Cuda)
        );
        assert_eq!(
            implemented_profile_provider(P18_LINUX_AMD_ROCM),
            Some(ProviderIdentity::Rocm)
        );
        assert_eq!(
            implemented_profile_provider(P18_MACOS_APPLE_METAL),
            Some(ProviderIdentity::Metal)
        );
        assert_eq!(implemented_profile_provider("linux-anything-else"), None);
        assert!(is_implemented_training_profile(P18_LINUX_NVIDIA_CUDA));
    }
}
