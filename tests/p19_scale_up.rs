use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::scale_up::{SCALE_UP_CONFIG_SCHEMA, SCALE_UP_RESULT_SCHEMA, plan_scale_up};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

fn canonical_scope() -> Value {
    json!({
        "vocabulary": 32_000,
        "width": 768,
        "ffn_width": 2_432,
        "layers": 12,
        "query_heads": 12,
        "kv_heads": 4,
        "head_width": 64,
        "context_length": 2_048,
        "untied_lm_head": true,
    })
}

fn config_value(model: Value, targets: u64, sla: u64) -> Value {
    json!({
        "schema": SCALE_UP_CONFIG_SCHEMA,
        "profile": PROTOTYPE_PROFILE,
        "requested_model": model,
        "requested_total_valid_targets": targets,
        "requested_completion_sla_seconds": sla,
    })
}

fn write_config(directory: &Path, value: &Value) -> std::path::PathBuf {
    let path = directory.join("scale-up.json");
    std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    path
}

fn plan(value: &Value) -> rust_llm_pretrain::error::Result<Value> {
    let directory = tempfile::tempdir().unwrap();
    let result = plan_scale_up(&write_config(directory.path(), value));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    result
}

#[test]
fn missing_configuration_is_a_typed_environment_failure() {
    let directory = tempfile::tempdir().unwrap();
    let error = plan_scale_up(&directory.path().join("missing.json")).unwrap_err();
    assert_eq!(error.code, "SCALE_UP_CONFIG_READ_FAILED");
    assert_eq!(error.exit_code(), 4);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn the_configuration_is_closed_versioned_and_profile_bound() {
    let mut open = config_value(canonical_scope(), 2_000_000_000, 36_000);
    open["surprise"] = json!(true);
    assert_eq!(plan(&open).unwrap_err().code, "SCALE_UP_CONFIG_INVALID");

    let mut wrong_schema = config_value(canonical_scope(), 2_000_000_000, 36_000);
    wrong_schema["schema"] = json!("python-slm-scale-up-config-v2");
    assert_eq!(
        plan(&wrong_schema).unwrap_err().code,
        "SCALE_UP_CONFIG_SCHEMA_UNSUPPORTED"
    );

    let mut deferred = config_value(canonical_scope(), 2_000_000_000, 36_000);
    deferred["profile"] = json!("prototype-linux-9070xt-v1");
    assert_eq!(plan(&deferred).unwrap_err().code, "DEFERRED_POST_P16");

    let mut incomplete = config_value(canonical_scope(), 2_000_000_000, 36_000);
    incomplete["requested_model"]
        .as_object_mut()
        .unwrap()
        .remove("layers");
    assert_eq!(
        plan(&incomplete).unwrap_err().code,
        "SCALE_UP_CONFIG_INVALID"
    );
}

#[test]
fn a_sla_only_amendment_reproduces_every_canonical_derived_identity() {
    let value = plan(&config_value(canonical_scope(), 2_000_000_000, 36_000)).unwrap();
    assert_eq!(value["schema"], SCALE_UP_RESULT_SCHEMA);
    assert_eq!(value["status"], "CANDIDATE_PLANNED");
    assert_eq!(value["qualification_status"], "SKIPPED");
    assert_eq!(value["amendment_status"], "UNAPPROVED_CANDIDATE");
    assert_eq!(value["baseline"]["model_identity"], "gqa-135m-v1");
    assert_eq!(value["baseline"]["parameters"], 135_285_504);
    assert_eq!(value["model"]["parameters"], 135_285_504);
    assert_eq!(value["model"]["embedding_parameters"], 24_576_000);
    assert_eq!(value["model"]["lm_head_parameters"], 24_576_000);
    assert_eq!(value["model"]["per_layer_attention_parameters"], 1_572_864);
    assert_eq!(value["model"]["per_layer_ffn_parameters"], 5_603_328);
    assert_eq!(value["model"]["per_layer_norm_parameters"], 1_536);
    assert_eq!(value["model"]["decoder_parameters"], 86_132_736);
    assert_eq!(value["model"]["final_norm_parameters"], 768);
    assert_eq!(
        value["memory"]["raw_training_state_floor_bytes"],
        2_705_710_080_u64
    );
    assert_eq!(
        value["memory"]["minimum_accelerator_bytes"],
        2_952_790_016_u64
    );
    assert_eq!(value["memory"]["alignment_quantum_bytes"], 268_435_456);
    assert_eq!(value["tokens"]["stored_prefix_ids"], 2_000_000_001_u64);
    assert_eq!(value["tokens"]["valid_training_targets"], 2_000_000_000_u64);
    assert_eq!(value["tokens"]["targets_per_full_update"], 65_536);
    assert_eq!(value["tokens"]["full_updates"], 30_517);
    assert_eq!(value["tokens"]["final_update_targets"], 37_888);
    assert_eq!(value["tokens"]["total_updates"], 30_518);
    assert_eq!(value["tokens"]["complete_spans"], 976_562);
    assert_eq!(value["tokens"]["final_span_targets"], 1_024);
    assert_eq!(value["schedule"]["optimizer"], "adamw-opt-001");
    assert_eq!(value["schedule"]["warmup_updates"], 1_000);
    assert_eq!(value["evaluation"]["event_interval_targets"], 100_000_000);
    assert_eq!(value["evaluation"]["evaluation_event_count"], 21);
    assert_eq!(
        value["evaluation"]["retention_anchor_targets"],
        json!([500_000_000, 1_000_000_000, 1_500_000_000, 2_000_000_000])
    );
    assert_eq!(value["sla"]["completion_sla_seconds"], 36_000);
    assert_eq!(value["sla"]["admission_seconds"], 32_400);
    assert_eq!(
        value["sla"]["actual_elapsed_limit_ns"],
        36_000_000_000_000_u64
    );
    assert_eq!(value["sla"]["admission_fraction"], "9/10");
    assert_eq!(value["increased_axes"], json!(["completion_sla_seconds"]));
    assert_eq!(
        value["required_rerun_phases"],
        json!(["P14", "P15", "P16", "P16A"])
    );
    assert_eq!(value["receipts_written"], false);
    let limitations = value["limitations"].as_array().unwrap();
    assert!(limitations.contains(&json!("no_owner_approval_claim")));
    assert!(limitations.contains(&json!("no_contract_amendment_claim")));
    assert!(limitations.contains(&json!("no_canonical_constant_change")));
    assert!(
        value["candidate_identity"]
            .as_str()
            .unwrap()
            .starts_with("scale-up-candidate-")
    );
    assert_eq!(value["scope_sha256"].as_str().unwrap().len(), 64);

    let keys = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        keys,
        [
            "amendment_status",
            "baseline",
            "candidate_identity",
            "evaluation",
            "increased_axes",
            "limitations",
            "memory",
            "model",
            "profile",
            "qualification_status",
            "receipts_written",
            "requested_model",
            "required_rerun_phases",
            "schema",
            "schedule",
            "scope_sha256",
            "sla",
            "status",
            "tokens",
        ]
        .into_iter()
        .collect()
    );
    assert!(serde_json::to_string(&value).unwrap().is_ascii());
}

#[test]
fn a_larger_model_recomputes_exact_parameters_and_memory() {
    let model = json!({
        "vocabulary": 32_000,
        "width": 1_024,
        "ffn_width": 2_816,
        "layers": 16,
        "query_heads": 16,
        "kv_heads": 4,
        "head_width": 64,
        "context_length": 2_048,
        "untied_lm_head": true,
    });
    let value = plan(&config_value(model, 2_000_000_000, 28_800)).unwrap();
    assert_eq!(value["model"]["parameters"], 245_924_864);
    assert_eq!(value["model"]["embedding_parameters"], 32_768_000);
    assert_eq!(value["model"]["per_layer_attention_parameters"], 2_621_440);
    assert_eq!(value["model"]["per_layer_ffn_parameters"], 8_650_752);
    assert_eq!(
        value["memory"]["raw_training_state_floor_bytes"],
        4_918_497_280_u64
    );
    assert_eq!(
        value["memory"]["minimum_accelerator_bytes"],
        5_100_273_664_u64
    );
    assert_eq!(value["increased_axes"], json!(["model_parameters"]));
    assert_eq!(
        value["required_rerun_phases"],
        json!([
            "P9B", "P10", "P11", "P12", "P13", "P14", "P15", "P16", "P16A"
        ])
    );
}

#[test]
fn target_growth_keeps_zero_overshoot_and_reruns_from_materialization() {
    let value = plan(&config_value(canonical_scope(), 3_000_000_000, 28_800)).unwrap();
    assert_eq!(value["tokens"]["stored_prefix_ids"], 3_000_000_001_u64);
    assert_eq!(value["tokens"]["full_updates"], 45_776);
    assert_eq!(value["tokens"]["final_update_targets"], 24_064);
    assert_eq!(value["tokens"]["total_updates"], 45_777);
    assert_eq!(value["tokens"]["complete_spans"], 1_464_843);
    assert_eq!(value["tokens"]["final_span_targets"], 1_536);
    assert_eq!(
        value["evaluation"]["retention_anchor_targets"],
        json!([
            750_000_000,
            1_500_000_000,
            2_250_000_000_u64,
            3_000_000_000_u64
        ])
    );
    assert_eq!(value["increased_axes"], json!(["valid_training_targets"]));
    assert_eq!(
        value["required_rerun_phases"],
        json!([
            "P8", "P9A", "P9B", "P10", "P11", "P12", "P13", "P14", "P15", "P16", "P16A"
        ])
    );

    let divisible = plan(&config_value(canonical_scope(), 2_147_483_648, 28_800)).unwrap();
    assert_eq!(divisible["tokens"]["full_updates"], 32_768);
    assert_eq!(divisible["tokens"]["final_update_targets"], 0);
    assert_eq!(divisible["tokens"]["total_updates"], 32_768);
}

#[test]
fn vocabulary_and_context_changes_rerun_from_their_owning_phases() {
    let mut vocabulary = canonical_scope();
    vocabulary["vocabulary"] = json!(48_000);
    let value = plan(&config_value(vocabulary, 2_000_000_000, 28_800)).unwrap();
    assert_eq!(value["required_rerun_phases"][0], "P7");
    assert_eq!(value["required_rerun_phases"][1], "P7A");

    // A context change alone is not one of the three amendment axes; it must ride on an
    // increased axis and reruns from token materialization.
    let mut context = canonical_scope();
    context["context_length"] = json!(4_096);
    assert_eq!(
        plan(&config_value(context.clone(), 2_000_000_000, 28_800))
            .unwrap_err()
            .code,
        "P19_SCOPE_NOT_INCREASED"
    );
    let value = plan(&config_value(context, 2_000_000_000, 36_000)).unwrap();
    assert_eq!(value["required_rerun_phases"][0], "P8");
    assert_eq!(value["tokens"]["complete_spans"], 488_281);
    assert_eq!(value["tokens"]["final_span_targets"], 1_024);
}

#[test]
fn every_scope_boundary_is_a_typed_rejection() {
    let cases: [(&str, Value, u64, u64); 8] = [
        (
            "P19_SCOPE_NOT_INCREASED",
            canonical_scope(),
            2_000_000_000,
            28_800,
        ),
        (
            "P19_SCOPE_DECREASED",
            {
                let mut model = canonical_scope();
                model["layers"] = json!(8);
                model
            },
            2_000_000_000,
            28_800,
        ),
        (
            "P19_MODEL_SHAPE_INVALID",
            {
                let mut model = canonical_scope();
                model["query_heads"] = json!(10);
                model
            },
            2_000_000_000,
            28_800,
        ),
        (
            "P19_MODEL_SHAPE_INVALID",
            {
                let mut model = canonical_scope();
                model["kv_heads"] = json!(5);
                model
            },
            2_000_000_000,
            28_800,
        ),
        (
            "P19_MODEL_SHAPE_INVALID",
            {
                let mut model = canonical_scope();
                model["untied_lm_head"] = json!(false);
                model
            },
            2_000_000_000,
            28_800,
        ),
        (
            "P19_VOCABULARY_INVALID",
            {
                let mut model = canonical_scope();
                model["vocabulary"] = json!(65_537);
                model
            },
            2_000_000_000,
            28_800,
        ),
        (
            "P19_TARGETS_INVALID",
            canonical_scope(),
            2_000_000_002,
            28_800,
        ),
        ("P19_SLA_INVALID", canonical_scope(), 2_000_000_000, 28_805),
    ];
    for (expected, model, targets, sla) in cases {
        assert_eq!(
            plan(&config_value(model, targets, sla)).unwrap_err().code,
            expected,
            "expected {expected}"
        );
    }

    let mut overflowing = canonical_scope();
    overflowing["vocabulary"] = json!(65_536);
    overflowing["width"] = json!(1_u64 << 50);
    overflowing["head_width"] = json!(1_u64 << 50);
    overflowing["query_heads"] = json!(1);
    overflowing["kv_heads"] = json!(1);
    assert_eq!(
        plan(&config_value(overflowing, 2_000_000_000, 28_800))
            .unwrap_err()
            .code,
        "P19_ARITHMETIC_OVERFLOW"
    );
}

#[test]
fn identical_scopes_share_one_candidate_identity_and_differing_scopes_do_not() {
    let first = plan(&config_value(canonical_scope(), 2_000_000_000, 36_000)).unwrap();
    let second = plan(&config_value(canonical_scope(), 2_000_000_000, 36_000)).unwrap();
    assert_eq!(first["candidate_identity"], second["candidate_identity"]);
    assert_eq!(first["scope_sha256"], second["scope_sha256"]);
    let different = plan(&config_value(canonical_scope(), 2_000_000_000, 72_000)).unwrap();
    assert_ne!(first["scope_sha256"], different["scope_sha256"]);
}

#[test]
fn the_installed_command_emits_one_compact_nonpublishing_result_line() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config(
        directory.path(),
        &config_value(canonical_scope(), 2_000_000_000, 36_000),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_python-slm"))
        .current_dir(directory.path())
        .args(["plan-scale-up", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let value: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["schema"], SCALE_UP_RESULT_SCHEMA);
    assert_eq!(value["qualification_status"], "SKIPPED");
    assert_eq!(value["amendment_status"], "UNAPPROVED_CANDIDATE");
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        1,
        "the command must not write any artifact"
    );
}
