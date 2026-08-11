use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LlamaConfig {
    pub vocab_size: usize,
    pub d_model: usize,
    pub d_ff: usize,
    pub n_layers: usize,
    pub n_heads: usize,
    pub n_kv_heads: usize,
    pub context_length: usize,
    pub rms_epsilon: f64,
    pub rope_theta: f32,
    pub tie_embeddings: bool,
}

impl Default for LlamaConfig {
    fn default() -> Self {
        Self {
            vocab_size: 32_000,
            d_model: 768,
            d_ff: 2_048,
            n_layers: 12,
            n_heads: 12,
            n_kv_heads: 4,
            context_length: 2_048,
            rms_epsilon: 1e-5,
            rope_theta: 10_000.0,
            // An untied head is required even to reach 124.7M. Tying produces 100.1M.
            tie_embeddings: false,
        }
    }
}

impl LlamaConfig {
    /// Keeps GQA and changes only the FFN width to produce 135,285,504 parameters.
    pub fn gqa_135m() -> Self {
        Self {
            d_ff: 2_432,
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.vocab_size > 0, "vocab_size must be positive");
        ensure!(
            self.vocab_size <= (u16::MAX as usize) + 1,
            "u16 storage supports at most 65,536 token IDs"
        );
        ensure!(
            self.d_model > 0 && self.d_ff > 0,
            "model widths must be positive"
        );
        ensure!(self.n_layers > 0, "n_layers must be positive");
        ensure!(
            self.n_heads > 0 && self.n_kv_heads > 0,
            "head counts must be positive"
        );
        ensure!(
            self.d_model.is_multiple_of(self.n_heads),
            "d_model must be divisible by n_heads"
        );
        ensure!(
            self.n_heads.is_multiple_of(self.n_kv_heads),
            "n_heads must be divisible by n_kv_heads"
        );
        ensure!(
            self.head_dim().is_multiple_of(2),
            "RoPE requires an even head dimension"
        );
        ensure!(
            self.context_length >= 2,
            "context_length must be at least 2"
        );
        ensure!(self.rms_epsilon > 0.0, "rms_epsilon must be positive");
        ensure!(self.rope_theta > 0.0, "rope_theta must be positive");
        Ok(())
    }

    pub fn head_dim(&self) -> usize {
        self.d_model / self.n_heads
    }

    pub fn kv_width(&self) -> usize {
        self.n_kv_heads * self.head_dim()
    }

    pub fn block_parameter_count(&self) -> u64 {
        let d = self.d_model as u64;
        let ff = self.d_ff as u64;
        let kv = self.kv_width() as u64;
        let attention = d * d + 2 * d * kv + d * d;
        let swiglu = 3 * d * ff;
        let norms = 2 * d;
        attention + swiglu + norms
    }

    pub fn parameter_count(&self) -> u64 {
        let d = self.d_model as u64;
        let vocab = self.vocab_size as u64;
        let embeddings = vocab * d;
        let output = if self.tie_embeddings { 0 } else { d * vocab };
        embeddings + self.n_layers as u64 * self.block_parameter_count() + d + output
    }

    /// Matmul-dominant training FLOPs/token. The range is triangular-causal to
    /// full-square attention accounting; elementwise and optimizer work is excluded.
    pub fn training_flops_per_token(&self) -> (u64, u64) {
        let d = self.d_model as u64;
        let ff = self.d_ff as u64;
        let layers = self.n_layers as u64;
        let length = self.context_length as u64;
        let kv = self.kv_width() as u64;

        // Q, K, V, O plus three SwiGLU matrices; 6 = forward + two backward GEMMs.
        let block_weights = 2 * d * d + 2 * d * kv + 3 * d * ff;
        let block_linear = 6 * layers * block_weights;
        let lm_head = 6 * d * self.vocab_size as u64;
        // QK^T and PV, including their backward matmuls.
        let triangular_attention = 6 * layers * (length + 1) * d;
        let full_attention = 12 * layers * length * d;
        (
            block_linear + lm_head + triangular_attention,
            block_linear + lm_head + full_attention,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TrainConfig {
    pub target_tokens: u64,
    pub micro_batch_size: usize,
    pub gradient_accumulation: usize,
    pub peak_learning_rate: f64,
    pub minimum_learning_rate: f64,
    pub warmup_steps: u64,
    pub beta_1: f32,
    pub beta_2: f32,
    pub adam_epsilon: f32,
    pub weight_decay: f32,
    pub log_every_micro_steps: u64,
    pub target_tokens_per_second: f64,
    pub vram_budget_gib: f64,
    pub allow_reference_attention: bool,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            target_tokens: 2_000_000_000,
            // Same 65,536 tokens/update as microbatch 32, accumulation 1, but a safer start.
            micro_batch_size: 16,
            gradient_accumulation: 2,
            peak_learning_rate: 2.5e-3,
            minimum_learning_rate: 0.0,
            warmup_steps: 1_000,
            beta_1: 0.9,
            beta_2: 0.95,
            adam_epsilon: 1e-8,
            weight_decay: 0.1,
            log_every_micro_steps: 10,
            target_tokens_per_second: 75_000.0,
            vram_budget_gib: 28.0,
            allow_reference_attention: false,
        }
    }
}

impl TrainConfig {
    pub fn validate(&self, model: &LlamaConfig) -> Result<()> {
        model.validate()?;
        ensure!(self.target_tokens > 0, "target_tokens must be positive");
        ensure!(
            self.micro_batch_size > 0,
            "micro_batch_size must be positive"
        );
        ensure!(
            self.gradient_accumulation > 0,
            "gradient_accumulation must be positive"
        );
        ensure!(
            self.peak_learning_rate > 0.0,
            "peak learning rate must be positive"
        );
        ensure!(
            (0.0..=self.peak_learning_rate).contains(&self.minimum_learning_rate),
            "minimum learning rate must be in [0, peak]"
        );
        ensure!(
            self.beta_1 > 0.0 && self.beta_1 < 1.0,
            "beta_1 must be in (0, 1)"
        );
        ensure!(
            self.beta_2 > 0.0 && self.beta_2 < 1.0,
            "beta_2 must be in (0, 1)"
        );
        ensure!(self.adam_epsilon > 0.0, "Adam epsilon must be positive");
        ensure!(self.weight_decay >= 0.0, "weight decay cannot be negative");
        ensure!(
            self.total_optimizer_steps(model) > self.warmup_steps,
            "warmup consumes the run"
        );
        ensure!(
            self.target_tokens_per_second > 0.0,
            "throughput target must be positive"
        );
        ensure!(self.vram_budget_gib > 0.0, "VRAM budget must be positive");
        Ok(())
    }

    pub fn tokens_per_micro_step(&self, model: &LlamaConfig) -> u64 {
        self.micro_batch_size as u64 * model.context_length as u64
    }

    pub fn tokens_per_optimizer_step(&self, model: &LlamaConfig) -> u64 {
        self.tokens_per_micro_step(model) * self.gradient_accumulation as u64
    }

    pub fn total_optimizer_steps(&self, model: &LlamaConfig) -> u64 {
        self.target_tokens
            .div_ceil(self.tokens_per_optimizer_step(model))
    }

    pub fn learning_rate(&self, optimizer_step: u64, model: &LlamaConfig) -> f64 {
        let total = self.total_optimizer_steps(model);
        if optimizer_step < self.warmup_steps {
            return self.peak_learning_rate * (optimizer_step + 1) as f64
                / self.warmup_steps.max(1) as f64;
        }
        let decay_steps = (total - self.warmup_steps).max(1);
        let progress =
            (optimizer_step - self.warmup_steps).min(decay_steps) as f64 / decay_steps as f64;
        let cosine = 0.5 * (1.0 + (PI * progress).cos());
        self.minimum_learning_rate + (self.peak_learning_rate - self.minimum_learning_rate) * cosine
    }

    pub fn enforce_optimized_kernel_gate(&self) -> Result<()> {
        if !self.allow_reference_attention {
            bail!(
                "optimized pre-training is unavailable: Burn 0.21 autodiff lowers attention to the O(L^2) fallback. Pass --allow-reference-attention only for correctness/smoke tests; it cannot meet the 8-hour target"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryEstimate {
    pub parameter_count: u64,
    pub persistent_state_gib: f64,
    pub hidden_mib: f64,
    pub q_mib: f64,
    pub each_kv_mib: f64,
    pub each_ffn_mib: f64,
    pub attention_matrix_per_layer_gib: f64,
    pub logits_gib: f64,
    pub reference_attention_matrices_gib: f64,
}

impl MemoryEstimate {
    pub fn bf16(model: &LlamaConfig, train: &TrainConfig) -> Self {
        let b = train.micro_batch_size as u64;
        let l = model.context_length as u64;
        let d = model.d_model as u64;
        let h = model.n_heads as u64;
        let kv = model.kv_width() as u64;
        let ff = model.d_ff as u64;
        let bytes_bf16 = 2_u64;
        let mib = 1024.0 * 1024.0;
        let gib = mib * 1024.0;
        let attention = b * h * l * l * bytes_bf16;

        Self {
            parameter_count: model.parameter_count(),
            // Conservative production envelope: BF16 weights/grads plus FP32
            // master weights and two moments. Burn's reference AdamW does not
            // itself provide this validated mixed-precision state layout.
            persistent_state_gib: model.parameter_count() as f64 * 16.0 / gib,
            hidden_mib: (b * l * d * bytes_bf16) as f64 / mib,
            q_mib: (b * l * d * bytes_bf16) as f64 / mib,
            each_kv_mib: (b * l * kv * bytes_bf16) as f64 / mib,
            each_ffn_mib: (b * l * ff * bytes_bf16) as f64 / mib,
            attention_matrix_per_layer_gib: attention as f64 / gib,
            logits_gib: (b * l * model.vocab_size as u64 * bytes_bf16) as f64 / gib,
            // Scores and probabilities are both retained by a conventional backward graph.
            reference_attention_matrices_gib: (2 * model.n_layers as u64 * attention) as f64 / gib,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_shape_has_exact_count() {
        let config = LlamaConfig::default();
        assert_eq!(config.block_parameter_count(), 6_292_992);
        assert_eq!(config.parameter_count(), 124_668_672);
    }

    #[test]
    fn tied_and_corrected_counts_are_explicit() {
        let tied = LlamaConfig {
            tie_embeddings: true,
            ..LlamaConfig::default()
        };
        assert_eq!(tied.parameter_count(), 100_092_672);
        assert_eq!(LlamaConfig::gqa_135m().parameter_count(), 135_285_504);
        let ordinary_mha = LlamaConfig {
            n_kv_heads: 12,
            ..LlamaConfig::default()
        };
        assert_eq!(ordinary_mha.parameter_count(), 134_105_856);
    }

    #[test]
    fn schedule_hits_peak_then_decays() {
        let model = LlamaConfig::default();
        let train = TrainConfig::default();
        assert_eq!(train.tokens_per_optimizer_step(&model), 65_536);
        assert!(train.learning_rate(0, &model) < train.peak_learning_rate);
        assert_eq!(
            train.learning_rate(train.warmup_steps - 1, &model),
            train.peak_learning_rate
        );
        assert!(
            train.learning_rate(train.total_optimizer_steps(&model) - 1, &model)
                < train.peak_learning_rate
        );
    }
}
