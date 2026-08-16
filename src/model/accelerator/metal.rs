//! macOS/Apple Silicon Metal model adapter over the provider-generic parity graph.
//!
//! Apple Silicon exposes one unified-memory device; parameters and activations are
//! shared device-visible allocations synchronized at command boundaries, never a
//! host-to-device staging copy.

use super::burn_graph::{GraphIdentity, run_repeated_parity};
use super::{AcceleratorCancellation, validate_repeated_provider_execution};
use crate::backend::{BURN_CUBECL_METAL, ProviderIdentity};
use anyhow::{Result, ensure};
use burn::backend::{Autodiff, Metal};
use half::bf16;

type Gpu = Autodiff<Metal<bf16, i32>>;

const IDENTITY: GraphIdentity = GraphIdentity {
    backend: BURN_CUBECL_METAL,
    provider: ProviderIdentity::Metal,
    error_prefix: "P18_METAL",
};

pub fn run_burn_cubecl_metal_model_parity(
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<super::ProviderParityResult> {
    ensure!(
        device_ordinal == 0,
        "P18_METAL_DEVICE_ORDINAL_UNSUPPORTED: Apple Silicon exposes one unified device"
    );
    let device = burn::backend::wgpu::WgpuDevice::default();
    let (first, second) =
        run_repeated_parity::<Gpu>(&IDENTITY, &device, device_ordinal, cancellation)?;
    validate_repeated_provider_execution(ProviderIdentity::Metal, &first, &second)
        .map_err(anyhow::Error::new)
}
