use rust_llm_pretrain::error::{ProductError, Result};
use rust_llm_pretrain::train::{
    GenerationSettingsV1, LossChunkV1, PromptCaseV1, PrototypeTrainingDefaultsV1,
    QualityEvaluationBackend, QualityPackV1, UnigramBaselineInputV1, aggregate_metrics,
    build_quality_result, evaluate_quality, unigram_baseline,
};

#[derive(Clone)]
struct TestQualityBackend {
    state_sha256: String,
    loss_per_target: f64,
    mutate: bool,
    nondeterministic: bool,
    generations: u64,
}

impl QualityEvaluationBackend for TestQualityBackend {
    fn state_sha256(&self) -> Result<String> {
        Ok(self.state_sha256.clone())
    }

    fn evaluate_held_out(&mut self, manifest: &str) -> Result<Vec<LossChunkV1>> {
        if manifest != "11".repeat(32) {
            return Err(ProductError::integrity(
                "TEST_MANIFEST_MISMATCH",
                "unexpected test manifest",
            ));
        }
        if self.mutate {
            self.state_sha256 = "ff".repeat(32);
        }
        Ok(vec![
            LossChunkV1 {
                first_target: 0,
                valid_targets: 2,
                negative_log_likelihood_sum: self.loss_per_target * 2.0,
            },
            LossChunkV1 {
                first_target: 2,
                valid_targets: 2,
                negative_log_likelihood_sum: self.loss_per_target * 2.0,
            },
        ])
    }

    fn generate(&mut self, prompt: &[u16], _settings: &GenerationSettingsV1) -> Result<Vec<u16>> {
        self.generations += 1;
        let suffix = if self.nondeterministic {
            self.generations as u16
        } else {
            3
        };
        Ok(vec![prompt[0], suffix])
    }
}

fn quality_pack() -> QualityPackV1 {
    QualityPackV1 {
        schema: "python-slm-quality-pack-v1".to_owned(),
        profile: "prototype-windows-5090-v1".to_owned(),
        held_out_manifest_sha256: "11".repeat(32),
        held_out_targets: 4,
        vocabulary_size: 4,
        initialized_checkpoint_sha256: "22".repeat(32),
        unigram_artifact_sha256: unigram().sha256().unwrap(),
        generation_settings: GenerationSettingsV1::deterministic_default(),
        prompts: vec![
            PromptCaseV1 {
                prompt_id: "arithmetic".to_owned(),
                prompt_token_ids: vec![1, 2],
            },
            PromptCaseV1 {
                prompt_id: "python-function".to_owned(),
                prompt_token_ids: vec![2, 3],
            },
        ],
    }
}

fn unigram() -> UnigramBaselineInputV1 {
    UnigramBaselineInputV1 {
        training_token_counts: vec![7, 1, 1, 1],
        held_out_token_counts: vec![1, 1, 1, 1],
    }
}

fn backend(state: &str, loss: f64) -> TestQualityBackend {
    TestQualityBackend {
        state_sha256: state.repeat(32),
        loss_per_target: loss,
        mutate: false,
        nondeterministic: false,
        generations: 0,
    }
}

#[test]
fn ordered_metrics_and_unigram_baseline_are_exact_and_finite() {
    let chunks = vec![
        LossChunkV1 {
            first_target: 0,
            valid_targets: 2,
            negative_log_likelihood_sum: 2.0,
        },
        LossChunkV1 {
            first_target: 2,
            valid_targets: 2,
            negative_log_likelihood_sum: 2.0,
        },
    ];
    let first = aggregate_metrics(&chunks).unwrap();
    assert_eq!(first, aggregate_metrics(&chunks).unwrap());
    assert_eq!(first.evaluated_targets, 4);
    assert_eq!(first.aggregate_loss, 1.0);
    assert!(first.aggregate_perplexity.is_finite());

    let unigram_first = unigram_baseline(&unigram(), 4).unwrap();
    assert_eq!(unigram_first, unigram_baseline(&unigram(), 4).unwrap());
    assert_eq!(unigram_first.evaluated_targets, 4);
    assert!(unigram_first.aggregate_loss.is_finite());
    assert!(unigram_first.aggregate_perplexity.is_finite());
}

#[test]
fn baselines_final_comparison_and_prompt_replay_are_deterministic() {
    let mut initialized = backend("22", 2.0);
    let mut final_checkpoint = backend("33", 0.5);
    let result = evaluate_quality(
        &quality_pack(),
        &mut initialized,
        &mut final_checkpoint,
        &"33".repeat(32),
        &unigram(),
    )
    .unwrap();
    assert_eq!(result.schema, "python-slm-quality-evaluation-result-v1");
    assert_eq!(result.status, "QUALITY_EVALUATED");
    assert_eq!(result.qualification_status, "SKIPPED");
    assert!(result.final_loss_below_initialized);
    assert!(result.final_loss_below_unigram);
    assert!(result.deterministic_outputs);
    assert!(result.backend_state_unchanged);
    assert_eq!(result.prompt_replays.len(), 2);
    assert!(result.prompt_replays.iter().all(|value| {
        value.deterministic && value.replay_count == 2 && !value.generated_token_ids.is_empty()
    }));
    let json = serde_json::to_string(&result).unwrap();
    assert!(!json.contains(":\\"));
    assert!(!json.contains("receipts"));
    assert!(!json.contains("pointer"));
}

#[test]
fn nonfinite_mutating_and_nondeterministic_inputs_fail_closed() {
    let error = aggregate_metrics(&[LossChunkV1 {
        first_target: 0,
        valid_targets: 1,
        negative_log_likelihood_sum: f64::NAN,
    }])
    .unwrap_err();
    assert_eq!(error.code, "P16A_EVALUATION_CHUNK_INVALID");

    let mut wrong_unigram = unigram();
    wrong_unigram.training_token_counts[0] += 1;
    let mut initialized = backend("22", 2.0);
    let mut final_checkpoint = backend("33", 0.5);
    assert_eq!(
        evaluate_quality(
            &quality_pack(),
            &mut initialized,
            &mut final_checkpoint,
            &"33".repeat(32),
            &wrong_unigram,
        )
        .unwrap_err()
        .code,
        "P16A_UNIGRAM_ARTIFACT_MISMATCH"
    );

    let mut initialized = backend("22", 1.0);
    let mut final_checkpoint = backend("33", 1.0);
    assert_eq!(
        evaluate_quality(
            &quality_pack(),
            &mut initialized,
            &mut final_checkpoint,
            &"33".repeat(32),
            &unigram(),
        )
        .unwrap_err()
        .code,
        "P16A_FINAL_LOSS_NOT_IMPROVED"
    );

    let mut initialized = backend("22", 2.0);
    initialized.mutate = true;
    let mut final_checkpoint = backend("33", 0.5);
    assert_eq!(
        evaluate_quality(
            &quality_pack(),
            &mut initialized,
            &mut final_checkpoint,
            &"33".repeat(32),
            &unigram(),
        )
        .unwrap_err()
        .code,
        "P16A_EVALUATION_MUTATED_STATE"
    );

    let mut initialized = backend("22", 2.0);
    let mut final_checkpoint = backend("33", 0.5);
    final_checkpoint.nondeterministic = true;
    assert_eq!(
        evaluate_quality(
            &quality_pack(),
            &mut initialized,
            &mut final_checkpoint,
            &"33".repeat(32),
            &unigram(),
        )
        .unwrap_err()
        .code,
        "P16A_GENERATION_NONDETERMINISTIC"
    );
}

#[test]
fn readiness_result_is_closed_and_claim_limited() {
    let first = build_quality_result(PrototypeTrainingDefaultsV1::canonical()).unwrap();
    assert_eq!(
        first,
        build_quality_result(PrototypeTrainingDefaultsV1::canonical()).unwrap()
    );
    assert_eq!(
        first.schema,
        "python-slm-quality-evaluation-implementation-result-v1"
    );
    assert_eq!(first.status, "IMPLEMENTATION_READY");
    assert_eq!(first.execution_status, "NOT_RUN");
    assert_eq!(first.qualification_status, "SKIPPED");
    assert_eq!(first.claims.prototype_quality, "UNVERIFIED");
    assert_eq!(first.claims.final_held_out_metrics, "UNVERIFIED");
    assert_eq!(first.claims.quality_pack_frozen_before_p15, "UNVERIFIED");
}

#[cfg(not(windows))]
#[test]
fn product_boundary_defers_before_reading_configuration() {
    let error = rust_llm_pretrain::commands::run([
        "python-slm".into(),
        "evaluate-quality".into(),
        "--config".into(),
        "missing.json".into(),
    ])
    .unwrap_err();
    assert_eq!(error.code, "DEFERRED_POST_P16");
}

#[cfg(windows)]
#[test]
fn product_boundary_emits_readiness_without_evaluating_a_model() {
    let config = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/train/prototype-windows-5090-v1.defaults.json")
        .canonicalize()
        .unwrap();
    let value = rust_llm_pretrain::commands::run([
        "python-slm".into(),
        "evaluate-quality".into(),
        "--config".into(),
        config.into_os_string(),
    ])
    .unwrap();
    assert_eq!(value["status"], "IMPLEMENTATION_READY");
    assert_eq!(value["execution_status"], "NOT_RUN");
    assert_eq!(value["claims"]["prototype_quality"], "UNVERIFIED");
}
