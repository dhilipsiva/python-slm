//! Windows/Linux NVIDIA CUDA model adapter over the provider-generic parity graph.

use super::burn_graph::{GraphIdentity, run_repeated_parity};
use super::{AcceleratorCancellation, validate_repeated_provider_execution};
use crate::backend::{BURN_CUBECL_CUDA, ProviderIdentity};
use anyhow::Result;
use burn::backend::{Autodiff, Cuda};
use half::bf16;

type Gpu = Autodiff<Cuda<bf16, i32>>;

const IDENTITY: GraphIdentity = GraphIdentity {
    backend: BURN_CUBECL_CUDA,
    provider: ProviderIdentity::Cuda,
    error_prefix: "P10",
};

pub fn run_burn_cubecl_cuda_model_parity(
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<super::AcceleratorModelResult> {
    let device = burn::backend::cuda::CudaDevice::new(device_ordinal);
    let (first, second) =
        run_repeated_parity::<Gpu>(&IDENTITY, &device, device_ordinal, cancellation)?;
    super::validate_repeated_accelerator_execution(&first, &second).map_err(anyhow::Error::new)
}

pub fn run_burn_cubecl_cuda_provider_parity(
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<super::ProviderParityResult> {
    let device = burn::backend::cuda::CudaDevice::new(device_ordinal);
    let (first, second) =
        run_repeated_parity::<Gpu>(&IDENTITY, &device, device_ordinal, cancellation)?;
    validate_repeated_provider_execution(ProviderIdentity::Cuda, &first, &second)
        .map_err(anyhow::Error::new)
}
