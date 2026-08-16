//! E1A root-cause diagnostics for the exact-gradient gate conflict.
//!
//! These probes isolate each candidate mechanism behind the device-versus-oracle
//! gradient divergence, so the E1A decision rests on measurements rather than on
//! ranked hypotheses. They are diagnostics, not gates: each one reports what it
//! observed and only fails when a mechanism it can control is wrong.
//!
//! Run with:
//!   cargo test --locked --features cuda --test e1a_numerical_probe -- --ignored --nocapture

#![cfg(feature = "cuda")]

use burn::{
    backend::{Autodiff, Cuda},
    tensor::{DType, Tensor, TensorCreationOptions, TensorData, backend::Backend},
};
use half::bf16;

type Gpu = Autodiff<Cuda<bf16, i32>>;
type Device = burn::backend::cuda::CudaDevice;

fn options(device: &Device) -> TensorCreationOptions<Gpu> {
    TensorCreationOptions::new(device.clone()).with_dtype(DType::F32)
}

fn upload(values: &[f32], device: &Device) -> Tensor<Gpu, 1> {
    Tensor::<Gpu, 1>::from_data(
        TensorData::new(values.to_vec(), [values.len()]),
        options(device),
    )
}

fn download(tensor: Tensor<Gpu, 1>) -> Vec<f32> {
    tensor
        .try_into_data()
        .expect("device readback")
        .to_vec::<f32>()
        .expect("f32 readback")
}

/// Operand magnitudes taken from the closed P9B fixture: parameters land on
/// k/64 grids, RoPE angles for head width two are exactly the integer positions,
/// and softmax operands are small shifted scores.
fn representative_operands() -> Vec<f32> {
    let mut values = Vec::new();
    for step in -15..=15 {
        values.push(step as f32 / 64.0);
    }
    values.extend([0.0, 1.0, 2.0, 3.0, 0.5, 1.5, 1.0 / 3.0, 2.0 / 3.0]);
    values.extend([1.0e-5, 1.0 + 1.0 / 64.0, 1.0 - 1.0 / 64.0]);
    values
}

fn report_unary(
    name: &str,
    inputs: &[f32],
    host: impl Fn(f32) -> f32,
    device_values: Vec<f32>,
) -> usize {
    let mut mismatches = 0;
    let mut worst = 0_i64;
    let mut worst_case = (0.0_f32, 0.0_f32, 0.0_f32);
    for (input, device_value) in inputs.iter().zip(&device_values) {
        let expected = host(*input);
        if expected.to_bits() != device_value.to_bits() {
            mismatches += 1;
            let ulps = (expected.to_bits() as i64 - device_value.to_bits() as i64).abs();
            if ulps > worst {
                worst = ulps;
                worst_case = (*input, expected, *device_value);
            }
        }
    }
    if mismatches == 0 {
        println!("  {name:<6} BIT-IDENTICAL over {} operands", inputs.len());
    } else {
        println!(
            "  {name:<6} DIVERGES on {mismatches}/{} operands, worst {worst} ULP \
             (input {}, host {:e}, device {:e})",
            inputs.len(),
            worst_case.0,
            worst_case.1,
            worst_case.2
        );
    }
    mismatches
}

/// Probe 1: do the device transcendentals equal Rust's host libm bit for bit?
/// This is the candidate the repository cannot control by any setting.
#[test]
#[ignore = "E1A diagnostic; run explicitly with --ignored"]
fn probe_transcendental_agreement() {
    let device = Device::new(0);
    let operands = representative_operands();
    let positive = operands
        .iter()
        .copied()
        .filter(|value| *value > 0.0)
        .collect::<Vec<_>>();

    println!("\n=== E1A probe 1: transcendental agreement (device vs Rust host) ===");
    let mut total = 0;
    total += report_unary(
        "exp",
        &operands,
        f32::exp,
        download(upload(&operands, &device).exp()),
    );
    total += report_unary(
        "ln",
        &positive,
        f32::ln,
        download(upload(&positive, &device).log()),
    );
    total += report_unary(
        "sin",
        &operands,
        f32::sin,
        download(upload(&operands, &device).sin()),
    );
    total += report_unary(
        "cos",
        &operands,
        f32::cos,
        download(upload(&operands, &device).cos()),
    );
    total += report_unary(
        "sqrt",
        &positive,
        f32::sqrt,
        download(upload(&positive, &device).sqrt()),
    );
    total += report_unary(
        "recip",
        &positive,
        |value| 1.0 / value,
        download(upload(&positive, &device).recip()),
    );
    println!("  => total diverging transcendental results: {total}");
}

/// Probe 2 and 3: separate elementwise multiplication, reduction order, and the
/// fused matmul path, so contraction and reassociation can be told apart.
#[test]
#[ignore = "E1A diagnostic; run explicitly with --ignored"]
fn probe_contraction_and_reassociation() {
    let device = Device::new(0);
    // Fixture-shaped operands: the oracle's dot products are width two and four.
    let left = (0..64)
        .map(|i| (i as f32 - 31.0) / 64.0)
        .collect::<Vec<_>>();
    let right = (0..64)
        .map(|i| (i as f32 * 7.0 % 31.0 - 15.0) / 64.0)
        .collect::<Vec<_>>();

    println!("\n=== E1A probe 2/3: contraction and reassociation ===");

    // Elementwise product alone, read back before any reduction can fuse with it.
    let products = download(upload(&left, &device) * upload(&right, &device));
    let host_products = left
        .iter()
        .zip(&right)
        .map(|(a, b)| a * b)
        .collect::<Vec<_>>();
    let product_mismatches = products
        .iter()
        .zip(&host_products)
        .filter(|(device_value, host_value)| device_value.to_bits() != host_value.to_bits())
        .count();
    println!(
        "  elementwise multiply: {} / {} differ from host",
        product_mismatches,
        host_products.len()
    );

    for width in [2_usize, 4, 8, 64] {
        let host_sequential = host_products[..width]
            .iter()
            .fold(0.0_f32, |total, value| total + value);

        // Reduce the already-exact products on device: isolates reduction order.
        let device_sum = download(upload(&products[..width], &device).sum())[0];

        // Fused dot product through matmul: multiply and add may contract.
        let matmul = Tensor::<Gpu, 2>::from_data(
            TensorData::new(left[..width].to_vec(), [1, width]),
            TensorCreationOptions::new(device.clone()).with_dtype(DType::F32),
        )
        .matmul(Tensor::<Gpu, 2>::from_data(
            TensorData::new(right[..width].to_vec(), [width, 1]),
            TensorCreationOptions::new(device.clone()).with_dtype(DType::F32),
        ));
        let device_matmul = matmul
            .try_into_data()
            .expect("matmul readback")
            .to_vec::<f32>()
            .expect("f32 readback")[0];

        println!(
            "  width {width:>2}: host_seq={:.9e} sum={} matmul={}",
            host_sequential,
            if device_sum.to_bits() == host_sequential.to_bits() {
                "exact".to_owned()
            } else {
                format!(
                    "DIFFERS ({} ULP)",
                    (device_sum.to_bits() as i64 - host_sequential.to_bits() as i64).abs()
                )
            },
            if device_matmul.to_bits() == host_sequential.to_bits() {
                "exact".to_owned()
            } else {
                format!(
                    "DIFFERS ({} ULP)",
                    (device_matmul.to_bits() as i64 - host_sequential.to_bits() as i64).abs()
                )
            }
        );
    }
    Gpu::sync(&device).expect("final synchronization");
}

/// Probe 4: which gradient artifacts diverge, and by how much, against the frozen
/// oracle. Quantifies the deviation the E1A decision must bound.
#[test]
#[ignore = "E1A diagnostic; run explicitly with --ignored"]
fn probe_gradient_deviation_profile() {
    use rust_llm_pretrain::model::{
        AcceleratorCancellation, cpu_oracle_fixture, run_burn_cubecl_cuda_fixture_observation,
    };

    let observation =
        run_burn_cubecl_cuda_fixture_observation(0, &AcceleratorCancellation::default())
            .expect("fixture observation");
    let oracle = cpu_oracle_fixture();

    println!("\n=== E1A probe 4: gradient deviation profile ===");
    println!(
        "  forward logits exact: {}",
        observation.logits_bf16_le_hex == oracle.logits_bf16_le_hex
    );
    println!(
        "  forward loss exact:   {}",
        observation.loss_f32_le_hex == oracle.loss_f32_le_hex
    );

    let device = hex::decode(&observation.gradient_f32_le_hex).expect("device gradient hex");
    let reference = hex::decode(&oracle.gradient_f32_le_hex).expect("oracle gradient hex");
    let mut worst_relative = 0.0_f64;
    let mut squared_error = 0.0_f64;
    let mut squared_reference = 0.0_f64;
    let mut dot = 0.0_f64;
    let mut device_norm = 0.0_f64;

    for artifact in &oracle.gradient_artifacts {
        let range = artifact.byte_offset..artifact.byte_offset + artifact.byte_length;
        let mut artifact_worst = 0.0_f64;
        let mut differing = 0;
        for (device_chunk, reference_chunk) in device[range.clone()]
            .chunks_exact(4)
            .zip(reference[range].chunks_exact(4))
        {
            let device_value =
                f32::from_le_bytes(device_chunk.try_into().expect("four bytes")) as f64;
            let reference_value =
                f32::from_le_bytes(reference_chunk.try_into().expect("four bytes")) as f64;
            if device_value.to_bits() != reference_value.to_bits() {
                differing += 1;
            }
            let error = (device_value - reference_value).abs();
            let relative = if reference_value == 0.0 {
                error
            } else {
                error / reference_value.abs()
            };
            artifact_worst = artifact_worst.max(relative);
            worst_relative = worst_relative.max(relative);
            squared_error += error * error;
            squared_reference += reference_value * reference_value;
            dot += device_value * reference_value;
            device_norm += device_value * device_value;
        }
        println!(
            "  {:<32} {differing:>4}/{:<4} differ, worst relative {artifact_worst:.3e}",
            artifact.name, artifact.elements
        );
    }

    let relative_l2 = squared_error.sqrt() / squared_reference.sqrt();
    let cosine = dot / (device_norm.sqrt() * squared_reference.sqrt());
    println!("  ---");
    println!("  worst elementwise relative deviation: {worst_relative:.6e}");
    println!("  gradient relative L2:                 {relative_l2:.6e}");
    println!("  gradient cosine similarity:           {cosine:.12}");
    println!("  frozen P2 policy bounds: relative_l2_max 0.03, cosine_min 0.999");
}
