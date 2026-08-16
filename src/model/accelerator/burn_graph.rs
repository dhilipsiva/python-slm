//! Provider-generic burn graph for the closed P9B parity fixture.
//!
//! Every implemented accelerator adapter executes this exact one-layer GQA graph:
//! BF16 parameters and activations, explicit FP32 normalization/attention/loss
//! accumulation, head-local adjacent-pair RoPE, inclusive causal GQA, SwiGLU, an
//! untied LM head, valid-target-normalized cross-entropy, autodiff, and ordered
//! FP32 gradient readback. Providers differ only in device construction, the
//! observation identity, and their typed error prefix.

use super::{
    ACCELERATOR_OBSERVATION_SCHEMA, AcceleratorCancellation, AcceleratorModelObservation,
    EXECUTION_STAGES, P10_MODEL_SEMANTICS,
};
use crate::backend::ProviderIdentity;
use crate::model::{
    CPU_ORACLE_FIXTURE_ID, RMS_NORM_EPSILON, bf16_bits_to_f32, cpu_oracle_fixture_parameters,
    f32_to_bf16_bits, rope_angle,
};
use anyhow::{Context, Result, anyhow, ensure};
use burn::tensor::{FloatDType, Tensor, TensorData, activation::softmax, backend::AutodiffBackend};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const WIDTH: usize = 4;
const HEAD_WIDTH: usize = 2;
const INPUT_TOKENS: [usize; 2] = [1, 2];
const TARGET_TOKENS: [usize; 2] = [2, 3];

pub(super) struct GraphIdentity {
    pub backend: &'static str,
    pub provider: ProviderIdentity,
    pub error_prefix: &'static str,
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn check(cancellation: &AcceleratorCancellation, boundary: &'static str) -> Result<()> {
    cancellation
        .require_active(boundary)
        .map_err(anyhow::Error::new)
}

fn parameter<'a, B: AutodiffBackend>(
    parameters: &'a BTreeMap<String, Tensor<B, 2>>,
    name: &str,
    prefix: &'static str,
) -> Result<&'a Tensor<B, 2>> {
    parameters
        .get(name)
        .ok_or_else(|| anyhow!("{prefix}_PARAMETER_MISSING: {name}"))
}

fn load_parameters<B: AutodiffBackend>(
    device: &B::Device,
    prefix: &'static str,
) -> Result<BTreeMap<String, Tensor<B, 2>>> {
    let mut parameters = BTreeMap::new();
    for source in cpu_oracle_fixture_parameters() {
        ensure!(
            source.shape.len() == 1 || source.shape.len() == 2,
            "{prefix}_PARAMETER_RANK_INVALID"
        );
        let rows = if source.shape.len() == 1 {
            1
        } else {
            source.shape[0]
        };
        let columns = *source
            .shape
            .last()
            .with_context(|| format!("{prefix}_PARAMETER_SHAPE_EMPTY"))?;
        let values = source
            .values_bf16_bits
            .into_iter()
            .map(bf16_bits_to_f32)
            .collect::<Vec<_>>();
        let tensor = Tensor::<B, 2>::from_data(TensorData::new(values, [rows, columns]), device)
            .cast(FloatDType::BF16)
            .require_grad();
        ensure!(
            parameters.insert(source.name, tensor).is_none(),
            "{prefix}_PARAMETER_DUPLICATE"
        );
    }
    ensure!(parameters.len() == 12, "{prefix}_PARAMETER_COUNT_INVALID");
    Ok(parameters)
}

fn rms_norm<B: AutodiffBackend>(input: Tensor<B, 2>, scale: Tensor<B, 2>) -> Tensor<B, 2> {
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

fn linear<B: AutodiffBackend>(input: Tensor<B, 2>, weight: Tensor<B, 2>) -> Tensor<B, 2> {
    input
        .cast(FloatDType::F32)
        .matmul(weight.cast(FloatDType::F32).transpose())
        .cast(FloatDType::BF16)
}

fn apply_rope<B: AutodiffBackend>(
    input: Tensor<B, 2>,
    device: &B::Device,
    prefix: &'static str,
) -> Result<Tensor<B, 2>> {
    let [rows, columns] = input.dims();
    ensure!(
        columns.is_multiple_of(HEAD_WIDTH),
        "{prefix}_ROPE_WIDTH_INVALID"
    );
    let input = input.cast(FloatDType::F32);
    let mut output = Vec::with_capacity(columns);
    for column in (0..columns).step_by(2) {
        let pair_index = (column % HEAD_WIDTH) / 2;
        let angles = (0..rows)
            .map(|position| rope_angle(position, pair_index, HEAD_WIDTH))
            .collect::<crate::error::Result<Vec<_>>>()
            .map_err(anyhow::Error::new)?;
        let angle = Tensor::<B, 2>::from_data(TensorData::new(angles, [rows, 1]), device)
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

fn forward<B: AutodiffBackend>(
    parameters: &BTreeMap<String, Tensor<B, 2>>,
    device: &B::Device,
    prefix: &'static str,
) -> Result<Tensor<B, 2>> {
    let embeddings = parameter(parameters, "tok_embeddings.weight", prefix)?;
    let mut hidden = Tensor::cat(
        INPUT_TOKENS
            .into_iter()
            .map(|token| embeddings.clone().slice([token..token + 1, 0..WIDTH]))
            .collect(),
        0,
    );

    let normalized = rms_norm(
        hidden.clone(),
        parameter(parameters, "blocks.0.attn_norm.weight", prefix)?.clone(),
    );
    let query = apply_rope(
        linear(
            normalized.clone(),
            parameter(parameters, "blocks.0.attn.q.weight", prefix)?.clone(),
        ),
        device,
        prefix,
    )?;
    let key = apply_rope(
        linear(
            normalized.clone(),
            parameter(parameters, "blocks.0.attn.k.weight", prefix)?.clone(),
        ),
        device,
        prefix,
    )?;
    let value = linear(
        normalized,
        parameter(parameters, "blocks.0.attn.v.weight", prefix)?.clone(),
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
            parameter(parameters, "blocks.0.attn.o.weight", prefix)?.clone(),
        )
        .cast(FloatDType::F32))
    .cast(FloatDType::BF16);

    let normalized = rms_norm(
        hidden.clone(),
        parameter(parameters, "blocks.0.ffn_norm.weight", prefix)?.clone(),
    );
    let gate = linear(
        normalized.clone(),
        parameter(parameters, "blocks.0.ffn.gate.weight", prefix)?.clone(),
    )
    .cast(FloatDType::F32);
    let up = linear(
        normalized,
        parameter(parameters, "blocks.0.ffn.up.weight", prefix)?.clone(),
    )
    .cast(FloatDType::F32);
    let sigmoid = gate.clone().mul_scalar(-1.0).exp().add_scalar(1.0).recip();
    let activated = (gate * sigmoid * up).cast(FloatDType::BF16);
    hidden = (hidden.cast(FloatDType::F32)
        + linear(
            activated,
            parameter(parameters, "blocks.0.ffn.down.weight", prefix)?.clone(),
        )
        .cast(FloatDType::F32))
    .cast(FloatDType::BF16);

    let normalized = rms_norm(
        hidden,
        parameter(parameters, "final_norm.weight", prefix)?.clone(),
    );
    Ok(linear(
        normalized,
        parameter(parameters, "lm_head.weight", prefix)?.clone(),
    ))
}

fn fused_loss<B: AutodiffBackend>(
    logits: Tensor<B, 2>,
    device: &B::Device,
    prefix: &'static str,
) -> Result<Tensor<B, 1>> {
    let logits_f32 = logits.cast(FloatDType::F32);
    let host = logits_f32
        .clone()
        .try_into_data()
        .with_context(|| format!("{prefix}_LOGIT_READ_FAILED"))?
        .to_vec::<f32>()
        .with_context(|| format!("{prefix}_LOGIT_DTYPE_INVALID"))?;
    ensure!(host.len() == 8, "{prefix}_LOGIT_COUNT_INVALID");
    let maxima = host
        .chunks_exact(4)
        .map(|row| row.iter().copied().fold(f32::NEG_INFINITY, f32::max))
        .collect::<Vec<_>>();
    let maxima =
        Tensor::<B, 2>::from_data(TensorData::new(maxima, [2, 1]), device).cast(FloatDType::F32);
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

fn execute_once<B: AutodiffBackend>(
    identity: &GraphIdentity,
    device: &B::Device,
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<AcceleratorModelObservation> {
    let prefix = identity.error_prefix;
    check(cancellation, "before-parameter-load")?;
    let outcome = (|| -> Result<AcceleratorModelObservation> {
        let parameters = load_parameters::<B>(device, prefix)?;
        check(cancellation, "after-parameter-load")?;
        let logits = forward(&parameters, device, prefix)?;
        B::sync(device).with_context(|| format!("{prefix}_FORWARD_SYNCHRONIZATION_FAILED"))?;
        check(cancellation, "after-forward")?;
        let loss = fused_loss(logits.clone(), device, prefix)?;
        B::sync(device).with_context(|| format!("{prefix}_LOSS_SYNCHRONIZATION_FAILED"))?;
        check(cancellation, "after-fused-loss")?;
        let gradients = loss.clone().backward();
        B::sync(device).with_context(|| format!("{prefix}_BACKWARD_SYNCHRONIZATION_FAILED"))?;
        check(cancellation, "after-backward")?;

        let logits_values = logits
            .cast(FloatDType::F32)
            .try_into_data()
            .with_context(|| format!("{prefix}_LOGIT_READ_FAILED"))?
            .to_vec::<f32>()
            .with_context(|| format!("{prefix}_LOGIT_DTYPE_INVALID"))?;
        let logits_bytes = logits_values
            .into_iter()
            .flat_map(|value| f32_to_bf16_bits(value).to_le_bytes())
            .collect::<Vec<_>>();
        let loss_value = loss
            .try_into_data()
            .with_context(|| format!("{prefix}_LOSS_READ_FAILED"))?
            .to_vec::<f32>()
            .with_context(|| format!("{prefix}_LOSS_DTYPE_INVALID"))?;
        ensure!(loss_value.len() == 1, "{prefix}_LOSS_COUNT_INVALID");
        let mut gradient_bytes = Vec::with_capacity(140 * 4);
        for source in cpu_oracle_fixture_parameters() {
            let gradient = parameter(&parameters, &source.name, prefix)?
                .grad(&gradients)
                .with_context(|| format!("{prefix}_GRADIENT_MISSING: {}", source.name))?
                .cast(FloatDType::F32)
                .try_into_data()
                .with_context(|| format!("{prefix}_GRADIENT_READ_FAILED: {}", source.name))?
                .to_vec::<f32>()
                .with_context(|| format!("{prefix}_GRADIENT_DTYPE_INVALID: {}", source.name))?;
            ensure!(
                gradient.len() == source.values_bf16_bits.len(),
                "{prefix}_GRADIENT_SHAPE_MISMATCH: {}",
                source.name
            );
            gradient_bytes.extend(gradient.into_iter().flat_map(f32::to_le_bytes));
        }
        check(cancellation, "after-readback")?;
        Ok(AcceleratorModelObservation {
            schema: ACCELERATOR_OBSERVATION_SCHEMA.to_owned(),
            backend: identity.backend.to_owned(),
            provider: identity.provider,
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
    let final_sync =
        B::sync(device).with_context(|| format!("{prefix}_FINAL_SYNCHRONIZATION_FAILED"));
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
            Err(primary.context(format!("{prefix}_CLEANUP_FAILED_AFTER_ERROR: {cleanup:#}")))
        }
    }
}

pub(super) fn run_repeated_parity<B: AutodiffBackend>(
    identity: &GraphIdentity,
    device: &B::Device,
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<(AcceleratorModelObservation, AcceleratorModelObservation)> {
    let first = execute_once::<B>(identity, device, device_ordinal, cancellation)?;
    check(cancellation, "between-repeated-executions")?;
    let second = execute_once::<B>(identity, device, device_ordinal, cancellation)?;
    Ok((first, second))
}
