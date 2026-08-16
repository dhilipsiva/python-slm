//! P18 macOS/Apple Silicon Metal primitive backend fixture behind the provider-neutral types.
//!
//! Apple unified memory is exercised as shared device-visible allocation and command
//! synchronization; no host-to-device staging copy is claimed.

use super::{
    BURN_CUBECL_METAL, BackendFixtureDiagnostics, COMPATIBILITY_ALLOCATION_BYTES, ProviderIdentity,
    f32_le_hex,
};
use anyhow::{Context, ensure};
use burn::{
    backend::{Autodiff, Metal},
    tensor::{FloatDType, Tensor, TensorData, backend::Backend},
};
use half::bf16;

pub fn run_burn_cubecl_metal_fixture() -> anyhow::Result<BackendFixtureDiagnostics> {
    type Gpu = Autodiff<Metal<bf16, i32>>;
    let device = burn::backend::wgpu::WgpuDevice::default();

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
        .context("P18_METAL_FORWARD_READ_FAILED")?
        .to_vec::<f32>()
        .context("P18_METAL_FORWARD_DTYPE_INVALID")?;
    let expected_forward = [19.0_f32, 22.0, 43.0, 50.0];
    ensure!(
        forward_values == expected_forward,
        "P18_METAL_FORWARD_MISMATCH"
    );
    ensure!(
        forward_values.iter().all(|value| value.is_finite()),
        "P18_METAL_NONFINITE_RESULT"
    );

    let loss = forward.cast(FloatDType::F32).powf_scalar(2.0).mean();
    let gradients = loss.backward();
    let gradient_a = a
        .grad(&gradients)
        .context("P18_METAL_GRADIENT_A_MISSING")?
        .cast(FloatDType::F32)
        .try_into_data()
        .context("P18_METAL_GRADIENT_A_READ_FAILED")?
        .to_vec::<f32>()
        .context("P18_METAL_GRADIENT_A_DTYPE_INVALID")?;
    let gradient_b = b
        .grad(&gradients)
        .context("P18_METAL_GRADIENT_B_MISSING")?
        .cast(FloatDType::F32)
        .try_into_data()
        .context("P18_METAL_GRADIENT_B_READ_FAILED")?
        .to_vec::<f32>()
        .context("P18_METAL_GRADIENT_B_DTYPE_INVALID")?;
    let expected_a = [124.5_f32, 145.0, 282.5, 329.0];
    let expected_b = [74.0_f32, 86.0, 105.0, 122.0];
    ensure!(
        gradient_a == expected_a && gradient_b == expected_b,
        "P18_METAL_GRADIENT_BYTES_MISMATCH"
    );
    ensure!(
        gradient_a
            .iter()
            .chain(&gradient_b)
            .all(|value| value.is_finite()),
        "P18_METAL_NONFINITE_GRADIENT"
    );

    let elements = usize::try_from(COMPATIBILITY_ALLOCATION_BYTES / 2)
        .context("P18_METAL_ALLOCATION_SIZE_OVERFLOW")?;
    let allocation = Tensor::<Gpu, 1>::zeros([elements], &device).add_scalar(7.0_f32);
    let first = allocation
        .clone()
        .slice(0..1)
        .cast(FloatDType::F32)
        .try_into_data()
        .context("P18_METAL_SENTINEL_FIRST_READ_FAILED")?
        .to_vec::<f32>()
        .context("P18_METAL_SENTINEL_FIRST_DTYPE_INVALID")?[0];
    let last = allocation
        .clone()
        .slice(elements - 1..elements)
        .cast(FloatDType::F32)
        .try_into_data()
        .context("P18_METAL_SENTINEL_LAST_READ_FAILED")?
        .to_vec::<f32>()
        .context("P18_METAL_SENTINEL_LAST_DTYPE_INVALID")?[0];
    ensure!(first == 7.0 && last == 7.0, "P18_METAL_SENTINEL_MISMATCH");
    Gpu::sync(&device).context("P18_METAL_SYNCHRONIZATION_FAILED")?;

    let diagnostics = BackendFixtureDiagnostics {
        schema: "python-slm-p18-burn-cubecl-metal-fixture-v1".to_owned(),
        status: "PASS".to_owned(),
        backend: BURN_CUBECL_METAL.to_owned(),
        provider: ProviderIdentity::Metal,
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
    Gpu::sync(&device).context("P18_METAL_FINAL_SYNCHRONIZATION_FAILED")?;
    Ok(diagnostics)
}
