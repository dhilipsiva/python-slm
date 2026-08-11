//! Llama-style decoder implemented with Burn tensor operations.
//!
//! This is the correctness/reference graph. Burn 0.21's autodiff backend expands
//! [`burn::tensor::module::attention`] into the quadratic attention fallback, so
//! production pre-training still requires a fused causal-attention backward op.

use crate::config::LlamaConfig;
use anyhow::Result;
use burn::{
    module::{Initializer, Module},
    nn::{
        Embedding, EmbeddingConfig, Linear, LinearConfig, RmsNorm, RmsNormConfig, RotaryEncoding,
        RotaryEncodingConfig, SwiGlu, SwiGluConfig,
    },
    tensor::{
        Int, Tensor,
        backend::Backend,
        module::{attention, linear},
        ops::AttentionModuleOptions,
    },
};

const INIT_STD: f64 = 0.02;

/// A decoder-only Llama model.
///
/// Inputs have shape `[batch, sequence]`; [`forward`](Self::forward) returns
/// logits with shape `[batch, sequence, vocabulary]`.
#[derive(Module, Debug)]
pub struct LlamaModel<B: Backend> {
    pub token_embeddings: Embedding<B>,
    pub blocks: Vec<DecoderBlock<B>>,
    pub final_norm: RmsNorm<B>,
    /// `None` means the token-embedding matrix is reused for the output projection.
    pub lm_head: Option<Linear<B>>,
    pub rope: RotaryEncoding<B>,
    pub vocab_size: usize,
    pub d_model: usize,
    pub context_length: usize,
}

impl<B: Backend> LlamaModel<B> {
    /// Construct a model after validating all divisibility and RoPE invariants.
    pub fn new(config: &LlamaConfig, device: &B::Device) -> Result<Self> {
        config.validate()?;

        let initializer = llama_initializer();
        let token_embeddings = EmbeddingConfig::new(config.vocab_size, config.d_model)
            .with_initializer(initializer.clone())
            .init(device);
        let blocks = (0..config.n_layers)
            .map(|_| DecoderBlock::new(config, device, &initializer))
            .collect();
        let final_norm = RmsNormConfig::new(config.d_model)
            .with_epsilon(config.rms_epsilon)
            .init(device);
        let lm_head = (!config.tie_embeddings)
            .then(|| linear_no_bias(config.d_model, config.vocab_size, device, &initializer));
        let rope = RotaryEncodingConfig::new(config.context_length, config.head_dim())
            .with_theta(config.rope_theta)
            .init(device);

        Ok(Self {
            token_embeddings,
            blocks,
            final_norm,
            lm_head,
            rope,
            vocab_size: config.vocab_size,
            d_model: config.d_model,
            context_length: config.context_length,
        })
    }

    /// Run the embedding, decoder blocks, and final RMSNorm without allocating logits.
    ///
    /// Keeping this separate lets a trainer project token chunks independently rather
    /// than materializing `[batch * sequence, vocabulary]` for the entire microbatch.
    pub fn forward_hidden(&self, token_ids: Tensor<B, 2, Int>) -> Tensor<B, 3> {
        let [_batch_size, sequence_length] = token_ids.dims();
        assert!(sequence_length > 0, "the input sequence must not be empty");
        assert!(
            sequence_length <= self.context_length,
            "sequence length {sequence_length} exceeds configured context length {}",
            self.context_length
        );

        let mut hidden = self.token_embeddings.forward(token_ids);
        for block in &self.blocks {
            hidden = block.forward(hidden, &self.rope);
        }
        self.final_norm.forward(hidden)
    }

    /// Project hidden states to vocabulary logits.
    ///
    /// This accepts any tensor rank whose final dimension is `d_model`, which is useful
    /// for a chunked linear-cross-entropy implementation.
    pub fn project_logits<const D: usize>(&self, hidden: Tensor<B, D>) -> Tensor<B, D> {
        match &self.lm_head {
            Some(head) => head.forward(hidden),
            None => linear(hidden, self.token_embeddings.weight.val().transpose(), None),
        }
    }

    /// Return logits with shape `[batch, sequence, vocabulary]`.
    pub fn forward(&self, token_ids: Tensor<B, 2, Int>) -> Tensor<B, 3> {
        self.project_logits(self.forward_hidden(token_ids))
    }
}

/// One pre-normalized decoder block.
#[derive(Module, Debug)]
pub struct DecoderBlock<B: Backend> {
    pub attention_norm: RmsNorm<B>,
    pub attention: GroupedQueryAttention<B>,
    pub ffn_norm: RmsNorm<B>,
    pub feed_forward: FeedForward<B>,
}

impl<B: Backend> DecoderBlock<B> {
    fn new(config: &LlamaConfig, device: &B::Device, initializer: &Initializer) -> Self {
        let norm = || {
            RmsNormConfig::new(config.d_model)
                .with_epsilon(config.rms_epsilon)
                .init(device)
        };

        Self {
            attention_norm: norm(),
            attention: GroupedQueryAttention::new(config, device, initializer),
            ffn_norm: norm(),
            feed_forward: FeedForward::new(config, device, initializer),
        }
    }

    fn forward(&self, hidden: Tensor<B, 3>, rope: &RotaryEncoding<B>) -> Tensor<B, 3> {
        // Pre-norm attention with an unscaled residual path.
        let residual = hidden.clone();
        let hidden = residual
            + self
                .attention
                .forward(self.attention_norm.forward(hidden), rope);

        // Pre-norm SwiGLU MLP with the second residual connection.
        let residual = hidden.clone();
        residual + self.feed_forward.forward(self.ffn_norm.forward(hidden))
    }
}

/// Grouped-query self-attention with distinct Q and KV projection widths.
#[derive(Module, Debug)]
pub struct GroupedQueryAttention<B: Backend> {
    pub query: Linear<B>,
    pub key: Linear<B>,
    pub value: Linear<B>,
    pub output: Linear<B>,
    pub n_heads: usize,
    pub n_kv_heads: usize,
    pub head_dim: usize,
    pub d_model: usize,
}

impl<B: Backend> GroupedQueryAttention<B> {
    fn new(config: &LlamaConfig, device: &B::Device, initializer: &Initializer) -> Self {
        let kv_width = config.kv_width();
        Self {
            query: linear_no_bias(config.d_model, config.d_model, device, initializer),
            key: linear_no_bias(config.d_model, kv_width, device, initializer),
            value: linear_no_bias(config.d_model, kv_width, device, initializer),
            output: linear_no_bias(config.d_model, config.d_model, device, initializer),
            n_heads: config.n_heads,
            n_kv_heads: config.n_kv_heads,
            head_dim: config.head_dim(),
            d_model: config.d_model,
        }
    }

    fn forward(&self, hidden: Tensor<B, 3>, rope: &RotaryEncoding<B>) -> Tensor<B, 3> {
        let [batch_size, sequence_length, _] = hidden.dims();

        let query = self
            .query
            .forward(hidden.clone())
            .reshape([batch_size, sequence_length, self.n_heads, self.head_dim])
            .swap_dims(1, 2);
        let key = self
            .key
            .forward(hidden.clone())
            .reshape([batch_size, sequence_length, self.n_kv_heads, self.head_dim])
            .swap_dims(1, 2);
        let value = self
            .value
            .forward(hidden)
            .reshape([batch_size, sequence_length, self.n_kv_heads, self.head_dim])
            .swap_dims(1, 2);

        // RoPE is applied while K still has only n_kv_heads. Repeating first would
        // waste rotation work and saved activations without changing the result.
        let query = rope.forward(query);
        let key = rope.forward(key);

        // Each KV head is shared by a contiguous group of Q heads. Value does not use
        // RoPE, but it is deliberately expanded at the same late boundary as key.
        let repeats = self.n_heads / self.n_kv_heads;
        let key = repeat_kv(key, repeats);
        let value = repeat_kv(value, repeats);

        let context = attention(
            query,
            key,
            value,
            None,
            None,
            AttentionModuleOptions {
                is_causal: true,
                ..Default::default()
            },
        );

        self.output.forward(context.swap_dims(1, 2).reshape([
            batch_size,
            sequence_length,
            self.d_model,
        ]))
    }
}

/// Bias-free `down(SiLU(gate(x)) * up(x))` feed-forward network.
#[derive(Module, Debug)]
pub struct FeedForward<B: Backend> {
    pub gate_up: SwiGlu<B>,
    pub down: Linear<B>,
}

impl<B: Backend> FeedForward<B> {
    fn new(config: &LlamaConfig, device: &B::Device, initializer: &Initializer) -> Self {
        Self {
            gate_up: SwiGluConfig::new(config.d_model, config.d_ff)
                .with_bias(false)
                .with_initializer(initializer.clone())
                .init(device),
            down: linear_no_bias(config.d_ff, config.d_model, device, initializer),
        }
    }

    fn forward(&self, hidden: Tensor<B, 3>) -> Tensor<B, 3> {
        self.down.forward(self.gate_up.forward(hidden))
    }
}

/// Expand `[batch, n_kv_heads, sequence, head_dim]` to query-head count.
///
/// The singleton group dimension is important: calling `repeat_dim(1, repeats)`
/// would produce `[kv0, kv1, ..., kv0, kv1, ...]`, not the required contiguous
/// `[kv0, kv0, ..., kv1, kv1, ...]` grouping.
fn repeat_kv<B: Backend>(tensor: Tensor<B, 4>, repeats: usize) -> Tensor<B, 4> {
    if repeats == 1 {
        return tensor;
    }
    let [batch_size, n_kv_heads, sequence_length, head_dim] = tensor.dims();
    tensor
        .unsqueeze_dim::<5>(2)
        .repeat_dim(2, repeats)
        .reshape([batch_size, n_kv_heads * repeats, sequence_length, head_dim])
}

fn llama_initializer() -> Initializer {
    Initializer::Normal {
        mean: 0.0,
        std: INIT_STD,
    }
}

fn linear_no_bias<B: Backend>(
    d_input: usize,
    d_output: usize,
    device: &B::Device,
    initializer: &Initializer,
) -> Linear<B> {
    LinearConfig::new(d_input, d_output)
        .with_bias(false)
        .with_initializer(initializer.clone())
        .init(device)
}

#[cfg(all(test, feature = "cpu-reference"))]
mod tests {
    use super::*;
    use burn::{
        backend::{Autodiff, Flex},
        tensor::{TensorData, Tolerance, ops::FloatElem},
    };

    fn tiny_config(tie_embeddings: bool) -> LlamaConfig {
        LlamaConfig {
            vocab_size: 32,
            d_model: 16,
            d_ff: 32,
            n_layers: 2,
            n_heads: 4,
            n_kv_heads: 2,
            context_length: 8,
            rms_epsilon: 1e-5,
            rope_theta: 10_000.0,
            tie_embeddings,
        }
    }

    #[test]
    fn forward_shape_supports_tied_and_untied_heads() {
        let device = Default::default();
        let ids = [[0_i32, 1, 2, 3, 4], [5, 6, 7, 8, 9]];

        for tied in [false, true] {
            let config = tiny_config(tied);
            let model = LlamaModel::<Flex>::new(&config, &device).unwrap();
            assert_eq!(model.num_params() as u64, config.parameter_count());
            let logits = model.forward(Tensor::from_data(ids, &device));
            assert_eq!(logits.dims(), [2, 5, 32]);
            assert_eq!(model.lm_head.is_none(), tied);
        }
    }

    #[test]
    fn causal_mask_prevents_future_tokens_from_changing_prefix_logits() {
        type FT = FloatElem<Flex>;

        let device = Default::default();
        Flex::seed(&device, 7);
        let model = LlamaModel::<Flex>::new(&tiny_config(false), &device).unwrap();
        let a = model.forward(Tensor::from_data([[1_i32, 2, 3, 4, 5]], &device));
        let b = model.forward(Tensor::from_data([[1_i32, 2, 3, 20, 21]], &device));

        let a_prefix = a.slice([0..1, 0..3, 0..32]).to_data();
        let b_prefix = b.slice([0..1, 0..3, 0..32]).to_data();
        a_prefix.assert_approx_eq::<FT>(&b_prefix, Tolerance::default());
    }

    #[test]
    fn reference_graph_backpropagates_through_attention_and_tied_head() {
        type TestBackend = Autodiff<Flex>;

        let device = Default::default();
        TestBackend::seed(&device, 11);
        let mut config = tiny_config(true);
        config.n_layers = 1;
        let model = LlamaModel::<TestBackend>::new(&config, &device).unwrap();
        let logits = model.forward(Tensor::from_data([[1_i32, 2, 3, 4]], &device));
        let gradients = logits.sum().backward();

        let embedding_grad = model
            .token_embeddings
            .weight
            .grad(&gradients)
            .expect("embedding must receive lookup and tied-output gradients");
        assert_eq!(embedding_grad.dims(), [32, 16]);
    }

    #[test]
    fn kv_expansion_repeats_each_head_contiguously() {
        let device = Default::default();
        let kv = Tensor::<Flex, 4>::from_data(
            TensorData::new(vec![1.0_f32, 2.0], [1, 2, 1, 1]),
            &device,
        );
        let expanded = repeat_kv(kv, 3).to_data();
        let expected = TensorData::new(vec![1.0_f32, 1.0, 1.0, 2.0, 2.0, 2.0], [1, 6, 1, 1]);
        expanded.assert_eq(&expected, false);
    }
}
