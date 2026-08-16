//! Linux/AMD ROCm/HIP model adapter over the provider-generic parity graph.

use super::full_model::{GraphIdentity, run_repeated_parity};
use super::{AcceleratorCancellation, validate_repeated_provider_execution};
use crate::backend::{BURN_CUBECL_ROCM, ProviderIdentity};
use anyhow::Result;
use burn::backend::{Autodiff, Rocm};
use burn_autodiff::checkpoint::strategy::BalancedCheckpointing;
use half::bf16;

type Gpu = Autodiff<Rocm<bf16, i32>, BalancedCheckpointing>;

const IDENTITY: GraphIdentity = GraphIdentity {
    backend: BURN_CUBECL_ROCM,
    provider: ProviderIdentity::Rocm,
    error_prefix: "P18_ROCM",
};

pub fn run_burn_cubecl_rocm_model_parity(
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<super::ProviderParityResult> {
    let device = burn::backend::rocm::RocmDevice::new(device_ordinal);
    let (first, second) =
        run_repeated_parity::<Gpu>(&IDENTITY, &device, device_ordinal, cancellation)?;
    validate_repeated_provider_execution(ProviderIdentity::Rocm, &first, &second)
        .map_err(anyhow::Error::new)
}
