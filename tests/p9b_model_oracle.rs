use rust_llm_pretrain::model::{
    CANONICAL_MODEL_ID, CANONICAL_PARAMETER_COUNT, ModelPreset, OptimizerGroup, bf16_bits_to_f32,
    canonical_parameter_specs, causal_attention_allowed, cpu_oracle_fixture, f32_to_bf16_bits,
    model_config, optimizer_rules, parameter_layout_sha256, query_to_key_value_head, rope_angle,
};
use std::collections::BTreeSet;

#[test]
fn canonical_configuration_layout_and_optimizer_groups_are_closed() {
    let config = model_config(ModelPreset::Gqa135mV1);
    config.validate().unwrap();
    assert_eq!(config.identity, CANONICAL_MODEL_ID);
    assert_eq!(config.parameter_count, CANONICAL_PARAMETER_COUNT);
    assert_eq!(config.ffn_width, 2_432);
    assert_eq!(config.query_heads, 12);
    assert_eq!(config.key_value_heads, 4);
    assert_eq!(config.head_width, 64);
    assert!(!config.biases);
    assert_eq!(config.dropout_f32_bits, "00000000");

    let specs = canonical_parameter_specs().unwrap();
    assert_eq!(specs.len(), 111);
    assert_eq!(
        specs.iter().map(|spec| spec.elements).sum::<u64>(),
        CANONICAL_PARAMETER_COUNT
    );
    assert_eq!(
        parameter_layout_sha256(&specs),
        "3a0116a9e046f58bf5a93170328e3b6f6fa7d930391a4cefdbf545e74b827007"
    );
    let names = specs
        .iter()
        .map(|spec| spec.name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), specs.len());
    assert!(specs.iter().all(|spec| {
        (spec.optimizer_group == OptimizerGroup::NoDecay) == spec.name.ends_with("norm.weight")
    }));

    let rules = optimizer_rules();
    assert_eq!(rules.algorithm, "adamw-opt-001");
    assert_eq!(rules.beta1.to_bits(), 0.9_f32.to_bits());
    assert_eq!(rules.beta2.to_bits(), 0.95_f32.to_bits());
    assert_eq!(rules.epsilon.to_bits(), 1.0e-8_f32.to_bits());
    assert_eq!(rules.decay_weight.to_bits(), 0.1_f32.to_bits());
    assert_eq!(rules.no_decay_weight.to_bits(), 0.0_f32.to_bits());
    assert_eq!(rules.global_l2_clip.to_bits(), 1.0_f32.to_bits());
}

#[test]
fn scalar_forward_loss_and_every_gradient_byte_are_literal() {
    let oracle = cpu_oracle_fixture();
    assert_eq!(oracle.schema, "python-slm-cpu-oracle-result-v1");
    assert_eq!(oracle.fixture_id, "gqa-scalar-oracle-v1");
    assert_eq!(
        oracle.logits_bf16_le_hex,
        "433ed13e11bee4be6fbe63be0e3f9a3c"
    );
    assert_eq!(oracle.loss_f32_le_hex, "f52fc23f");
    assert_eq!(
        oracle.gradient_sha256,
        "60a399908fc3125c6ed07193e14138fb979aed50f0317435ebcf0ea53e0e05ed"
    );
    assert_eq!(oracle.gradient_f32_le_hex.len(), oracle.parameter_count * 8);
    assert_eq!(oracle.gradient_artifacts.len(), 12);
    assert!(oracle.finite);
    assert_eq!(oracle.loss_normalized_by_valid_targets, 2);

    let mut offset = 0;
    for artifact in &oracle.gradient_artifacts {
        assert_eq!(artifact.byte_offset, offset);
        assert_eq!(artifact.byte_length, artifact.elements * 4);
        assert_eq!(artifact.f32_le_sha256.len(), 64);
        offset += artifact.byte_length;
    }
    assert_eq!(offset * 2, oracle.gradient_f32_le_hex.len());

    let value = serde_json::to_value(&oracle).unwrap();
    let keys = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        keys,
        [
            "accumulation",
            "activation_storage",
            "bf16_cast_gradient",
            "bf16_rounding",
            "causal_mask",
            "ffn_width",
            "finite",
            "fixture_id",
            "gqa_mapping",
            "gradient_artifacts",
            "gradient_f32_le_hex",
            "gradient_sha256",
            "gradient_storage",
            "head_width",
            "input_token_ids",
            "key_value_heads",
            "layers",
            "logits_bf16_le_hex",
            "logits_sha256",
            "loss_f32_le_hex",
            "loss_normalized_by_valid_targets",
            "model_semantics",
            "parameter_count",
            "parameter_storage",
            "query_heads",
            "rms_norm_epsilon_f32_le_hex",
            "rope",
            "schema",
            "sequence_length",
            "target_token_ids",
            "vocabulary_size",
            "width",
        ]
        .into_iter()
        .collect()
    );
    assert!(serde_json::to_string(&value).unwrap().is_ascii());
}

#[test]
fn bf16_rope_gqa_and_mask_boundaries_are_exact() {
    assert_eq!(f32_to_bf16_bits(1.0), 0x3f80);
    assert_eq!(bf16_bits_to_f32(0x3f80), 1.0);
    assert_eq!(
        (0..12).map(query_to_key_value_head).collect::<Vec<_>>(),
        [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3]
    );
    assert_eq!(rope_angle(0, 31, 64).unwrap(), 0.0);
    assert_eq!(
        rope_angle(2_048, 0, 64).unwrap().to_bits(),
        2_048_f32.to_bits()
    );
    assert_eq!(
        rope_angle(0, 32, 64).unwrap_err().code,
        "MODEL_ROPE_PAIR_INVALID"
    );
    assert!(causal_attention_allowed(7, 7, false));
    assert!(!causal_attention_allowed(7, 8, false));
    assert!(!causal_attention_allowed(7, 0, true));
}
