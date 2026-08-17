//! Windows/Linux NVIDIA CUDA model adapter over the provider-generic parity graph.

use super::full_model::{GraphIdentity, run_repeated_batched_parity, run_repeated_parity};
use super::{AcceleratorCancellation, validate_repeated_provider_execution};
use crate::backend::{BURN_CUBECL_CUDA, ProviderIdentity};
use anyhow::Result;
use burn::backend::{Autodiff, Cuda};
use burn_autodiff::checkpoint::strategy::BalancedCheckpointing;
use half::bf16;

// Must match the training backend strategy, or the conformance gate would verify a
// path production no longer uses.
type Gpu = Autodiff<Cuda<bf16, i32>, BalancedCheckpointing>;

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

/// The same conformance gate, executed at a chosen batch width.
///
/// The fixture at one sequence does not cover the shape production runs, and batch
/// width demonstrably moves the gradient: kernels of different shapes reduce in
/// different orders. Running the frozen oracle comparison at the operating width is
/// what makes `PRECISION-002` a gate on the configuration that will actually train.
pub fn run_burn_cubecl_cuda_batched_model_parity(
    device_ordinal: usize,
    batch: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<super::AcceleratorModelResult> {
    let device = burn::backend::cuda::CudaDevice::new(device_ordinal);
    let (first, second) = run_repeated_batched_parity::<Gpu>(
        &IDENTITY,
        &device,
        device_ordinal,
        batch,
        cancellation,
    )?;
    super::validate_repeated_accelerator_execution(&first, &second).map_err(anyhow::Error::new)
}

/// One raw fixture observation for diagnostics, without the parity verdict.
pub fn run_burn_cubecl_cuda_fixture_observation(
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<super::AcceleratorModelObservation> {
    let device = burn::backend::cuda::CudaDevice::new(device_ordinal);
    let (first, _) = run_repeated_parity::<Gpu>(&IDENTITY, &device, device_ordinal, cancellation)?;
    Ok(first)
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
