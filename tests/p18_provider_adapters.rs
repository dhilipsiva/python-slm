use rust_llm_pretrain::backend::{
    BURN_CUBECL_CUDA, BURN_CUBECL_METAL, BURN_CUBECL_ROCM, BackendRequest, BackendRequestKind,
    CandidateResult, PROTOTYPE_PROFILE, ProviderIdentity, RuntimeBackend,
    burn_cubecl_cuda_capability, burn_cubecl_metal_capability, burn_cubecl_rocm_capability,
    provider_backend_name, select_candidate,
    tuples::{
        DISCRETE_STAGING_MEMORY_PATH, P18_LINUX_AMD_ROCM, P18_LINUX_NVIDIA_CUDA,
        P18_MACOS_APPLE_METAL, P18_WINDOWS_NVIDIA_CUDA_REGRESSION, PROVIDER_ADAPTER_MATRIX_SCHEMA,
        UNIFIED_MEMORY_PATH, implemented_profile_provider, mandatory_tuple_lanes,
        provider_adapter_matrix, require_implemented_tuple,
    },
};
use rust_llm_pretrain::error::Result;
use rust_llm_pretrain::model::{
    ACCELERATOR_OBSERVATION_SCHEMA, AcceleratorModelObservation, CANONICAL_MODEL_ID,
    CPU_ORACLE_FIXTURE_ID, P10_MODEL_SEMANTICS, PROVIDER_PARITY_RESULT_SCHEMA,
    accelerator_execution_plan, accelerator_execution_stages, cpu_oracle_fixture,
    validate_repeated_accelerator_execution, validate_repeated_provider_execution,
};
use rust_llm_pretrain::platform::HostPlatform;
use rust_llm_pretrain::storage::{CorpusSplit, TokenSequenceEntry};
use rust_llm_pretrain::train::trainer::{
    BackendStateArtifact, BatchGradient, DeterministicTrainer, EVALUATION_TARGETS,
    EvaluationResult, TARGETS_PER_FULL_UPDATE, TrainerBackend, TrainerIdentity, TrainingBatch,
    state_bundle_sha256,
};
use rust_llm_pretrain::train::{
    AsyncBatchTransfer, LoaderCancellation, SpanLoader, SpanSource, TransferPipeline,
    UnifiedSharedTransfer,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn the_tuple_matrix_holds_exactly_the_four_mandatory_lanes() {
    let matrix = provider_adapter_matrix();
    assert_eq!(matrix.schema, PROVIDER_ADAPTER_MATRIX_SCHEMA);
    assert_eq!(matrix.deferred_selection_code, "DEFERRED_POST_P16");
    let identities = matrix
        .lanes
        .iter()
        .map(|lane| (lane.tuple_id, lane.host, lane.provider, lane.memory_path))
        .collect::<Vec<_>>();
    assert_eq!(
        identities,
        [
            (
                P18_WINDOWS_NVIDIA_CUDA_REGRESSION,
                HostPlatform::WindowsX86_64,
                ProviderIdentity::Cuda,
                DISCRETE_STAGING_MEMORY_PATH,
            ),
            (
                P18_LINUX_NVIDIA_CUDA,
                HostPlatform::LinuxX86_64,
                ProviderIdentity::Cuda,
                DISCRETE_STAGING_MEMORY_PATH,
            ),
            (
                P18_LINUX_AMD_ROCM,
                HostPlatform::LinuxX86_64,
                ProviderIdentity::Rocm,
                DISCRETE_STAGING_MEMORY_PATH,
            ),
            (
                P18_MACOS_APPLE_METAL,
                HostPlatform::MacosAppleSilicon,
                ProviderIdentity::Metal,
                UNIFIED_MEMORY_PATH,
            ),
        ]
    );
    for lane in &matrix.lanes {
        assert_eq!(lane.support_level, "implemented");
        assert_eq!(lane.execution_status, "UNVERIFIED");
        assert_eq!(lane.qualification_status, "SKIPPED");
        assert_eq!(lane.backend, provider_backend_name(lane.provider));
    }
    assert!(
        matrix
            .limitations
            .contains(&"no_two_billion_target_run_claim")
    );
    assert!(
        matrix
            .limitations
            .contains(&"no_cross_provider_checkpoint_migration_claim")
    );

    let encoded = serde_json::to_value(&matrix).unwrap();
    assert_eq!(encoded.as_object().unwrap().len(), 4);
    assert_eq!(encoded["lanes"].as_array().unwrap().len(), 4);
    assert_eq!(
        encoded["lanes"][0]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>()
            .len(),
        9
    );
    assert!(serde_json::to_string(&encoded).unwrap().is_ascii());
}

#[test]
fn stable_wire_values_for_the_new_backends_and_lanes() {
    assert_eq!(
        serde_json::to_string(&BackendRequestKind::BurnCubeclRocm).unwrap(),
        "\"burn-cubecl-rocm\""
    );
    assert_eq!(
        serde_json::to_string(&BackendRequestKind::BurnCubeclMetal).unwrap(),
        "\"burn-cubecl-metal\""
    );
    for (variant, backend) in [
        (
            RuntimeBackend::BurnCubeclRocm {
                device_uuid: "GPU-1".to_owned(),
            },
            "burn-cubecl-rocm",
        ),
        (
            RuntimeBackend::BurnCubeclMetal {
                device_uuid: "GPU-1".to_owned(),
            },
            "burn-cubecl-metal",
        ),
    ] {
        let value = serde_json::to_value(&variant).unwrap();
        assert_eq!(value["backend"], backend);
        assert_eq!(value.as_object().unwrap().len(), 2);
    }
    assert_eq!(
        serde_json::to_value(ProviderIdentity::Rocm).unwrap(),
        "rocm"
    );
    assert_eq!(
        serde_json::to_value(ProviderIdentity::Metal).unwrap(),
        "metal"
    );
}

#[test]
fn unlisted_tuples_and_profiles_stay_deferred() {
    for (host, provider) in [
        (HostPlatform::WindowsX86_64, ProviderIdentity::Rocm),
        (HostPlatform::WindowsX86_64, ProviderIdentity::Metal),
        (HostPlatform::LinuxX86_64, ProviderIdentity::Metal),
        (HostPlatform::MacosAppleSilicon, ProviderIdentity::Cuda),
        (HostPlatform::MacosAppleSilicon, ProviderIdentity::Rocm),
    ] {
        assert_eq!(
            require_implemented_tuple(host, provider).unwrap_err().code,
            "DEFERRED_POST_P16"
        );
    }
    for lane in mandatory_tuple_lanes() {
        assert_eq!(
            require_implemented_tuple(lane.host, lane.provider).unwrap(),
            lane
        );
    }
    assert_eq!(implemented_profile_provider("some-future-profile"), None);
    assert_eq!(
        implemented_profile_provider(PROTOTYPE_PROFILE),
        Some(ProviderIdentity::Cuda)
    );
}

fn candidate(capability: rust_llm_pretrain::backend::BackendCapability) -> CandidateResult {
    CandidateResult {
        capability,
        compiled: true,
        implemented: true,
        passed: true,
        failure_code: None,
    }
}

fn request(
    profile: &str,
    provider: ProviderIdentity,
    backend: BackendRequestKind,
) -> BackendRequest {
    BackendRequest {
        profile: profile.to_owned(),
        provider,
        backend,
        device_uuid: None,
    }
}

#[test]
fn lane_profiles_select_only_their_exact_provider_backend() {
    let candidates = [
        candidate(burn_cubecl_cuda_capability()),
        candidate(burn_cubecl_rocm_capability()),
        candidate(burn_cubecl_metal_capability()),
    ];
    let cases: [(&str, ProviderIdentity, &str); 4] = [
        (
            P18_WINDOWS_NVIDIA_CUDA_REGRESSION,
            ProviderIdentity::Cuda,
            BURN_CUBECL_CUDA,
        ),
        (
            P18_LINUX_NVIDIA_CUDA,
            ProviderIdentity::Cuda,
            BURN_CUBECL_CUDA,
        ),
        (P18_LINUX_AMD_ROCM, ProviderIdentity::Rocm, BURN_CUBECL_ROCM),
        (
            P18_MACOS_APPLE_METAL,
            ProviderIdentity::Metal,
            BURN_CUBECL_METAL,
        ),
    ];
    for (profile, provider, backend) in cases {
        let selected = select_candidate(
            &request(profile, provider, BackendRequestKind::Auto),
            &candidates,
        )
        .unwrap();
        assert_eq!(selected.capability.backend, backend);
        assert_eq!(selected.capability.provider, provider);
    }
}

#[test]
fn selection_stays_fail_closed_across_the_lane_matrix() {
    let candidates = [
        candidate(burn_cubecl_cuda_capability()),
        candidate(burn_cubecl_rocm_capability()),
    ];

    // A provider that does not belong to the requested lane fails before candidate policy.
    assert_eq!(
        select_candidate(
            &request(
                P18_LINUX_AMD_ROCM,
                ProviderIdentity::Cuda,
                BackendRequestKind::Auto
            ),
            &candidates
        )
        .unwrap_err()
        .code,
        "DEFERRED_POST_P16"
    );
    // The prototype profile remains CUDA-only.
    assert_eq!(
        select_candidate(
            &request(
                PROTOTYPE_PROFILE,
                ProviderIdentity::Metal,
                BackendRequestKind::Auto
            ),
            &candidates
        )
        .unwrap_err()
        .code,
        "DEFERRED_POST_P16"
    );
    // Unknown profiles keep the stable deferred code.
    assert_eq!(
        select_candidate(
            &request(
                "prototype-linux-9070xt-v1",
                ProviderIdentity::Rocm,
                BackendRequestKind::Auto
            ),
            &candidates
        )
        .unwrap_err()
        .code,
        "DEFERRED_POST_P16"
    );
    // Explicit selection of a missing or failed backend never falls back.
    assert_eq!(
        select_candidate(
            &request(
                P18_MACOS_APPLE_METAL,
                ProviderIdentity::Metal,
                BackendRequestKind::BurnCubeclMetal
            ),
            &candidates
        )
        .unwrap_err()
        .code,
        "P2_BACKEND_NOT_AVAILABLE"
    );
    let mut failed_rocm = candidate(burn_cubecl_rocm_capability());
    failed_rocm.passed = false;
    failed_rocm.failure_code = Some("P18_ROCM_GRADIENT_BYTES_MISMATCH".to_owned());
    assert_eq!(
        select_candidate(
            &request(
                P18_LINUX_AMD_ROCM,
                ProviderIdentity::Rocm,
                BackendRequestKind::BurnCubeclRocm
            ),
            &[candidate(burn_cubecl_cuda_capability()), failed_rocm]
        )
        .unwrap_err()
        .code,
        "P2_BACKEND_NOT_AVAILABLE"
    );
}

#[test]
fn provider_capabilities_are_closed_and_exact() {
    for (capability, backend, provider, framework) in [
        (
            burn_cubecl_cuda_capability(),
            BURN_CUBECL_CUDA,
            ProviderIdentity::Cuda,
            "burn-cubecl",
        ),
        (
            burn_cubecl_rocm_capability(),
            BURN_CUBECL_ROCM,
            ProviderIdentity::Rocm,
            "burn-cubecl",
        ),
        (
            burn_cubecl_metal_capability(),
            BURN_CUBECL_METAL,
            ProviderIdentity::Metal,
            "burn-cubecl-wgpu",
        ),
    ] {
        assert_eq!(capability.backend, backend);
        assert_eq!(capability.provider, provider);
        assert_eq!(capability.support_level, "implemented");
        assert_eq!(capability.framework, framework);
        assert!(capability.autodiff);
        assert!(capability.bf16);
        assert!(capability.exact_gradient_bytes);
        assert_eq!(capability.compatibility_allocation_bytes, 2_952_790_016);
    }
}

fn passing_observation(provider: ProviderIdentity) -> AcceleratorModelObservation {
    let oracle = cpu_oracle_fixture();
    AcceleratorModelObservation {
        schema: ACCELERATOR_OBSERVATION_SCHEMA.to_owned(),
        backend: provider_backend_name(provider).to_owned(),
        provider,
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
fn every_provider_validates_against_the_same_literal_oracle_bytes() {
    for provider in [
        ProviderIdentity::Cuda,
        ProviderIdentity::Rocm,
        ProviderIdentity::Metal,
    ] {
        let observation = passing_observation(provider);
        let result =
            validate_repeated_provider_execution(provider, &observation, &observation).unwrap();
        assert_eq!(result.schema, PROVIDER_PARITY_RESULT_SCHEMA);
        assert_eq!(result.status, "PARITY_OK");
        assert_eq!(result.qualification_status, "SKIPPED");
        assert_eq!(result.support_level, "implemented");
        assert_eq!(result.model_identity, CANONICAL_MODEL_ID);
        assert_eq!(result.backend, provider_backend_name(provider));
        assert_eq!(result.provider, provider);
        assert!(result.checks.gradient_bytes_exact);
        assert!(result.checks.repeated_execution_exact);
        assert!(!result.receipts_written);
        assert!(
            result
                .limitations
                .contains(&"no_two_billion_target_run_claim")
        );
        assert!(
            result
                .limitations
                .contains(&"no_performance_equivalence_claim")
        );

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
    }
}

#[test]
fn provider_parity_fails_closed_on_identity_drift_and_cleanup() {
    // A CUDA observation cannot satisfy the ROCm lane.
    let cuda = passing_observation(ProviderIdentity::Cuda);
    assert_eq!(
        validate_repeated_provider_execution(ProviderIdentity::Rocm, &cuda, &cuda)
            .unwrap_err()
            .code,
        "P18_OBSERVATION_IDENTITY_MISMATCH"
    );

    // One flipped gradient bit fails the exact-byte gate.
    let passing = passing_observation(ProviderIdentity::Rocm);
    let mut drifted = passing.clone();
    let mut gradient = hex::decode(&drifted.gradient_f32_le_hex).unwrap();
    gradient[0] ^= 1;
    drifted.gradient_f32_le_hex = hex::encode(&gradient);
    drifted.gradient_sha256 = sha256_hex(&gradient);
    assert_eq!(
        validate_repeated_provider_execution(ProviderIdentity::Rocm, &drifted, &drifted)
            .unwrap_err()
            .code,
        "P18_ACCELERATOR_PARITY_FAILED"
    );

    // Missing cleanup fails closed.
    let mut unreleased = passing.clone();
    unreleased.owned_resources_released = false;
    assert_eq!(
        validate_repeated_provider_execution(ProviderIdentity::Rocm, &unreleased, &unreleased)
            .unwrap_err()
            .code,
        "P18_ACCELERATOR_PARITY_FAILED"
    );

    // Two executions must be byte-identical.
    let mut second = passing.clone();
    second.device_ordinal = 1;
    assert_eq!(
        validate_repeated_provider_execution(ProviderIdentity::Rocm, &passing, &second)
            .unwrap_err()
            .code,
        "P18_REPEATED_EXECUTION_DRIFT"
    );

    // The immutable P10 validator remains pinned to the CUDA fixture.
    let rocm = passing_observation(ProviderIdentity::Rocm);
    assert_eq!(
        validate_repeated_accelerator_execution(&rocm, &rocm)
            .unwrap_err()
            .code,
        "P10_OBSERVATION_IDENTITY_MISMATCH"
    );
}

struct SyntheticSource {
    entries: Vec<TokenSequenceEntry>,
    spans: Vec<Vec<u16>>,
}

impl SyntheticSource {
    fn ordered(count: u64) -> Self {
        let mut entries = Vec::new();
        let mut spans = Vec::new();
        let mut first_id = 0;
        for sequence in 0..count {
            let valid_targets = 4;
            entries.push(TokenSequenceEntry {
                split: CorpusSplit::Train,
                sequence,
                first_id,
                logical_ids: valid_targets + 1,
                valid_targets,
            });
            spans.push(
                (0..=valid_targets)
                    .map(|offset| (first_id + offset + 10) as u16)
                    .collect(),
            );
            first_id += valid_targets;
        }
        Self { entries, spans }
    }
}

impl SpanSource for SyntheticSource {
    fn sequence_entries(&self) -> &[TokenSequenceEntry] {
        &self.entries
    }

    fn read_sequence(&self, _split: CorpusSplit, sequence: u64) -> Result<Vec<u16>> {
        Ok(self.spans[sequence as usize].clone())
    }
}

#[test]
fn unified_transfer_shares_the_exact_allocation_without_a_staging_copy() {
    let source = SyntheticSource::ordered(1);
    let mut loader = SpanLoader::new(
        &source,
        CorpusSplit::Train,
        1,
        LoaderCancellation::default(),
    )
    .unwrap();
    let span = loader.next_span().unwrap().unwrap();
    let shared_pointer = span.token_ids().as_ptr();
    let expected_bytes = span.bytes();

    let mut transfer = UnifiedSharedTransfer::new();
    assert_eq!(transfer.memory_path(), UNIFIED_MEMORY_PATH);
    let ticket = transfer.submit(span).unwrap();
    assert_eq!(transfer.live_shared_allocations(), 1);
    let batch = transfer.wait(ticket).unwrap();
    assert!(batch.synchronized);
    assert_eq!(batch.shared_token_ids().as_ptr(), shared_pointer);
    assert_eq!(batch.bytes, expected_bytes);
    assert_eq!(batch.split, CorpusSplit::Train);
    assert_eq!(batch.sequence, 0);
    assert_eq!(batch.first_id, 0);
    assert_eq!(batch.valid_targets, 4);
    assert_eq!(batch.shared_token_ids(), &[10, 11, 12, 13, 14]);
    drop(batch);
    assert_eq!(transfer.live_shared_allocations(), 0);
}

#[test]
fn unified_transfer_retires_in_source_order_and_cleans_up_on_drop() {
    let source = SyntheticSource::ordered(5);

    // Out-of-order retirement is a typed integrity failure.
    let mut loader = SpanLoader::new(
        &source,
        CorpusSplit::Train,
        2,
        LoaderCancellation::default(),
    )
    .unwrap();
    let mut transfer = UnifiedSharedTransfer::new();
    let first = transfer
        .submit(loader.next_span().unwrap().unwrap())
        .unwrap();
    let second = transfer
        .submit(loader.next_span().unwrap().unwrap())
        .unwrap();
    assert_eq!(
        transfer.wait(second).unwrap_err().code,
        "P18_UNIFIED_RETIREMENT_ORDER_INVALID"
    );
    let delivered = transfer.wait(first).unwrap();
    assert_eq!(delivered.sequence, 0);
    drop(delivered);
    assert_eq!(transfer.live_shared_allocations(), 0);

    // The pipeline retires unified batches in exact source order.
    let loader = SpanLoader::new(
        &source,
        CorpusSplit::Train,
        2,
        LoaderCancellation::default(),
    )
    .unwrap();
    let transfer = UnifiedSharedTransfer::new();
    let probe = transfer.allocation_probe();
    let mut pipeline = TransferPipeline::new(loader, transfer, 3).unwrap();
    let mut sequences = Vec::new();
    while let Some(batch) = pipeline.next_device_batch().unwrap() {
        assert!(batch.synchronized);
        sequences.push(batch.sequence);
    }
    assert_eq!(sequences, [0, 1, 2, 3, 4]);
    assert_eq!(probe.live(), 0);

    // Dropping the pipeline mid-stream releases every in-flight shared allocation.
    let loader = SpanLoader::new(
        &source,
        CorpusSplit::Train,
        2,
        LoaderCancellation::default(),
    )
    .unwrap();
    let transfer = UnifiedSharedTransfer::new();
    let probe = transfer.allocation_probe();
    let mut pipeline = TransferPipeline::new(loader, transfer, 3).unwrap();
    let held = pipeline.next_device_batch().unwrap().unwrap();
    assert!(pipeline.in_flight() > 0);
    drop(pipeline);
    assert_eq!(probe.live(), 1);
    drop(held);
    assert_eq!(probe.live(), 0);
}

#[derive(Clone, Default)]
struct ScalarBackend {
    state: u64,
}

impl ScalarBackend {
    fn artifacts(&self) -> Vec<BackendStateArtifact> {
        let mut artifacts = [
            ("parameters_bf16", "state/parameters.bf16", 1_u8),
            ("master_parameters_fp32", "state/master.f32", 2),
            ("adamw_first_moments_fp32", "state/moment1.f32", 3),
            ("adamw_second_moments_fp32", "state/moment2.f32", 4),
            ("backend_runtime_state", "state/runtime.bin", 5),
        ]
        .into_iter()
        .map(|(role, path, tag)| {
            let mut bytes = vec![tag];
            bytes.extend_from_slice(&self.state.to_le_bytes());
            BackendStateArtifact {
                role: role.to_owned(),
                relative_path: path.to_owned(),
                bytes,
            }
        })
        .collect::<Vec<_>>();
        artifacts.sort_by(|left, right| left.role.cmp(&right.role));
        artifacts
    }
}

impl TrainerBackend for ScalarBackend {
    fn accumulate(&mut self, batch: &TrainingBatch) -> Result<BatchGradient> {
        Ok(BatchGradient {
            loss_sum: batch.valid_targets as f64 * 0.5,
            gradient_sums: vec![
                batch.valid_targets as f32,
                batch.valid_targets as f32 * 0.25,
            ],
            host_rng_state: batch
                .first_target
                .wrapping_add(batch.valid_targets)
                .to_le_bytes()
                .to_vec(),
            device_rng_state: batch.valid_targets.to_le_bytes().to_vec(),
        })
    }

    fn apply_update(
        &mut self,
        gradients: &[f32],
        learning_rate: f32,
        one_based_update: u64,
        valid_targets: u64,
    ) -> Result<String> {
        let mut hasher = Sha256::new();
        hasher.update(self.state.to_le_bytes());
        hasher.update(one_based_update.to_le_bytes());
        hasher.update(valid_targets.to_le_bytes());
        hasher.update(learning_rate.to_le_bytes());
        for value in gradients {
            hasher.update(value.to_le_bytes());
        }
        let digest = hasher.finalize();
        self.state = u64::from_le_bytes(digest[..8].try_into().unwrap());
        Ok(hex::encode(digest))
    }

    fn evaluate(&mut self, validation_span_manifest_sha256: &str) -> Result<EvaluationResult> {
        let mut hasher = Sha256::new();
        hasher.update(b"python-slm/p18-evaluation/v1\0");
        hasher.update(validation_span_manifest_sha256.as_bytes());
        hasher.update(self.state.to_le_bytes());
        Ok(EvaluationResult {
            evaluated_targets: EVALUATION_TARGETS,
            aggregate_loss: 12.5,
            result_sha256: hex::encode(hasher.finalize()),
        })
    }

    fn snapshot(&self) -> Result<Vec<BackendStateArtifact>> {
        Ok(self.artifacts())
    }

    fn restore(&mut self, artifacts: &[BackendStateArtifact]) -> Result<()> {
        let runtime = artifacts
            .iter()
            .find(|artifact| artifact.role == "backend_runtime_state")
            .unwrap();
        self.state = u64::from_le_bytes(runtime.bytes[1..].try_into().unwrap());
        Ok(())
    }
}

fn lane_identity(profile: &str) -> TrainerIdentity {
    TrainerIdentity {
        profile: profile.to_owned(),
        model_identity: CANONICAL_MODEL_ID.to_owned(),
        model_parameter_layout_sha256: "11".repeat(32),
        backend_identity_sha256: "22".repeat(32),
        device_identity_sha256: "33".repeat(32),
        corpus_manifest_sha256: "44".repeat(32),
        training_span_manifest_sha256: "55".repeat(32),
        validation_span_manifest_sha256: "66".repeat(32),
        tokenizer_artifact_sha256: "77".repeat(32),
        environment_identity_sha256: "88".repeat(32),
        implementation_artifact_sha256: "99".repeat(32),
    }
}

fn batch(first_target: u64, valid_targets: u64) -> TrainingBatch {
    TrainingBatch {
        first_target,
        valid_targets,
        input_ids: vec![7; valid_targets as usize],
        target_ids: vec![8; valid_targets as usize],
    }
}

#[test]
fn lane_profiles_keep_byte_identical_resume_behind_the_provider_interface() {
    let half = TARGETS_PER_FULL_UPDATE / 2;
    for profile in [P18_LINUX_AMD_ROCM, P18_MACOS_APPLE_METAL] {
        let identity = lane_identity(profile);
        identity.validate().unwrap();

        // Uninterrupted execution: two complete optimizer updates with a mid-run checkpoint.
        let mut uninterrupted =
            DeterministicTrainer::new(identity.clone(), ScalarBackend::default(), vec![1], vec![2])
                .unwrap();
        assert!(
            uninterrupted
                .process_batch(&batch(0, half))
                .unwrap()
                .is_none()
        );
        assert!(
            uninterrupted
                .process_batch(&batch(half, half))
                .unwrap()
                .is_some()
        );
        let checkpoint = uninterrupted.snapshot().unwrap();
        assert!(
            uninterrupted
                .process_batch(&batch(TARGETS_PER_FULL_UPDATE, half))
                .unwrap()
                .is_none()
        );
        let uninterrupted_event = uninterrupted
            .process_batch(&batch(TARGETS_PER_FULL_UPDATE + half, half))
            .unwrap()
            .unwrap();
        let uninterrupted_bundle = state_bundle_sha256(&uninterrupted.snapshot().unwrap()).unwrap();

        // Resumed execution from the mid-run checkpoint must be byte-identical.
        let mut resumed =
            DeterministicTrainer::from_snapshot(checkpoint, &identity, ScalarBackend::default())
                .unwrap();
        assert!(
            resumed
                .process_batch(&batch(TARGETS_PER_FULL_UPDATE, half))
                .unwrap()
                .is_none()
        );
        let resumed_event = resumed
            .process_batch(&batch(TARGETS_PER_FULL_UPDATE + half, half))
            .unwrap()
            .unwrap();
        assert_eq!(
            resumed_event.update_state_sha256,
            uninterrupted_event.update_state_sha256
        );
        assert_eq!(
            resumed_event.normalized_loss_f64_le_hex,
            uninterrupted_event.normalized_loss_f64_le_hex
        );
        assert_eq!(
            state_bundle_sha256(&resumed.snapshot().unwrap()).unwrap(),
            uninterrupted_bundle
        );
    }

    // Everything outside the implemented tuple set still fails closed.
    assert_eq!(
        lane_identity("prototype-linux-9070xt-v1")
            .validate()
            .unwrap_err()
            .code,
        "P12_IDENTITY_MISMATCH"
    );
}

#[cfg(feature = "cuda")]
#[test]
fn cuda_provider_runner_is_compiled_without_launching_hardware() {
    let _: fn(
        usize,
        &rust_llm_pretrain::model::AcceleratorCancellation,
    ) -> anyhow::Result<rust_llm_pretrain::model::ProviderParityResult> =
        rust_llm_pretrain::model::run_burn_cubecl_cuda_provider_parity;
}

#[cfg(all(feature = "rocm", target_os = "linux"))]
#[test]
fn rocm_runner_and_transfer_are_compiled_without_launching_hardware() {
    let _: fn(
        usize,
        &rust_llm_pretrain::model::AcceleratorCancellation,
    ) -> anyhow::Result<rust_llm_pretrain::model::ProviderParityResult> =
        rust_llm_pretrain::model::run_burn_cubecl_rocm_model_parity;
    let _: fn() -> anyhow::Result<rust_llm_pretrain::backend::BackendFixtureDiagnostics> =
        rust_llm_pretrain::backend::rocm::run_burn_cubecl_rocm_fixture;
    let _: fn(
        u32,
    ) -> rust_llm_pretrain::error::Result<
        rust_llm_pretrain::train::rocm_transfer::RocmPinnedTransfer,
    > = rust_llm_pretrain::train::rocm_transfer::RocmPinnedTransfer::new;
}

#[cfg(all(feature = "metal", target_os = "macos"))]
#[test]
fn metal_runner_is_compiled_without_launching_hardware() {
    let _: fn(
        usize,
        &rust_llm_pretrain::model::AcceleratorCancellation,
    ) -> anyhow::Result<rust_llm_pretrain::model::ProviderParityResult> =
        rust_llm_pretrain::model::run_burn_cubecl_metal_model_parity;
    let _: fn() -> anyhow::Result<rust_llm_pretrain::backend::BackendFixtureDiagnostics> =
        rust_llm_pretrain::backend::metal::run_burn_cubecl_metal_fixture;
}
