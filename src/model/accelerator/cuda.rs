use super::{
    ACCELERATOR_OBSERVATION_SCHEMA, AcceleratorCancellation, AcceleratorModelObservation,
    EXECUTION_STAGES, P10_MODEL_SEMANTICS, validate_repeated_accelerator_execution,
};
use crate::backend::{BURN_CUBECL_CUDA, ProviderIdentity};
use crate::model::{
    CPU_ORACLE_FIXTURE_ID, RMS_NORM_EPSILON, bf16_bits_to_f32, cpu_oracle_fixture_parameters,
    f32_to_bf16_bits, rope_angle,
};
use anyhow::{Context, Result, anyhow, ensure};
use burn::{
    backend::{Autodiff, Cuda},
    tensor::{FloatDType, Tensor, TensorData, activation::softmax, backend::Backend},
};
use half::bf16;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const WIDTH: usize = 4;
const HEAD_WIDTH: usize = 2;
const INPUT_TOKENS: [usize; 2] = [1, 2];
const TARGET_TOKENS: [usize; 2] = [2, 3];

type Gpu = Autodiff<Cuda<bf16, i32>>;
type Matrix = Tensor<Gpu, 2>;
type Parameters = BTreeMap<String, Matrix>;

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn check(cancellation: &AcceleratorCancellation, boundary: &'static str) -> Result<()> {
    cancellation
        .require_active(boundary)
        .map_err(anyhow::Error::new)
}

fn parameter<'a>(parameters: &'a Parameters, name: &str) -> Result<&'a Matrix> {
    parameters
        .get(name)
        .ok_or_else(|| anyhow!("P10_PARAMETER_MISSING: {name}"))
}

fn load_parameters(device: &burn::backend::cuda::CudaDevice) -> Result<Parameters> {
    let mut parameters = BTreeMap::new();
    for source in cpu_oracle_fixture_parameters() {
        ensure!(
            source.shape.len() == 1 || source.shape.len() == 2,
            "P10_PARAMETER_RANK_INVALID"
        );
        let rows = if source.shape.len() == 1 {
            1
        } else {
            source.shape[0]
        };
        let columns = *source.shape.last().context("P10_PARAMETER_SHAPE_EMPTY")?;
        let values = source
            .values_bf16_bits
            .into_iter()
            .map(bf16_bits_to_f32)
            .collect::<Vec<_>>();
        let tensor = Tensor::<Gpu, 2>::from_data(TensorData::new(values, [rows, columns]), device)
            .cast(FloatDType::BF16)
            .require_grad();
        ensure!(
            parameters.insert(source.name, tensor).is_none(),
            "P10_PARAMETER_DUPLICATE"
        );
    }
    ensure!(parameters.len() == 12, "P10_PARAMETER_COUNT_INVALID");
    Ok(parameters)
}

fn rms_norm(input: Matrix, scale: Matrix) -> Matrix {
    let input_f32 = input.cast(FloatDType::F32);
    let inverse = input_f32
        .clone()
        .powf_scalar(2.0)
        .mean_dim(1)
        .add_scalar(RMS_NORM_EPSILON)
        .sqrt()
        .recip();
    (input_f32 * inverse * scale.cast(FloatDType::F32)).cast(FloatDType::BF16)
}

fn linear(input: Matrix, weight: Matrix) -> Matrix {
    input
        .cast(FloatDType::F32)
        .matmul(weight.cast(FloatDType::F32).transpose())
        .cast(FloatDType::BF16)
}

fn apply_rope(input: Matrix, device: &burn::backend::cuda::CudaDevice) -> Result<Matrix> {
    let [rows, columns] = input.dims();
    ensure!(columns.is_multiple_of(HEAD_WIDTH), "P10_ROPE_WIDTH_INVALID");
    let input = input.cast(FloatDType::F32);
    let mut output = Vec::with_capacity(columns);
    for column in (0..columns).step_by(2) {
        let pair_index = (column % HEAD_WIDTH) / 2;
        let angles = (0..rows)
            .map(|position| rope_angle(position, pair_index, HEAD_WIDTH))
            .collect::<crate::error::Result<Vec<_>>>()
            .map_err(anyhow::Error::new)?;
        let angle = Tensor::<Gpu, 2>::from_data(TensorData::new(angles, [rows, 1]), device)
            .cast(FloatDType::F32);
        let cosine = angle.clone().cos();
        let sine = angle.sin();
        let left = input.clone().slice([0..rows, column..column + 1]);
        let right = input.clone().slice([0..rows, column + 1..column + 2]);
        output.push(
            (left.clone() * cosine.clone() - right.clone() * sine.clone()).cast(FloatDType::BF16),
        );
        output.push((left * sine + right * cosine).cast(FloatDType::BF16));
    }
    Ok(Tensor::cat(output, 1))
}

fn forward(parameters: &Parameters, device: &burn::backend::cuda::CudaDevice) -> Result<Matrix> {
    let embeddings = parameter(parameters, "tok_embeddings.weight")?;
    let mut hidden = Tensor::cat(
        INPUT_TOKENS
            .into_iter()
            .map(|token| embeddings.clone().slice([token..token + 1, 0..WIDTH]))
            .collect(),
        0,
    );

    let normalized = rms_norm(
        hidden.clone(),
        parameter(parameters, "blocks.0.attn_norm.weight")?.clone(),
    );
    let query = apply_rope(
        linear(
            normalized.clone(),
            parameter(parameters, "blocks.0.attn.q.weight")?.clone(),
        ),
        device,
    )?;
    let key = apply_rope(
        linear(
            normalized.clone(),
            parameter(parameters, "blocks.0.attn.k.weight")?.clone(),
        ),
        device,
    )?;
    let value = linear(
        normalized,
        parameter(parameters, "blocks.0.attn.v.weight")?.clone(),
    );

    let mut attention_rows = Vec::with_capacity(2);
    for query_position in 0..2 {
        let mut contexts = Vec::with_capacity(2);
        for query_head in 0..2 {
            let query_start = query_head * HEAD_WIDTH;
            let query_slice = query
                .clone()
                .slice([
                    query_position..query_position + 1,
                    query_start..query_start + HEAD_WIDTH,
                ])
                .cast(FloatDType::F32);
            let key_slice = key
                .clone()
                .slice([0..query_position + 1, 0..HEAD_WIDTH])
                .cast(FloatDType::F32);
            let scores = query_slice
                .matmul(key_slice.transpose())
                .div_scalar((HEAD_WIDTH as f32).sqrt());
            let probabilities = softmax(scores, 1);
            contexts.push(
                probabilities
                    .matmul(
                        value
                            .clone()
                            .slice([0..query_position + 1, 0..HEAD_WIDTH])
                            .cast(FloatDType::F32),
                    )
                    .cast(FloatDType::BF16),
            );
        }
        attention_rows.push(Tensor::cat(contexts, 1));
    }
    let attention = Tensor::cat(attention_rows, 0);
    hidden = (hidden.cast(FloatDType::F32)
        + linear(
            attention,
            parameter(parameters, "blocks.0.attn.o.weight")?.clone(),
        )
        .cast(FloatDType::F32))
    .cast(FloatDType::BF16);

    let normalized = rms_norm(
        hidden.clone(),
        parameter(parameters, "blocks.0.ffn_norm.weight")?.clone(),
    );
    let gate = linear(
        normalized.clone(),
        parameter(parameters, "blocks.0.ffn.gate.weight")?.clone(),
    )
    .cast(FloatDType::F32);
    let up = linear(
        normalized,
        parameter(parameters, "blocks.0.ffn.up.weight")?.clone(),
    )
    .cast(FloatDType::F32);
    let sigmoid = gate.clone().mul_scalar(-1.0).exp().add_scalar(1.0).recip();
    let activated = (gate * sigmoid * up).cast(FloatDType::BF16);
    hidden = (hidden.cast(FloatDType::F32)
        + linear(
            activated,
            parameter(parameters, "blocks.0.ffn.down.weight")?.clone(),
        )
        .cast(FloatDType::F32))
    .cast(FloatDType::BF16);

    let normalized = rms_norm(hidden, parameter(parameters, "final_norm.weight")?.clone());
    Ok(linear(
        normalized,
        parameter(parameters, "lm_head.weight")?.clone(),
    ))
}

fn fused_loss(logits: Matrix, device: &burn::backend::cuda::CudaDevice) -> Result<Tensor<Gpu, 1>> {
    let logits_f32 = logits.cast(FloatDType::F32);
    let host = logits_f32
        .clone()
        .try_into_data()
        .context("P10_LOGIT_READ_FAILED")?
        .to_vec::<f32>()
        .context("P10_LOGIT_DTYPE_INVALID")?;
    ensure!(host.len() == 8, "P10_LOGIT_COUNT_INVALID");
    let maxima = host
        .chunks_exact(4)
        .map(|row| row.iter().copied().fold(f32::NEG_INFINITY, f32::max))
        .collect::<Vec<_>>();
    let maxima =
        Tensor::<Gpu, 2>::from_data(TensorData::new(maxima, [2, 1]), device).cast(FloatDType::F32);
    let log_partition = (logits_f32.clone() - maxima.clone()).exp().sum_dim(1).log() + maxima;
    let targets = Tensor::cat(
        TARGET_TOKENS
            .into_iter()
            .enumerate()
            .map(|(row, target)| logits_f32.clone().slice([row..row + 1, target..target + 1]))
            .collect(),
        0,
    );
    Ok((log_partition - targets).sum().div_scalar(2.0))
}

fn execute_once(
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<AcceleratorModelObservation> {
    check(cancellation, "before-parameter-load")?;
    let device = burn::backend::cuda::CudaDevice::new(device_ordinal);
    let outcome = (|| -> Result<AcceleratorModelObservation> {
        let parameters = load_parameters(&device)?;
        check(cancellation, "after-parameter-load")?;
        let logits = forward(&parameters, &device)?;
        Gpu::sync(&device).context("P10_FORWARD_SYNCHRONIZATION_FAILED")?;
        check(cancellation, "after-forward")?;
        let loss = fused_loss(logits.clone(), &device)?;
        Gpu::sync(&device).context("P10_LOSS_SYNCHRONIZATION_FAILED")?;
        check(cancellation, "after-fused-loss")?;
        let gradients = loss.clone().backward();
        Gpu::sync(&device).context("P10_BACKWARD_SYNCHRONIZATION_FAILED")?;
        check(cancellation, "after-backward")?;

        let logits_values = logits
            .cast(FloatDType::F32)
            .try_into_data()
            .context("P10_LOGIT_READ_FAILED")?
            .to_vec::<f32>()
            .context("P10_LOGIT_DTYPE_INVALID")?;
        let logits_bytes = logits_values
            .into_iter()
            .flat_map(|value| f32_to_bf16_bits(value).to_le_bytes())
            .collect::<Vec<_>>();
        let loss_value = loss
            .try_into_data()
            .context("P10_LOSS_READ_FAILED")?
            .to_vec::<f32>()
            .context("P10_LOSS_DTYPE_INVALID")?;
        ensure!(loss_value.len() == 1, "P10_LOSS_COUNT_INVALID");
        let mut gradient_bytes = Vec::with_capacity(140 * 4);
        for source in cpu_oracle_fixture_parameters() {
            let gradient = parameter(&parameters, &source.name)?
                .grad(&gradients)
                .with_context(|| format!("P10_GRADIENT_MISSING: {}", source.name))?
                .cast(FloatDType::F32)
                .try_into_data()
                .with_context(|| format!("P10_GRADIENT_READ_FAILED: {}", source.name))?
                .to_vec::<f32>()
                .with_context(|| format!("P10_GRADIENT_DTYPE_INVALID: {}", source.name))?;
            ensure!(
                gradient.len() == source.values_bf16_bits.len(),
                "P10_GRADIENT_SHAPE_MISMATCH: {}",
                source.name
            );
            gradient_bytes.extend(gradient.into_iter().flat_map(f32::to_le_bytes));
        }
        check(cancellation, "after-readback")?;
        Ok(AcceleratorModelObservation {
            schema: ACCELERATOR_OBSERVATION_SCHEMA.to_owned(),
            backend: BURN_CUBECL_CUDA.to_owned(),
            provider: ProviderIdentity::Cuda,
            device_ordinal,
            fixture_id: CPU_ORACLE_FIXTURE_ID.to_owned(),
            model_semantics: P10_MODEL_SEMANTICS.to_owned(),
            parameter_layout_sha256: super::accelerator_execution_plan()
                .map_err(anyhow::Error::new)?
                .parameter_layout_sha256,
            input_token_ids: INPUT_TOKENS.to_vec(),
            target_token_ids: TARGET_TOKENS.to_vec(),
            logits_bf16_le_hex: hex::encode(logits_bytes),
            loss_f32_le_hex: hex::encode(loss_value[0].to_le_bytes()),
            gradient_f32_le_hex: hex::encode(&gradient_bytes),
            gradient_sha256: sha256_hex(&gradient_bytes),
            stages_completed: EXECUTION_STAGES[..6]
                .iter()
                .map(|stage| (*stage).to_owned())
                .collect(),
            synchronized: true,
            owned_resources_released: false,
        })
    })();
    let final_sync = Gpu::sync(&device).context("P10_FINAL_SYNCHRONIZATION_FAILED");
    match (outcome, final_sync) {
        (Ok(mut observation), Ok(())) => {
            observation.stages_completed.extend(
                EXECUTION_STAGES[6..]
                    .iter()
                    .map(|stage| (*stage).to_owned()),
            );
            observation.owned_resources_released = true;
            Ok(observation)
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(primary), Err(cleanup)) => {
            Err(primary.context(format!("P10_CLEANUP_FAILED_AFTER_ERROR: {cleanup:#}")))
        }
    }
}

pub fn run_burn_cubecl_cuda_model_parity(
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<super::AcceleratorModelResult> {
    let first = execute_once(device_ordinal, cancellation)?;
    check(cancellation, "between-repeated-executions")?;
    let second = execute_once(device_ordinal, cancellation)?;
    validate_repeated_accelerator_execution(&first, &second).map_err(anyhow::Error::new)
}
