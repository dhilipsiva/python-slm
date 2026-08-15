use rust_llm_pretrain::{
    backend::{BURN_CUBECL_CUDA, ProviderIdentity},
    model::{
        ACCELERATOR_MODEL_SCHEMA, ACCELERATOR_OBSERVATION_SCHEMA, AcceleratorCancellation,
        AcceleratorModelObservation, CPU_ORACLE_FIXTURE_ID, P10_MODEL_SEMANTICS,
        accelerator_execution_plan, accelerator_execution_stages, cpu_oracle_fixture,
        cpu_oracle_fixture_parameters, validate_repeated_accelerator_execution,
    },
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn passing_observation() -> AcceleratorModelObservation {
    let oracle = cpu_oracle_fixture();
    AcceleratorModelObservation {
        schema: ACCELERATOR_OBSERVATION_SCHEMA.to_owned(),
        backend: BURN_CUBECL_CUDA.to_owned(),
        provider: ProviderIdentity::Cuda,
        device_ordinal: 0,
        fixture_id: CPU_ORACLE_FIXTURE_ID.to_owned(),
        model_semantics: P10_MODEL_SEMANTICS.to_owned(),
        parameter_layout_sha256: accelerator_execution_plan()
            .unwrap()
            .parameter_layout_sha256,
        input_token_ids: oracle.input_token_ids,
        target_token_ids: oracle.target_token_ids,
        logits_bf16_le_hex: oracle.logits_bf16_le_hex,
        loss_f32_le_hex: oracle.loss_f32_le_hex,
        gradient_f32_le_hex: oracle.gradient_f32_le_hex,
        gradient_sha256: oracle.gradient_sha256,
        stages_completed: accelerator_execution_stages(),
        synchronized: true,
        owned_resources_released: true,
    }
}

#[test]
fn canonical_plan_and_result_are_closed_and_nonpublishing() {
    let observation = passing_observation();
    let result = validate_repeated_accelerator_execution(&observation, &observation).unwrap();
    assert_eq!(result.schema, ACCELERATOR_MODEL_SCHEMA);
    assert_eq!(result.status, "PARITY_OK");
    assert_eq!(result.qualification_status, "SKIPPED");
    assert_eq!(result.support_level, "implemented");
    assert_eq!(result.plan.parameter_count, 135_285_504);
    assert_eq!(result.plan.parameter_artifact_count, 111);
    assert_eq!(result.plan.parameter_storage, "bf16");
    assert_eq!(result.plan.accumulation, "fp32");
    assert!(result.checks.logits_exact);
    assert!(result.checks.loss_exact);
    assert!(result.checks.gradient_bytes_exact);
    assert!(result.checks.cleanup_complete);
    assert!(result.checks.repeated_execution_exact);
    assert!(!result.receipts_written);

    let value = serde_json::to_value(&result).unwrap();
    let keys = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        keys,
        [
            "backend",
            "checks",
            "expected_gradient_sha256",
            "expected_logits_sha256",
            "expected_loss_sha256",
            "fixture_id",
            "limitations",
            "model_identity",
            "observation_sha256",
            "plan",
            "provider",
            "qualification_status",
            "receipts_written",
            "schema",
            "status",
            "support_level",
        ]
        .into_iter()
        .collect()
    );
    assert!(serde_json::to_string(&value).unwrap().is_ascii());
}

#[test]
fn every_exact_parity_and_cleanup_boundary_fails_closed() {
    let passing = passing_observation();

    let mut changed = passing.clone();
    changed.logits_bf16_le_hex.replace_range(0..2, "00");
    assert_eq!(
        validate_repeated_accelerator_execution(&changed, &changed)
            .unwrap_err()
            .code,
        "P10_ACCELERATOR_PARITY_FAILED"
    );

    let mut changed = passing.clone();
    changed.loss_f32_le_hex.replace_range(0..2, "00");
    assert_eq!(
        validate_repeated_accelerator_execution(&changed, &changed)
            .unwrap_err()
            .code,
        "P10_ACCELERATOR_PARITY_FAILED"
    );

    let mut changed = passing.clone();
    let mut gradient = hex::decode(&changed.gradient_f32_le_hex).unwrap();
    gradient[0] ^= 1;
    changed.gradient_f32_le_hex = hex::encode(&gradient);
    changed.gradient_sha256 = sha256_hex(&gradient);
    assert_eq!(
        validate_repeated_accelerator_execution(&changed, &changed)
            .unwrap_err()
            .code,
        "P10_ACCELERATOR_PARITY_FAILED"
    );

    let mut changed = passing.clone();
    changed.stages_completed.swap(0, 1);
    assert_eq!(
        validate_repeated_accelerator_execution(&changed, &changed)
            .unwrap_err()
            .code,
        "P10_ACCELERATOR_PARITY_FAILED"
    );

    for mut changed in [passing.clone(), passing.clone()] {
        if changed.synchronized {
            changed.synchronized = false;
        } else {
            changed.owned_resources_released = false;
        }
        assert_eq!(
            validate_repeated_accelerator_execution(&changed, &changed)
                .unwrap_err()
                .code,
            "P10_ACCELERATOR_PARITY_FAILED"
        );
    }
    let mut cleanup_failed = passing.clone();
    cleanup_failed.owned_resources_released = false;
    assert_eq!(
        validate_repeated_accelerator_execution(&cleanup_failed, &cleanup_failed)
            .unwrap_err()
            .code,
        "P10_ACCELERATOR_PARITY_FAILED"
    );
}

#[test]
fn repeated_execution_device_drift_and_cancellation_are_typed() {
    let first = passing_observation();
    let mut second = first.clone();
    second.device_ordinal = 1;
    assert_eq!(
        validate_repeated_accelerator_execution(&first, &second)
            .unwrap_err()
            .code,
        "P10_REPEATED_EXECUTION_DRIFT"
    );

    let first_handle = AcceleratorCancellation::default();
    let second_handle = first_handle.clone();
    second_handle.cancel();
    assert!(first_handle.is_cancelled());
    assert_eq!(
        first_handle
            .require_active("before-parameter-load")
            .unwrap_err()
            .code,
        "P10_EXECUTION_CANCELLED"
    );
}

#[test]
fn p9b_fixture_parameter_source_is_complete_and_stable() {
    let parameters = cpu_oracle_fixture_parameters();
    assert_eq!(parameters.len(), 12);
    assert_eq!(
        parameters
            .iter()
            .map(|parameter| parameter.values_bf16_bits.len())
            .sum::<usize>(),
        140
    );
    let bytes = parameters
        .iter()
        .flat_map(|parameter| {
            parameter
                .values_bf16_bits
                .iter()
                .flat_map(|bits| bits.to_le_bytes())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sha256_hex(&bytes),
        "fdb11c7cf88d509a74b767d95f0632d851581f82094e97d73ef645c45fbce308"
    );
}

#[cfg(feature = "cuda")]
#[test]
fn cuda_runner_is_compiled_without_launching_hardware() {
    let _: fn(
        usize,
        &AcceleratorCancellation,
    ) -> anyhow::Result<rust_llm_pretrain::model::AcceleratorModelResult> =
        rust_llm_pretrain::model::run_burn_cubecl_cuda_model_parity;
}
