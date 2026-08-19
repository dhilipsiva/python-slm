use rust_llm_pretrain::backend::{BURN_CUBECL_CUDA, PROTOTYPE_PROFILE};
use rust_llm_pretrain::commands;
use rust_llm_pretrain::train::profile::{
    ACTUAL_ELAPSED_LIMIT_NS, ADMISSION_SECONDS, COMPLETION_SLA_SECONDS, DEFAULT_CONFIG_BYTES,
    DEFAULT_CONFIG_SCHEMA, DIAGNOSTICS_SCHEMA, GRADIENT_ACCUMULATION_STEPS, LOADER_BUFFER_SPANS,
    MICRO_BATCH_SEQUENCES, ProfileDiagnosticsV1, ProfileObservationV1, PrototypeTrainingDefaultsV1,
    TRANSFER_IN_FLIGHT, build_result, parse_default_configuration, parse_diagnostics,
};
use rust_llm_pretrain::train::trainer::{
    EVALUATION_TARGETS, EVENT_INTERVAL_TARGETS, RETENTION_ANCHORS, TARGETS_PER_FULL_UPDATE,
};
use serde_json::json;

fn observation(sample_id: &str, targets: u64, elapsed_ns: u64) -> ProfileObservationV1 {
    ProfileObservationV1 {
        sample_id: sample_id.to_owned(),
        valid_targets: targets,
        synchronized_elapsed_ns: elapsed_ns,
        loader_wait_ns: elapsed_ns / 10,
        evaluation_ns: 0,
        checkpoint_ns: 0,
        peak_accelerator_bytes: Some(8_000_000_000),
        synchronized: true,
        cleanup_complete: true,
    }
}

#[test]
fn selected_defaults_compose_every_frozen_phase_boundary() {
    let defaults = parse_default_configuration(DEFAULT_CONFIG_BYTES).unwrap();
    assert_eq!(defaults.schema, DEFAULT_CONFIG_SCHEMA);
    assert_eq!(defaults.profile, PROTOTYPE_PROFILE);
    assert_eq!(defaults.support_level, "implemented");
    assert_eq!(defaults.qualification_status, "SKIPPED");
    assert_eq!(defaults.batch.micro_batch_sequences, MICRO_BATCH_SEQUENCES);
    assert_eq!(
        defaults.batch.gradient_accumulation_steps,
        GRADIENT_ACCUMULATION_STEPS
    );
    assert_eq!(
        defaults.batch.micro_batch_targets * defaults.batch.gradient_accumulation_steps,
        TARGETS_PER_FULL_UPDATE
    );
    assert_eq!(defaults.loader.host_buffer_spans, LOADER_BUFFER_SPANS);
    assert_eq!(
        defaults.loader.maximum_in_flight_transfers,
        TRANSFER_IN_FLIGHT
    );
    assert_eq!(defaults.checkpoint.interval_targets, EVENT_INTERVAL_TARGETS);
    assert_eq!(defaults.checkpoint.retention_anchors, RETENTION_ANCHORS);
    assert_eq!(defaults.evaluation.interval_targets, EVENT_INTERVAL_TARGETS);
    assert_eq!(defaults.evaluation.evaluated_targets, EVALUATION_TARGETS);
    assert_eq!(defaults.backend.backend, BURN_CUBECL_CUDA);
    assert_eq!(defaults.backend.selection, "explicit");
    assert_eq!(defaults.sla.admission_seconds, ADMISSION_SECONDS);
    assert_eq!(defaults.sla.completion_seconds, COMPLETION_SLA_SECONDS);
    assert_eq!(
        defaults.sla.actual_elapsed_limit_ns,
        ACTUAL_ELAPSED_LIMIT_NS
    );
    assert_eq!(defaults.sla.claim_status, "DESIGN_TARGET_UNVERIFIED");
    assert_eq!(
        defaults.sha256().unwrap(),
        "41fe1c97639fbbb974094fe7d6ef250a15f0d80aab30eb2e87fca48eea0f447d"
    );
}

#[test]
fn defaults_are_closed_and_contract_constants_cannot_be_retuned() {
    let mut value = serde_json::to_value(PrototypeTrainingDefaultsV1::canonical()).unwrap();
    // Three, because four is now the canonical value: the profile moved to eight
    // sequences per dispatch and four accumulation steps, so a probe that still
    // used four would assert nothing.
    value["batch"]["gradient_accumulation_steps"] = json!(3);
    assert_eq!(
        parse_default_configuration(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .code,
        "P14_DEFAULT_CONFIG_MISMATCH"
    );

    let mut value = serde_json::to_value(PrototypeTrainingDefaultsV1::canonical()).unwrap();
    value["sla"]["admission_seconds"] = json!(25_919);
    assert_eq!(
        parse_default_configuration(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .code,
        "P14_DEFAULT_CONFIG_MISMATCH"
    );

    let mut value = serde_json::to_value(PrototypeTrainingDefaultsV1::canonical()).unwrap();
    value["unexpected"] = json!(true);
    assert_eq!(
        parse_default_configuration(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .code,
        "P14_CONFIG_INVALID"
    );
}

#[test]
fn optional_diagnostics_are_deterministic_and_never_become_performance_evidence() {
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let digest = configuration.sha256().unwrap();
    let diagnostics = ProfileDiagnosticsV1 {
        schema: DIAGNOSTICS_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        configuration_sha256: digest,
        observations: vec![
            observation("sample-01", 65_536, 1_000_000_000),
            observation("sample-02", 65_536, 2_000_000_000),
        ],
    };
    let result = build_result(configuration.clone(), Some(diagnostics)).unwrap();
    assert_eq!(result.status, "DEFAULT_CONFIGURED");
    assert_eq!(result.performance_status, "UNVERIFIED");
    assert_eq!(result.qualification_status, "SKIPPED");
    assert_eq!(result.configuration, configuration);
    assert_eq!(result.diagnostics.sample_count, 2);
    assert_eq!(
        result.diagnostics.minimum_milli_targets_per_second,
        Some(32_768_000)
    );
    assert_eq!(
        result.diagnostics.maximum_milli_targets_per_second,
        Some(65_536_000)
    );
    assert_eq!(result.diagnostics.synchronized_samples, 2);
    assert_eq!(result.diagnostics.cleanup_complete_samples, 2);
    assert!(
        result
            .limitations
            .contains(&"not-performance-admission".to_owned())
    );
}

#[test]
fn diagnostics_require_exact_identity_order_and_bounded_arithmetic() {
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let digest = configuration.sha256().unwrap();
    let wrong_identity = ProfileDiagnosticsV1 {
        schema: DIAGNOSTICS_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        configuration_sha256: "00".repeat(32),
        observations: Vec::new(),
    };
    assert_eq!(
        wrong_identity.validate(&digest).unwrap_err().code,
        "P14_DIAGNOSTIC_IDENTITY_MISMATCH"
    );

    let invalid = ProfileDiagnosticsV1 {
        schema: DIAGNOSTICS_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        configuration_sha256: digest.clone(),
        observations: vec![
            observation("sample-02", 1, 1),
            observation("sample-01", 1, 1),
        ],
    };
    assert_eq!(
        invalid.validate(&digest).unwrap_err().code,
        "P14_DIAGNOSTIC_INVALID"
    );

    let malformed = br#"{"schema":"python-slm-prototype-profile-diagnostics-v1","profile":"prototype-windows-5090-v1","configuration_sha256":"bad","observations":[],"extra":true}"#;
    assert_eq!(
        parse_diagnostics(malformed, &digest).unwrap_err().code,
        "P14_DIAGNOSTICS_INVALID"
    );
}

#[cfg(windows)]
#[test]
fn bench_reads_explicit_controls_and_writes_no_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("defaults.json");
    std::fs::write(&config_path, DEFAULT_CONFIG_BYTES).unwrap();
    let configuration = PrototypeTrainingDefaultsV1::canonical();
    let diagnostics = ProfileDiagnosticsV1 {
        schema: DIAGNOSTICS_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        configuration_sha256: configuration.sha256().unwrap(),
        observations: vec![observation("sample-01", 65_536, 1_000_000_000)],
    };
    let diagnostics_path = directory.path().join("diagnostics.json");
    std::fs::write(&diagnostics_path, serde_json::to_vec(&diagnostics).unwrap()).unwrap();
    let before = std::fs::read_dir(directory.path()).unwrap().count();
    let result = commands::run([
        "python-slm".into(),
        "bench".into(),
        "--config".into(),
        config_path.into_os_string(),
        "--diagnostics".into(),
        diagnostics_path.into_os_string(),
    ])
    .unwrap();
    assert_eq!(result["schema"], "python-slm-prototype-profile-result-v1");
    assert_eq!(result["performance_status"], "UNVERIFIED");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), before);
    let public = serde_json::to_string(&result).unwrap();
    assert!(!public.contains(directory.path().to_string_lossy().as_ref()));
    assert!(!public.contains("docs/receipts"));
}

#[cfg(not(windows))]
#[test]
fn bench_defers_before_reading_configuration_off_windows() {
    let error = commands::run([
        "python-slm".into(),
        "bench".into(),
        "--config".into(),
        "does-not-exist.json".into(),
    ])
    .unwrap_err();
    assert_eq!(error.code, "DEFERRED_POST_P16");
}
