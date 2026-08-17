//! Provider-generic full-model GQA training graph.
//!
//! One configuration-parameterized graph serves every implemented accelerator
//! adapter at both the closed P9B fixture scale and the canonical
//! `gqa-135m-v1` scale. The frozen mixed-precision semantics are implemented
//! exactly as the scalar oracle defines them: every tensor is FP32 on device,
//! and each frozen BF16 storage point applies straight-through quantization —
//! the forward value is the BF16 round-to-nearest-even quantization while the
//! backward pass is the exact identity (`bf16_cast_gradient: "identity"`).
//! Host-derived constants (RoPE tables, the causal mask, token indices) are
//! created with an explicit F32 dtype so device-default BF16 creation can never
//! round them.
//!
//! Training splits one step into several backward passes so that no stage of the
//! graph is retained longer than it is needed: the cross-entropy head is
//! differentiated one position chunk at a time, and each layer is forwarded once
//! untracked to record its boundary and recomputed with a tape only while its own
//! gradient is being taken. The stages are joined by the vector-Jacobian seed in
//! [`vector_jacobian_seed`], which is exact, so the split changes peak memory
//! without changing the gradient.

use super::{
    ACCELERATOR_OBSERVATION_SCHEMA, AcceleratorCancellation, AcceleratorModelObservation,
    EXECUTION_STAGES, P10_MODEL_SEMANTICS,
};
use crate::model::{
    CPU_ORACLE_FIXTURE_ID, RMS_NORM_EPSILON, bf16_bits_to_f32, f32_to_bf16_bits, rope_angle,
};
use crate::train::full_state::GqaDimensions;
use crate::train::trainer::AdamwParameterState;
use anyhow::{Context, Result, anyhow, ensure};
use burn::tensor::{
    DType, FloatDType, Int, Tensor, TensorCreationOptions, TensorData, activation::softmax,
    backend::AutodiffBackend,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(crate) struct GraphIdentity {
    pub backend: &'static str,
    pub provider: crate::backend::ProviderIdentity,
    pub error_prefix: &'static str,
}

pub(crate) struct FullGraphOutput {
    pub loss_f32: f32,
    pub gradient_f32: Vec<f32>,
    pub logits_bf16_bits: Vec<u16>,
}

fn f32_options<B: AutodiffBackend>(device: &B::Device) -> TensorCreationOptions<B> {
    TensorCreationOptions::new(device.clone()).with_dtype(DType::F32)
}

/// Straight-through BF16 storage: the forward value is the exact BF16
/// round-to-nearest-even quantization, the backward pass is the identity.
fn bf16_store<B: AutodiffBackend, const D: usize>(tensor: Tensor<B, D>) -> Tensor<B, D> {
    let quantized = tensor
        .clone()
        .cast(FloatDType::BF16)
        .cast(FloatDType::F32)
        .detach();
    let residual = (quantized - tensor.clone().detach()).detach();
    tensor + residual
}

/// Flattened positions per cross-entropy chunk.
///
/// The `[positions, vocabulary]` logits are by far the largest tensor in the
/// graph, so bounding the head means bounding this width rather than the sequence
/// length or the batch. It is a Rust constant because the frozen profile defaults
/// file is byte-pinned and cannot take a new field.
///
/// The value is comfortably above the two-position P9B fixture, so the fixture
/// stays a single chunk and its loss keeps the exact reduction order the frozen
/// oracle bytes were taken from.
pub(crate) const HEAD_CHUNK_POSITIONS: usize = 512;

/// One dispatch's tokens, flattened over the batch.
///
/// Every sequence in a dispatch shares a length; callers group by length so the
/// ragged final update stays exact rather than padded into a uniform shape.
struct BatchTokens {
    batch: usize,
    positions: usize,
    input_ids: Vec<i32>,
    target_ids: Vec<i32>,
}

fn flatten_sequences(sequences: &[(&[u16], &[u16])], max_context: usize) -> Result<BatchTokens> {
    ensure!(!sequences.is_empty(), "E1_GRAPH_BATCH_EMPTY");
    let positions = sequences[0].0.len();
    ensure!(
        positions > 0 && positions <= max_context,
        "E1_GRAPH_SEQUENCE_INVALID"
    );
    ensure!(
        sequences
            .iter()
            .all(|(inputs, targets)| inputs.len() == positions && targets.len() == positions),
        "E1_GRAPH_SEQUENCE_INVALID"
    );
    let widen = |token: &u16| i32::from(*token);
    Ok(BatchTokens {
        batch: sequences.len(),
        positions,
        input_ids: sequences
            .iter()
            .flat_map(|(inputs, _)| inputs.iter().map(widen))
            .collect(),
        target_ids: sequences
            .iter()
            .flat_map(|(_, targets)| targets.iter().map(widen))
            .collect(),
    })
}

/// Half-open chunk boundaries over the flattened positions of one dispatch.
fn head_chunks(rows: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..rows)
        .step_by(HEAD_CHUNK_POSITIONS)
        .map(move |start| (start, (start + HEAD_CHUNK_POSITIONS).min(rows)))
}

/// The vector-Jacobian seed that resumes one backward pass from another's result.
///
/// `sum(value * cotangent)` differentiates to exactly `cotangent` at `value`: the
/// reduction contributes a seed of one and the multiplication passes `cotangent`
/// through scaled by it, and multiplication by one is exact in IEEE-754. That is
/// what lets a step be split into stages without perturbing the gradient. burn
/// seeds `backward()` with ones over the root's shape and never requires a scalar
/// root, but reducing here keeps that seed a single element rather than a tensor
/// the size of `value`.
fn vector_jacobian_seed<B: AutodiffBackend>(
    value: Tensor<B, 3>,
    cotangent: Tensor<B::InnerBackend, 3>,
) -> Tensor<B, 1> {
    (value * Tensor::from_inner(cotangent)).sum()
}

/// Gradient totals across the several backward passes one training step is split
/// into.
///
/// Each pass differentiates a different stage, so a parameter absent from one
/// contributes nothing to it, and a parameter that appears in several — the LM
/// head, once per cross-entropy chunk — is summed on device rather than read back
/// repeatedly.
struct GradientAccumulator<B: AutodiffBackend> {
    totals: BTreeMap<String, Tensor<B::InnerBackend, 2>>,
}

impl<B: AutodiffBackend> GradientAccumulator<B> {
    fn new() -> Self {
        Self {
            totals: BTreeMap::new(),
        }
    }

    fn absorb(&mut self, parameters: &BTreeMap<String, Tensor<B, 2>>, gradients: &B::Gradients) {
        for (name, parameter) in parameters {
            let Some(gradient) = parameter.grad(gradients) else {
                continue;
            };
            let total = match self.totals.remove(name) {
                Some(previous) => previous + gradient,
                None => gradient,
            };
            self.totals.insert(name.clone(), total);
        }
    }

    /// Read every gradient back in PARAM-001 order through a single host transfer.
    ///
    /// Concatenating first replaces one device synchronization per parameter with
    /// one for the whole model, which at canonical scale is 111 stalls per
    /// micro-batch turned into one.
    fn read(mut self, dimensions: &GqaDimensions) -> Result<Vec<f32>> {
        let layout = dimensions.parameter_layout();
        let mut flattened = Vec::with_capacity(layout.len());
        let mut expected = 0_usize;
        for (name, shape) in &layout {
            let gradient = self
                .totals
                .remove(name)
                .ok_or_else(|| anyhow!("E1_GRAPH_GRADIENT_MISSING: {name}"))?;
            let elements = shape.iter().product::<usize>();
            ensure!(
                gradient.dims().iter().product::<usize>() == elements,
                "E1_GRAPH_GRADIENT_SHAPE_MISMATCH: {name}"
            );
            expected += elements;
            flattened.push(gradient.reshape([elements]));
        }
        let values = Tensor::cat(flattened, 0)
            .try_into_data()
            .context("E1_GRAPH_GRADIENT_READ_FAILED")?
            .to_vec::<f32>()
            .context("E1_GRAPH_GRADIENT_DTYPE_INVALID")?;
        ensure!(values.len() == expected, "E1_GRAPH_GRADIENT_SHAPE_MISMATCH");
        Ok(values)
    }
}

pub(crate) struct FullModelGraph<B: AutodiffBackend> {
    dimensions: GqaDimensions,
    parameters: BTreeMap<String, Tensor<B, 2>>,
}

impl<B: AutodiffBackend> FullModelGraph<B> {
    /// Load BF16-representable parameter values as FP32 device tensors in the
    /// stable PARAM-001 layout. One-dimensional parameters are stored `[1, n]`.
    pub fn load(
        dimensions: GqaDimensions,
        states: &[AdamwParameterState],
        device: &B::Device,
    ) -> Result<Self> {
        let layout = dimensions.parameter_layout();
        ensure!(
            states.len() == layout.len(),
            "E1_GRAPH_PARAMETER_COUNT_MISMATCH"
        );
        let mut parameters = BTreeMap::new();
        for (state, (name, shape)) in states.iter().zip(&layout) {
            ensure!(&state.name == name, "E1_GRAPH_PARAMETER_ORDER_MISMATCH");
            let elements = shape.iter().product::<usize>();
            ensure!(
                state.parameter_bf16.len() == elements,
                "E1_GRAPH_PARAMETER_SHAPE_MISMATCH: {name}"
            );
            let rows = if shape.len() == 1 { 1 } else { shape[0] };
            let columns = *shape.last().expect("layout shapes are nonempty");
            let values = state
                .parameter_bf16
                .iter()
                .map(|bits| bf16_bits_to_f32(*bits))
                .collect::<Vec<_>>();
            let tensor = Tensor::<B, 2>::from_data(
                TensorData::new(values, [rows, columns]),
                f32_options::<B>(device),
            )
            .require_grad();
            ensure!(
                parameters.insert(name.clone(), tensor).is_none(),
                "E1_GRAPH_PARAMETER_DUPLICATE: {name}"
            );
        }
        Ok(Self {
            dimensions,
            parameters,
        })
    }

    pub fn dimensions(&self) -> &GqaDimensions {
        &self.dimensions
    }

    fn parameter(&self, name: &str) -> Result<&Tensor<B, 2>> {
        self.parameters
            .get(name)
            .ok_or_else(|| anyhow!("E1_GRAPH_PARAMETER_MISSING: {name}"))
    }

    /// A forward-only clone whose parameters carry no gradient requirement, so
    /// running the graph through it builds no autodiff tape at all.
    ///
    /// `detach()` is not sufficient and was the earlier mistake here: burn's
    /// `float_detach` deliberately re-applies the `require_grad` flag it found, so
    /// a detached parameter still roots a tape and every intermediate is still
    /// retained. Clearing the flag is what actually stops one forming. Evaluation
    /// and the checkpoint pass both depend on that.
    ///
    /// Device buffers are shared with the original, so this is a handle clone
    /// rather than a copy of the parameters.
    pub fn untracked(&self) -> Self {
        Self {
            dimensions: self.dimensions,
            parameters: self
                .parameters
                .iter()
                .map(|(name, tensor)| (name.clone(), tensor.clone().set_require_grad(false)))
                .collect(),
        }
    }

    fn rms_norm(&self, input: Tensor<B, 3>, scale_name: &str) -> Result<Tensor<B, 3>> {
        let [_, _, width] = input.dims();
        let scale = self.parameter(scale_name)?.clone().reshape([1, 1, width]);
        // Square by multiplication, never `powf_scalar(2.0)`: the generic power
        // derivative reaches the gradient through exp/log and loses the last
        // FP32 digits even though its forward value is exact.
        let squares = input.clone() * input.clone();
        let inverse = squares
            .mean_dim(2)
            .add_scalar(RMS_NORM_EPSILON)
            .sqrt()
            .recip();
        Ok(bf16_store(input * inverse * scale))
    }

    /// One GEMM over rows that are already flattened.
    fn linear_rows(&self, input: Tensor<B, 2>, weight_name: &str) -> Result<Tensor<B, 2>> {
        let weight = self.parameter(weight_name)?.clone();
        Ok(bf16_store(input.matmul(weight.transpose())))
    }

    /// One GEMM over the flattened batch. Folding `[B, L]` into a single row axis
    /// keeps this a single large matmul instead of one dispatch per sequence.
    /// `bf16_store` is elementwise, so applying it to the flat rows is the same
    /// value as applying it after the reshape.
    fn linear(&self, input: Tensor<B, 3>, weight_name: &str) -> Result<Tensor<B, 3>> {
        let [batch, positions, width] = input.dims();
        let projected = self.linear_rows(input.reshape([batch * positions, width]), weight_name)?;
        let [_, outputs] = projected.dims();
        Ok(projected.reshape([batch, positions, outputs]))
    }

    fn apply_rope(
        &self,
        input: Tensor<B, 3>,
        tables: &(Tensor<B, 2>, Tensor<B, 2>),
    ) -> Result<Tensor<B, 3>> {
        let [batch, positions, width] = input.dims();
        ensure!(
            width.is_multiple_of(self.dimensions.head_width),
            "E1_GRAPH_ROPE_WIDTH_INVALID"
        );
        let half = width / 2;
        // The frozen tables cover the maximum context; broadcast one slice across
        // the batch rather than rebuilding them per sequence.
        let cosine = tables
            .0
            .clone()
            .slice([0..positions, 0..half])
            .reshape([1, positions, half]);
        let sine = tables
            .1
            .clone()
            .slice([0..positions, 0..half])
            .reshape([1, positions, half]);
        let pairs = input.reshape([batch, positions, half, 2]);
        let even = pairs
            .clone()
            .slice([0..batch, 0..positions, 0..half, 0..1])
            .reshape([batch, positions, half]);
        let odd = pairs
            .slice([0..batch, 0..positions, 0..half, 1..2])
            .reshape([batch, positions, half]);
        let rotated_even = bf16_store(even.clone() * cosine.clone() - odd.clone() * sine.clone());
        let rotated_odd = bf16_store(even * sine + odd * cosine);
        Ok(Tensor::stack::<4>(vec![rotated_even, rotated_odd], 3)
            .reshape([batch, positions, width]))
    }

    fn embed(&self, tokens: &BatchTokens, device: &B::Device) -> Result<Tensor<B, 3>> {
        let token_indices = Tensor::<B, 1, Int>::from_data(
            TensorData::new(tokens.input_ids.clone(), [tokens.batch * tokens.positions]),
            device,
        );
        Ok(self
            .parameter("tok_embeddings.weight")?
            .clone()
            .select(0, token_indices)
            .reshape([tokens.batch, tokens.positions, self.dimensions.width]))
    }

    /// The inclusive causal mask for one dispatch, broadcast across the batch and
    /// the heads rather than sliced per sequence.
    fn causal_mask(&self, positions: usize, constants: &GraphConstants<B>) -> Tensor<B, 3> {
        constants
            .causal_mask
            .clone()
            .slice([0..positions, 0..positions])
            .reshape([1, positions, positions])
    }

    /// One pre-norm block: grouped-query attention with a residual, then SwiGLU
    /// with a residual.
    ///
    /// Factored out so activation checkpointing can run it once untracked to
    /// record its output boundary and once with a tape to differentiate it.
    fn layer(
        &self,
        block: usize,
        hidden: Tensor<B, 3>,
        mask: &Tensor<B, 3>,
        constants: &GraphConstants<B>,
    ) -> Result<Tensor<B, 3>> {
        let [batch, positions, width] = hidden.dims();
        let head_width = self.dimensions.head_width;
        let query_heads = self.dimensions.query_heads;
        let key_value_heads = self.dimensions.key_value_heads;
        let queries_per_kv = query_heads / key_value_heads;
        let scale = (head_width as f32).sqrt();

        let normalized =
            self.rms_norm(hidden.clone(), &format!("blocks.{block}.attn_norm.weight"))?;
        let query = self.apply_rope(
            self.linear(normalized.clone(), &format!("blocks.{block}.attn.q.weight"))?,
            &constants.query_rope,
        )?;
        let key = self.apply_rope(
            self.linear(normalized.clone(), &format!("blocks.{block}.attn.k.weight"))?,
            &constants.key_rope,
        )?;
        let value = self.linear(normalized, &format!("blocks.{block}.attn.v.weight"))?;

        // Fold heads into the batch axis so all heads run as one batched matmul,
        // and expand each K/V head across its query group instead of re-slicing it
        // per query head.
        let folded = batch * query_heads;
        let query = query
            .reshape([batch, positions, query_heads, head_width])
            .swap_dims(1, 2)
            .reshape([folded, positions, head_width]);
        let group = |projection: Tensor<B, 3>| {
            projection
                .reshape([batch, positions, key_value_heads, head_width])
                .swap_dims(1, 2)
                .reshape([batch, key_value_heads, 1, positions, head_width])
                .repeat_dim(2, queries_per_kv)
                .reshape([folded, positions, head_width])
        };
        let key = group(key);
        let value = group(value);

        let scores = query.matmul(key.swap_dims(1, 2)).div_scalar(scale) + mask.clone();
        let probabilities = softmax(scores, 2);
        let context = bf16_store(probabilities.matmul(value));
        let attention = self.linear(
            context
                .reshape([batch, query_heads, positions, head_width])
                .swap_dims(1, 2)
                .reshape([batch, positions, width]),
            &format!("blocks.{block}.attn.o.weight"),
        )?;
        let hidden = bf16_store(hidden + attention);

        let normalized =
            self.rms_norm(hidden.clone(), &format!("blocks.{block}.ffn_norm.weight"))?;
        let gate = self.linear(
            normalized.clone(),
            &format!("blocks.{block}.ffn.gate.weight"),
        )?;
        let up = self.linear(normalized, &format!("blocks.{block}.ffn.up.weight"))?;
        let sigmoid = gate.clone().neg().exp().add_scalar(1.0).recip();
        let activated = bf16_store(gate * sigmoid * up);
        let projected = self.linear(activated, &format!("blocks.{block}.ffn.down.weight"))?;
        Ok(bf16_store(hidden + projected))
    }

    /// Embedding through the final RMSNorm, as one continuous graph.
    fn forward_body(
        &self,
        tokens: &BatchTokens,
        constants: &GraphConstants<B>,
        device: &B::Device,
    ) -> Result<Tensor<B, 3>> {
        let mask = self.causal_mask(tokens.positions, constants);
        let mut hidden = self.embed(tokens, device)?;
        for block in 0..self.dimensions.layers {
            hidden = self.layer(block, hidden, &mask, constants)?;
        }
        self.rms_norm(hidden, "final_norm.weight")
    }

    /// Summed cross-entropy over one chunk of flattened positions, divided by
    /// `normalizer`, and optionally the chunk's BF16 logits bits.
    fn head_chunk(
        &self,
        rows: Tensor<B, 2>,
        targets: &[i32],
        normalizer: f32,
        capture_logits: bool,
        device: &B::Device,
    ) -> Result<(Tensor<B, 1>, Vec<u16>)> {
        let logits = self.linear_rows(rows, "lm_head.weight")?;
        let logits_bits = if capture_logits {
            logits
                .clone()
                .try_into_data()
                .context("E1_GRAPH_LOGIT_READ_FAILED")?
                .to_vec::<f32>()
                .context("E1_GRAPH_LOGIT_DTYPE_INVALID")?
                .into_iter()
                .map(f32_to_bf16_bits)
                .collect()
        } else {
            Vec::new()
        };

        let maxima = logits.clone().max_dim(1).detach();
        let log_partition = (logits.clone() - maxima.clone()).exp().sum_dim(1).log() + maxima;
        let target_indices = Tensor::<B, 2, Int>::from_data(
            TensorData::new(targets.to_vec(), [targets.len(), 1]),
            device,
        );
        let target_logits = logits.gather(1, target_indices);
        Ok((
            (log_partition - target_logits).sum().div_scalar(normalizer),
            logits_bits,
        ))
    }

    /// Forward-only summed cross-entropy over a whole dispatch.
    fn head_loss(
        &self,
        hidden: Tensor<B, 3>,
        tokens: &BatchTokens,
        normalizer: f32,
        device: &B::Device,
    ) -> Result<Tensor<B, 1>> {
        let width = self.dimensions.width;
        let rows = tokens.batch * tokens.positions;
        let hidden = hidden.reshape([rows, width]);
        let mut losses = Vec::new();
        for (start, end) in head_chunks(rows) {
            let (loss, _) = self.head_chunk(
                hidden.clone().slice([start..end, 0..width]),
                &tokens.target_ids[start..end],
                normalizer,
                false,
                device,
            )?;
            losses.push(loss);
        }
        Ok(Tensor::cat(losses, 0).sum())
    }

    /// Differentiate the cross-entropy head one chunk at a time.
    ///
    /// Each chunk is backwarded as soon as it is built, so the
    /// `[.., vocabulary]` tape exists for one chunk rather than for the whole
    /// dispatch. Returns the summed loss, any captured logits bits, and the
    /// cotangent on the final normalized hidden state, which is what the body
    /// backward is then resumed from.
    fn chunked_head_backward(
        &self,
        hidden: Tensor<B::InnerBackend, 3>,
        tokens: &BatchTokens,
        normalizer: f32,
        capture_logits: bool,
        accumulator: &mut GradientAccumulator<B>,
        device: &B::Device,
    ) -> Result<(f32, Vec<u16>, Tensor<B::InnerBackend, 3>)> {
        let width = self.dimensions.width;
        let rows = tokens.batch * tokens.positions;
        let hidden = hidden.reshape([rows, width]);
        let mut losses = Vec::new();
        let mut cotangents = Vec::new();
        let mut logits_bits = Vec::new();
        for (start, end) in head_chunks(rows) {
            let chunk =
                Tensor::from_inner(hidden.clone().slice([start..end, 0..width])).require_grad();
            let (loss, bits) = self.head_chunk(
                chunk.clone(),
                &tokens.target_ids[start..end],
                normalizer,
                capture_logits,
                device,
            )?;
            logits_bits.extend(bits);
            losses.push(loss.clone().inner());
            let gradients = loss.backward();
            accumulator.absorb(&self.parameters, &gradients);
            cotangents.push(
                chunk
                    .grad(&gradients)
                    .ok_or_else(|| anyhow!("E1_GRAPH_GRADIENT_MISSING: lm_head input"))?,
            );
        }
        // Chunk losses are summed on the host in dispatch order, which is fixed,
        // so the total is deterministic. It is not bit-identical to one device
        // reduction over every position, but nothing frozen compares it: the P9B
        // fixture is a single chunk and so keeps the oracle's exact order.
        let loss_f32 = Tensor::cat(losses, 0)
            .try_into_data()
            .context("E1_GRAPH_LOSS_READ_FAILED")?
            .to_vec::<f32>()
            .context("E1_GRAPH_LOSS_DTYPE_INVALID")?
            .into_iter()
            .sum();
        Ok((
            loss_f32,
            logits_bits,
            Tensor::cat(cotangents, 0).reshape([tokens.batch, tokens.positions, width]),
        ))
    }

    /// Forward, loss, backward, and ordered FP32 gradient readback for one
    /// dispatch. `normalizer` is `1.0` for trainer sum-mode accumulation and the
    /// valid-target count for the oracle-mean parity fixture.
    ///
    /// The step is staged rather than taken as one tape. Every layer is forwarded
    /// untracked to record the hidden state at its boundary, the head is
    /// differentiated in chunks, and each layer is then recomputed with a tape
    /// only while its own gradient is taken. Retained state falls from every
    /// intermediate of every layer to the layer boundaries plus one live layer,
    /// at the cost of forwarding the body twice. The stages are joined by an
    /// exact vector-Jacobian seed, so the result is the same gradient the single
    /// tape produced.
    pub fn training_step(
        &self,
        sequences: &[(&[u16], &[u16])],
        constants: &GraphConstants<B>,
        normalizer: f32,
        capture_logits: bool,
        device: &B::Device,
    ) -> Result<FullGraphOutput> {
        let tokens = flatten_sequences(sequences, self.dimensions.max_context)?;
        let mask = self.causal_mask(tokens.positions, constants);
        let untracked = self.untracked();

        // Checkpoint pass: no tape, so nothing but the boundaries survives it.
        let embedded = self.embed(&tokens, device)?;
        let mut boundaries = Vec::with_capacity(self.dimensions.layers + 1);
        boundaries.push(embedded.clone().inner());
        for block in 0..self.dimensions.layers {
            let input = Tensor::from_inner(boundaries[block].clone());
            boundaries.push(untracked.layer(block, input, &mask, constants)?.inner());
        }
        drop(untracked);

        // The head is differentiated first: it produces the cotangent every
        // earlier stage is resumed from.
        let body_output = boundaries
            .pop()
            .ok_or_else(|| anyhow!("E1_GRAPH_CHECKPOINT_EMPTY"))?;
        let body_output = Tensor::from_inner(body_output).require_grad();
        let normalized = self.rms_norm(body_output.clone(), "final_norm.weight")?;
        let mut accumulator = GradientAccumulator::<B>::new();
        let (loss_f32, logits_bf16_bits, cotangent) = self.chunked_head_backward(
            normalized.clone().inner(),
            &tokens,
            normalizer,
            capture_logits,
            &mut accumulator,
            device,
        )?;

        let gradients = vector_jacobian_seed(normalized, cotangent).backward();
        accumulator.absorb(&self.parameters, &gradients);
        let mut cotangent = body_output
            .grad(&gradients)
            .ok_or_else(|| anyhow!("E1_GRAPH_GRADIENT_MISSING: final_norm.weight input"))?;

        // Recompute and differentiate one layer at a time, in reverse.
        for block in (0..self.dimensions.layers).rev() {
            let input = boundaries
                .pop()
                .ok_or_else(|| anyhow!("E1_GRAPH_CHECKPOINT_EMPTY"))?;
            let input = Tensor::from_inner(input).require_grad();
            let output = self.layer(block, input.clone(), &mask, constants)?;
            let gradients = vector_jacobian_seed(output, cotangent).backward();
            accumulator.absorb(&self.parameters, &gradients);
            cotangent = input
                .grad(&gradients)
                .ok_or_else(|| anyhow!("E1_GRAPH_GRADIENT_MISSING: blocks.{block} input"))?;
        }

        let gradients = vector_jacobian_seed(embedded, cotangent).backward();
        accumulator.absorb(&self.parameters, &gradients);

        Ok(FullGraphOutput {
            loss_f32,
            gradient_f32: accumulator.read(&self.dimensions)?,
            logits_bf16_bits,
        })
    }

    /// Forward-only summed losses over a held-out set, grouped into batched
    /// dispatches by sequence length and read back once.
    ///
    /// The per-dispatch losses stay on device and are concatenated for a single
    /// host transfer: reading each one individually would force a full device
    /// synchronization per dispatch and dominate evaluation wall-clock. Call this
    /// on an [`untracked`](Self::untracked) graph — evaluation must never build a
    /// tape, which would grow without bound across the held-out set and could not
    /// affect training state anyway.
    pub fn validation_loss_sums(
        &self,
        spans: &[(Vec<u16>, Vec<u16>)],
        batch_sequences: usize,
        constants: &GraphConstants<B>,
        device: &B::Device,
    ) -> Result<Vec<f32>> {
        if spans.is_empty() {
            return Ok(Vec::new());
        }
        ensure!(batch_sequences > 0, "E1_GRAPH_BATCH_EMPTY");
        let mut losses = Vec::new();
        for group in group_by_length(spans) {
            for chunk in group.chunks(batch_sequences) {
                let tokens = flatten_sequences(chunk, self.dimensions.max_context)?;
                let hidden = self.forward_body(&tokens, constants, device)?;
                losses.push(self.head_loss(hidden, &tokens, 1.0, device)?);
            }
        }
        Tensor::cat(losses, 0)
            .try_into_data()
            .context("E1_GRAPH_LOSS_READ_FAILED")?
            .to_vec::<f32>()
            .context("E1_GRAPH_LOSS_DTYPE_INVALID")
    }
}

/// Group spans by exact length, preserving first-seen order.
///
/// Sequences in one dispatch must share a length. The canonical run is uniform
/// except for its final update, which is 18 full spans plus one 1,024-target span
/// — grouping keeps that exact instead of padding it into a uniform shape, which
/// would change valid-target accounting.
pub(crate) fn group_by_length(spans: &[(Vec<u16>, Vec<u16>)]) -> Vec<Vec<(&[u16], &[u16])>> {
    let mut lengths: Vec<usize> = Vec::new();
    let mut groups: Vec<Vec<(&[u16], &[u16])>> = Vec::new();
    for (inputs, targets) in spans {
        let entry = (inputs.as_slice(), targets.as_slice());
        match lengths.iter().position(|length| *length == inputs.len()) {
            Some(index) => groups[index].push(entry),
            None => {
                lengths.push(inputs.len());
                groups.push(vec![entry]);
            }
        }
    }
    groups
}

/// Device-resident graph constants shared by every sequence: the inclusive
/// causal mask and the exact host-FP32 RoPE tables for the query and key
/// projection widths, all built once for the maximum context and sliced per
/// sequence.
pub(crate) struct GraphConstants<B: AutodiffBackend> {
    pub causal_mask: Tensor<B, 2>,
    pub query_rope: (Tensor<B, 2>, Tensor<B, 2>),
    pub key_rope: (Tensor<B, 2>, Tensor<B, 2>),
}

fn rope_tables<B: AutodiffBackend>(
    dimensions: &GqaDimensions,
    projection_width: usize,
    device: &B::Device,
) -> Result<(Tensor<B, 2>, Tensor<B, 2>)> {
    let positions = dimensions.max_context;
    let half = projection_width / 2;
    let pair_half = dimensions.head_width / 2;
    let mut cosines = Vec::with_capacity(positions * half);
    let mut sines = Vec::with_capacity(positions * half);
    for position in 0..positions {
        for pair in 0..half {
            let angle = rope_angle(position, pair % pair_half, dimensions.head_width)
                .map_err(anyhow::Error::new)?;
            cosines.push(angle.cos());
            sines.push(angle.sin());
        }
    }
    Ok((
        Tensor::<B, 2>::from_data(
            TensorData::new(cosines, [positions, half]),
            f32_options::<B>(device),
        )
        .detach(),
        Tensor::<B, 2>::from_data(
            TensorData::new(sines, [positions, half]),
            f32_options::<B>(device),
        )
        .detach(),
    ))
}

pub(crate) fn graph_constants<B: AutodiffBackend>(
    dimensions: &GqaDimensions,
    device: &B::Device,
) -> Result<GraphConstants<B>> {
    let max_context = dimensions.max_context;
    let mut values = vec![0.0_f32; max_context * max_context];
    for query in 0..max_context {
        for key in (query + 1)..max_context {
            values[query * max_context + key] = f32::NEG_INFINITY;
        }
    }
    let causal_mask = Tensor::<B, 2>::from_data(
        TensorData::new(values, [max_context, max_context]),
        f32_options::<B>(device),
    )
    .detach();
    Ok(GraphConstants {
        causal_mask,
        query_rope: rope_tables::<B>(dimensions, dimensions.width, device)?,
        key_rope: rope_tables::<B>(dimensions, dimensions.key_value_width(), device)?,
    })
}

fn check(cancellation: &AcceleratorCancellation, boundary: &'static str) -> Result<()> {
    cancellation
        .require_active(boundary)
        .map_err(anyhow::Error::new)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

const FIXTURE_INPUT_TOKENS: [u16; 2] = [1, 2];
const FIXTURE_TARGET_TOKENS: [u16; 2] = [2, 3];

fn oracle_fixture_states() -> Vec<AdamwParameterState> {
    crate::model::cpu_oracle_fixture_parameters()
        .into_iter()
        .map(|parameter| {
            let master_weights = parameter
                .values_bf16_bits
                .iter()
                .map(|bits| bf16_bits_to_f32(*bits))
                .collect::<Vec<_>>();
            let elements = master_weights.len();
            AdamwParameterState {
                weight_decay: crate::train::full_state::weight_decay_for(&parameter.name),
                name: parameter.name,
                master_weights,
                parameter_bf16: parameter.values_bf16_bits,
                first_moments: vec![0.0; elements],
                second_moments: vec![0.0; elements],
            }
        })
        .collect()
}

fn execute_fixture_once<B: AutodiffBackend>(
    identity: &GraphIdentity,
    device: &B::Device,
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<AcceleratorModelObservation> {
    let prefix = identity.error_prefix;
    check(cancellation, "before-parameter-load")?;
    let outcome = (|| -> Result<AcceleratorModelObservation> {
        let graph = FullModelGraph::<B>::load(
            GqaDimensions::oracle_fixture(),
            &oracle_fixture_states(),
            device,
        )?;
        check(cancellation, "after-parameter-load")?;
        let constants = graph_constants::<B>(graph.dimensions(), device)?;
        // The fixture runs through the batched path at one sequence, so the
        // exact-forward conformance gate keeps covering the code production uses.
        let output = graph.training_step(
            &[(&FIXTURE_INPUT_TOKENS[..], &FIXTURE_TARGET_TOKENS[..])],
            &constants,
            FIXTURE_TARGET_TOKENS.len() as f32,
            true,
            device,
        )?;
        B::sync(device).with_context(|| format!("{prefix}_FORWARD_SYNCHRONIZATION_FAILED"))?;
        check(cancellation, "after-forward")?;
        check(cancellation, "after-fused-loss")?;
        check(cancellation, "after-backward")?;
        let logits_bytes = output
            .logits_bf16_bits
            .iter()
            .flat_map(|bits| bits.to_le_bytes())
            .collect::<Vec<_>>();
        let gradient_bytes = output
            .gradient_f32
            .iter()
            .flat_map(|gradient| gradient.to_le_bytes())
            .collect::<Vec<_>>();
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
            input_token_ids: FIXTURE_INPUT_TOKENS
                .iter()
                .map(|token| *token as usize)
                .collect(),
            target_token_ids: FIXTURE_TARGET_TOKENS
                .iter()
                .map(|token| *token as usize)
                .collect(),
            logits_bf16_le_hex: hex::encode(&logits_bytes),
            loss_f32_le_hex: hex::encode(output.loss_f32.to_le_bytes()),
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

pub(crate) fn run_repeated_parity<B: AutodiffBackend>(
    identity: &GraphIdentity,
    device: &B::Device,
    device_ordinal: usize,
    cancellation: &AcceleratorCancellation,
) -> Result<(AcceleratorModelObservation, AcceleratorModelObservation)> {
    let first = execute_fixture_once::<B>(identity, device, device_ordinal, cancellation)?;
    check(cancellation, "between-repeated-executions")?;
    let second = execute_fixture_once::<B>(identity, device, device_ordinal, cancellation)?;
    Ok((first, second))
}
