use rust_llm_pretrain::train::{
    DEFAULT_CONFIG_BYTES, STABILITY_PLAN_BYTES, StabilityPlanV1, build_stability_result,
    parse_default_configuration, parse_stability_plan,
};

#[test]
fn bounded_ladder_is_deterministic_restart_exact_and_claim_limited() {
    let configuration = parse_default_configuration(DEFAULT_CONFIG_BYTES).unwrap();
    let configuration_sha256 = configuration.sha256().unwrap();
    let plan = parse_stability_plan(STABILITY_PLAN_BYTES, &configuration_sha256).unwrap();
    assert_eq!(plan, StabilityPlanV1::canonical(&configuration_sha256));

    let first = build_stability_result(configuration.clone(), plan.clone()).unwrap();
    let second = build_stability_result(configuration, plan).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.schema, "python-slm-stability-ladder-result-v1");
    assert_eq!(first.status, "LADDER_OK");
    assert_eq!(first.qualification_status, "SKIPPED");
    assert_eq!(first.hardware_status, "UNVERIFIED");
    assert_eq!(first.long_duration_status, "UNVERIFIED");
    assert_eq!(first.performance_admission_status, "UNVERIFIED");
    assert_eq!(first.completion_sla_status, "UNVERIFIED");
    assert_eq!(first.execution_surface, "provider-neutral-synthetic");
    assert_eq!(first.trials.len(), 6);
    assert!(first.restart_equivalence);
    assert!(first.repeated_stability_equivalence);
    assert!(first.owned_temporary_root_removed);
    assert!(first.trials.iter().all(|trial| {
        trial.status == "PASSED"
            && trial.configuration_frozen
            && trial.implementation_frozen
            && trial.completed_updates == trial.planned_updates
    }));
    assert_eq!(
        first
            .trials
            .iter()
            .find(|trial| trial.trial_kind == "restart")
            .unwrap()
            .checkpoint_roundtrip_exact,
        Some(true)
    );
    let stability_hashes = first
        .trials
        .iter()
        .filter(|trial| trial.trial_kind == "stability")
        .map(|trial| trial.final_state_sha256.as_str())
        .collect::<Vec<_>>();
    assert_eq!(stability_hashes.len(), 3);
    assert!(stability_hashes.windows(2).all(|pair| pair[0] == pair[1]));

    let json = serde_json::to_string(&first).unwrap();
    assert!(!json.contains(":\\"));
    assert!(!json.contains("receipts"));
    assert!(!json.contains("acceptance"));
    assert!(!json.contains("pointer"));
}

#[cfg(not(windows))]
#[test]
fn product_boundary_defers_before_reading_inputs_off_windows() {
    let error = rust_llm_pretrain::commands::run([
        "python-slm".into(),
        "bench".into(),
        "--config".into(),
        "missing-defaults.json".into(),
        "--stability-plan".into(),
        "missing-stability.json".into(),
    ])
    .unwrap_err();
    assert_eq!(error.code, "DEFERRED_POST_P16");
}

#[cfg(windows)]
#[test]
fn product_boundary_emits_only_the_closed_local_result() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = root
        .join("src/train/prototype-windows-5090-v1.defaults.json")
        .canonicalize()
        .unwrap();
    let plan = root
        .join("src/train/prototype-windows-5090-v1.stability.json")
        .canonicalize()
        .unwrap();
    let value = rust_llm_pretrain::commands::run([
        "python-slm".into(),
        "bench".into(),
        "--config".into(),
        config.into_os_string(),
        "--stability-plan".into(),
        plan.into_os_string(),
    ])
    .unwrap();
    assert_eq!(value["schema"], "python-slm-stability-ladder-result-v1");
    assert_eq!(value["qualification_status"], "SKIPPED");
    assert_eq!(value["hardware_status"], "UNVERIFIED");
    assert_eq!(value["owned_temporary_root_removed"], true);
}
