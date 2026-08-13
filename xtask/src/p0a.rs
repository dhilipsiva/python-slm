use crate::error::{IoContext, Result, XtaskError};
use crate::hash;
use crate::p0;
use crate::process::{FileRef, RecordedCommand, Recorder};
use crate::publication;
use crate::time;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const INTERFACE_ID: &str = "portable-interface-v2";
const PROFILE_ID: &str = "prototype-windows-5090-v1";
const OUTPUT_ROOT: &str = "docs/receipts/P0A";

const SCHEMA_PATHS: &[&str] = &[
    "docs/schemas/P0A/python-slm-p0a-approval-v1.schema.json",
    "docs/schemas/P0A/python-slm-p0a-phase-acceptance-v1.schema.json",
    "docs/schemas/P0A/python-slm-p0a-phase-evidence-v1.schema.json",
    "docs/schemas/P0A/python-slm-p0a-phase-pointer-v1.schema.json",
    "docs/schemas/P0A/python-slm-p0a-schema-bundle-v1.schema.json",
    "docs/schemas/P0A/python-slm-p0a-source-identity-v1.schema.json",
    "docs/schemas/P0A/python-slm-p0-dependency-v1.schema.json",
    "docs/schemas/P0A/python-slm-parser-boundary-v1.schema.json",
    "docs/schemas/portable-v2/python-slm-prototype-profile-v1.schema.json",
    "docs/schemas/portable-v2/python-slm-prototype-sla-v1.schema.json",
    "docs/schemas/portable-v2/python-slm-training-memory-policy-v1.schema.json",
    "docs/schemas/portable-v2/tree-sitter-python-compatibility-v1.json",
];

const SNAPSHOTS: &[(&str, &str)] = &[
    (
        "docs/rebuild-contract-v2.md",
        "artifacts/rebuild-contract-v2.md",
    ),
    (
        "docs/decision-ledger-v2.md",
        "artifacts/decision-ledger-v2.md",
    ),
    (
        "docs/adr/0000-prototype-first-portable-interface.md",
        "artifacts/0000-prototype-first-portable-interface.md",
    ),
    ("docs/ARCHITECTURE.md", "artifacts/ARCHITECTURE.md"),
    ("AGENTS.md", "artifacts/AGENTS.md"),
    ("TODO.md", "artifacts/TODO-preapproval.md"),
    ("Cargo.lock", "artifacts/Cargo.lock"),
];

const GENERATED_SOURCES: &[(&str, &str)] = &[
    (
        "bindings/rust/build.rs",
        "4e069992deffe54f04909739e954f581409c7d167cb5ba1f775717258751568f",
    ),
    (
        "src/grammar.json",
        "2adc15e78f4dae293e85de1517c45bcee2f8b075fa30a083c658925b944afec4",
    ),
    (
        "src/node-types.json",
        "a2456847bea3adff5b2222b2f7b03a870159470d8908622204e6eb29ee2fe45e",
    ),
    (
        "src/parser.c",
        "a895f10b3cf7b2608f3283b43cd5cfed70971c7ee4a0136abbaaccbc4a7a25e0",
    ),
    (
        "src/scanner.c",
        "6db82134ac2d4c90a1a1475487a625cface02662ebda9b7478cad9c7147e9afe",
    ),
];

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceIdentity {
    schema: &'static str,
    commit: String,
    tree: String,
    branch: String,
    dirty: bool,
    cargo_lock_sha256: String,
    verifier_source_sha256: String,
    schema_bundle_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct P0Dependency {
    schema: &'static str,
    status: &'static str,
    baseline_commit: &'static str,
    receipt_commit: &'static str,
    receipt_sha256: &'static str,
    contract_sha256: &'static str,
    decision_ledger_sha256: &'static str,
    reconciliation_commit: &'static str,
    reconciliation_tree: &'static str,
    run_id: &'static str,
    seal_sha256: &'static str,
    historical_cargo_lock_sha256: &'static str,
    oracle_commit: &'static str,
    oracle_normalized_sha256: &'static str,
    verified_at_source_commit: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct SchemaEntry {
    path: String,
    sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct SchemaBundle {
    schema: &'static str,
    entries: Vec<SchemaEntry>,
    bundle_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandEvidence {
    id: String,
    argv: Vec<String>,
    cwd: String,
    exit_code: i32,
    status: &'static str,
    stdout: FileRef,
    stderr: FileRef,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct EvidenceSource {
    commit: String,
    tree: String,
    branch: String,
    dirty: bool,
    cargo_lock_sha256: String,
    identity_ref: FileRef,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct EvidenceP0Dependency {
    status: &'static str,
    baseline_commit: &'static str,
    receipt_commit: &'static str,
    receipt_sha256: &'static str,
    contract_sha256: &'static str,
    decision_ledger_sha256: &'static str,
    reconciliation_commit: &'static str,
    reconciliation_tree: &'static str,
    run_id: &'static str,
    seal_sha256: &'static str,
    historical_cargo_lock_sha256: &'static str,
    oracle_commit: &'static str,
    oracle_normalized_sha256: &'static str,
    reference: FileRef,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct Authority {
    machine_evidence: &'static str,
    phase_acceptance: &'static str,
    required_approvals: [&'static str; 2],
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct SealDescriptor {
    path: &'static str,
    entries: usize,
    coverage_rule: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    schema: &'static str,
    phase_id: &'static str,
    interface_id: &'static str,
    profile_id: &'static str,
    run_id: String,
    status: &'static str,
    generated_at_utc: String,
    source: EvidenceSource,
    p0_dependency: EvidenceP0Dependency,
    artifacts: Vec<FileRef>,
    commands: Vec<CommandEvidence>,
    authority: Authority,
    errors: Vec<Value>,
    seal: SealDescriptor,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    schema: String,
    phase_id: String,
    run_id: String,
    role: String,
    decision: String,
    owner_identity: String,
    review_reference: String,
    utc_timestamp: String,
    run_evidence_sha256: String,
    seal_sha256: String,
    explicit_dual_role_authority: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRef {
    role: String,
    decision: String,
    path: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Acceptance {
    schema: String,
    phase_id: String,
    interface_id: String,
    profile_id: String,
    sequence: u32,
    status: String,
    acceptance_kind: String,
    run_path: String,
    run_evidence_sha256: String,
    seal_path: String,
    seal_sha256: String,
    approvals: Vec<ApprovalRef>,
    approval_commit: String,
    preapproval_commit: String,
    todo_preapproval_sha256: String,
    previous_acceptance_sha256: Option<String>,
    created_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Pointer {
    schema: String,
    phase_id: String,
    interface_id: String,
    profile_id: String,
    acceptance_path: String,
    acceptance_sha256: String,
    updated_at: String,
}

#[derive(Clone)]
struct AmendmentInputs {
    source: SourceIdentity,
    p0_dependency: P0Dependency,
    schema_bundle: SchemaBundle,
    parser_boundary: Value,
    snapshots: Vec<(String, Vec<u8>)>,
}

struct EmittedRun {
    evidence_sha256: String,
    seal_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AttemptMetadata {
    schema: String,
    run_id: String,
    generated_at_utc: String,
    source_commit: String,
    source_tree: String,
    source_branch: String,
    cargo_lock_sha256: String,
    verifier_source_sha256: String,
    schema_bundle_sha256: String,
}

pub fn prepare(repository: &Path, supplied_root: &Path) -> Result<Value> {
    let output_root = publication::require_output_root(repository, supplied_root)?;
    publication::require_no_follow_tree(&output_root)?;
    let mut recorder = Recorder::default();
    let approved_p0 = p0::verify(repository, &mut recorder)?;
    let source = source_identity(repository, &mut recorder)?;
    require_clean_candidate(repository, &mut recorder)?;
    require_unchecked_p0a(repository)?;
    require_portable_policy(repository)?;

    let schema_bundle = build_schema_bundle(repository)?;
    let parser_boundary = build_parser_boundary(repository)?;
    require_zero_python_boundary(repository, &mut recorder)?;
    let source_artifact = SourceIdentity {
        schema: "python-slm-p0a-source-identity-v1",
        schema_bundle_sha256: schema_bundle.bundle_sha256.clone(),
        ..source.clone()
    };
    let p0_dependency = P0Dependency {
        schema: "python-slm-p0-dependency-v1",
        status: approved_p0.status,
        baseline_commit: p0::BASELINE_COMMIT,
        receipt_commit: p0::RECEIPT_COMMIT,
        receipt_sha256: p0::RECEIPT_SHA256,
        contract_sha256: p0::CONTRACT_SHA256,
        decision_ledger_sha256: p0::LEDGER_SHA256,
        reconciliation_commit: p0::RECONCILIATION_COMMIT,
        reconciliation_tree: p0::RECONCILIATION_TREE,
        run_id: p0::RUN_ID,
        seal_sha256: p0::SEAL_SHA256,
        historical_cargo_lock_sha256: p0::HISTORICAL_CARGO_LOCK_SHA256,
        oracle_commit: p0::ORACLE_COMMIT,
        oracle_normalized_sha256: p0::ORACLE_NORMALIZED_SHA256,
        verified_at_source_commit: source.commit.clone(),
    };
    let mut snapshots = Vec::new();
    for (source_path, destination) in SNAPSHOTS {
        snapshots.push((
            (*destination).to_owned(),
            fs::read(repository.join(source_path)).io_context(
                "AMENDMENT_ARTIFACT_READ_FAILED",
                format!("could not read {source_path}"),
            )?,
        ));
    }
    let inputs = AmendmentInputs {
        source: source_artifact,
        p0_dependency,
        schema_bundle,
        parser_boundary,
        snapshots,
    };

    let (generated_at_utc, prefix, entropy) = time::now();
    let suffix_material = format!("{}:{entropy}:{}", std::process::id(), source.commit);
    let suffix = &hash::bytes(suffix_material.as_bytes())[..24];
    let run_id = format!("{prefix}-{suffix}");
    publication::create_dir_all(&output_root)?;
    publication::create_dir_all(&output_root.join("runs"))?;
    publication::create_dir_all(&output_root.join(".staging"))?;
    recover_interrupted_stages(repository, &output_root)?;
    let stage_container = output_root
        .join(".staging")
        .join(format!("{run_id}.work-{entropy}"));
    publication::create_dir(&stage_container)?;
    let attempt = AttemptMetadata {
        schema: "python-slm-p0a-attempt-v1".to_owned(),
        run_id: run_id.clone(),
        generated_at_utc: generated_at_utc.clone(),
        source_commit: inputs.source.commit.clone(),
        source_tree: inputs.source.tree.clone(),
        source_branch: inputs.source.branch.clone(),
        cargo_lock_sha256: inputs.source.cargo_lock_sha256.clone(),
        verifier_source_sha256: inputs.source.verifier_source_sha256.clone(),
        schema_bundle_sha256: inputs.source.schema_bundle_sha256.clone(),
    };
    publication::write_json_new_via_owned_temp(
        &stage_container.join("attempt.json"),
        &attempt,
        "attempt",
    )?;
    let stage_run = stage_container.join(&run_id);
    let commands = recorder.commands().to_vec();
    let outcome = emit_run_stage(
        &stage_run,
        &run_id,
        &generated_at_utc,
        &inputs,
        &commands,
        "AWAITING_REVIEW",
        Vec::new(),
    )
    .and_then(|emitted| {
        validate_run(repository, &mut Recorder::default(), &stage_run)?;
        publish_staged_run(&stage_run, &output_root.join("runs").join(&run_id))?;
        Ok(emitted)
    });
    match outcome {
        Ok(emitted) => {
            // Publication is already durable. Any interrupted cleanup is recovered on the
            // next invocation and must not retroactively turn the sealed run into a failure.
            if fs::remove_file(stage_container.join("attempt.json")).is_ok() {
                let _ = remove_empty_stage_container(&stage_container);
            }
            Ok(json!({
                "schema": "python-slm-p0a-prepare-result-v1",
                "phase_id": "P0A",
                "status": "AWAITING_REVIEW",
                "run_path": format!("runs/{run_id}"),
                "run_evidence_sha256": emitted.evidence_sha256,
                "seal_sha256": emitted.seal_sha256,
                "source_commit": source.commit,
                "next_action": "commit the immutable run, obtain technical and data-governance approval JSONs, then run --finalize"
            }))
        }
        Err(original) => {
            close_failed_attempt(
                repository,
                &output_root,
                &stage_container,
                &run_id,
                &generated_at_utc,
                &inputs,
                &commands,
                &original,
                entropy,
            )?;
            Err(original)
        }
    }
}

fn emit_run_stage(
    run_root: &Path,
    run_id: &str,
    generated_at_utc: &str,
    inputs: &AmendmentInputs,
    recorded_commands: &[RecordedCommand],
    status: &'static str,
    errors: Vec<Value>,
) -> Result<EmittedRun> {
    publication::create_dir(run_root)?;
    publication::create_dir(&run_root.join("artifacts"))?;
    publication::create_dir(&run_root.join("commands"))?;
    for (destination, bytes) in &inputs.snapshots {
        publication::write_new(&run_root.join(destination), bytes)?;
    }
    publication::write_json_new(
        &run_root.join("artifacts/p0-dependency.json"),
        &inputs.p0_dependency,
    )?;
    publication::write_json_new(
        &run_root.join("artifacts/schema-bundle.json"),
        &inputs.schema_bundle,
    )?;
    publication::write_json_new(
        &run_root.join("artifacts/parser-boundary.json"),
        &inputs.parser_boundary,
    )?;
    publication::write_json_new(
        &run_root.join("artifacts/source-identity.json"),
        &inputs.source,
    )?;

    let commands = write_commands(run_root, recorded_commands)?;
    let mut artifacts = Vec::new();
    for (_, destination) in SNAPSHOTS {
        artifacts.push(publication::file_ref_from(run_root, destination)?);
    }
    for destination in [
        "artifacts/p0-dependency.json",
        "artifacts/schema-bundle.json",
        "artifacts/parser-boundary.json",
        "artifacts/source-identity.json",
    ] {
        artifacts.push(publication::file_ref_from(run_root, destination)?);
    }
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    if artifacts.len() != 11 {
        return Err(XtaskError::new(
            "P0A_ARTIFACT_COUNT_INVALID",
            crate::error::Category::Internal,
            format!(
                "expected 11 amendment artifacts, observed {}",
                artifacts.len()
            ),
            "Inspect the fixed P0A snapshot list.",
        ));
    }
    let identity_ref = publication::file_ref_from(run_root, "artifacts/source-identity.json")?;
    let dependency_ref = publication::file_ref_from(run_root, "artifacts/p0-dependency.json")?;
    let is_pass = status == "AWAITING_REVIEW";
    let evidence = Evidence {
        schema: "python-slm-p0a-phase-evidence-v1",
        phase_id: "P0A",
        interface_id: INTERFACE_ID,
        profile_id: PROFILE_ID,
        run_id: run_id.to_owned(),
        status,
        generated_at_utc: generated_at_utc.to_owned(),
        source: EvidenceSource {
            commit: inputs.source.commit.clone(),
            tree: inputs.source.tree.clone(),
            branch: inputs.source.branch.clone(),
            dirty: false,
            cargo_lock_sha256: inputs.source.cargo_lock_sha256.clone(),
            identity_ref,
        },
        p0_dependency: EvidenceP0Dependency {
            status: "PASS",
            baseline_commit: p0::BASELINE_COMMIT,
            receipt_commit: p0::RECEIPT_COMMIT,
            receipt_sha256: p0::RECEIPT_SHA256,
            contract_sha256: p0::CONTRACT_SHA256,
            decision_ledger_sha256: p0::LEDGER_SHA256,
            reconciliation_commit: p0::RECONCILIATION_COMMIT,
            reconciliation_tree: p0::RECONCILIATION_TREE,
            run_id: p0::RUN_ID,
            seal_sha256: p0::SEAL_SHA256,
            historical_cargo_lock_sha256: p0::HISTORICAL_CARGO_LOCK_SHA256,
            oracle_commit: p0::ORACLE_COMMIT,
            oracle_normalized_sha256: p0::ORACLE_NORMALIZED_SHA256,
            reference: dependency_ref,
        },
        artifacts,
        commands,
        authority: Authority {
            machine_evidence: if is_pass { "PASS" } else { "FAIL" },
            phase_acceptance: "PENDING",
            required_approvals: ["technical", "data_governance"],
        },
        errors,
        seal: SealDescriptor {
            path: "SHA256SUMS",
            entries: 12 + 2 * recorder_command_count(run_root)?,
            coverage_rule: "all_run_files_except_seal",
        },
    };
    publication::write_json_new(&run_root.join("evidence.json"), &evidence)?;
    require_receipt_redaction(run_root)?;
    let (entry_count, seal_sha256) = publication::seal(run_root)?;
    if entry_count != evidence.seal.entries {
        return Err(XtaskError::new(
            "P0A_SEAL_COUNT_INTERNAL_MISMATCH",
            crate::error::Category::Internal,
            format!(
                "expected {} seal entries, observed {entry_count}",
                evidence.seal.entries
            ),
            "Inspect P0A receipt emission.",
        ));
    }
    publication::verify_seal(run_root, &seal_sha256)?;
    Ok(EmittedRun {
        evidence_sha256: hash::file(&run_root.join("evidence.json"))?,
        seal_sha256,
    })
}

fn recover_interrupted_stages(repository: &Path, output_root: &Path) -> Result<()> {
    let staging_root = output_root.join(".staging");
    if !staging_root.exists() {
        return Ok(());
    }
    let mut groups: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut empty_preidentity_work = Vec::new();
    for entry in fs::read_dir(&staging_root).io_context(
        "P0A_STAGING_RECOVERY_FAILED",
        "could not enumerate P0A staging attempts",
    )? {
        let entry = entry.io_context(
            "P0A_STAGING_RECOVERY_FAILED",
            "could not read P0A staging entry",
        )?;
        if !entry
            .file_type()
            .io_context(
                "P0A_STAGING_RECOVERY_FAILED",
                "could not inspect P0A staging entry",
            )?
            .is_dir()
        {
            return Err(XtaskError::integrity(
                "P0A_STAGING_RECOVERY_INVALID",
                "P0A staging contains a non-directory entry",
            ));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((run_id, kind)) = parse_stage_container_name(&name) else {
            return Err(XtaskError::integrity(
                "P0A_STAGING_RECOVERY_INVALID",
                format!("unrecognized P0A staging directory {name}"),
            ));
        };
        if kind == StageContainerKind::Work && !entry.path().join("attempt.json").exists() {
            recover_preidentity_work_container(&entry.path())?;
            // create_dir succeeds before attempt.json is durably created. That precise,
            // empty or owned-partial create-new state has no receipt identity and is safe
            // to remove. Any other content is retained and rejected.
            empty_preidentity_work.push(entry.path());
            continue;
        }
        groups
            .entry(run_id.to_owned())
            .or_default()
            .push(entry.path());
    }
    for container in empty_preidentity_work {
        remove_empty_stage_container(&container)?;
    }
    for (run_id, mut containers) in groups {
        containers.sort();
        let final_run = output_root.join("runs").join(&run_id);
        if final_run.exists() {
            let seal = hash::file(&final_run.join("SHA256SUMS"))?;
            publication::verify_seal(&final_run, &seal)?;
            for container in containers {
                remove_owned_stage_container(output_root, &container)?;
            }
            continue;
        }
        let attempts: Vec<(PathBuf, AttemptMetadata)> = containers
            .iter()
            .filter_map(|container| {
                let path = container.join("attempt.json");
                path.is_file().then(|| {
                    read_json_closed(&path, "P0A_ATTEMPT_METADATA_INVALID")
                        .map(|attempt| (container.clone(), attempt))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if attempts.len() != 1 {
            return Err(XtaskError::integrity(
                "P0A_STAGING_RECOVERY_INVALID",
                format!("interrupted run {run_id} must have exactly one durable attempt identity"),
            ));
        }
        let attempt = &attempts[0].1;
        if attempt.schema != "python-slm-p0a-attempt-v1"
            || attempt.run_id != run_id
            || !valid_utc_timestamp(&attempt.generated_at_utc)
        {
            return Err(XtaskError::integrity(
                "P0A_ATTEMPT_METADATA_INVALID",
                "interrupted attempt metadata is malformed",
            ));
        }
        let mut recorder = Recorder::default();
        p0::verify(repository, &mut recorder)?;
        let inputs = rebuild_inputs_from_attempt(repository, &mut recorder, attempt)?;
        let (_, _, entropy) = time::now();
        let recovery_container = staging_root.join(format!("{run_id}.recovery-{entropy}"));
        publication::create_dir(&recovery_container)?;
        let recovery_run = recovery_container.join(&run_id);
        let errors = vec![json!({
            "code": "P0A_ATTEMPT_INTERRUPTED",
            "category": 4,
            "message": "the prior receipt-bearing P0A attempt ended before atomic publication",
            "remediation": "Retain this sealed FAIL run and retry from the latest clean committed candidate."
        })];
        let emitted = emit_run_stage(
            &recovery_run,
            &run_id,
            &attempt.generated_at_utc,
            &inputs,
            recorder.commands(),
            "FAIL",
            errors,
        )?;
        validate_failed_run(&recovery_run, &emitted)?;
        publish_staged_run(&recovery_run, &final_run)?;
        remove_empty_stage_container(&recovery_container)?;
        for container in containers {
            remove_owned_stage_container(output_root, &container)?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StageContainerKind {
    Work,
    Fail,
    Recovery,
}

fn parse_stage_container_name(name: &str) -> Option<(&str, StageContainerKind)> {
    let run_id = name.get(..44).filter(|value| valid_run_id(value))?;
    let suffix = name.get(44..)?;
    for (prefix, kind) in [
        (".work-", StageContainerKind::Work),
        (".fail-", StageContainerKind::Fail),
        (".recovery-", StageContainerKind::Recovery),
    ] {
        if let Some(entropy) = suffix.strip_prefix(prefix)
            && !entropy.is_empty()
            && entropy.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Some((run_id, kind));
        }
    }
    None
}

fn recover_preidentity_work_container(path: &Path) -> Result<()> {
    let mut entries = fs::read_dir(path).io_context(
        "P0A_STAGING_RECOVERY_FAILED",
        "could not enumerate preidentity work container",
    )?;
    let Some(entry) = entries.next() else {
        return Ok(());
    };
    let entry = entry.io_context(
        "P0A_STAGING_RECOVERY_FAILED",
        "could not read preidentity work entry",
    )?;
    if entries.next().is_some() {
        return Err(XtaskError::integrity(
            "P0A_STAGING_RECOVERY_INVALID",
            "preidentity work container has ambiguous content",
        ));
    }
    let name = entry.file_name().to_string_lossy().into_owned();
    let metadata = fs::symlink_metadata(entry.path()).io_context(
        "P0A_STAGING_RECOVERY_FAILED",
        "could not inspect preidentity attempt temporary",
    )?;
    if !valid_owned_temp_name(&name, "attempt.json.tmp-attempt-")
        || metadata.file_type().is_symlink()
        || !metadata.is_file()
    {
        return Err(XtaskError::integrity(
            "P0A_STAGING_RECOVERY_INVALID",
            "preidentity work container does not contain only an owned attempt temporary",
        ));
    }
    fs::remove_file(entry.path()).io_context(
        "P0A_STAGING_RECOVERY_FAILED",
        "could not remove partial preidentity attempt temporary",
    )
}

fn valid_owned_temp_name(name: &str, prefix: &str) -> bool {
    let Some(suffix) = name.strip_prefix(prefix) else {
        return false;
    };
    let mut parts = suffix.split('-');
    parts
        .next()
        .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts.next().is_none()
}

fn rebuild_inputs_from_attempt(
    repository: &Path,
    recorder: &mut Recorder,
    attempt: &AttemptMetadata,
) -> Result<AmendmentInputs> {
    if !valid_git_sha(&attempt.source_commit)
        || !valid_git_sha(&attempt.source_tree)
        || !hash::is_lower_sha256(&attempt.cargo_lock_sha256)
        || !hash::is_lower_sha256(&attempt.verifier_source_sha256)
        || !hash::is_lower_sha256(&attempt.schema_bundle_sha256)
        || attempt.source_branch.is_empty()
    {
        return Err(XtaskError::integrity(
            "P0A_ATTEMPT_METADATA_INVALID",
            "interrupted attempt source identity is malformed",
        ));
    }
    recorder.git_success(
        repository,
        &[
            "merge-base",
            "--is-ancestor",
            &attempt.source_commit,
            "HEAD",
        ],
        "P0A_ATTEMPT_SOURCE_NOT_ANCESTOR",
    )?;
    let tree = git_line(
        recorder,
        repository,
        &["rev-parse", &format!("{}^{{tree}}", attempt.source_commit)],
        "P0A_ATTEMPT_SOURCE_TREE_INVALID",
    )?;
    if tree != attempt.source_tree {
        return Err(XtaskError::integrity(
            "P0A_ATTEMPT_SOURCE_TREE_INVALID",
            "interrupted attempt tree does not match its commit",
        ));
    }
    let cargo_lock = git_blob(recorder, repository, &attempt.source_commit, "Cargo.lock")?;
    if hash::bytes(&cargo_lock) != attempt.cargo_lock_sha256 {
        return Err(XtaskError::integrity(
            "P0A_ATTEMPT_CARGO_LOCK_INVALID",
            "interrupted attempt Cargo.lock hash does not match its source commit",
        ));
    }
    require_locked_parser_packages(&cargo_lock)?;
    let schema_bundle = schema_bundle_from_commit(repository, recorder, &attempt.source_commit)?;
    if schema_bundle.bundle_sha256 != attempt.schema_bundle_sha256 {
        return Err(XtaskError::integrity(
            "P0A_ATTEMPT_SCHEMA_BUNDLE_INVALID",
            "interrupted attempt schema bundle hash does not rederive",
        ));
    }
    let verifier_source_sha256 =
        verifier_bundle_hash_from_commit(repository, recorder, &attempt.source_commit)?;
    if verifier_source_sha256 != attempt.verifier_source_sha256 {
        return Err(XtaskError::integrity(
            "P0A_ATTEMPT_VERIFIER_INVALID",
            "interrupted attempt verifier hash does not rederive",
        ));
    }
    let compatibility = git_blob(
        recorder,
        repository,
        &attempt.source_commit,
        "docs/schemas/portable-v2/tree-sitter-python-compatibility-v1.json",
    )?;
    let parser_boundary = expected_parser_boundary(hash::bytes(&compatibility));
    let snapshots = SNAPSHOTS
        .iter()
        .map(|(source_path, destination)| {
            git_blob(recorder, repository, &attempt.source_commit, source_path)
                .map(|bytes| ((*destination).to_owned(), bytes))
        })
        .collect::<Result<Vec<_>>>()?;
    let source = SourceIdentity {
        schema: "python-slm-p0a-source-identity-v1",
        commit: attempt.source_commit.clone(),
        tree: attempt.source_tree.clone(),
        branch: attempt.source_branch.clone(),
        dirty: false,
        cargo_lock_sha256: attempt.cargo_lock_sha256.clone(),
        verifier_source_sha256,
        schema_bundle_sha256: schema_bundle.bundle_sha256.clone(),
    };
    let p0_dependency = P0Dependency {
        schema: "python-slm-p0-dependency-v1",
        status: "PASS",
        baseline_commit: p0::BASELINE_COMMIT,
        receipt_commit: p0::RECEIPT_COMMIT,
        receipt_sha256: p0::RECEIPT_SHA256,
        contract_sha256: p0::CONTRACT_SHA256,
        decision_ledger_sha256: p0::LEDGER_SHA256,
        reconciliation_commit: p0::RECONCILIATION_COMMIT,
        reconciliation_tree: p0::RECONCILIATION_TREE,
        run_id: p0::RUN_ID,
        seal_sha256: p0::SEAL_SHA256,
        historical_cargo_lock_sha256: p0::HISTORICAL_CARGO_LOCK_SHA256,
        oracle_commit: p0::ORACLE_COMMIT,
        oracle_normalized_sha256: p0::ORACLE_NORMALIZED_SHA256,
        verified_at_source_commit: attempt.source_commit.clone(),
    };
    Ok(AmendmentInputs {
        source,
        p0_dependency,
        schema_bundle,
        parser_boundary,
        snapshots,
    })
}

fn schema_bundle_from_commit(
    repository: &Path,
    recorder: &mut Recorder,
    commit: &str,
) -> Result<SchemaBundle> {
    let mut entries = Vec::new();
    let mut manifest = String::new();
    for path in canonical_schema_paths() {
        let bytes = git_blob(recorder, repository, commit, path)?;
        serde_json::from_slice::<Value>(&bytes).map_err(|error| {
            XtaskError::integrity(
                "P0A_ATTEMPT_SCHEMA_JSON_INVALID",
                format!("{commit}:{path} is not valid JSON: {error}"),
            )
        })?;
        let sha256 = hash::bytes(&bytes);
        manifest.push_str(&sha256);
        manifest.push_str("  ");
        manifest.push_str(path);
        manifest.push('\n');
        entries.push(SchemaEntry {
            path: path.to_owned(),
            sha256,
        });
    }
    Ok(SchemaBundle {
        schema: "python-slm-p0a-schema-bundle-v1",
        entries,
        bundle_sha256: hash::bytes(manifest.as_bytes()),
    })
}

#[allow(clippy::too_many_arguments)]
fn close_failed_attempt(
    _repository: &Path,
    output_root: &Path,
    abandoned_stage: &Path,
    run_id: &str,
    generated_at_utc: &str,
    inputs: &AmendmentInputs,
    commands: &[RecordedCommand],
    original: &XtaskError,
    entropy: u128,
) -> Result<()> {
    let failure_container = output_root
        .join(".staging")
        .join(format!("{run_id}.fail-{entropy}"));
    publication::create_dir(&failure_container).map_err(|error| {
        XtaskError::new(
            "P0A_FAIL_RECEIPT_CLOSE_FAILED",
            crate::error::Category::Environment,
            format!(
                "qualification failed with {}; could not create FAIL staging: {}",
                original.code, error.message
            ),
            "Preserve the owned .staging attempt and rerun P0A recovery after restoring storage.",
        )
    })?;
    let failure_run = failure_container.join(run_id);
    let errors = vec![safe_failure_error(original)];
    let emitted = emit_run_stage(
        &failure_run,
        run_id,
        generated_at_utc,
        inputs,
        commands,
        "FAIL",
        errors,
    )
    .map_err(|error| {
        XtaskError::new(
            "P0A_FAIL_RECEIPT_CLOSE_FAILED",
            crate::error::Category::Environment,
            format!(
                "qualification failed with {}; FAIL sealing also failed with {}: {}",
                original.code, error.code, error.message
            ),
            "Preserve the owned .staging attempt and rerun P0A recovery after restoring storage.",
        )
    })?;
    validate_failed_run(&failure_run, &emitted)?;
    publish_staged_run(&failure_run, &output_root.join("runs").join(run_id))?;
    remove_empty_stage_container(&failure_container)?;
    remove_owned_stage_container(output_root, abandoned_stage)?;
    Ok(())
}

fn safe_failure_error(original: &XtaskError) -> Value {
    // Never serialize raw error prose into a receipt. I/O diagnostics routinely contain
    // absolute paths, host names, or environment-derived values. The stable code and
    // category retain the machine-actionable identity; detailed command diagnostics are
    // already independently redacted before transcript publication.
    let code = original
        .code
        .bytes()
        .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
        .then_some(original.code.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("P0A_INTERNAL_FAILURE");
    json!({
        "code": code,
        "category": original.exit_code(),
        "message": format!("P0A qualification terminated with error code {code}."),
        "remediation": "Inspect the sanitized command transcripts, retain this sealed FAIL run, correct the bounded failure, and retry from a new clean committed candidate."
    })
}

fn validate_failed_run(run_root: &Path, emitted: &EmittedRun) -> Result<()> {
    publication::verify_seal(run_root, &emitted.seal_sha256)?;
    require_receipt_redaction(run_root)?;
    let evidence: Value = read_json_closed(
        &run_root.join("evidence.json"),
        "P0A_FAILURE_EVIDENCE_JSON_INVALID",
    )?;
    require_object_keys(
        &evidence,
        &[
            "schema",
            "phase_id",
            "interface_id",
            "profile_id",
            "run_id",
            "status",
            "generated_at_utc",
            "source",
            "p0_dependency",
            "artifacts",
            "commands",
            "authority",
            "errors",
            "seal",
        ],
        "P0A_FAILURE_EVIDENCE_FIELDS_INVALID",
    )?;
    validate_evidence_shape(run_root, &evidence)?;
    validate_p0_evidence_constants(&evidence)?;
    let seal_entries = fs::read_to_string(run_root.join("SHA256SUMS"))
        .io_context("P0A_SEAL_READ_FAILED", "could not read P0A failure seal")?
        .lines()
        .count() as u64;
    if evidence["status"] != json!("FAIL")
        || evidence["authority"]["machine_evidence"] != json!("FAIL")
        || evidence["authority"]["phase_acceptance"] != json!("PENDING")
        || evidence["errors"]
            .as_array()
            .is_none_or(|errors| errors.is_empty())
        || hash::file(&run_root.join("evidence.json"))? != emitted.evidence_sha256
        || evidence["seal"]["entries"].as_u64() != Some(seal_entries)
    {
        return Err(XtaskError::integrity(
            "P0A_FAILURE_RECEIPT_INVALID",
            "sealed failure run does not preserve the terminal error and blocked authority state",
        ));
    }
    for error in evidence["errors"].as_array().expect("checked nonempty") {
        require_object_keys(
            error,
            &["code", "category", "message", "remediation"],
            "P0A_FAILURE_ERROR_INVALID",
        )?;
        let code = error["code"].as_str().unwrap_or_default();
        let category = error["category"].as_u64().unwrap_or_default();
        if code.is_empty()
            || !code
                .bytes()
                .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
            || !(1..=5).contains(&category)
            || error["message"].as_str().is_none_or(str::is_empty)
            || error["remediation"].as_str().is_none_or(str::is_empty)
        {
            return Err(XtaskError::integrity(
                "P0A_FAILURE_ERROR_INVALID",
                "P0A failure error is malformed",
            ));
        }
    }
    validate_all_references(run_root, &evidence)?;
    Ok(())
}

fn publish_staged_run(stage_run: &Path, final_run: &Path) -> Result<()> {
    if final_run.exists() {
        return Err(XtaskError::integrity(
            "P0A_RUN_ALREADY_EXISTS",
            format!("immutable run already exists: {}", final_run.display()),
        ));
    }
    match fs::rename(stage_run, final_run) {
        Ok(()) => Ok(()),
        Err(_) if !stage_run.exists() && final_run.is_dir() => {
            let seal = hash::file(&final_run.join("SHA256SUMS"))?;
            publication::verify_seal(final_run, &seal)
        }
        Err(error) => Err(XtaskError::environment(
            "P0A_RUN_PUBLICATION_FAILED",
            format!(
                "could not publish create-new run {}: {error}",
                final_run.display()
            ),
        )),
    }
}

fn remove_empty_stage_container(container: &Path) -> Result<()> {
    fs::remove_dir(container).io_context(
        "P0A_STAGING_CLEANUP_FAILED",
        format!(
            "could not remove empty staging directory {}",
            container.display()
        ),
    )
}

fn remove_owned_stage_container(output_root: &Path, container: &Path) -> Result<()> {
    let staging_root = output_root.join(".staging");
    if container.parent() != Some(staging_root.as_path()) || container == staging_root {
        return Err(XtaskError::integrity(
            "P0A_STAGING_OWNERSHIP_INVALID",
            "refused cleanup outside the exact P0A staging directory",
        ));
    }
    let metadata = fs::symlink_metadata(container).io_context(
        "P0A_STAGING_CLEANUP_FAILED",
        format!(
            "could not inspect staging directory {}",
            container.display()
        ),
    )?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(XtaskError::integrity(
            "P0A_STAGING_OWNERSHIP_INVALID",
            "refused cleanup of a non-directory or symbolic-link staging target",
        ));
    }
    fs::remove_dir_all(container).io_context(
        "P0A_STAGING_CLEANUP_FAILED",
        format!(
            "could not remove owned staging directory {}",
            container.display()
        ),
    )
}

fn source_identity(repository: &Path, recorder: &mut Recorder) -> Result<SourceIdentity> {
    let commit = git_line(
        recorder,
        repository,
        &["rev-parse", "HEAD"],
        "SOURCE_COMMIT_INVALID",
    )?;
    let tree = git_line(
        recorder,
        repository,
        &["rev-parse", "HEAD^{tree}"],
        "SOURCE_TREE_INVALID",
    )?;
    let branch = git_line(
        recorder,
        repository,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        "SOURCE_BRANCH_INVALID",
    )?;
    let cargo_lock_sha256 = hash::file(&repository.join("Cargo.lock"))?;
    let verifier_source_sha256 = verifier_bundle_hash(repository)?;
    Ok(SourceIdentity {
        schema: "python-slm-p0a-source-identity-v1",
        commit,
        tree,
        branch,
        dirty: false,
        cargo_lock_sha256,
        verifier_source_sha256,
        schema_bundle_sha256: String::new(),
    })
}

fn require_clean_candidate(repository: &Path, recorder: &mut Recorder) -> Result<()> {
    let status = recorder.git_text(
        repository,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
            ":(exclude)docs/receipts/P0A",
        ],
        "P0A_SOURCE_STATUS_FAILED",
    )?;
    if !status.trim().is_empty() {
        return Err(XtaskError::integrity(
            "P0A_SOURCE_DIRTY",
            "the amendment candidate must be committed and clean outside docs/receipts/P0A",
        ));
    }
    Ok(())
}

fn require_unchecked_p0a(repository: &Path) -> Result<()> {
    let bytes = fs::read(repository.join("TODO.md"))
        .io_context("P0A_TODO_READ_FAILED", "could not read TODO.md")?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| XtaskError::integrity("P0A_TODO_ENCODING_INVALID", "TODO.md is not UTF-8"))?;
    if text.contains('\r')
        || text.matches("- [ ] P0A complete").count() != 1
        || text.contains("- [x] P0A complete")
    {
        return Err(XtaskError::integrity(
            "P0A_PREAPPROVAL_TODO_INVALID",
            "the amendment candidate must contain exactly one unchecked P0A checkbox in canonical LF UTF-8",
        ));
    }
    Ok(())
}

fn require_portable_policy(repository: &Path) -> Result<()> {
    let profile: Value = read_json_closed(
        &repository.join("docs/schemas/portable-v2/python-slm-prototype-profile-v1.schema.json"),
        "PROTOTYPE_PROFILE_SCHEMA_INVALID",
    )?;
    let memory: Value = read_json_closed(
        &repository
            .join("docs/schemas/portable-v2/python-slm-training-memory-policy-v1.schema.json"),
        "TRAINING_MEMORY_SCHEMA_INVALID",
    )?;
    let sla: Value = read_json_closed(
        &repository.join("docs/schemas/portable-v2/python-slm-prototype-sla-v1.schema.json"),
        "PROTOTYPE_SLA_SCHEMA_INVALID",
    )?;
    for (value, pointer, expected) in [
        (&profile, "/properties/profile_id/const", json!(PROFILE_ID)),
        (
            &profile,
            "/properties/interface_id/const",
            json!(INTERFACE_ID),
        ),
        (
            &profile,
            "/properties/support_tier/enum",
            json!([
                "designed",
                "implemented",
                "tuple_qualified",
                "full_run_qualified"
            ]),
        ),
        (
            &profile,
            "/properties/host/properties/os_family/const",
            json!("windows"),
        ),
        (
            &profile,
            "/properties/host/properties/architecture/const",
            json!("x86_64"),
        ),
        (
            &profile,
            "/properties/host/properties/rust_target/const",
            json!("x86_64-pc-windows-msvc"),
        ),
        (
            &profile,
            "/properties/host/properties/cpu_model/const",
            json!("AMD Ryzen 9 9950X3D 16-Core Processor"),
        ),
        (
            &profile,
            "/properties/host/properties/c_cpp_toolchain/const",
            json!("msvc"),
        ),
        (
            &profile,
            "/properties/accelerator/properties/provider/const",
            json!("cuda"),
        ),
        (
            &profile,
            "/properties/accelerator/properties/vendor/const",
            json!("nvidia"),
        ),
        (
            &profile,
            "/properties/accelerator/properties/device_model/const",
            json!("NVIDIA GeForce RTX 5090"),
        ),
        (
            &profile,
            "/properties/accelerator/properties/compute_capability/const",
            json!("12.0"),
        ),
        (
            &profile,
            "/properties/accelerator/properties/architecture/const",
            json!("sm_120"),
        ),
        (
            &profile,
            "/properties/accelerator/properties/memory_model/const",
            json!("dedicated"),
        ),
        (
            &profile,
            "/properties/model/properties/preset/const",
            json!("gqa-135m-v1"),
        ),
        (
            &profile,
            "/properties/model/properties/parameter_count/const",
            json!(135_285_504_u64),
        ),
        (
            &profile,
            "/properties/model/properties/valid_training_targets/const",
            json!(2_000_000_000_u64),
        ),
        (
            &memory,
            "/properties/minimum_accelerator_bytes/const",
            json!(2_952_790_016_u64),
        ),
        (
            &memory,
            "/properties/bytes_per_parameter/const",
            json!(20_u64),
        ),
        (
            &memory,
            "/properties/unaligned_bytes/const",
            json!(2_705_710_080_u64),
        ),
        (
            &memory,
            "/properties/alignment_bytes/const",
            json!(268_435_456_u64),
        ),
        (&sla, "/properties/sla_seconds/const", json!(28_800_u64)),
        (
            &sla,
            "/properties/admission_seconds/const",
            json!(25_920_u64),
        ),
        (
            &sla,
            "/properties/actual_elapsed_limit_ns/const",
            json!(28_800_000_000_000_u64),
        ),
        (
            &sla,
            "/properties/fresh_samples_per_overhead_class/const",
            json!(5_u64),
        ),
        (
            &sla,
            "/properties/overhead_admissibility_predicate/const",
            json!("O_bound<25920"),
        ),
        (
            &sla,
            "/properties/required_rate_formula/const",
            json!("2000000000/(25920-O_bound)"),
        ),
        (
            &sla,
            "/properties/rate_admissibility_predicate/const",
            json!("R_qual>=R_required"),
        ),
        (
            &sla,
            "/properties/admission_formula/const",
            json!("ceil(2000000000/R_qual+O_bound)<=25920"),
        ),
        (
            &sla,
            "/properties/clock_start/const",
            json!("immediately_before_frozen_artifact_verification_and_open"),
        ),
        (
            &sla,
            "/properties/clock_stop/const",
            json!("after_final_checkpoint_is_durable"),
        ),
        (
            &sla,
            "/properties/recovery_downtime_counts/const",
            json!(true),
        ),
        (
            &sla,
            "/properties/suspend_inclusive_clock/const",
            json!(
                "monotonic_wall_clock_includes_host_suspend_system_sleep_and_resumed_execution_downtime"
            ),
        ),
    ] {
        if value.pointer(pointer) != Some(&expected) {
            return Err(XtaskError::integrity(
                "PORTABLE_POLICY_CONSTANT_MISMATCH",
                format!("portable policy constant {pointer} is not {expected}"),
            ));
        }
    }
    if profile.pointer("/properties/deferred_profiles/const")
        != Some(&json!([
            { "capability": "linux-host", "status": "DEFERRED_POST_P16", "earliest_phase": "P17" },
            { "capability": "macos-host", "status": "DEFERRED_POST_P16", "earliest_phase": "P17" },
            { "capability": "gcc-clang-host-toolchains", "status": "DEFERRED_POST_P16", "earliest_phase": "P17" },
            { "capability": "cuda-on-linux", "status": "DEFERRED_POST_P16", "earliest_phase": "P18" },
            { "capability": "rocm-hip-amd", "status": "DEFERRED_POST_P16", "earliest_phase": "P18" },
            { "capability": "metal-apple-silicon", "status": "DEFERRED_POST_P16", "earliest_phase": "P18" }
        ]))
    {
        return Err(XtaskError::integrity(
            "PORTABLE_DEFERRED_MATRIX_MISMATCH",
            "deferred OS/provider matrix does not equal the frozen post-P16 plan",
        ));
    }
    if minimum_accelerator_bytes(135_285_504) != 2_952_790_016 {
        return Err(XtaskError::new(
            "TRAINING_MEMORY_ARITHMETIC_INTERNAL_ERROR",
            crate::error::Category::Internal,
            "20P alignment arithmetic does not reproduce the frozen memory gate",
            "Correct the integer arithmetic before preparing evidence.",
        ));
    }
    Ok(())
}

fn minimum_accelerator_bytes(parameter_count: u64) -> u64 {
    let unaligned = parameter_count
        .checked_mul(20)
        .expect("frozen count fits u64");
    let alignment = 256_u64 * 1024 * 1024;
    unaligned.div_ceil(alignment) * alignment
}

#[cfg(test)]
fn required_rate(overhead_seconds: f64) -> Option<f64> {
    (overhead_seconds.is_finite() && (0.0..25_920.0).contains(&overhead_seconds))
        .then(|| 2_000_000_000_f64 / (25_920_f64 - overhead_seconds))
}

#[cfg(test)]
fn actual_within_sla(elapsed_ns: u64) -> bool {
    elapsed_ns <= 28_800_000_000_000
}

fn git_line(
    recorder: &mut Recorder,
    repository: &Path,
    args: &[&str],
    code: &'static str,
) -> Result<String> {
    let text = recorder.git_text(repository, args, code)?;
    let line = text.trim_end_matches(['\r', '\n']);
    if line.is_empty() || line.contains(['\r', '\n']) {
        return Err(XtaskError::integrity(
            code,
            "Git identity output was empty or multiline",
        ));
    }
    Ok(line.to_owned())
}

fn verifier_bundle_hash(repository: &Path) -> Result<String> {
    let mut paths = vec!["xtask/Cargo.toml".to_owned()];
    for entry in fs::read_dir(repository.join("xtask/src")).io_context(
        "VERIFIER_SOURCE_ENUMERATION_FAILED",
        "could not enumerate xtask/src",
    )? {
        let entry = entry.io_context(
            "VERIFIER_SOURCE_ENUMERATION_FAILED",
            "could not read xtask source entry",
        )?;
        if entry
            .file_type()
            .io_context(
                "VERIFIER_SOURCE_ENUMERATION_FAILED",
                "could not inspect xtask source",
            )?
            .is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("rs")
        {
            paths.push(format!("xtask/src/{}", entry.file_name().to_string_lossy()));
        }
    }
    paths.sort();
    let mut manifest = String::new();
    for path in paths {
        manifest.push_str(&hash::file(&repository.join(&path))?);
        manifest.push_str("  ");
        manifest.push_str(&path);
        manifest.push('\n');
    }
    Ok(hash::bytes(manifest.as_bytes()))
}

fn build_schema_bundle(repository: &Path) -> Result<SchemaBundle> {
    let mut paths = Vec::new();
    for directory in ["docs/schemas/P0A", "docs/schemas/portable-v2"] {
        for entry in fs::read_dir(repository.join(directory)).io_context(
            "P0A_SCHEMA_ENUMERATION_FAILED",
            format!("could not enumerate {directory}"),
        )? {
            let entry = entry.io_context(
                "P0A_SCHEMA_ENUMERATION_FAILED",
                "could not read schema entry",
            )?;
            let kind = entry.file_type().io_context(
                "P0A_SCHEMA_ENUMERATION_FAILED",
                "could not inspect schema entry",
            )?;
            if !kind.is_file() {
                return Err(XtaskError::integrity(
                    "P0A_SCHEMA_TREE_INVALID",
                    format!(
                        "schema directory contains a non-regular entry: {}",
                        entry.path().display()
                    ),
                ));
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".schema.json") {
                paths.push(format!("{directory}/{name}"));
            } else if directory.ends_with("portable-v2")
                && name == "tree-sitter-python-compatibility-v1.json"
            {
                // The compatibility corpus is parser evidence, not a JSON Schema.
                let bytes = fs::read(entry.path()).io_context(
                    "PARSER_COMPATIBILITY_READ_FAILED",
                    "could not read the parser compatibility corpus",
                )?;
                serde_json::from_slice::<Value>(&bytes).map_err(|error| {
                    XtaskError::integrity(
                        "PARSER_COMPATIBILITY_JSON_INVALID",
                        format!("parser compatibility corpus is not valid JSON: {error}"),
                    )
                })?;
                paths.push(format!("{directory}/{name}"));
            } else {
                return Err(XtaskError::integrity(
                    "P0A_SCHEMA_TREE_INVALID",
                    format!("unexpected file in schema directory: {directory}/{name}"),
                ));
            }
        }
    }
    paths.sort();
    let expected_paths: BTreeSet<&str> = SCHEMA_PATHS.iter().copied().collect();
    let observed_paths: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
    if observed_paths != expected_paths || paths.len() != SCHEMA_PATHS.len() {
        return Err(XtaskError::integrity(
            "P0A_SCHEMA_BUNDLE_PATHS_INVALID",
            "schema tree does not equal the frozen 12-file P0A/portable bundle",
        ));
    }
    let mut entries = Vec::new();
    let mut manifest = String::new();
    for path in paths {
        let bytes = fs::read(repository.join(&path))
            .io_context("P0A_SCHEMA_READ_FAILED", format!("could not read {path}"))?;
        serde_json::from_slice::<Value>(&bytes).map_err(|error| {
            XtaskError::integrity(
                "P0A_SCHEMA_JSON_INVALID",
                format!("{path} is not valid JSON: {error}"),
            )
        })?;
        let digest = hash::bytes(&bytes);
        manifest.push_str(&digest);
        manifest.push_str("  ");
        manifest.push_str(&path);
        manifest.push('\n');
        entries.push(SchemaEntry {
            path,
            sha256: digest,
        });
    }
    if entries.len() != 12 {
        return Err(XtaskError::integrity(
            "P0A_SCHEMA_BUNDLE_COUNT_INVALID",
            format!(
                "expected 12 closed P0A/portable schema and compatibility files, observed {}",
                entries.len()
            ),
        ));
    }
    Ok(SchemaBundle {
        schema: "python-slm-p0a-schema-bundle-v1",
        entries,
        bundle_sha256: hash::bytes(manifest.as_bytes()),
    })
}

fn canonical_schema_paths() -> Vec<&'static str> {
    let mut paths = SCHEMA_PATHS.to_vec();
    paths.sort_unstable();
    paths
}

fn build_parser_boundary(repository: &Path) -> Result<Value> {
    let cargo_lock = fs::read(repository.join("Cargo.lock")).io_context(
        "PARSER_LOCK_READ_FAILED",
        "could not read Cargo.lock for the parser dependency boundary",
    )?;
    require_locked_parser_packages(&cargo_lock)?;
    require_registry_archive(
        "tree-sitter-0.25.8.crate",
        "6d7b8994f367f16e6fa14b5aebbcb350de5d7cbea82dc5b00ae997dd71680dd2",
    )?;
    require_registry_archive(
        "tree-sitter-python-0.25.0.crate",
        "6bf85fd39652e740bf60f46f4cda9492c3a9ad75880575bf14960f775cb74a1c",
    )?;
    let registry_root = cargo_registry_source_root()?;
    let package = find_registry_package(&registry_root, "tree-sitter-python-0.25.0")?;
    let runtime_package = find_registry_package(&registry_root, "tree-sitter-0.25.8")?;
    for (relative, expected) in GENERATED_SOURCES {
        let path = package.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        hash::require_file(&path, expected, "PARSER_GENERATED_SOURCE_MISMATCH")?;
    }
    let compatibility_path = "docs/schemas/portable-v2/tree-sitter-python-compatibility-v1.json";
    let compatibility_sha256 = hash::file(&repository.join(compatibility_path))?;
    let runtime_sources = [
        (
            "binding_rust/build.rs",
            "6c1a090f4ac12621effce41a9bac54182b155f252a84b3bbb51cbcd58330593d",
        ),
        (
            "include/tree_sitter/api.h",
            "937c5389713f99318ac7adbdb3a95e9976bbd211e35e37ed5df878f29aa86c30",
        ),
        (
            "src/lib.c",
            "6bca070a6a70740c8e8af244af8a7311dbea09fbb60372376f5260facaf785f0",
        ),
    ];
    for (relative, expected) in runtime_sources {
        hash::require_file(
            &runtime_package.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR)),
            expected,
            "PARSER_RUNTIME_SOURCE_MISMATCH",
        )?;
    }
    require_python_free_native_build(&package, &runtime_package)?;
    Ok(expected_parser_boundary(compatibility_sha256))
}

fn require_python_free_native_build(package: &Path, runtime_package: &Path) -> Result<()> {
    for path in [
        package.join("bindings/rust/build.rs"),
        runtime_package.join("binding_rust/build.rs"),
    ] {
        let text = fs::read_to_string(&path).io_context(
            "PARSER_BUILD_AUDIT_FAILED",
            format!("could not read {}", path.display()),
        )?;
        let compact: String = text
            .to_ascii_lowercase()
            .chars()
            .filter(|value| !value.is_whitespace())
            .collect();
        if [
            "command::new(\"python",
            "command::new(\"python3",
            "command::new(\"py\"",
            "command::new(\"pip",
            "python.exe",
            "python3.exe",
            "pip.exe",
            "pyo3",
            "cpython",
        ]
        .iter()
        .any(|needle| compact.contains(needle))
        {
            return Err(XtaskError::integrity(
                "PYTHON_NATIVE_BUILD_BOUNDARY_VIOLATION",
                "pinned native parser build source invokes or embeds Python",
            ));
        }
    }
    Ok(())
}

fn require_zero_python_boundary(repository: &Path, recorder: &mut Recorder) -> Result<()> {
    require_fixed_process_boundary(repository)?;
    let output = recorder.run_locked_cargo_metadata(repository)?;
    let metadata: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        XtaskError::integrity(
            "XTASK_DEPENDENCY_METADATA_INVALID",
            format!("locked Cargo metadata is invalid JSON: {error}"),
        )
    })?;
    require_python_free_xtask_closure(repository, &metadata)?;
    recorder.mark_locked_cargo_metadata_audit_pass(&output.stdout)
}

fn require_fixed_process_boundary(repository: &Path) -> Result<()> {
    let source_root = repository.join("xtask/src");
    let mut source_files = Vec::new();
    for entry in fs::read_dir(&source_root).io_context(
        "XTASK_PROCESS_AUDIT_FAILED",
        "could not enumerate xtask process sources",
    )? {
        let entry = entry.io_context(
            "XTASK_PROCESS_AUDIT_FAILED",
            "could not read xtask process source entry",
        )?;
        let kind = entry.file_type().io_context(
            "XTASK_PROCESS_AUDIT_FAILED",
            "could not inspect xtask process source entry",
        )?;
        if kind.is_symlink() || !kind.is_file() {
            return Err(XtaskError::integrity(
                "XTASK_PROCESS_SOURCE_INVALID",
                "xtask/src must contain only regular source files",
            ));
        }
        if entry.path().extension().and_then(|value| value.to_str()) != Some("rs") {
            return Err(XtaskError::integrity(
                "XTASK_PROCESS_SOURCE_INVALID",
                "xtask/src contains a non-Rust source entry",
            ));
        }
        source_files.push(entry.path());
    }
    source_files.sort();
    let mut command_sites = Vec::new();
    for path in source_files {
        let text = fs::read_to_string(&path).io_context(
            "XTASK_PROCESS_AUDIT_FAILED",
            format!("could not read {}", path.display()),
        )?;
        let compact: String = text
            .chars()
            .filter(|value| !value.is_whitespace())
            .collect();
        let is_process_module =
            path.file_name().and_then(|value| value.to_str()) == Some("process.rs");
        let fully_qualified = ["std::process::", "Command::new("].concat();
        let relative_qualified = ["process::", "Command::new("].concat();
        let direct_import = ["use", "std::process::", "Command"].concat();
        let grouped_import = ["use", "std::process::{", "Command"].concat();
        let spawn = [".sp", "awn("].concat();
        if !is_process_module
            && (compact.contains(&fully_qualified)
                || compact.contains(&relative_qualified)
                || compact.contains(&direct_import)
                || compact.contains(&grouped_import)
                || compact.contains(&spawn))
        {
            return Err(XtaskError::integrity(
                "XTASK_PROCESS_BOUNDARY_VIOLATION",
                "xtask contains an unapproved process-construction surface",
            ));
        }
        if !is_process_module {
            continue;
        }
        let mut rest = compact.as_str();
        while let Some(index) = rest.find("Command::new(") {
            let tail = &rest[index..];
            let end = tail.find(')').ok_or_else(|| {
                XtaskError::integrity(
                    "XTASK_PROCESS_BOUNDARY_VIOLATION",
                    "xtask contains a malformed process-construction site",
                )
            })?;
            command_sites.push(tail[..=end].to_owned());
            rest = &tail[end + 1..];
        }
    }
    command_sites.sort();
    if command_sites != ["Command::new(\"cargo\")", "Command::new(\"git\")"] {
        return Err(XtaskError::integrity(
            "XTASK_PROCESS_BOUNDARY_VIOLATION",
            format!(
                "xtask child-process sites are not exactly the fixed cargo and git boundary: {command_sites:?}"
            ),
        ));
    }
    Ok(())
}

fn require_python_free_xtask_closure(repository: &Path, metadata: &Value) -> Result<()> {
    let packages = metadata["packages"].as_array().ok_or_else(|| {
        XtaskError::integrity(
            "XTASK_DEPENDENCY_METADATA_INVALID",
            "Cargo metadata has no package array",
        )
    })?;
    let nodes = metadata["resolve"]["nodes"].as_array().ok_or_else(|| {
        XtaskError::integrity(
            "XTASK_DEPENDENCY_METADATA_INVALID",
            "Cargo metadata has no resolved node array",
        )
    })?;
    let package_by_id: BTreeMap<&str, &Value> = packages
        .iter()
        .map(|package| {
            package["id"]
                .as_str()
                .map(|id| (id, package))
                .ok_or_else(|| {
                    XtaskError::integrity(
                        "XTASK_DEPENDENCY_METADATA_INVALID",
                        "Cargo package has no string identity",
                    )
                })
        })
        .collect::<Result<_>>()?;
    let node_by_id: BTreeMap<&str, &Value> = nodes
        .iter()
        .map(|node| {
            node["id"].as_str().map(|id| (id, node)).ok_or_else(|| {
                XtaskError::integrity(
                    "XTASK_DEPENDENCY_METADATA_INVALID",
                    "Cargo resolve node has no string identity",
                )
            })
        })
        .collect::<Result<_>>()?;
    let xtask_manifest = fs::canonicalize(repository.join("xtask/Cargo.toml")).io_context(
        "XTASK_DEPENDENCY_METADATA_INVALID",
        "could not canonicalize xtask manifest",
    )?;
    let roots: Vec<&str> = packages
        .iter()
        .filter(|package| package["name"] == json!("xtask"))
        .filter_map(|package| {
            let manifest = package["manifest_path"].as_str()?;
            fs::canonicalize(manifest)
                .ok()
                .filter(|path| path == &xtask_manifest)
                .and_then(|_| package["id"].as_str())
        })
        .collect();
    if roots.len() != 1 {
        return Err(XtaskError::integrity(
            "XTASK_DEPENDENCY_ROOT_INVALID",
            "locked metadata does not identify exactly one local xtask package",
        ));
    }

    let mut pending = vec![roots[0]];
    let mut closure = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !closure.insert(id) {
            continue;
        }
        let node = node_by_id.get(id).ok_or_else(|| {
            XtaskError::integrity(
                "XTASK_DEPENDENCY_METADATA_INVALID",
                "xtask dependency is missing from the resolved graph",
            )
        })?;
        for dependency in node["deps"].as_array().ok_or_else(|| {
            XtaskError::integrity(
                "XTASK_DEPENDENCY_METADATA_INVALID",
                "Cargo resolve dependency list is malformed",
            )
        })? {
            let admitted = dependency["dep_kinds"].as_array().is_some_and(|kinds| {
                kinds.iter().any(|kind| {
                    kind["kind"].is_null()
                        || matches!(kind["kind"].as_str(), Some("normal" | "build"))
                })
            });
            if admitted {
                pending.push(dependency["pkg"].as_str().ok_or_else(|| {
                    XtaskError::integrity(
                        "XTASK_DEPENDENCY_METADATA_INVALID",
                        "Cargo dependency has no package identity",
                    )
                })?);
            }
        }
    }

    for id in closure {
        let package = package_by_id.get(id).ok_or_else(|| {
            XtaskError::integrity(
                "XTASK_DEPENDENCY_METADATA_INVALID",
                "resolved xtask package is absent from metadata",
            )
        })?;
        let name = package["name"].as_str().ok_or_else(|| {
            XtaskError::integrity(
                "XTASK_DEPENDENCY_METADATA_INVALID",
                "resolved package has no name",
            )
        })?;
        let normalized_name = name.to_ascii_lowercase().replace(['-', '_'], "");
        if normalized_name.contains("python")
            || normalized_name.starts_with("pyo3")
            || normalized_name.starts_with("cpython")
            || normalized_name.starts_with("rustpython")
        {
            return Err(XtaskError::integrity(
                "PYTHON_DEPENDENCY_BOUNDARY_VIOLATION",
                format!("xtask runtime/build closure contains prohibited package {name}"),
            ));
        }
        if !package["links"].is_null() {
            return Err(XtaskError::integrity(
                "XTASK_NATIVE_MODULE_BOUNDARY_VIOLATION",
                format!("xtask runtime/build closure contains native links package {name}"),
            ));
        }
        let targets = package["targets"].as_array().ok_or_else(|| {
            XtaskError::integrity(
                "XTASK_DEPENDENCY_METADATA_INVALID",
                "resolved package target list is malformed",
            )
        })?;
        for target in targets {
            let kinds = target["kind"].as_array().ok_or_else(|| {
                XtaskError::integrity(
                    "XTASK_DEPENDENCY_METADATA_INVALID",
                    "resolved target kind is malformed",
                )
            })?;
            if kinds
                .iter()
                .any(|kind| matches!(kind.as_str(), Some("cdylib" | "dylib" | "staticlib")))
            {
                return Err(XtaskError::integrity(
                    "XTASK_NATIVE_MODULE_BOUNDARY_VIOLATION",
                    format!("xtask closure contains native library target in package {name}"),
                ));
            }
            if kinds.iter().any(|kind| kind == "custom-build") {
                let source = target["src_path"].as_str().ok_or_else(|| {
                    XtaskError::integrity(
                        "XTASK_BUILD_SCRIPT_AUDIT_FAILED",
                        "build-script target has no source path",
                    )
                })?;
                require_python_free_build_script(package, Path::new(source))?;
            }
        }
    }
    Ok(())
}

fn require_python_free_build_script(package: &Value, path: &Path) -> Result<()> {
    let manifest = PathBuf::from(package["manifest_path"].as_str().ok_or_else(|| {
        XtaskError::integrity(
            "XTASK_BUILD_SCRIPT_AUDIT_FAILED",
            "build-script package has no manifest path",
        )
    })?);
    let package_root = fs::canonicalize(manifest.parent().ok_or_else(|| {
        XtaskError::integrity(
            "XTASK_BUILD_SCRIPT_AUDIT_FAILED",
            "build-script manifest has no package directory",
        )
    })?)
    .io_context(
        "XTASK_BUILD_SCRIPT_AUDIT_FAILED",
        "could not canonicalize build-script package",
    )?;
    let source = fs::canonicalize(path).io_context(
        "XTASK_BUILD_SCRIPT_AUDIT_FAILED",
        "could not canonicalize build-script source",
    )?;
    if !source.starts_with(&package_root) {
        return Err(XtaskError::integrity(
            "XTASK_BUILD_SCRIPT_PATH_ESCAPE",
            "build-script source escapes its locked package directory",
        ));
    }
    let text = fs::read_to_string(&source).io_context(
        "XTASK_BUILD_SCRIPT_AUDIT_FAILED",
        "could not read build-script source",
    )?;
    let compact: String = text
        .to_ascii_lowercase()
        .chars()
        .filter(|value| !value.is_whitespace())
        .collect();
    if [
        "command::new(\"python",
        "command::new(\"python3",
        "command::new(\"py\"",
        "command::new(\"pip",
        "python.exe",
        "python3.exe",
        "pyo3",
        "cpython",
        "rustpython",
        "pip.exe",
    ]
    .iter()
    .any(|needle| compact.contains(needle))
    {
        return Err(XtaskError::integrity(
            "PYTHON_BUILD_SCRIPT_BOUNDARY_VIOLATION",
            "locked xtask build-script closure contains process or Python integration code",
        ));
    }
    Ok(())
}

fn require_locked_parser_packages(cargo_lock: &[u8]) -> Result<()> {
    require_locked_registry_package(
        cargo_lock,
        "tree-sitter",
        "0.25.8",
        "6d7b8994f367f16e6fa14b5aebbcb350de5d7cbea82dc5b00ae997dd71680dd2",
    )?;
    require_locked_registry_package(
        cargo_lock,
        "tree-sitter-python",
        "0.25.0",
        "6bf85fd39652e740bf60f46f4cda9492c3a9ad75880575bf14960f775cb74a1c",
    )
}

fn require_locked_registry_package(
    cargo_lock: &[u8],
    name: &str,
    version: &str,
    checksum: &str,
) -> Result<()> {
    let text = std::str::from_utf8(cargo_lock)
        .map_err(|_| XtaskError::integrity("PARSER_LOCK_INVALID", "Cargo.lock is not UTF-8"))?;
    let expected_name = format!("name = \"{name}\"");
    let expected_version = format!("version = \"{version}\"");
    let expected_checksum = format!("checksum = \"{checksum}\"");
    let matches = text
        .split("[[package]]")
        .filter(|block| {
            let lines: BTreeSet<&str> = block.lines().map(str::trim).collect();
            lines.contains(expected_name.as_str())
                && lines.contains(expected_version.as_str())
                && lines
                    .contains("source = \"registry+https://github.com/rust-lang/crates.io-index\"")
                && lines.contains(expected_checksum.as_str())
        })
        .count();
    if matches != 1 {
        return Err(XtaskError::integrity(
            "PARSER_LOCK_INVALID",
            format!("Cargo.lock does not contain exactly one pinned {name} {version} package"),
        ));
    }
    Ok(())
}

fn require_registry_archive(file_name: &str, expected_sha256: &str) -> Result<()> {
    let cache_root = cargo_home()?.join("registry/cache");
    let mut candidates = Vec::new();
    for registry in fs::read_dir(&cache_root).io_context(
        "PARSER_ARCHIVE_NOT_AVAILABLE",
        format!("could not enumerate {}", cache_root.display()),
    )? {
        let registry = registry.io_context(
            "PARSER_ARCHIVE_NOT_AVAILABLE",
            "could not read Cargo registry cache entry",
        )?;
        if registry
            .file_type()
            .io_context(
                "PARSER_ARCHIVE_NOT_AVAILABLE",
                "could not inspect Cargo registry cache entry",
            )?
            .is_dir()
        {
            let candidate = registry.path().join(file_name);
            if candidate.is_file() {
                candidates.push(candidate);
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    if candidates.len() != 1 {
        return Err(XtaskError::environment(
            "PARSER_ARCHIVE_IDENTITY_AMBIGUOUS",
            format!(
                "expected one cached {file_name}, observed {}",
                candidates.len()
            ),
        ));
    }
    hash::require_file(
        &candidates[0],
        expected_sha256,
        "PARSER_ARCHIVE_CHECKSUM_MISMATCH",
    )
}

fn expected_parser_boundary(compatibility_sha256: String) -> Value {
    let compatibility_path = "docs/schemas/portable-v2/tree-sitter-python-compatibility-v1.json";
    let runtime_sources = [
        (
            "binding_rust/build.rs",
            "6c1a090f4ac12621effce41a9bac54182b155f252a84b3bbb51cbcd58330593d",
        ),
        (
            "include/tree_sitter/api.h",
            "937c5389713f99318ac7adbdb3a95e9976bbd211e35e37ed5df878f29aa86c30",
        ),
        (
            "src/lib.c",
            "6bca070a6a70740c8e8af244af8a7311dbea09fbb60372376f5260facaf785f0",
        ),
    ];
    let generated_sources = GENERATED_SOURCES
        .iter()
        .map(|(path, sha256)| json!({"path": path, "sha256": sha256}))
        .collect::<Vec<_>>();
    json!({
        "schema": "python-slm-parser-boundary-v1",
        "tree_sitter": {
            "version": "0.25.8",
            "package_checksum": "6d7b8994f367f16e6fa14b5aebbcb350de5d7cbea82dc5b00ae997dd71680dd2",
            "runtime_source_identity": "crates.io:tree-sitter@0.25.8#6d7b8994f367f16e6fa14b5aebbcb350de5d7cbea82dc5b00ae997dd71680dd2"
        },
        "tree_sitter_python": {
            "version": "0.25.0",
            "package_checksum": "6bf85fd39652e740bf60f46f4cda9492c3a9ad75880575bf14960f775cb74a1c",
            "language_abi_version": 15
        },
        "runtime_sources": runtime_sources.into_iter().map(|(path, sha256)| json!({"path": path, "sha256": sha256})).collect::<Vec<_>>(),
        "generated_sources": generated_sources,
        "compatibility_corpus": {"path": compatibility_path, "sha256": compatibility_sha256},
        "python_codegen": false,
        "normalized_build_flags": {
            "tree_sitter_runtime": [
                "feature_bindgen=false", "feature_wasm=false", "flag_if_supported=-std=c11",
                "flag_if_supported=-fvisibility=hidden", "flag_if_supported=-Wshadow",
                "flag_if_supported=-Wno-unused-parameter", "flag_if_supported=-Wno-incompatible-pointer-types",
                "include=src", "include=src/wasm", "include=include", "define=_POSIX_C_SOURCE=200112L",
                "define=_DEFAULT_SOURCE", "warnings=false", "file=src/lib.c"
            ],
            "tree_sitter_python": [
                "std=c11", "include=src", "flag_if_supported=-Wno-unused-value",
                "target_env=msvc:flag=-utf-8", "file=src/parser.c", "file=src/scanner.c"
            ]
        },
        "native_role": "parser_only",
        "allowed_semantics": ["SOURCE-002", "SOURCE-003", "DEDUP-001", "DECONTAM-001"]
    })
}

fn find_registry_package(registry_root: &Path, package_name: &str) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    for registry in fs::read_dir(registry_root).io_context(
        "PARSER_PACKAGE_NOT_AVAILABLE",
        format!("could not enumerate {}", registry_root.display()),
    )? {
        let registry = registry.io_context(
            "PARSER_PACKAGE_NOT_AVAILABLE",
            "could not read Cargo registry entry",
        )?;
        if registry
            .file_type()
            .io_context(
                "PARSER_PACKAGE_NOT_AVAILABLE",
                "could not inspect Cargo registry entry",
            )?
            .is_dir()
        {
            let candidate = registry.path().join(package_name);
            if candidate.join("Cargo.toml").is_file() {
                candidates.push(candidate);
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    if candidates.len() != 1 {
        return Err(XtaskError::environment(
            "PARSER_PACKAGE_IDENTITY_AMBIGUOUS",
            format!(
                "expected one cached {package_name} source, observed {}",
                candidates.len()
            ),
        ));
    }
    Ok(candidates.remove(0))
}

fn cargo_registry_source_root() -> Result<PathBuf> {
    Ok(cargo_home()?.join("registry/src"))
}

fn cargo_home() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("CARGO_HOME") {
        return Ok(PathBuf::from(value));
    }
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).ok_or_else(|| {
            XtaskError::environment(
                "CARGO_HOME_UNAVAILABLE",
                "could not locate the Cargo home directory",
            )
        })?;
    Ok(PathBuf::from(home).join(".cargo"))
}

fn write_commands(run_root: &Path, commands: &[RecordedCommand]) -> Result<Vec<CommandEvidence>> {
    let mut evidence = Vec::new();
    for command in commands {
        let stdout = command.stdout_ref();
        let stderr = command.stderr_ref();
        publication::write_new(&run_root.join(&stdout.path), &command.stdout)?;
        publication::write_new(&run_root.join(&stderr.path), &command.stderr)?;
        evidence.push(CommandEvidence {
            id: command.id.clone(),
            argv: command.argv.clone(),
            cwd: command.cwd.clone(),
            exit_code: command.exit_code,
            status: command.status,
            stdout,
            stderr,
        });
    }
    Ok(evidence)
}

fn recorder_command_count(run_root: &Path) -> Result<usize> {
    let count = fs::read_dir(run_root.join("commands"))
        .io_context(
            "COMMAND_ENUMERATION_FAILED",
            "could not enumerate retained commands",
        )?
        .count();
    if count % 2 != 0 {
        return Err(XtaskError::new(
            "COMMAND_ARTIFACT_COUNT_INVALID",
            crate::error::Category::Internal,
            "retained command transcript count is not even",
            "Inspect the P0A command writer.",
        ));
    }
    Ok(count / 2)
}

fn require_receipt_redaction(run_root: &Path) -> Result<()> {
    let mut stack = vec![run_root.to_path_buf()];
    let mut private_values = Vec::new();
    for name in [
        "USERPROFILE",
        "HOME",
        "TEMP",
        "TMP",
        "COMPUTERNAME",
        "USERNAME",
    ] {
        if let Some(value) = std::env::var_os(name) {
            let value = value.to_string_lossy().into_owned();
            if value.len() >= 3 {
                private_values.push(value.clone());
                private_values.push(value.replace('\\', "/"));
            }
        }
    }
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).io_context(
            "RECEIPT_REDACTION_FAILED",
            format!("could not enumerate {}", directory.display()),
        )? {
            let entry =
                entry.io_context("RECEIPT_REDACTION_FAILED", "could not read receipt entry")?;
            if entry
                .file_type()
                .io_context(
                    "RECEIPT_REDACTION_FAILED",
                    "could not inspect receipt entry",
                )?
                .is_dir()
            {
                stack.push(entry.path());
                continue;
            }
            let bytes = fs::read(entry.path())
                .io_context("RECEIPT_REDACTION_FAILED", "could not read receipt file")?;
            if bytes.contains(&b'\r') {
                return Err(XtaskError::integrity(
                    "RECEIPT_ENCODING_INVALID",
                    "receipt text contains CR bytes",
                ));
            }
            let text = String::from_utf8(bytes).map_err(|_| {
                XtaskError::integrity(
                    "RECEIPT_ENCODING_INVALID",
                    "receipt file is not strict UTF-8",
                )
            })?;
            if has_absolute_drive_path(&text) || text.contains("\\\\") {
                return Err(XtaskError::integrity(
                    "RECEIPT_PATH_REDACTION_FAILED",
                    format!(
                        "receipt contains an absolute local path: {}",
                        entry.path().display()
                    ),
                ));
            }
            let lower = text.to_ascii_lowercase();
            if lower.contains("password=")
                || lower.contains("authorization: bearer ")
                || lower.contains("api_key=")
            {
                return Err(XtaskError::integrity(
                    "RECEIPT_SECRET_REDACTION_FAILED",
                    "receipt contains a credential-like value",
                ));
            }
            if private_values
                .iter()
                .any(|value| !value.is_empty() && text.contains(value))
            {
                return Err(XtaskError::integrity(
                    "RECEIPT_PRIVATE_VALUE_REDACTION_FAILED",
                    "receipt contains a host-private value",
                ));
            }
        }
    }
    Ok(())
}

fn has_absolute_drive_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(2)).any(|index| {
        bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'/' | b'\\')
            && (index == 0
                || !bytes[index - 1].is_ascii_alphanumeric()
                    && !matches!(bytes[index - 1], b'+' | b'.' | b'-'))
    })
}

pub fn finalize(repository: &Path, supplied_root: &Path) -> Result<Value> {
    let output_root = publication::require_output_root(repository, supplied_root)?;
    publication::require_no_follow_tree(&output_root)?;
    let mut recorder = Recorder::default();
    p0::verify(repository, &mut recorder)?;
    let (approval_sequence, technical_path, technical, governance_path, governance) =
        load_latest_approval_pair(&output_root)?;
    validate_approval_pair(&technical, &governance)?;
    recover_all_acceptance_temporaries(&output_root)?;
    let next_sequence = next_acceptance_sequence(&output_root)?;

    let run_id = technical.run_id.clone();
    let run_root = output_root.join("runs").join(&run_id);
    let run = validate_run(repository, &mut recorder, &run_root)?;
    if run.evidence_sha256 != technical.run_evidence_sha256
        || run.evidence_sha256 != governance.run_evidence_sha256
        || run.seal_sha256 != technical.seal_sha256
        || run.seal_sha256 != governance.seal_sha256
    {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_RUN_BINDING_INVALID",
            "owner approvals do not bind the exact validated run evidence and seal",
        ));
    }
    let approval_refs = vec![
        ApprovalRef {
            role: "technical".to_owned(),
            decision: "APPROVE".to_owned(),
            path: slash_relative(&output_root, &technical_path)?,
            sha256: hash::file(&technical_path)?,
        },
        ApprovalRef {
            role: "data_governance".to_owned(),
            decision: "APPROVE".to_owned(),
            path: slash_relative(&output_root, &governance_path)?,
            sha256: hash::file(&governance_path)?,
        },
    ];
    let observed_approval_sequence = approval_reference_sequence(&approval_refs)?;
    if observed_approval_sequence != approval_sequence {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_SEQUENCE_INVALID",
            "approval references do not match the selected approval attempt",
        ));
    }

    let retained_acceptance = if next_sequence > 1 {
        let path = output_root.join(format!("acceptances/{:08}.json", next_sequence - 1));
        Some(read_json_closed::<Acceptance>(
            &path,
            "P0A_ACCEPTANCE_JSON_INVALID",
        )?)
    } else {
        None
    };
    let matching_retained = retained_acceptance.as_ref().is_some_and(|acceptance| {
        acceptance.approvals == approval_refs
            && acceptance.run_path == format!("runs/{run_id}")
            && acceptance.run_evidence_sha256 == run.evidence_sha256
            && acceptance.seal_sha256 == run.seal_sha256
            && acceptance.preapproval_commit == run.preapproval_commit
            && acceptance.todo_preapproval_sha256 == run.todo_preapproval_sha256
    });
    let publication_sequence = if matching_retained {
        next_sequence - 1
    } else {
        require_selected_predecessor_for_new_acceptance(
            &output_root,
            retained_acceptance.as_ref(),
            next_sequence,
            approval_sequence,
        )?;
        next_sequence
    };
    let existing_acceptance =
        select_matching_retained_acceptance(retained_acceptance.as_ref(), matching_retained);
    let approval_commit = existing_acceptance.map_or_else(
        || {
            git_line(
                &mut recorder,
                repository,
                &["rev-parse", "HEAD"],
                "APPROVAL_COMMIT_INVALID",
            )
        },
        |acceptance| Ok(acceptance.approval_commit.clone()),
    )?;
    require_approval_commit(
        repository,
        &mut recorder,
        &approval_commit,
        &technical_path,
        &governance_path,
        &run.preapproval_commit,
    )?;
    if let Some(existing) = existing_acceptance.as_ref()
        && let Some(result) = recover_or_complete_publication(
            repository,
            &mut recorder,
            &output_root,
            publication_sequence,
            existing,
            &approval_commit,
            &approval_refs,
            &run,
            &run_id,
        )?
    {
        return Ok(result);
    }
    require_repository_clean(repository, &mut recorder)?;

    publication::create_dir_all(&output_root.join("acceptances"))?;
    let previous_acceptance_sha256 = load_previous_acceptance_hash(&output_root, next_sequence)?;
    let (created_at, _, _) = time::now();
    let acceptance = Acceptance {
        schema: "python-slm-p0a-phase-acceptance-v1".to_owned(),
        phase_id: "P0A".to_owned(),
        interface_id: INTERFACE_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        sequence: next_sequence,
        status: "PASS".to_owned(),
        acceptance_kind: "owner_approved_contract_amendment".to_owned(),
        run_path: format!("runs/{run_id}"),
        run_evidence_sha256: run.evidence_sha256.clone(),
        seal_path: format!("runs/{run_id}/SHA256SUMS"),
        seal_sha256: run.seal_sha256.clone(),
        approvals: approval_refs,
        approval_commit: approval_commit.clone(),
        preapproval_commit: run.preapproval_commit.clone(),
        todo_preapproval_sha256: run.todo_preapproval_sha256.clone(),
        previous_acceptance_sha256,
        created_at: created_at.clone(),
    };
    validate_acceptance_shape(&acceptance)?;
    let acceptance_relative = format!("acceptances/{next_sequence:08}.json");
    let acceptance_path = output_root.join(&acceptance_relative);
    let acceptance_bytes = canonical_json_bytes(
        &acceptance,
        "P0A_ACCEPTANCE_SERIALIZATION_FAILED",
        "could not serialize P0A acceptance",
    )?;
    let acceptance_sha256 = hash::bytes(&acceptance_bytes);
    let pointer = Pointer {
        schema: "python-slm-p0a-phase-pointer-v1".to_owned(),
        phase_id: "P0A".to_owned(),
        interface_id: INTERFACE_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        acceptance_path: acceptance_relative,
        acceptance_sha256: acceptance_sha256.clone(),
        updated_at: created_at,
    };
    validate_pointer_shape(&pointer)?;
    publish_acceptance_and_pointer(&output_root, &acceptance_path, &acceptance_bytes, &pointer)?;
    let selected = load_pointer(&output_root)?;
    if selected.acceptance_sha256 != acceptance_sha256
        || selected.acceptance_path != pointer.acceptance_path
    {
        return Err(XtaskError::integrity(
            "P0A_POINTER_REREAD_FAILED",
            "published pointer does not select the acceptance just created",
        ));
    }
    Ok(finalize_result(&pointer, &approval_commit))
}

fn canonical_json_bytes<T: Serialize>(
    value: &T,
    code: &'static str,
    message: &'static str,
) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        XtaskError::new(
            code,
            crate::error::Category::Internal,
            format!("{message}: {error}"),
            "Inspect the closed P0A data model.",
        )
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
fn recover_or_complete_publication(
    repository: &Path,
    recorder: &mut Recorder,
    output_root: &Path,
    sequence: u32,
    existing: &Acceptance,
    approval_commit: &str,
    approval_refs: &[ApprovalRef],
    run: &ValidatedRun,
    run_id: &str,
) -> Result<Option<Value>> {
    if next_acceptance_sequence(output_root)? != sequence + 1 {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_TAIL_INVALID",
            "recoverable publication must be the sole greatest acceptance tail",
        ));
    }
    let expected = Acceptance {
        schema: "python-slm-p0a-phase-acceptance-v1".to_owned(),
        phase_id: "P0A".to_owned(),
        interface_id: INTERFACE_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        sequence,
        status: "PASS".to_owned(),
        acceptance_kind: "owner_approved_contract_amendment".to_owned(),
        run_path: format!("runs/{run_id}"),
        run_evidence_sha256: run.evidence_sha256.clone(),
        seal_path: format!("runs/{run_id}/SHA256SUMS"),
        seal_sha256: run.seal_sha256.clone(),
        approvals: approval_refs.to_vec(),
        approval_commit: approval_commit.to_owned(),
        preapproval_commit: run.preapproval_commit.clone(),
        todo_preapproval_sha256: run.todo_preapproval_sha256.clone(),
        previous_acceptance_sha256: load_previous_acceptance_hash(output_root, sequence)?,
        created_at: existing.created_at.clone(),
    };
    validate_acceptance_shape(existing)?;
    if existing != &expected {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_TAIL_INVALID",
            "retained acceptance tail is not the exact owner-approved publication candidate",
        ));
    }
    let relative = format!("{OUTPUT_ROOT}/acceptances/{sequence:08}.json");
    let committed_probe =
        recorder.run_git(repository, &["cat-file", "-e", &format!("HEAD:{relative}")])?;
    let acceptance_committed = committed_probe.status.success();
    let acceptance_path = output_root.join(format!("acceptances/{sequence:08}.json"));
    let acceptance_sha256 = hash::file(&acceptance_path)?;
    let pointer = Pointer {
        schema: "python-slm-p0a-phase-pointer-v1".to_owned(),
        phase_id: "P0A".to_owned(),
        interface_id: INTERFACE_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        acceptance_path: format!("acceptances/{sequence:08}.json"),
        acceptance_sha256: acceptance_sha256.clone(),
        updated_at: existing.created_at.clone(),
    };
    validate_pointer_shape(&pointer)?;
    recover_pointer_temporary(output_root, &pointer)?;
    let status = recorder.git_text(
        repository,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        "REPOSITORY_STATUS_FAILED",
    )?;
    if acceptance_committed {
        if !status.trim().is_empty() {
            return Err(XtaskError::integrity(
                "P0A_PUBLICATION_REPOSITORY_DIRTY",
                "committed P0A publication cannot be accepted with a dirty repository",
            ));
        }
    } else {
        require_publication_recovery_head(repository, recorder, approval_commit)?;
        validate_recoverable_publication_status(&status, &relative)?;
    }
    let pointer_path = output_root.join("evidence.json");
    if acceptance_committed {
        if !pointer_path.is_file() || load_pointer(output_root)? != pointer {
            return Err(XtaskError::integrity(
                "P0A_PUBLICATION_SPLIT_COMMIT",
                "committed acceptance exists without its exact selected pointer in the same publication state",
            ));
        }
        require_committed_publication_pair(
            repository,
            recorder,
            sequence,
            approval_commit,
            &pointer,
        )?;
    }
    let already_selected = if pointer_path.is_file() {
        let observed = load_pointer(output_root)?;
        if observed == pointer {
            true
        } else {
            let observed_sequence = parse_acceptance_path(&observed.acceptance_path)?;
            if sequence == 1 || observed_sequence + 1 != sequence {
                return Err(XtaskError::integrity(
                    "P0A_PUBLICATION_STATE_INDETERMINATE",
                    "existing pointer is neither the exact predecessor nor the recoverable tail",
                ));
            }
            let predecessor = output_root.join(&observed.acceptance_path);
            hash::require_file(
                &predecessor,
                &observed.acceptance_sha256,
                "P0A_PREDECESSOR_POINTER_HASH_MISMATCH",
            )?;
            false
        }
    } else {
        if sequence != 1 {
            return Err(XtaskError::integrity(
                "P0A_PUBLICATION_STATE_INDETERMINATE",
                "non-genesis acceptance tail has no predecessor pointer",
            ));
        }
        false
    };
    if !already_selected {
        replace_pointer_atomically(&pointer_path, &pointer)?;
    }
    let selected = load_pointer(output_root)?;
    if selected != pointer {
        return Err(XtaskError::integrity(
            "P0A_POINTER_REREAD_FAILED",
            "recovered publication pointer does not select the exact acceptance tail",
        ));
    }
    Ok(Some(finalize_result(&pointer, approval_commit)))
}

fn require_publication_recovery_head(
    repository: &Path,
    recorder: &mut Recorder,
    approval_commit: &str,
) -> Result<()> {
    let head = git_line(
        recorder,
        repository,
        &["rev-parse", "HEAD"],
        "P0A_PUBLICATION_RECOVERY_HEAD_INVALID",
    )?;
    if head != approval_commit {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_RECOVERY_HEAD_INVALID",
            "uncommitted publication recovery requires HEAD to equal the bound approval commit",
        ));
    }
    Ok(())
}

fn require_committed_publication_pair(
    repository: &Path,
    recorder: &mut Recorder,
    sequence: u32,
    approval_commit: &str,
    pointer: &Pointer,
) -> Result<()> {
    let acceptance_relative = format!("{OUTPUT_ROOT}/{}", pointer.acceptance_path);
    let commits = recorder.git_text(
        repository,
        &[
            "log",
            "--format=%H",
            "--diff-filter=A",
            "--no-renames",
            "HEAD",
            "--",
            &acceptance_relative,
        ],
        "P0A_PUBLICATION_COMMIT_LOOKUP_FAILED",
    )?;
    let commits: Vec<&str> = commits.lines().filter(|line| !line.is_empty()).collect();
    if commits.len() != 1 || !valid_git_sha(commits[0]) {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_COMMIT_INVALID",
            "committed acceptance must be created by exactly one publication commit",
        ));
    }
    let publication_commit = commits[0];
    let parents = recorder.git_text(
        repository,
        &["rev-list", "--parents", "-n", "1", publication_commit],
        "P0A_PUBLICATION_PARENT_INVALID",
    )?;
    let parents: Vec<&str> = parents.split_whitespace().collect();
    if parents.len() != 2 || parents[1] != approval_commit {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_PARENT_INVALID",
            "publication commit is not the single-parent child of the approval commit",
        ));
    }
    let changed = recorder.git_text(
        repository,
        &[
            "diff",
            "--name-status",
            "--no-renames",
            approval_commit,
            publication_commit,
        ],
        "P0A_PUBLICATION_COMMIT_DIFF_FAILED",
    )?;
    let pointer_status = if sequence == 1 { "A" } else { "M" };
    let expected: BTreeSet<String> = [
        format!("A\t{acceptance_relative}"),
        format!("{pointer_status}\t{OUTPUT_ROOT}/evidence.json"),
    ]
    .into_iter()
    .collect();
    let observed: BTreeSet<String> = changed
        .lines()
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if observed != expected {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_SPLIT_COMMIT",
            "acceptance and selected pointer were not committed together as the exact publication pair",
        ));
    }
    let live_pointer_hash = hash::file(&repository.join(OUTPUT_ROOT).join("evidence.json"))?;
    for (path, expected_hash) in [
        (acceptance_relative, pointer.acceptance_sha256.as_str()),
        (
            format!("{OUTPUT_ROOT}/evidence.json"),
            live_pointer_hash.as_str(),
        ),
    ] {
        let committed = git_blob(recorder, repository, publication_commit, &path)?;
        if hash::bytes(&committed) != expected_hash {
            return Err(XtaskError::integrity(
                "P0A_PUBLICATION_COMMIT_BINDING_INVALID",
                format!("publication commit does not retain the exact live bytes for {path}"),
            ));
        }
    }
    Ok(())
}

fn validate_recoverable_publication_status(status: &str, acceptance_path: &str) -> Result<()> {
    let mut saw_acceptance = false;
    for line in status.lines().filter(|line| !line.is_empty()) {
        if line.len() < 4 {
            return Err(XtaskError::integrity(
                "P0A_PUBLICATION_REPOSITORY_DIRTY",
                "malformed Git status while recovering publication",
            ));
        }
        let state = &line[..2];
        let path = &line[3..];
        if path == acceptance_path && state == "??" {
            saw_acceptance = true;
        } else if path == format!("{OUTPUT_ROOT}/evidence.json")
            && matches!(state, "??" | " M" | "M ")
        {
            // A crash may occur before or after the pointer replacement.
        } else {
            return Err(XtaskError::integrity(
                "P0A_PUBLICATION_REPOSITORY_DIRTY",
                format!("unexpected dirty path during publication recovery: {line}"),
            ));
        }
    }
    if !saw_acceptance {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_TAIL_NOT_CREATE_NEW",
            "recoverable acceptance tail must be an untracked create-new file",
        ));
    }
    Ok(())
}

fn finalize_result(pointer: &Pointer, approval_commit: &str) -> Value {
    json!({
        "schema": "python-slm-p0a-finalize-result-v1",
        "phase_id": "P0A",
        "status": "PASS",
        "acceptance_path": pointer.acceptance_path,
        "acceptance_sha256": pointer.acceptance_sha256,
        "approval_commit": approval_commit,
        "next_action": "commit the acceptance and pointer, then create the one-line P0A checkbox-only commit"
    })
}

fn publish_acceptance_and_pointer(
    output_root: &Path,
    acceptance_path: &Path,
    acceptance_bytes: &[u8],
    pointer: &Pointer,
) -> Result<()> {
    publish_acceptance_and_pointer_with(
        output_root,
        acceptance_path,
        acceptance_bytes,
        pointer,
        replace_pointer_atomically,
    )
}

fn publish_acceptance_and_pointer_with<F>(
    output_root: &Path,
    acceptance_path: &Path,
    acceptance_bytes: &[u8],
    pointer: &Pointer,
    mut replace: F,
) -> Result<()>
where
    F: FnMut(&Path, &Pointer) -> Result<()>,
{
    let pointer_path = output_root.join("evidence.json");
    let prior_pointer = if pointer_path.exists() {
        Some(fs::read(&pointer_path).io_context(
            "P0A_POINTER_READ_FAILED",
            "could not read prior selected pointer",
        )?)
    } else {
        None
    };
    publication::write_new_via_owned_temp(acceptance_path, acceptance_bytes, "acceptance")?;
    if let Err(original) = replace(&pointer_path, pointer) {
        let expected_pointer = canonical_json_bytes(
            pointer,
            "P0A_POINTER_SERIALIZATION_FAILED",
            "could not serialize P0A pointer",
        )?;
        let observed = fs::read(&pointer_path).ok();
        if observed.as_deref() == Some(expected_pointer.as_slice())
            && fs::read(acceptance_path).ok().as_deref() == Some(acceptance_bytes)
        {
            return Ok(());
        }
        if observed == prior_pointer {
            fs::remove_file(acceptance_path).io_context(
                "P0A_PUBLICATION_ROLLBACK_FAILED",
                "could not remove the exact acceptance created by the failed publication",
            )?;
            if acceptance_path.exists() {
                return Err(XtaskError::integrity(
                    "P0A_PUBLICATION_ROLLBACK_FAILED",
                    "failed publication acceptance remains after rollback",
                ));
            }
            return Err(original);
        }
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_STATE_INDETERMINATE",
            "pointer state changed unexpectedly while publishing the acceptance pair",
        ));
    }
    if fs::read(acceptance_path).ok().as_deref() != Some(acceptance_bytes) {
        return Err(XtaskError::integrity(
            "P0A_ACCEPTANCE_REREAD_FAILED",
            "acceptance bytes changed during publication",
        ));
    }
    Ok(())
}

pub fn check_selected(repository: &Path, supplied_root: &Path) -> Result<Value> {
    let output_root = publication::require_output_root(repository, supplied_root)?;
    publication::require_no_follow_tree(&output_root)?;
    let mut recorder = Recorder::default();
    p0::verify(repository, &mut recorder)?;
    require_repository_clean(repository, &mut recorder)?;
    let pointer = load_pointer(&output_root)?;
    let acceptance_path = output_root.join(&pointer.acceptance_path);
    hash::require_file(
        &acceptance_path,
        &pointer.acceptance_sha256,
        "P0A_ACCEPTANCE_POINTER_HASH_MISMATCH",
    )?;
    let acceptance: Acceptance = read_json_closed(&acceptance_path, "P0A_ACCEPTANCE_JSON_INVALID")?;
    validate_acceptance_shape(&acceptance)?;
    validate_selected_acceptance(&output_root, &pointer, &acceptance)?;
    if pointer.updated_at != acceptance.created_at {
        return Err(XtaskError::integrity(
            "P0A_POINTER_TIME_MISMATCH",
            "selected pointer and acceptance do not share one publication timestamp",
        ));
    }
    let run = validate_run(
        repository,
        &mut recorder,
        &output_root.join(&acceptance.run_path),
    )?;
    if run.evidence_sha256 != acceptance.run_evidence_sha256
        || run.seal_sha256 != acceptance.seal_sha256
        || run.preapproval_commit != acceptance.preapproval_commit
        || run.todo_preapproval_sha256 != acceptance.todo_preapproval_sha256
    {
        return Err(XtaskError::integrity(
            "P0A_ACCEPTANCE_RUN_BINDING_INVALID",
            "selected acceptance does not bind the validated run",
        ));
    }
    validate_acceptance_approvals(&output_root, &acceptance)?;
    validate_acceptance_chain(repository, &mut recorder, &output_root, &acceptance)?;
    validate_checkbox_commit(
        repository,
        &mut recorder,
        &output_root,
        &pointer,
        &acceptance,
    )?;
    Ok(json!({
        "schema": "python-slm-p0a-selected-result-v1",
        "phase_id": "P0A",
        "status": "PASS",
        "profile_id": PROFILE_ID,
        "acceptance_path": pointer.acceptance_path,
        "acceptance_sha256": pointer.acceptance_sha256,
        "run_path": acceptance.run_path,
        "run_evidence_sha256": acceptance.run_evidence_sha256,
        "seal_sha256": acceptance.seal_sha256
    }))
}

#[derive(Debug)]
struct ValidatedRun {
    evidence_sha256: String,
    seal_sha256: String,
    preapproval_commit: String,
    todo_preapproval_sha256: String,
}

fn validate_run(
    repository: &Path,
    recorder: &mut Recorder,
    run_root: &Path,
) -> Result<ValidatedRun> {
    publication::require_no_follow_tree(run_root)?;
    let evidence_path = run_root.join("evidence.json");
    let evidence_sha256 = hash::file(&evidence_path)?;
    let evidence: Value = read_json_closed(&evidence_path, "P0A_EVIDENCE_JSON_INVALID")?;
    require_object_keys(
        &evidence,
        &[
            "schema",
            "phase_id",
            "interface_id",
            "profile_id",
            "run_id",
            "status",
            "generated_at_utc",
            "source",
            "p0_dependency",
            "artifacts",
            "commands",
            "authority",
            "errors",
            "seal",
        ],
        "P0A_EVIDENCE_FIELDS_INVALID",
    )?;
    require_json_string(&evidence, "/schema", "python-slm-p0a-phase-evidence-v1")?;
    require_json_string(&evidence, "/phase_id", "P0A")?;
    require_json_string(&evidence, "/interface_id", INTERFACE_ID)?;
    require_json_string(&evidence, "/profile_id", PROFILE_ID)?;
    require_json_string(&evidence, "/status", "AWAITING_REVIEW")?;
    validate_evidence_shape(run_root, &evidence)?;
    require_json_string(&evidence, "/authority/machine_evidence", "PASS")?;
    require_json_string(&evidence, "/authority/phase_acceptance", "PENDING")?;
    if evidence
        .pointer("/errors")
        .and_then(Value::as_array)
        .is_none_or(|errors| !errors.is_empty())
    {
        return Err(XtaskError::integrity(
            "P0A_EVIDENCE_ERRORS_INVALID",
            "AWAITING_REVIEW evidence must contain no errors",
        ));
    }
    validate_p0_evidence_constants(&evidence)?;
    let seal_path = run_root.join("SHA256SUMS");
    let seal_sha256 = hash::file(&seal_path)?;
    publication::verify_seal(run_root, &seal_sha256)?;
    let seal_entries = fs::read_to_string(&seal_path)
        .io_context("P0A_SEAL_READ_FAILED", "could not read P0A run seal")?
        .lines()
        .count() as u64;
    if evidence.pointer("/seal/entries").and_then(Value::as_u64) != Some(seal_entries) {
        return Err(XtaskError::integrity(
            "P0A_SEAL_DESCRIPTOR_INVALID",
            "evidence seal count does not equal the complete retained manifest",
        ));
    }
    require_receipt_redaction(run_root)?;
    validate_all_references(run_root, &evidence)?;
    let preapproval_commit = evidence
        .pointer("/source/commit")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            XtaskError::integrity("P0A_SOURCE_IDENTITY_INVALID", "missing source commit")
        })?
        .to_owned();
    validate_source_binding(
        repository,
        recorder,
        run_root,
        &evidence,
        &preapproval_commit,
    )?;
    let todo_preapproval_sha256 = hash::file(&run_root.join("artifacts/TODO-preapproval.md"))?;
    Ok(ValidatedRun {
        evidence_sha256,
        seal_sha256,
        preapproval_commit,
        todo_preapproval_sha256,
    })
}

fn validate_p0_evidence_constants(evidence: &Value) -> Result<()> {
    for (pointer, expected) in [
        ("/p0_dependency/baseline_commit", p0::BASELINE_COMMIT),
        ("/p0_dependency/receipt_commit", p0::RECEIPT_COMMIT),
        ("/p0_dependency/receipt_sha256", p0::RECEIPT_SHA256),
        ("/p0_dependency/contract_sha256", p0::CONTRACT_SHA256),
        ("/p0_dependency/decision_ledger_sha256", p0::LEDGER_SHA256),
        (
            "/p0_dependency/reconciliation_commit",
            p0::RECONCILIATION_COMMIT,
        ),
        (
            "/p0_dependency/reconciliation_tree",
            p0::RECONCILIATION_TREE,
        ),
        ("/p0_dependency/run_id", p0::RUN_ID),
        ("/p0_dependency/seal_sha256", p0::SEAL_SHA256),
        (
            "/p0_dependency/historical_cargo_lock_sha256",
            p0::HISTORICAL_CARGO_LOCK_SHA256,
        ),
        ("/p0_dependency/oracle_commit", p0::ORACLE_COMMIT),
        (
            "/p0_dependency/oracle_normalized_sha256",
            p0::ORACLE_NORMALIZED_SHA256,
        ),
    ] {
        require_json_string(evidence, pointer, expected)?;
    }
    require_json_string(evidence, "/p0_dependency/status", "PASS")
}

fn validate_evidence_shape(run_root: &Path, evidence: &Value) -> Result<()> {
    require_object_keys(
        &evidence["source"],
        &[
            "commit",
            "tree",
            "branch",
            "dirty",
            "cargo_lock_sha256",
            "identity_ref",
        ],
        "P0A_SOURCE_FIELDS_INVALID",
    )?;
    require_object_keys(
        &evidence["p0_dependency"],
        &[
            "status",
            "baseline_commit",
            "receipt_commit",
            "receipt_sha256",
            "contract_sha256",
            "decision_ledger_sha256",
            "reconciliation_commit",
            "reconciliation_tree",
            "run_id",
            "seal_sha256",
            "historical_cargo_lock_sha256",
            "oracle_commit",
            "oracle_normalized_sha256",
            "reference",
        ],
        "P0A_P0_DEPENDENCY_FIELDS_INVALID",
    )?;
    require_object_keys(
        &evidence["authority"],
        &["machine_evidence", "phase_acceptance", "required_approvals"],
        "P0A_AUTHORITY_FIELDS_INVALID",
    )?;
    require_object_keys(
        &evidence["seal"],
        &["path", "entries", "coverage_rule"],
        "P0A_SEAL_DESCRIPTOR_INVALID",
    )?;
    let run_id = evidence["run_id"]
        .as_str()
        .ok_or_else(|| XtaskError::integrity("P0A_RUN_ID_INVALID", "evidence run_id is missing"))?;
    if !valid_run_id(run_id)
        || run_root.file_name().and_then(|value| value.to_str()) != Some(run_id)
    {
        return Err(XtaskError::integrity(
            "P0A_RUN_ID_INVALID",
            "evidence run_id is malformed or does not equal its immutable directory",
        ));
    }
    let generated = evidence["generated_at_utc"].as_str().unwrap_or_default();
    let branch = evidence["source"]["branch"].as_str().unwrap_or_default();
    if !valid_utc_timestamp(generated)
        || !valid_git_sha(evidence["source"]["commit"].as_str().unwrap_or_default())
        || !valid_git_sha(evidence["source"]["tree"].as_str().unwrap_or_default())
        || branch.is_empty()
        || branch.trim() != branch
        || evidence["source"]["dirty"] != json!(false)
        || !hash::is_lower_sha256(
            evidence["source"]["cargo_lock_sha256"]
                .as_str()
                .unwrap_or_default(),
        )
        || evidence["authority"]["required_approvals"] != json!(["technical", "data_governance"])
        || evidence["seal"]["path"] != json!("SHA256SUMS")
        || evidence["seal"]["coverage_rule"] != json!("all_run_files_except_seal")
        || evidence["seal"]["entries"]
            .as_u64()
            .is_none_or(|value| value == 0)
    {
        return Err(XtaskError::integrity(
            "P0A_EVIDENCE_SHAPE_INVALID",
            "nested P0A evidence fields are malformed or violate the closed authority model",
        ));
    }
    Ok(())
}

fn validate_all_references(run_root: &Path, evidence: &Value) -> Result<()> {
    let expected_artifacts: BTreeSet<&str> = [
        "artifacts/0000-prototype-first-portable-interface.md",
        "artifacts/AGENTS.md",
        "artifacts/ARCHITECTURE.md",
        "artifacts/Cargo.lock",
        "artifacts/TODO-preapproval.md",
        "artifacts/decision-ledger-v2.md",
        "artifacts/p0-dependency.json",
        "artifacts/parser-boundary.json",
        "artifacts/rebuild-contract-v2.md",
        "artifacts/schema-bundle.json",
        "artifacts/source-identity.json",
    ]
    .into_iter()
    .collect();
    let artifacts = evidence
        .pointer("/artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XtaskError::integrity("P0A_ARTIFACT_REFS_INVALID", "artifacts is not an array")
        })?;
    if artifacts.len() != expected_artifacts.len() {
        return Err(XtaskError::integrity(
            "P0A_ARTIFACT_REFS_INVALID",
            "artifact reference count does not equal the closed P0A artifact set",
        ));
    }
    let mut observed = BTreeSet::new();
    for reference in artifacts {
        observed.insert(validate_file_reference(run_root, reference)?);
    }
    if observed != expected_artifacts {
        return Err(XtaskError::integrity(
            "P0A_ARTIFACT_REFS_INVALID",
            "artifact references do not equal the fixed P0A artifact set",
        ));
    }
    if validate_file_reference(run_root, &evidence["source"]["identity_ref"])?
        != "artifacts/source-identity.json"
        || validate_file_reference(run_root, &evidence["p0_dependency"]["reference"])?
            != "artifacts/p0-dependency.json"
    {
        return Err(XtaskError::integrity(
            "P0A_IDENTITY_REFERENCE_INVALID",
            "source or P0 dependency reference points to the wrong artifact",
        ));
    }
    let commands = evidence
        .pointer("/commands")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XtaskError::integrity("P0A_COMMAND_REFS_INVALID", "commands is not an array")
        })?;
    if commands.is_empty() {
        return Err(XtaskError::integrity(
            "P0A_COMMAND_REFS_INVALID",
            "P0A evidence contains no commands",
        ));
    }
    for (index, command) in commands.iter().enumerate() {
        require_object_keys(
            command,
            &[
                "id",
                "argv",
                "cwd",
                "exit_code",
                "status",
                "stdout",
                "stderr",
            ],
            "P0A_COMMAND_REFS_INVALID",
        )?;
        let expected_id = format!("C{:02}", index + 1);
        let argv = command.pointer("/argv").and_then(Value::as_array);
        if command.pointer("/id").and_then(Value::as_str) != Some(&expected_id)
            || command.pointer("/cwd").and_then(Value::as_str) != Some("${REPO}")
            || command.pointer("/exit_code").and_then(Value::as_i64) != Some(0)
            || command.pointer("/status").and_then(Value::as_str) != Some("PASS")
            || argv.is_none_or(|values| !valid_recorded_command_argv(values))
        {
            return Err(XtaskError::integrity(
                "P0A_COMMAND_REFS_INVALID",
                format!("malformed command {expected_id}"),
            ));
        }
        for stream in ["stdout", "stderr"] {
            let path = validate_file_reference(run_root, &command[stream])?;
            if path != format!("commands/{expected_id}.{stream}.txt") {
                return Err(XtaskError::integrity(
                    "P0A_COMMAND_REFS_INVALID",
                    format!("wrong {stream} path for {expected_id}"),
                ));
            }
        }
    }
    Ok(())
}

fn valid_recorded_command_argv(values: &[Value]) -> bool {
    let Some(argv) = values.iter().map(Value::as_str).collect::<Option<Vec<_>>>() else {
        return false;
    };
    if argv.first() == Some(&"git") {
        // Git invocations are produced only by Recorder's direct fixed binary boundary;
        // exclude mutation/config/network/subprocess features from retained qualification
        // evidence even if a future caller accidentally reaches that recorder surface.
        return argv.len() >= 2
            && !matches!(
                argv[1],
                "add"
                    | "am"
                    | "apply"
                    | "branch"
                    | "checkout"
                    | "cherry-pick"
                    | "clean"
                    | "clone"
                    | "commit"
                    | "config"
                    | "fetch"
                    | "gc"
                    | "init"
                    | "merge"
                    | "mv"
                    | "pull"
                    | "push"
                    | "rebase"
                    | "remote"
                    | "reset"
                    | "restore"
                    | "revert"
                    | "rm"
                    | "stash"
                    | "submodule"
                    | "switch"
                    | "tag"
                    | "worktree"
            )
            && !argv.iter().any(|argument| {
                argument.starts_with("--exec-path=")
                    || argument.starts_with("--git-dir=")
                    || argument.starts_with("--work-tree=")
                    || argument == &"-c"
                    || argument.starts_with("--config-env=")
                    || argument.starts_with("--upload-pack=")
                    || argument.starts_with("--receive-pack=")
            });
    }
    argv == [
        "cargo",
        "metadata",
        "--locked",
        "--offline",
        "--format-version",
        "1",
    ]
}

fn validate_source_binding(
    repository: &Path,
    recorder: &mut Recorder,
    run_root: &Path,
    evidence: &Value,
    commit: &str,
) -> Result<()> {
    if !valid_git_sha(commit) {
        return Err(XtaskError::integrity(
            "P0A_SOURCE_COMMIT_INVALID",
            "source commit is malformed",
        ));
    }
    recorder.git_success(
        repository,
        &["cat-file", "-e", &format!("{commit}^{{commit}}")],
        "P0A_SOURCE_COMMIT_MISSING",
    )?;
    recorder.git_success(
        repository,
        &["merge-base", "--is-ancestor", commit, "HEAD"],
        "P0A_SOURCE_NOT_ANCESTOR",
    )?;
    let tree = git_line(
        recorder,
        repository,
        &["rev-parse", &format!("{commit}^{{tree}}")],
        "P0A_SOURCE_TREE_INVALID",
    )?;
    if evidence.pointer("/source/tree").and_then(Value::as_str) != Some(tree.as_str()) {
        return Err(XtaskError::integrity(
            "P0A_SOURCE_TREE_INVALID",
            "evidence tree does not match its source commit",
        ));
    }
    let source_identity: Value = read_json_closed(
        &run_root.join("artifacts/source-identity.json"),
        "P0A_SOURCE_IDENTITY_JSON_INVALID",
    )?;
    require_object_keys(
        &source_identity,
        &[
            "schema",
            "commit",
            "tree",
            "branch",
            "dirty",
            "cargo_lock_sha256",
            "verifier_source_sha256",
            "schema_bundle_sha256",
        ],
        "P0A_SOURCE_IDENTITY_FIELDS_INVALID",
    )?;
    require_json_string(
        &source_identity,
        "/schema",
        "python-slm-p0a-source-identity-v1",
    )?;
    if source_identity["commit"] != evidence["source"]["commit"]
        || source_identity["tree"] != evidence["source"]["tree"]
        || source_identity["branch"] != evidence["source"]["branch"]
        || source_identity["cargo_lock_sha256"] != evidence["source"]["cargo_lock_sha256"]
        || source_identity["dirty"] != json!(false)
    {
        return Err(XtaskError::integrity(
            "P0A_SOURCE_IDENTITY_MISMATCH",
            "source artifact and evidence disagree",
        ));
    }
    for (source_path, artifact_path) in SNAPSHOTS {
        let blob = git_blob(recorder, repository, commit, source_path)?;
        let artifact_hash = hash::file(&run_root.join(artifact_path))?;
        if hash::bytes(&blob) != artifact_hash {
            return Err(XtaskError::integrity(
                "P0A_SOURCE_ARTIFACT_MISMATCH",
                format!("retained {artifact_path} does not equal {commit}:{source_path}"),
            ));
        }
    }
    let schema_bundle: Value = read_json_closed(
        &run_root.join("artifacts/schema-bundle.json"),
        "P0A_SCHEMA_BUNDLE_JSON_INVALID",
    )?;
    require_object_keys(
        &schema_bundle,
        &["schema", "entries", "bundle_sha256"],
        "P0A_SCHEMA_BUNDLE_FIELDS_INVALID",
    )?;
    require_json_string(&schema_bundle, "/schema", "python-slm-p0a-schema-bundle-v1")?;
    let entries = schema_bundle["entries"].as_array().ok_or_else(|| {
        XtaskError::integrity(
            "P0A_SCHEMA_BUNDLE_INVALID",
            "schema bundle entries is not an array",
        )
    })?;
    if entries.len() != 12 {
        return Err(XtaskError::integrity(
            "P0A_SCHEMA_BUNDLE_INVALID",
            "schema bundle does not contain 12 entries",
        ));
    }
    let mut manifest = String::new();
    for (index, entry) in entries.iter().enumerate() {
        require_object_keys(entry, &["path", "sha256"], "P0A_SCHEMA_BUNDLE_INVALID")?;
        let path = entry["path"].as_str().ok_or_else(|| {
            XtaskError::integrity("P0A_SCHEMA_BUNDLE_INVALID", "schema path missing")
        })?;
        let digest = entry["sha256"].as_str().ok_or_else(|| {
            XtaskError::integrity("P0A_SCHEMA_BUNDLE_INVALID", "schema hash missing")
        })?;
        if Some(path) != canonical_schema_paths().get(index).copied()
            || !hash::is_lower_sha256(digest)
        {
            return Err(XtaskError::integrity(
                "P0A_SCHEMA_BUNDLE_INVALID",
                "schema entries do not equal the exact ordered P0A/portable inventory",
            ));
        }
        let blob = git_blob(recorder, repository, commit, path)?;
        if hash::bytes(&blob) != digest {
            return Err(XtaskError::integrity(
                "P0A_SCHEMA_SOURCE_MISMATCH",
                format!("schema hash does not match {commit}:{path}"),
            ));
        }
        manifest.push_str(digest);
        manifest.push_str("  ");
        manifest.push_str(path);
        manifest.push('\n');
    }
    let bundle_sha = hash::bytes(manifest.as_bytes());
    if schema_bundle["bundle_sha256"] != json!(bundle_sha)
        || source_identity["schema_bundle_sha256"] != schema_bundle["bundle_sha256"]
    {
        return Err(XtaskError::integrity(
            "P0A_SCHEMA_BUNDLE_HASH_INVALID",
            "schema bundle hash does not recompute",
        ));
    }
    let verifier_sha = verifier_bundle_hash_from_commit(repository, recorder, commit)?;
    if source_identity["verifier_source_sha256"] != json!(verifier_sha) {
        return Err(XtaskError::integrity(
            "P0A_VERIFIER_SOURCE_HASH_INVALID",
            "verifier source bundle hash does not recompute",
        ));
    }
    let cargo_lock = git_blob(recorder, repository, commit, "Cargo.lock")?;
    let cargo_lock_sha = hash::bytes(&cargo_lock);
    if evidence["source"]["cargo_lock_sha256"] != json!(cargo_lock_sha)
        || source_identity["cargo_lock_sha256"] != evidence["source"]["cargo_lock_sha256"]
    {
        return Err(XtaskError::integrity(
            "P0A_CARGO_LOCK_BINDING_INVALID",
            "source identity does not bind the committed Cargo.lock bytes",
        ));
    }
    require_locked_parser_packages(&cargo_lock)?;
    let compatibility_blob = git_blob(
        recorder,
        repository,
        commit,
        "docs/schemas/portable-v2/tree-sitter-python-compatibility-v1.json",
    )?;
    let expected_parser = expected_parser_boundary(hash::bytes(&compatibility_blob));
    let retained_parser: Value = read_json_closed(
        &run_root.join("artifacts/parser-boundary.json"),
        "P0A_PARSER_BOUNDARY_JSON_INVALID",
    )?;
    if retained_parser != expected_parser {
        return Err(XtaskError::integrity(
            "P0A_PARSER_BOUNDARY_INVALID",
            "retained parser boundary does not equal the exact pinned native parser model",
        ));
    }
    validate_p0_dependency_artifact(run_root, commit)?;
    Ok(())
}

fn validate_p0_dependency_artifact(run_root: &Path, commit: &str) -> Result<()> {
    let value: Value = read_json_closed(
        &run_root.join("artifacts/p0-dependency.json"),
        "P0A_P0_DEPENDENCY_JSON_INVALID",
    )?;
    require_object_keys(
        &value,
        &[
            "schema",
            "status",
            "baseline_commit",
            "receipt_commit",
            "receipt_sha256",
            "contract_sha256",
            "decision_ledger_sha256",
            "reconciliation_commit",
            "reconciliation_tree",
            "run_id",
            "seal_sha256",
            "historical_cargo_lock_sha256",
            "oracle_commit",
            "oracle_normalized_sha256",
            "verified_at_source_commit",
        ],
        "P0A_P0_DEPENDENCY_FIELDS_INVALID",
    )?;
    for (pointer, expected) in [
        ("/schema", "python-slm-p0-dependency-v1"),
        ("/status", "PASS"),
        ("/baseline_commit", p0::BASELINE_COMMIT),
        ("/receipt_commit", p0::RECEIPT_COMMIT),
        ("/receipt_sha256", p0::RECEIPT_SHA256),
        ("/contract_sha256", p0::CONTRACT_SHA256),
        ("/decision_ledger_sha256", p0::LEDGER_SHA256),
        ("/reconciliation_commit", p0::RECONCILIATION_COMMIT),
        ("/reconciliation_tree", p0::RECONCILIATION_TREE),
        ("/run_id", p0::RUN_ID),
        ("/seal_sha256", p0::SEAL_SHA256),
        (
            "/historical_cargo_lock_sha256",
            p0::HISTORICAL_CARGO_LOCK_SHA256,
        ),
        ("/oracle_commit", p0::ORACLE_COMMIT),
        ("/oracle_normalized_sha256", p0::ORACLE_NORMALIZED_SHA256),
        ("/verified_at_source_commit", commit),
    ] {
        require_json_string(&value, pointer, expected)?;
    }
    Ok(())
}

fn verifier_bundle_hash_from_commit(
    repository: &Path,
    recorder: &mut Recorder,
    commit: &str,
) -> Result<String> {
    let paths = recorder.git_text(
        repository,
        &[
            "ls-tree",
            "-r",
            "--name-only",
            commit,
            "--",
            "xtask/Cargo.toml",
            "xtask/src",
        ],
        "P0A_VERIFIER_TREE_INVALID",
    )?;
    let mut paths: Vec<&str> = paths.lines().filter(|line| !line.is_empty()).collect();
    paths.sort();
    if paths.is_empty() {
        return Err(XtaskError::integrity(
            "P0A_VERIFIER_TREE_INVALID",
            "source commit has no xtask verifier files",
        ));
    }
    let mut manifest = String::new();
    for path in paths {
        let blob = git_blob(recorder, repository, commit, path)?;
        manifest.push_str(&hash::bytes(&blob));
        manifest.push_str("  ");
        manifest.push_str(path);
        manifest.push('\n');
    }
    Ok(hash::bytes(manifest.as_bytes()))
}

fn git_blob(
    recorder: &mut Recorder,
    repository: &Path,
    commit: &str,
    path: &str,
) -> Result<Vec<u8>> {
    let output = recorder.git_success(
        repository,
        &["show", &format!("{commit}:{path}")],
        "P0A_SOURCE_BLOB_MISSING",
    )?;
    Ok(output.stdout)
}

fn validate_file_reference<'a>(run_root: &Path, value: &'a Value) -> Result<&'a str> {
    require_object_keys(
        value,
        &["path", "sha256", "bytes"],
        "P0A_FILE_REFERENCE_INVALID",
    )?;
    let path = value
        .pointer("/path")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P0A_FILE_REFERENCE_INVALID",
                "file reference path is missing",
            )
        })?;
    let expected = publication::file_ref_from(run_root, path)?;
    if value.pointer("/sha256").and_then(Value::as_str) != Some(expected.sha256.as_str())
        || value.pointer("/bytes").and_then(Value::as_u64) != Some(expected.bytes)
    {
        return Err(XtaskError::integrity(
            "P0A_FILE_REFERENCE_INVALID",
            format!("file reference does not match retained bytes: {path}"),
        ));
    }
    Ok(path)
}

fn load_latest_approval_pair(
    output_root: &Path,
) -> Result<(u32, PathBuf, Approval, PathBuf, Approval)> {
    let directory = output_root.join("approvals");
    let mut technical = BTreeMap::new();
    let mut governance = BTreeMap::new();
    for entry in fs::read_dir(&directory).io_context(
        "P0A_APPROVALS_MISSING",
        "P0A approval directory is absent; agents cannot supply governance decisions",
    )? {
        let entry = entry.io_context(
            "P0A_APPROVAL_READ_FAILED",
            "could not read approval directory entry",
        )?;
        if !entry
            .file_type()
            .io_context(
                "P0A_APPROVAL_READ_FAILED",
                "could not inspect approval entry",
            )?
            .is_file()
        {
            return Err(XtaskError::integrity(
                "P0A_APPROVAL_TREE_INVALID",
                "approval directory contains a non-file",
            ));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let (role, digits) = if let Some(value) = name
            .strip_prefix("technical-")
            .and_then(|v| v.strip_suffix(".json"))
        {
            ("technical", value)
        } else if let Some(value) = name
            .strip_prefix("data-governance-")
            .and_then(|v| v.strip_suffix(".json"))
        {
            ("data_governance", value)
        } else {
            return Err(XtaskError::integrity(
                "P0A_APPROVAL_PATH_INVALID",
                format!("unexpected approval file {name}"),
            ));
        };
        if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(XtaskError::integrity(
                "P0A_APPROVAL_PATH_INVALID",
                format!("malformed approval sequence {name}"),
            ));
        }
        let sequence: u32 = digits.parse().expect("eight digits parse");
        if sequence == 0 {
            return Err(XtaskError::integrity(
                "P0A_APPROVAL_PATH_INVALID",
                "approval attempt sequence zero is reserved",
            ));
        }
        let approval: Approval = read_json_closed(&entry.path(), "P0A_APPROVAL_JSON_INVALID")?;
        let target = if role == "technical" {
            &mut technical
        } else {
            &mut governance
        };
        if target.insert(sequence, (entry.path(), approval)).is_some() {
            return Err(XtaskError::integrity(
                "P0A_APPROVAL_DUPLICATE",
                format!("duplicate {role} approval sequence"),
            ));
        }
    }
    let technical_sequences: Vec<u32> = technical.keys().copied().collect();
    let governance_sequences: Vec<u32> = governance.keys().copied().collect();
    if technical_sequences != governance_sequences
        || technical_sequences
            .iter()
            .enumerate()
            .any(|(index, sequence)| *sequence != index as u32 + 1)
    {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_SEQUENCE_INVALID",
            "approval attempts must be complete paired sequences beginning at one",
        ));
    }
    let sequence = *technical_sequences.last().ok_or_else(|| {
        XtaskError::gate(
            "P0A_OWNER_APPROVALS_REQUIRED",
            "both technical and data-governance approval JSONs are required",
            "Have the named owners create separate signed approval records for the sealed run.",
        )
    })?;
    let (technical_path, technical_approval) = technical.remove(&sequence).unwrap();
    let (governance_path, governance_approval) = governance.remove(&sequence).unwrap();
    Ok((
        sequence,
        technical_path,
        technical_approval,
        governance_path,
        governance_approval,
    ))
}

fn validate_approval_pair(technical: &Approval, governance: &Approval) -> Result<()> {
    validate_approval(technical, "technical")?;
    validate_approval(governance, "data_governance")?;
    if technical.run_id != governance.run_id
        || technical.run_evidence_sha256 != governance.run_evidence_sha256
        || technical.seal_sha256 != governance.seal_sha256
    {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_PAIR_MISMATCH",
            "the two approvals bind different runs",
        ));
    }
    if technical.owner_identity == governance.owner_identity
        && !(technical.explicit_dual_role_authority && governance.explicit_dual_role_authority)
    {
        return Err(XtaskError::gate(
            "P0A_DUAL_ROLE_AUTHORITY_MISSING",
            "one owner signed both roles without explicit dual authority in both decisions",
            "Use distinct authorized owners or record dual authority in both separate decisions.",
        ));
    }
    Ok(())
}

fn validate_approval(approval: &Approval, role: &str) -> Result<()> {
    if approval.schema != "python-slm-p0a-approval-v1"
        || approval.phase_id != "P0A"
        || approval.role != role
        || approval.decision != "APPROVE"
        || approval.owner_identity.trim() != approval.owner_identity
        || approval.owner_identity.is_empty()
        || approval.review_reference.trim() != approval.review_reference
        || approval.review_reference.is_empty()
        || !valid_run_id(&approval.run_id)
        || !hash::is_lower_sha256(&approval.run_evidence_sha256)
        || !hash::is_lower_sha256(&approval.seal_sha256)
        || !valid_utc_timestamp(&approval.utc_timestamp)
    {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_INVALID",
            format!("malformed or non-approving {role} decision"),
        ));
    }
    Ok(())
}

fn require_approval_commit(
    repository: &Path,
    recorder: &mut Recorder,
    approval_commit: &str,
    technical_path: &Path,
    governance_path: &Path,
    preapproval_commit: &str,
) -> Result<()> {
    recorder.git_success(
        repository,
        &["merge-base", "--is-ancestor", approval_commit, "HEAD"],
        "P0A_APPROVAL_COMMIT_NOT_ANCESTOR",
    )?;
    recorder.git_success(
        repository,
        &[
            "merge-base",
            "--is-ancestor",
            preapproval_commit,
            approval_commit,
        ],
        "P0A_PREAPPROVAL_NOT_ANCESTOR",
    )?;
    let parents = recorder.git_text(
        repository,
        &["rev-list", "--parents", "-n", "1", approval_commit],
        "P0A_APPROVAL_PARENT_INVALID",
    )?;
    let parent_tokens: Vec<&str> = parents.split_whitespace().collect();
    if parent_tokens.len() != 2 || parent_tokens[0] != approval_commit {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_PARENT_INVALID",
            "the approval commit must have exactly one parent",
        ));
    }
    let parent = parent_tokens[1].to_owned();
    let changed = recorder.git_text(
        repository,
        &["diff", "--name-status", &parent, approval_commit],
        "P0A_APPROVAL_COMMIT_DIFF_FAILED",
    )?;
    let actual: BTreeSet<String> = changed
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.strip_prefix("A\t").unwrap_or(line).to_owned())
        .collect();
    let expected: BTreeSet<String> = [
        slash_relative(repository, technical_path)?,
        slash_relative(repository, governance_path)?,
    ]
    .into_iter()
    .collect();
    if actual != expected || changed.lines().any(|line| !line.starts_with("A\t")) {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_COMMIT_SCOPE_INVALID",
            "the approval commit must create exactly the two structured approval JSON files",
        ));
    }
    for path in expected {
        let parent_probe =
            recorder.run_git(repository, &["cat-file", "-e", &format!("{parent}:{path}")])?;
        if parent_probe.status.success() {
            return Err(XtaskError::integrity(
                "P0A_APPROVAL_NOT_CREATE_NEW",
                format!("approval path already existed in the parent commit: {path}"),
            ));
        }
        let committed = git_blob(recorder, repository, approval_commit, &path)?;
        let live = fs::read(repository.join(&path))
            .io_context("P0A_APPROVAL_READ_FAILED", format!("could not read {path}"))?;
        if committed != live {
            return Err(XtaskError::integrity(
                "P0A_APPROVAL_COMMIT_BINDING_INVALID",
                format!("live approval bytes differ from {approval_commit}:{path}"),
            ));
        }
    }
    Ok(())
}

fn require_repository_clean(repository: &Path, recorder: &mut Recorder) -> Result<()> {
    let status = recorder.git_text(
        repository,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        "REPOSITORY_STATUS_FAILED",
    )?;
    if !status.trim().is_empty() {
        return Err(XtaskError::integrity(
            "REPOSITORY_DIRTY",
            "the repository must be completely clean for owner-finalization validation",
        ));
    }
    Ok(())
}

fn next_acceptance_sequence(output_root: &Path) -> Result<u32> {
    let directory = output_root.join("acceptances");
    let mut sequences = BTreeSet::new();
    if directory.exists() {
        for entry in fs::read_dir(&directory).io_context(
            "P0A_ACCEPTANCE_ENUMERATION_FAILED",
            "could not enumerate acceptances",
        )? {
            let entry = entry.io_context(
                "P0A_ACCEPTANCE_ENUMERATION_FAILED",
                "could not read acceptance entry",
            )?;
            if !entry
                .file_type()
                .io_context(
                    "P0A_ACCEPTANCE_ENUMERATION_FAILED",
                    "could not inspect acceptance entry",
                )?
                .is_file()
            {
                return Err(XtaskError::integrity(
                    "P0A_ACCEPTANCE_PATH_INVALID",
                    "acceptance directory contains a non-file",
                ));
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let sequence = parse_acceptance_path(&format!("acceptances/{name}"))?;
            if !sequences.insert(sequence) {
                return Err(XtaskError::integrity(
                    "P0A_ACCEPTANCE_SEQUENCE_INVALID",
                    "acceptance directory contains a duplicate numeric sequence",
                ));
            }
        }
    }
    let previous = sequences.last().copied().unwrap_or(0);
    if sequences.len() != previous as usize
        || sequences
            .iter()
            .enumerate()
            .any(|(index, value)| *value != index as u32 + 1)
    {
        return Err(XtaskError::integrity(
            "P0A_ACCEPTANCE_SEQUENCE_GAP",
            "acceptance history must be a contiguous sequence beginning at one",
        ));
    }
    previous.checked_add(1).ok_or_else(|| {
        XtaskError::integrity(
            "P0A_ACCEPTANCE_SEQUENCE_EXHAUSTED",
            "acceptance sequence overflow",
        )
    })
}

fn parse_acceptance_path(value: &str) -> Result<u32> {
    let digits = value
        .strip_prefix("acceptances/")
        .and_then(|name| name.strip_suffix(".json"))
        .ok_or_else(|| {
            XtaskError::integrity(
                "P0A_ACCEPTANCE_PATH_INVALID",
                format!("malformed acceptance path {value}"),
            )
        })?;
    if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(XtaskError::integrity(
            "P0A_ACCEPTANCE_PATH_INVALID",
            format!("malformed acceptance path {value}"),
        ));
    }
    let sequence: u32 = digits.parse().expect("eight digits parse");
    if sequence == 0 {
        return Err(XtaskError::integrity(
            "P0A_ACCEPTANCE_PATH_INVALID",
            "acceptance sequence zero is reserved",
        ));
    }
    Ok(sequence)
}

fn load_previous_acceptance_hash(output_root: &Path, next: u32) -> Result<Option<String>> {
    if next == 1 {
        return Ok(None);
    }
    let path = output_root.join(format!("acceptances/{:08}.json", next - 1));
    Ok(Some(hash::file(&path)?))
}

fn require_selected_predecessor_for_new_acceptance(
    output_root: &Path,
    retained: Option<&Acceptance>,
    next: u32,
    approval_sequence: u32,
) -> Result<()> {
    if next == 1 {
        if retained.is_some() || output_root.join("evidence.json").exists() {
            return Err(XtaskError::integrity(
                "P0A_PUBLICATION_STATE_INDETERMINATE",
                "genesis acceptance has an unexpected retained predecessor or pointer",
            ));
        }
        return Ok(());
    }
    let predecessor = retained.ok_or_else(|| {
        XtaskError::integrity(
            "P0A_PUBLICATION_STATE_INDETERMINATE",
            "non-genesis acceptance has no retained predecessor",
        )
    })?;
    validate_acceptance_shape(predecessor)?;
    if predecessor.sequence + 1 != next {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_STATE_INDETERMINATE",
            "retained predecessor sequence does not precede the new acceptance",
        ));
    }
    require_approval_sequence_successor(
        approval_reference_sequence(&predecessor.approvals)?,
        approval_sequence,
    )?;
    let pointer = load_pointer(output_root)?;
    validate_selected_acceptance(output_root, &pointer, predecessor)?;
    hash::require_file(
        &output_root.join(&pointer.acceptance_path),
        &pointer.acceptance_sha256,
        "P0A_POINTER_ACCEPTANCE_HASH_MISMATCH",
    )?;
    Ok(())
}

fn select_matching_retained_acceptance(
    retained: Option<&Acceptance>,
    matches_current_approval: bool,
) -> Option<&Acceptance> {
    retained.filter(|_| matches_current_approval)
}

fn recover_acceptance_temporary(acceptance_path: &Path) -> Result<()> {
    let Some(parent) = acceptance_path.parent() else {
        return Err(XtaskError::integrity(
            "P0A_ACCEPTANCE_TEMP_RECOVERY_INVALID",
            "acceptance path has no parent",
        ));
    };
    if !parent.exists() {
        return Ok(());
    }
    let file_name = acceptance_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            XtaskError::integrity(
                "P0A_ACCEPTANCE_TEMP_RECOVERY_INVALID",
                "acceptance file name is not UTF-8",
            )
        })?;
    let prefix = format!("{file_name}.tmp-acceptance-");
    let mut candidate = None;
    for entry in fs::read_dir(parent).io_context(
        "P0A_ACCEPTANCE_TEMP_RECOVERY_FAILED",
        "could not enumerate acceptance temporaries",
    )? {
        let entry = entry.io_context(
            "P0A_ACCEPTANCE_TEMP_RECOVERY_FAILED",
            "could not read acceptance temporary entry",
        )?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&prefix) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).io_context(
            "P0A_ACCEPTANCE_TEMP_RECOVERY_FAILED",
            "could not inspect acceptance temporary",
        )?;
        if !valid_owned_temp_name(&name, &prefix)
            || metadata.file_type().is_symlink()
            || !metadata.is_file()
            || candidate.is_some()
        {
            return Err(XtaskError::integrity(
                "P0A_ACCEPTANCE_TEMP_RECOVERY_INVALID",
                "acceptance temporary is ambiguous or not an exact owned regular file",
            ));
        }
        candidate = Some(entry.path());
    }
    if let Some(temporary) = candidate {
        // The immutable final is either absent (pre-publication partial/full temp) or
        // already present (hard-link succeeded before temp cleanup). In both states the
        // owned temporary name is non-authoritative and safe to discard; the final is
        // never removed or overwritten here.
        fs::remove_file(temporary).io_context(
            "P0A_ACCEPTANCE_TEMP_RECOVERY_FAILED",
            "could not remove owned acceptance temporary",
        )?;
    }
    Ok(())
}

fn recover_all_acceptance_temporaries(output_root: &Path) -> Result<()> {
    let directory = output_root.join("acceptances");
    if !directory.exists() {
        return Ok(());
    }
    let mut finals = BTreeSet::new();
    for entry in fs::read_dir(&directory).io_context(
        "P0A_ACCEPTANCE_TEMP_RECOVERY_FAILED",
        "could not enumerate acceptance temporaries before sequence validation",
    )? {
        let entry = entry.io_context(
            "P0A_ACCEPTANCE_TEMP_RECOVERY_FAILED",
            "could not read acceptance temporary entry",
        )?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((base, _)) = name.split_once(".tmp-acceptance-") else {
            continue;
        };
        parse_acceptance_path(&format!("acceptances/{base}"))?;
        finals.insert(directory.join(base));
    }
    for path in finals {
        recover_acceptance_temporary(&path)?;
    }
    Ok(())
}

fn replace_pointer_atomically(path: &Path, pointer: &Pointer) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(pointer).map_err(|error| {
        XtaskError::new(
            "P0A_POINTER_SERIALIZATION_FAILED",
            crate::error::Category::Internal,
            format!("could not serialize pointer: {error}"),
            "Inspect the closed pointer model.",
        )
    })?;
    bytes.push(b'\n');
    let old = if path.exists() {
        Some(fs::read(path).io_context("P0A_POINTER_READ_FAILED", "could not read prior pointer")?)
    } else {
        None
    };
    let entropy = time::now().2;
    let temporary =
        path.with_extension(format!("json.tmp-pointer-{}-{entropy}", std::process::id()));
    publication::write_new(&temporary, &bytes)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(XtaskError::environment(
            "P0A_POINTER_REPLACE_FAILED",
            format!("could not atomically replace selected pointer: {error}"),
        ));
    }
    if fs::read(path).ok().as_deref() != Some(bytes.as_slice()) {
        if let Some(old) = old {
            let rollback = path.with_extension(format!("json.rollback-{}", std::process::id()));
            publication::write_new(&rollback, &old)?;
            fs::rename(&rollback, path).io_context(
                "P0A_POINTER_ROLLBACK_FAILED",
                "could not restore prior pointer",
            )?;
        } else {
            let _ = fs::remove_file(path);
        }
        return Err(XtaskError::integrity(
            "P0A_POINTER_REREAD_FAILED",
            "pointer bytes changed during atomic publication",
        ));
    }
    Ok(())
}

fn recover_pointer_temporary(output_root: &Path, expected: &Pointer) -> Result<()> {
    let expected_bytes = canonical_json_bytes(
        expected,
        "P0A_POINTER_SERIALIZATION_FAILED",
        "could not serialize recovery pointer",
    )?;
    let mut candidates = Vec::new();
    for entry in fs::read_dir(output_root).io_context(
        "P0A_POINTER_TEMP_RECOVERY_FAILED",
        "could not enumerate P0A root for pointer recovery",
    )? {
        let entry = entry.io_context(
            "P0A_POINTER_TEMP_RECOVERY_FAILED",
            "could not read P0A root entry during pointer recovery",
        )?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let prefix = "evidence.json.tmp-pointer-";
        if !name.starts_with(prefix) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).io_context(
            "P0A_POINTER_TEMP_RECOVERY_FAILED",
            "could not inspect pointer temporary",
        )?;
        if !valid_owned_temp_name(&name, prefix)
            || metadata.file_type().is_symlink()
            || !metadata.is_file()
        {
            return Err(XtaskError::integrity(
                "P0A_POINTER_TEMP_RECOVERY_INVALID",
                "pointer temporary has an ambiguous name or file type",
            ));
        }
        let bytes = fs::read(entry.path()).io_context(
            "P0A_POINTER_TEMP_RECOVERY_FAILED",
            "could not read pointer temporary",
        )?;
        candidates.push((entry.path(), bytes == expected_bytes));
    }
    if candidates.len() > 1 {
        return Err(XtaskError::integrity(
            "P0A_POINTER_TEMP_RECOVERY_AMBIGUOUS",
            "multiple exact pointer temporaries require owner inspection",
        ));
    }
    let Some((temporary, temporary_is_complete)) = candidates.pop() else {
        return Ok(());
    };
    let pointer_path = output_root.join("evidence.json");
    let expected_sequence = parse_acceptance_path(&expected.acceptance_path)?;
    if pointer_path.is_file() {
        let observed_bytes = fs::read(&pointer_path)
            .io_context("P0A_POINTER_READ_FAILED", "could not read selected pointer")?;
        if observed_bytes == expected_bytes {
            fs::remove_file(&temporary).io_context(
                "P0A_POINTER_TEMP_RECOVERY_FAILED",
                "could not remove redundant exact pointer temporary",
            )?;
            return Ok(());
        }
        let observed = load_pointer(output_root)?;
        let observed_sequence = parse_acceptance_path(&observed.acceptance_path)?;
        if expected_sequence <= 1 || observed_sequence + 1 != expected_sequence {
            return Err(XtaskError::integrity(
                "P0A_POINTER_TEMP_RECOVERY_INVALID",
                "selected pointer is not the exact predecessor of the recovered temporary",
            ));
        }
        hash::require_file(
            &output_root.join(&observed.acceptance_path),
            &observed.acceptance_sha256,
            "P0A_PREDECESSOR_POINTER_HASH_MISMATCH",
        )?;
    } else if expected_sequence != 1 {
        return Err(XtaskError::integrity(
            "P0A_POINTER_TEMP_RECOVERY_INVALID",
            "non-genesis pointer temporary has no selected predecessor",
        ));
    }
    if !temporary_is_complete {
        fs::remove_file(&temporary).io_context(
            "P0A_POINTER_TEMP_RECOVERY_FAILED",
            "could not discard partial owned pointer temporary",
        )?;
        return Ok(());
    }
    fs::rename(&temporary, &pointer_path).io_context(
        "P0A_POINTER_TEMP_RECOVERY_FAILED",
        "could not complete the exact interrupted pointer replacement",
    )?;
    if fs::read(&pointer_path).ok().as_deref() != Some(expected_bytes.as_slice()) {
        return Err(XtaskError::integrity(
            "P0A_POINTER_TEMP_RECOVERY_INVALID",
            "recovered pointer bytes changed during atomic replacement",
        ));
    }
    Ok(())
}

fn load_pointer(output_root: &Path) -> Result<Pointer> {
    let pointer: Pointer = read_json_closed(
        &output_root.join("evidence.json"),
        "P0A_POINTER_JSON_INVALID",
    )?;
    validate_pointer_shape(&pointer)?;
    Ok(pointer)
}

fn validate_pointer_shape(pointer: &Pointer) -> Result<()> {
    if pointer.schema != "python-slm-p0a-phase-pointer-v1"
        || pointer.phase_id != "P0A"
        || pointer.interface_id != INTERFACE_ID
        || pointer.profile_id != PROFILE_ID
        || parse_acceptance_path(&pointer.acceptance_path).is_err()
        || !hash::is_lower_sha256(&pointer.acceptance_sha256)
        || !valid_utc_timestamp(&pointer.updated_at)
    {
        return Err(XtaskError::integrity(
            "P0A_POINTER_INVALID",
            "selected P0A pointer is malformed",
        ));
    }
    Ok(())
}

fn validate_selected_acceptance(
    output_root: &Path,
    pointer: &Pointer,
    acceptance: &Acceptance,
) -> Result<()> {
    let pointer_sequence = parse_acceptance_path(&pointer.acceptance_path)?;
    if pointer_sequence != acceptance.sequence {
        return Err(XtaskError::integrity(
            "P0A_POINTER_SEQUENCE_MISMATCH",
            "selected pointer path does not equal the retained acceptance sequence",
        ));
    }
    if next_acceptance_sequence(output_root)? != acceptance.sequence + 1 {
        return Err(XtaskError::integrity(
            "P0A_POINTER_ROLLBACK",
            "selected pointer does not identify the greatest retained acceptance sequence",
        ));
    }
    Ok(())
}

fn validate_acceptance_shape(acceptance: &Acceptance) -> Result<()> {
    let run_id = acceptance.run_path.strip_prefix("runs/");
    let run_path_valid = run_id.is_some_and(valid_run_id)
        && acceptance.run_path == format!("runs/{}", run_id.unwrap());
    let expected_seal = format!("{}/SHA256SUMS", acceptance.run_path);
    let approval_sequence = approval_reference_sequence(&acceptance.approvals).ok();
    if acceptance.schema != "python-slm-p0a-phase-acceptance-v1"
        || acceptance.phase_id != "P0A"
        || acceptance.interface_id != INTERFACE_ID
        || acceptance.profile_id != PROFILE_ID
        || acceptance.sequence == 0
        || acceptance.status != "PASS"
        || acceptance.acceptance_kind != "owner_approved_contract_amendment"
        || !run_path_valid
        || acceptance.seal_path != expected_seal
        || !hash::is_lower_sha256(&acceptance.run_evidence_sha256)
        || !hash::is_lower_sha256(&acceptance.seal_sha256)
        || acceptance.approvals.len() != 2
        || acceptance.approvals[0].role != "technical"
        || acceptance.approvals[0].decision != "APPROVE"
        || !hash::is_lower_sha256(&acceptance.approvals[0].sha256)
        || acceptance.approvals[1].role != "data_governance"
        || acceptance.approvals[1].decision != "APPROVE"
        || !hash::is_lower_sha256(&acceptance.approvals[1].sha256)
        || approval_sequence.is_none()
        || !valid_git_sha(&acceptance.approval_commit)
        || !valid_git_sha(&acceptance.preapproval_commit)
        || !hash::is_lower_sha256(&acceptance.todo_preapproval_sha256)
        || acceptance
            .previous_acceptance_sha256
            .as_deref()
            .is_some_and(|value| !hash::is_lower_sha256(value))
        || !valid_utc_timestamp(&acceptance.created_at)
    {
        return Err(XtaskError::integrity(
            "P0A_ACCEPTANCE_INVALID",
            "P0A acceptance is malformed",
        ));
    }
    Ok(())
}

fn parse_approval_path(value: &str, role: &str) -> Result<u32> {
    let prefix = match role {
        "technical" => "approvals/technical-",
        "data_governance" => "approvals/data-governance-",
        _ => {
            return Err(XtaskError::integrity(
                "P0A_APPROVAL_PATH_INVALID",
                "approval role is not recognized",
            ));
        }
    };
    let digits = value
        .strip_prefix(prefix)
        .and_then(|name| name.strip_suffix(".json"))
        .ok_or_else(|| {
            XtaskError::integrity(
                "P0A_APPROVAL_PATH_INVALID",
                format!("malformed {role} approval path {value}"),
            )
        })?;
    if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_PATH_INVALID",
            format!("malformed {role} approval path {value}"),
        ));
    }
    let sequence: u32 = digits.parse().expect("eight digits parse");
    if sequence == 0 {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_PATH_INVALID",
            "approval attempt sequence zero is reserved",
        ));
    }
    Ok(sequence)
}

fn approval_reference_sequence(approvals: &[ApprovalRef]) -> Result<u32> {
    if approvals.len() != 2 {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_PATH_INVALID",
            "exactly two approval references are required",
        ));
    }
    let technical = parse_approval_path(&approvals[0].path, "technical")?;
    let governance = parse_approval_path(&approvals[1].path, "data_governance")?;
    if technical != governance {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_SEQUENCE_INVALID",
            "technical and data-governance approvals use different attempt sequences",
        ));
    }
    Ok(technical)
}

fn require_approval_sequence_successor(previous: u32, current: u32) -> Result<()> {
    if previous >= current {
        return Err(XtaskError::integrity(
            "P0A_APPROVAL_SEQUENCE_REPLAY",
            "acceptance chain reuses or rolls back an approval attempt sequence",
        ));
    }
    Ok(())
}

fn validate_acceptance_approvals(output_root: &Path, acceptance: &Acceptance) -> Result<()> {
    let mut approvals = Vec::new();
    for reference in &acceptance.approvals {
        let path = output_root.join(reference.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        hash::require_file(
            &path,
            &reference.sha256,
            "P0A_APPROVAL_REFERENCE_HASH_MISMATCH",
        )?;
        approvals.push(read_json_closed::<Approval>(
            &path,
            "P0A_APPROVAL_JSON_INVALID",
        )?);
    }
    validate_approval_pair(&approvals[0], &approvals[1])?;
    if approvals.iter().any(|approval| {
        approval.run_id != acceptance.run_path.trim_start_matches("runs/")
            || approval.run_evidence_sha256 != acceptance.run_evidence_sha256
            || approval.seal_sha256 != acceptance.seal_sha256
    }) {
        return Err(XtaskError::integrity(
            "P0A_ACCEPTANCE_APPROVAL_BINDING_INVALID",
            "acceptance approvals do not bind its run",
        ));
    }
    Ok(())
}

fn validate_acceptance_chain(
    repository: &Path,
    recorder: &mut Recorder,
    output_root: &Path,
    selected: &Acceptance,
) -> Result<()> {
    let mut current = selected.clone();
    loop {
        let expected_name = format!("acceptances/{:08}.json", current.sequence);
        let path = output_root.join(&expected_name);
        if !path.is_file() {
            return Err(XtaskError::integrity(
                "P0A_ACCEPTANCE_CHAIN_MISSING",
                format!("acceptance chain is missing {expected_name}"),
            ));
        }
        validate_acceptance_shape(&current)?;
        validate_acceptance_approvals(output_root, &current)?;
        let technical_path = output_root.join(
            current.approvals[0]
                .path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        let governance_path = output_root.join(
            current.approvals[1]
                .path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        require_approval_commit(
            repository,
            recorder,
            &current.approval_commit,
            &technical_path,
            &governance_path,
            &current.preapproval_commit,
        )?;
        for (relative, expected) in [
            (
                format!("{OUTPUT_ROOT}/{}/evidence.json", current.run_path),
                current.run_evidence_sha256.as_str(),
            ),
            (
                format!("{OUTPUT_ROOT}/{}", current.seal_path),
                current.seal_sha256.as_str(),
            ),
        ] {
            let committed = git_blob(recorder, repository, &current.approval_commit, &relative)?;
            if hash::bytes(&committed) != expected {
                return Err(XtaskError::integrity(
                    "P0A_APPROVAL_RUN_COMMIT_BINDING_INVALID",
                    format!("approval commit does not contain the selected run bytes: {relative}"),
                ));
            }
        }
        let run = validate_run(repository, recorder, &output_root.join(&current.run_path))?;
        if run.evidence_sha256 != current.run_evidence_sha256
            || run.seal_sha256 != current.seal_sha256
            || run.preapproval_commit != current.preapproval_commit
            || run.todo_preapproval_sha256 != current.todo_preapproval_sha256
        {
            return Err(XtaskError::integrity(
                "P0A_ACCEPTANCE_CHAIN_BINDING_INVALID",
                format!("{expected_name} does not bind its retained run"),
            ));
        }
        if current.sequence == 1 {
            if current.previous_acceptance_sha256.is_some() {
                return Err(XtaskError::integrity(
                    "P0A_ACCEPTANCE_CHAIN_GENESIS_INVALID",
                    "first P0A acceptance must have a null predecessor",
                ));
            }
            break;
        }
        let previous_path =
            output_root.join(format!("acceptances/{:08}.json", current.sequence - 1));
        let expected_previous = current
            .previous_acceptance_sha256
            .as_deref()
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P0A_ACCEPTANCE_CHAIN_LINK_MISSING",
                    format!("{expected_name} has no predecessor hash"),
                )
            })?;
        hash::require_file(
            &previous_path,
            expected_previous,
            "P0A_ACCEPTANCE_CHAIN_HASH_MISMATCH",
        )?;
        let previous: Acceptance = read_json_closed(&previous_path, "P0A_ACCEPTANCE_JSON_INVALID")?;
        validate_acceptance_shape(&previous)?;
        let current_approval_sequence = approval_reference_sequence(&current.approvals)?;
        let previous_approval_sequence = approval_reference_sequence(&previous.approvals)?;
        require_approval_sequence_successor(previous_approval_sequence, current_approval_sequence)?;
        current = previous;
    }
    Ok(())
}

fn validate_checkbox_commit(
    repository: &Path,
    recorder: &mut Recorder,
    output_root: &Path,
    pointer: &Pointer,
    acceptance: &Acceptance,
) -> Result<()> {
    let head = git_line(
        recorder,
        repository,
        &["rev-parse", "HEAD"],
        "P0A_CHECKBOX_COMMIT_INVALID",
    )?;
    let acceptance_repo_path = format!("{OUTPUT_ROOT}/{}", pointer.acceptance_path);
    let publication_commits = recorder.git_text(
        repository,
        &[
            "log",
            "--format=%H",
            "--diff-filter=A",
            "--no-renames",
            "HEAD",
            "--",
            &acceptance_repo_path,
        ],
        "P0A_PUBLICATION_COMMIT_LOOKUP_FAILED",
    )?;
    let publication_commits: Vec<&str> = publication_commits
        .lines()
        .filter(|line| !line.is_empty())
        .collect();
    if publication_commits.len() != 1 || !valid_git_sha(publication_commits[0]) {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_COMMIT_INVALID",
            "selected acceptance must be created by one unique publication commit",
        ));
    }
    let publication_commit = publication_commits[0];
    let publication_parents = recorder.git_text(
        repository,
        &["rev-list", "--parents", "-n", "1", publication_commit],
        "P0A_PUBLICATION_PARENT_INVALID",
    )?;
    let parent_tokens: Vec<&str> = publication_parents.split_whitespace().collect();
    if parent_tokens.len() != 2 || parent_tokens[1] != acceptance.approval_commit {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_PARENT_INVALID",
            "publication commit must be the single-parent child of the bound approval commit",
        ));
    }
    let publication_changed = recorder.git_text(
        repository,
        &[
            "diff",
            "--name-status",
            "--no-renames",
            &acceptance.approval_commit,
            publication_commit,
        ],
        "P0A_PUBLICATION_COMMIT_DIFF_FAILED",
    )?;
    let pointer_status = if acceptance.sequence == 1 { "A" } else { "M" };
    let expected_publication: BTreeSet<String> = [
        format!("A\t{acceptance_repo_path}"),
        format!("{pointer_status}\t{OUTPUT_ROOT}/evidence.json"),
    ]
    .into_iter()
    .collect();
    let observed_publication: BTreeSet<String> = publication_changed
        .lines()
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if observed_publication != expected_publication {
        return Err(XtaskError::integrity(
            "P0A_PUBLICATION_COMMIT_SCOPE_INVALID",
            "publication commit must change exactly the create-new acceptance and selected pointer",
        ));
    }
    let ancestry = recorder.git_text(
        repository,
        &[
            "rev-list",
            "--parents",
            &format!("{publication_commit}..{head}"),
        ],
        "P0A_CHECKBOX_COMMIT_LOOKUP_FAILED",
    )?;
    let checkbox_candidates: Vec<&str> = ancestry
        .lines()
        .filter_map(|line| {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            (tokens.len() == 2 && tokens[1] == publication_commit).then_some(tokens[0])
        })
        .collect();
    if checkbox_candidates.len() != 1 {
        return Err(XtaskError::integrity(
            "P0A_CHECKBOX_COMMIT_INVALID",
            "exactly one checkbox closure commit must be the direct child of publication",
        ));
    }
    let checkbox_commit = checkbox_candidates[0];
    let changed = recorder.git_text(
        repository,
        &["diff", "--name-only", publication_commit, checkbox_commit],
        "P0A_CHECKBOX_DIFF_FAILED",
    )?;
    if changed
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        != ["TODO.md"]
    {
        return Err(XtaskError::integrity(
            "P0A_CHECKBOX_COMMIT_SCOPE_INVALID",
            "the final P0A closure commit must change only TODO.md",
        ));
    }
    let before = recorder.git_success(
        repository,
        &["show", &format!("{publication_commit}:TODO.md")],
        "P0A_CHECKBOX_PARENT_TODO_MISSING",
    )?;
    let after = recorder.git_success(
        repository,
        &["show", &format!("{checkbox_commit}:TODO.md")],
        "P0A_CHECKBOX_HEAD_TODO_MISSING",
    )?;
    let before_text = std::str::from_utf8(&before.stdout).map_err(|_| {
        XtaskError::integrity(
            "P0A_CHECKBOX_ENCODING_INVALID",
            "parent TODO.md is not UTF-8",
        )
    })?;
    let after_text = std::str::from_utf8(&after.stdout).map_err(|_| {
        XtaskError::integrity(
            "P0A_CHECKBOX_ENCODING_INVALID",
            "checked TODO.md is not UTF-8",
        )
    })?;
    if before_text.matches("- [ ] P0A complete").count() != 1
        || before_text.contains("- [x] P0A complete")
    {
        return Err(XtaskError::integrity(
            "P0A_CHECKBOX_PARENT_INVALID",
            "pointer commit does not have exactly one unchecked P0A checkbox",
        ));
    }
    let expected_after = before_text.replacen("- [ ] P0A complete", "- [x] P0A complete", 1);
    if after_text != expected_after {
        return Err(XtaskError::integrity(
            "P0A_CHECKBOX_DIFF_INVALID",
            "P0A closure is not the exact one-line unchecked-to-checked transition",
        ));
    }
    let preapproval_todo = recorder.git_success(
        repository,
        &[
            "show",
            &format!("{}:TODO.md", acceptance.preapproval_commit),
        ],
        "P0A_PREAPPROVAL_TODO_MISSING",
    )?;
    if hash::bytes(&preapproval_todo.stdout) != acceptance.todo_preapproval_sha256 {
        return Err(XtaskError::integrity(
            "P0A_PREAPPROVAL_TODO_HASH_MISMATCH",
            "acceptance does not bind the preapproval TODO blob",
        ));
    }
    if hash::bytes(&before.stdout) != acceptance.todo_preapproval_sha256 {
        return Err(XtaskError::integrity(
            "P0A_CHECKBOX_PARENT_TODO_MISMATCH",
            "publication commit TODO does not equal the sealed preapproval TODO bytes",
        ));
    }
    let pointer_from_parent = recorder.git_success(
        repository,
        &[
            "show",
            &format!("{publication_commit}:{OUTPUT_ROOT}/evidence.json"),
        ],
        "P0A_POINTER_NOT_COMMITTED_BEFORE_CHECKBOX",
    )?;
    let live_pointer = fs::read(output_root.join("evidence.json"))
        .io_context("P0A_POINTER_READ_FAILED", "could not read selected pointer")?;
    if pointer_from_parent.stdout != live_pointer {
        return Err(XtaskError::integrity(
            "P0A_POINTER_COMMIT_BINDING_INVALID",
            "checkbox parent does not contain the selected pointer bytes",
        ));
    }
    let acceptance_from_parent = recorder.git_success(
        repository,
        &[
            "show",
            &format!(
                "{publication_commit}:{OUTPUT_ROOT}/{}",
                pointer.acceptance_path
            ),
        ],
        "P0A_ACCEPTANCE_NOT_COMMITTED_BEFORE_CHECKBOX",
    )?;
    if hash::bytes(&acceptance_from_parent.stdout) != pointer.acceptance_sha256 {
        return Err(XtaskError::integrity(
            "P0A_ACCEPTANCE_COMMIT_BINDING_INVALID",
            "checkbox parent does not contain the selected acceptance bytes",
        ));
    }
    let live_todo = fs::read_to_string(repository.join("TODO.md")).io_context(
        "P0A_CHECKBOX_LIVE_TODO_MISSING",
        "could not read live TODO.md",
    )?;
    if live_todo.matches("- [x] P0A complete").count() != 1
        || live_todo.contains("- [ ] P0A complete")
    {
        return Err(XtaskError::integrity(
            "P0A_CHECKBOX_LIVE_STATE_INVALID",
            "current TODO no longer retains exactly one checked P0A closure marker",
        ));
    }
    Ok(())
}

fn read_json_closed<T: for<'de> Deserialize<'de>>(path: &Path, code: &'static str) -> Result<T> {
    let bytes = fs::read(path).io_context(code, format!("could not read {}", path.display()))?;
    if bytes.contains(&b'\r') || !bytes.ends_with(b"\n") {
        return Err(XtaskError::integrity(
            code,
            format!("{} is not canonical LF JSON", path.display()),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        XtaskError::integrity(code, format!("invalid JSON in {}: {error}", path.display()))
    })
}

fn require_object_keys(value: &Value, expected: &[&str], code: &'static str) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| XtaskError::integrity(code, "expected a JSON object"))?;
    let actual: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = expected.iter().copied().collect();
    if actual != expected {
        return Err(XtaskError::integrity(
            code,
            format!("closed field set differs: expected {expected:?}, observed {actual:?}"),
        ));
    }
    Ok(())
}

fn require_json_string(value: &Value, pointer: &str, expected: &str) -> Result<()> {
    if value.pointer(pointer).and_then(Value::as_str) != Some(expected) {
        return Err(XtaskError::integrity(
            "P0A_EVIDENCE_FIELD_INVALID",
            format!("field {pointer} does not equal {expected:?}"),
        ));
    }
    Ok(())
}

fn slash_relative(root: &Path, path: &Path) -> Result<String> {
    let canonical_root = fs::canonicalize(root).io_context(
        "P0A_PATH_CANONICALIZATION_FAILED",
        format!("could not canonicalize containment root {}", root.display()),
    )?;
    let canonical_path = fs::canonicalize(path).io_context(
        "P0A_PATH_CANONICALIZATION_FAILED",
        format!("could not canonicalize retained path {}", path.display()),
    )?;
    canonical_path
        .strip_prefix(&canonical_root)
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .map_err(|_| {
            XtaskError::integrity(
                "P0A_PATH_CONTAINMENT_INVALID",
                format!(
                    "{} is outside {}",
                    canonical_path.display(),
                    canonical_root.display()
                ),
            )
        })
}

fn valid_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_run_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 44
        && bytes[8] == b'T'
        && bytes[18] == b'Z'
        && bytes[19] == b'-'
        && bytes[..8].iter().all(u8::is_ascii_digit)
        && bytes[9..18].iter().all(u8::is_ascii_digit)
        && bytes[20..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        && valid_calendar_components(
            parse_digits(&bytes[..4]),
            parse_digits(&bytes[4..6]),
            parse_digits(&bytes[6..8]),
            parse_digits(&bytes[9..11]),
            parse_digits(&bytes[11..13]),
            parse_digits(&bytes[13..15]),
        )
}

fn valid_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || *bytes.last().unwrap() != b'Z'
    {
        return false;
    }
    let fraction_ok = bytes.len() == 20
        || (bytes.len() > 21
            && bytes[19] == b'.'
            && bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit));
    fraction_ok
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[11..13].iter().all(u8::is_ascii_digit)
        && bytes[14..16].iter().all(u8::is_ascii_digit)
        && bytes[17..19].iter().all(u8::is_ascii_digit)
        && valid_calendar_components(
            parse_digits(&bytes[..4]),
            parse_digits(&bytes[5..7]),
            parse_digits(&bytes[8..10]),
            parse_digits(&bytes[11..13]),
            parse_digits(&bytes[14..16]),
            parse_digits(&bytes[17..19]),
        )
}

fn parse_digits(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |value, byte| {
        value * 10 + u32::from(byte.saturating_sub(b'0'))
    })
}

fn valid_calendar_components(
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> bool {
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    year > 0 && (1..=days).contains(&day) && hour < 24 && minute < 60 && second < 60
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use tree_sitter::{Node, Parser};

    #[test]
    fn drive_path_detection_does_not_treat_https_as_a_drive() {
        assert!(!has_absolute_drive_path(
            "source=https://github.com/example/repo"
        ));
        assert!(has_absolute_drive_path(r"C:\Users\example"));
        assert!(has_absolute_drive_path("C:/Users/example"));
    }

    #[test]
    fn identifiers_are_closed() {
        assert!(valid_run_id("20260813T123456789Z-0123456789abcdef01234567"));
        assert!(!valid_run_id("20260813T12345678Z-0123456789abcdef01234567"));
        assert!(!valid_run_id(
            "20261313T123456789Z-0123456789abcdef01234567"
        ));
        assert!(!valid_run_id(
            "20260230T123456789Z-0123456789abcdef01234567"
        ));
        assert!(valid_utc_timestamp("2026-08-13T12:34:56.123456789Z"));
        assert!(valid_utc_timestamp("2024-02-29T23:59:59Z"));
        assert!(!valid_utc_timestamp("2023-02-29T23:59:59Z"));
        assert!(!valid_utc_timestamp("2026-13-13T12:34:56Z"));
        assert!(!valid_utc_timestamp("2026-04-31T12:34:56Z"));
        assert!(!valid_utc_timestamp("2026-08-13T24:34:56Z"));
        assert!(!valid_utc_timestamp("2026-08-13T12:60:56Z"));
        assert!(!valid_utc_timestamp("2026-08-13T12:34:60Z"));
        assert!(!valid_utc_timestamp("2026-08-13T12:34:56.Z"));
        assert!(!valid_utc_timestamp("2026-08-13T12:34:56+00:00"));
    }

    #[test]
    fn approvals_require_distinct_roles_or_explicit_dual_authority() {
        let base = Approval {
            schema: "python-slm-p0a-approval-v1".to_owned(),
            phase_id: "P0A".to_owned(),
            run_id: "20260813T123456789Z-0123456789abcdef01234567".to_owned(),
            role: "technical".to_owned(),
            decision: "APPROVE".to_owned(),
            owner_identity: "owner".to_owned(),
            review_reference: "signed review".to_owned(),
            utc_timestamp: "2026-08-13T12:34:56Z".to_owned(),
            run_evidence_sha256: "a".repeat(64),
            seal_sha256: "b".repeat(64),
            explicit_dual_role_authority: false,
        };
        let mut governance = base.clone();
        governance.role = "data_governance".to_owned();
        assert_eq!(
            validate_approval_pair(&base, &governance).unwrap_err().code,
            "P0A_DUAL_ROLE_AUTHORITY_MISSING"
        );
        governance.owner_identity = "another owner".to_owned();
        validate_approval_pair(&base, &governance).unwrap();
    }

    #[test]
    fn memory_and_sla_boundaries_are_exact() {
        assert_eq!(135_285_504_u64 * 20, 2_705_710_080);
        assert_eq!(minimum_accelerator_bytes(135_285_504), 2_952_790_016);
        let targets = 2_000_000_000_f64;
        let admission = 25_920_f64;
        let zero_overhead_rate = targets / admission;
        assert_eq!((targets / zero_overhead_rate).ceil() as u64, 25_920);
        let overhead = 600_f64;
        let required = required_rate(overhead).unwrap();
        assert_eq!((targets / required + overhead).ceil() as u64, 25_920);
        assert!(required_rate(25_920.0).is_none());
        assert!(required_rate(25_921.0).is_none());
        assert!(actual_within_sla(28_800_000_000_000));
        assert!(!actual_within_sla(28_800_000_000_001));
    }

    #[test]
    fn pointer_replacement_preserves_closed_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("evidence.json");
        let mut pointer = Pointer {
            schema: "python-slm-p0a-phase-pointer-v1".to_owned(),
            phase_id: "P0A".to_owned(),
            interface_id: INTERFACE_ID.to_owned(),
            profile_id: PROFILE_ID.to_owned(),
            acceptance_path: "acceptances/00000001.json".to_owned(),
            acceptance_sha256: "a".repeat(64),
            updated_at: "2026-08-13T12:34:56Z".to_owned(),
        };
        replace_pointer_atomically(&path, &pointer).unwrap();
        pointer.acceptance_path = "acceptances/00000002.json".to_owned();
        pointer.acceptance_sha256 = "b".repeat(64);
        replace_pointer_atomically(&path, &pointer).unwrap();
        let observed: Pointer = read_json_closed(&path, "TEST_POINTER").unwrap();
        assert_eq!(observed.acceptance_path, "acceptances/00000002.json");
        assert_eq!(observed.acceptance_sha256, "b".repeat(64));
    }

    #[test]
    fn exact_pointer_temporary_completes_interrupted_pre_rename_publication() {
        let temp = tempfile::tempdir().unwrap();
        publication::create_dir_all(&temp.path().join("acceptances")).unwrap();
        let acceptance = b"acceptance\n";
        publication::write_new(&temp.path().join("acceptances/00000001.json"), acceptance).unwrap();
        let pointer = Pointer {
            schema: "python-slm-p0a-phase-pointer-v1".to_owned(),
            phase_id: "P0A".to_owned(),
            interface_id: INTERFACE_ID.to_owned(),
            profile_id: PROFILE_ID.to_owned(),
            acceptance_path: "acceptances/00000001.json".to_owned(),
            acceptance_sha256: hash::bytes(acceptance),
            updated_at: "2026-08-13T12:34:56Z".to_owned(),
        };
        let bytes = canonical_json_bytes(
            &pointer,
            "TEST_POINTER_SERIALIZATION",
            "could not serialize test pointer",
        )
        .unwrap();
        let temporary = temp.path().join("evidence.json.tmp-pointer-123-456");
        publication::write_new(&temporary, &bytes).unwrap();
        recover_pointer_temporary(temp.path(), &pointer).unwrap();
        assert!(!temporary.exists());
        assert_eq!(fs::read(temp.path().join("evidence.json")).unwrap(), bytes);

        fs::remove_file(temp.path().join("evidence.json")).unwrap();
        let partial = temp.path().join("evidence.json.tmp-pointer-123-789");
        publication::write_new(&partial, b"partial").unwrap();
        recover_pointer_temporary(temp.path(), &pointer).unwrap();
        assert!(!partial.exists());
        assert!(!temp.path().join("evidence.json").exists());

        let ambiguous = temp.path().join("evidence.json.tmp-pointer-invalid");
        publication::write_new(&ambiguous, b"not the expected pointer\n").unwrap();
        assert_eq!(
            recover_pointer_temporary(temp.path(), &pointer)
                .unwrap_err()
                .code,
            "P0A_POINTER_TEMP_RECOVERY_INVALID"
        );
        assert!(ambiguous.exists());
    }

    #[test]
    fn failed_pointer_publication_rolls_back_only_its_owned_acceptance() {
        let temp = tempfile::tempdir().unwrap();
        publication::create_dir_all(&temp.path().join("acceptances")).unwrap();
        let acceptance_path = temp.path().join("acceptances/00000001.json");
        let acceptance_bytes = b"{\"status\":\"PASS\"}\n";
        let pointer = Pointer {
            schema: "python-slm-p0a-phase-pointer-v1".to_owned(),
            phase_id: "P0A".to_owned(),
            interface_id: INTERFACE_ID.to_owned(),
            profile_id: PROFILE_ID.to_owned(),
            acceptance_path: "acceptances/00000001.json".to_owned(),
            acceptance_sha256: hash::bytes(acceptance_bytes),
            updated_at: "2026-08-13T12:34:56Z".to_owned(),
        };
        let error = publish_acceptance_and_pointer_with(
            temp.path(),
            &acceptance_path,
            acceptance_bytes,
            &pointer,
            |_, _| {
                Err(XtaskError::environment(
                    "TEST_POINTER_FAILURE",
                    "injected pointer failure",
                ))
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "TEST_POINTER_FAILURE");
        assert!(!acceptance_path.exists());
        assert!(!temp.path().join("evidence.json").exists());

        publish_acceptance_and_pointer_with(
            temp.path(),
            &acceptance_path,
            acceptance_bytes,
            &pointer,
            |path, value| {
                replace_pointer_atomically(path, value)?;
                Err(XtaskError::environment(
                    "TEST_POST_RENAME_FAILURE",
                    "injected report after successful rename",
                ))
            },
        )
        .unwrap();
        assert_eq!(fs::read(&acceptance_path).unwrap(), acceptance_bytes);
        assert_eq!(load_pointer(temp.path()).unwrap(), pointer);
    }

    #[test]
    fn post_admission_failure_run_is_sealed_and_unselected() {
        let temp = tempfile::tempdir().unwrap();
        let run_id = "20260813T123456789Z-0123456789abcdef01234567";
        let run_root = temp.path().join("stage").join(run_id);
        publication::create_dir_all(run_root.parent().unwrap()).unwrap();
        let inputs = sample_amendment_inputs();
        let commands = vec![RecordedCommand {
            id: "C01".to_owned(),
            argv: vec!["git".to_owned(), "status".to_owned()],
            cwd: "${REPO}".to_owned(),
            exit_code: 0,
            status: "PASS",
            stdout: Vec::new(),
            stderr: Vec::new(),
        }];
        let emitted = emit_run_stage(
            &run_root,
            run_id,
            "2026-08-13T12:34:56Z",
            &inputs,
            &commands,
            "FAIL",
            vec![json!({
                "code": "INJECTED_FAILURE",
                "category": 5,
                "message": "injected post-admission gate failure",
                "remediation": "retain the failure and retry"
            })],
        )
        .unwrap();
        validate_failed_run(&run_root, &emitted).unwrap();
        let final_root = temp.path().join("runs").join(run_id);
        publication::create_dir_all(final_root.parent().unwrap()).unwrap();
        publish_staged_run(&run_root, &final_root).unwrap();
        publication::verify_seal(&final_root, &emitted.seal_sha256).unwrap();
        let evidence: Value =
            read_json_closed(&final_root.join("evidence.json"), "TEST_FAILURE_EVIDENCE").unwrap();
        assert_eq!(evidence["status"], "FAIL");
        assert!(!temp.path().join("evidence.json").exists());
        assert!(!temp.path().join("acceptances").exists());
    }

    #[test]
    fn retained_command_shape_admits_only_read_only_git_or_exact_cargo_metadata() {
        let values = |argv: &[&str]| argv.iter().map(|value| json!(value)).collect::<Vec<_>>();
        assert!(valid_recorded_command_argv(&values(&[
            "git",
            "status",
            "--porcelain=v1"
        ])));
        assert!(valid_recorded_command_argv(&values(&[
            "cargo",
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1"
        ])));
        for rejected in [
            values(&["cargo", "metadata"]),
            values(&["cargo", "run", "malicious"]),
            values(&["git", "commit", "-m", "mutation"]),
            values(&["git", "-c", "core.fsmonitor=malicious", "status"]),
            values(&["git", "clone", "https://example.invalid/repo"]),
        ] {
            assert!(!valid_recorded_command_argv(&rejected), "{rejected:?}");
        }
    }

    #[test]
    fn emitted_schema_bundle_uses_validator_canonical_order_and_hash() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let emitted = build_schema_bundle(&repository).unwrap();
        let expected = canonical_schema_paths();
        assert_eq!(
            emitted
                .entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        let mut manifest = String::new();
        for entry in &emitted.entries {
            manifest.push_str(&entry.sha256);
            manifest.push_str("  ");
            manifest.push_str(&entry.path);
            manifest.push('\n');
        }
        assert_eq!(emitted.bundle_sha256, hash::bytes(manifest.as_bytes()));
    }

    #[test]
    fn failure_projection_never_retains_raw_diagnostic_or_remediation() {
        let raw = XtaskError::new(
            "STORAGE_FAILURE",
            crate::error::Category::Environment,
            r"could not write C:\Users\private\secret password=hunter2",
            r"delete C:\Users\private\secret and retry with api_key=value",
        );
        let projected = safe_failure_error(&raw);
        let text = serde_json::to_string(&projected).unwrap();
        assert_eq!(projected["code"], "STORAGE_FAILURE");
        assert!(!text.contains("private"));
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("api_key"));

        let malformed = XtaskError::new(
            "PATH=C:/private",
            crate::error::Category::Internal,
            "raw",
            "raw",
        );
        assert_eq!(
            safe_failure_error(&malformed)["code"],
            "P0A_INTERNAL_FAILURE"
        );
    }

    #[test]
    fn staging_recovery_removes_only_exact_empty_preidentity_work_container() {
        let temp = tempfile::tempdir().unwrap();
        let output_root = temp.path().join("P0A");
        let staging = output_root.join(".staging");
        publication::create_dir_all(&staging).unwrap();
        let run_id = "20260813T123456789Z-0123456789abcdef01234567";
        let empty = staging.join(format!("{run_id}.work-123"));
        publication::create_dir(&empty).unwrap();
        recover_interrupted_stages(temp.path(), &output_root).unwrap();
        assert!(!empty.exists());

        let partial = staging.join(format!("{run_id}.work-124"));
        publication::create_dir(&partial).unwrap();
        publication::write_new(
            &partial.join("attempt.json.tmp-attempt-123-456"),
            b"partial",
        )
        .unwrap();
        recover_interrupted_stages(temp.path(), &output_root).unwrap();
        assert!(!partial.exists());

        let nonempty = staging.join(format!("{run_id}.work-456"));
        publication::create_dir(&nonempty).unwrap();
        publication::write_new(&nonempty.join("unexpected"), b"retain").unwrap();
        assert_eq!(
            recover_interrupted_stages(temp.path(), &output_root)
                .unwrap_err()
                .code,
            "P0A_STAGING_RECOVERY_INVALID"
        );
        assert_eq!(fs::read(nonempty.join("unexpected")).unwrap(), b"retain");
        assert!(parse_stage_container_name(&format!("{run_id}.work-abc")).is_none());
    }

    #[test]
    fn partial_acceptance_temporary_is_discarded_without_touching_final() {
        let temp = tempfile::tempdir().unwrap();
        let acceptances = temp.path().join("acceptances");
        publication::create_dir_all(&acceptances).unwrap();
        let final_path = acceptances.join("00000001.json");
        let temporary = acceptances.join("00000001.json.tmp-acceptance-123-456");
        publication::write_new(&temporary, b"partial").unwrap();
        recover_acceptance_temporary(&final_path).unwrap();
        assert!(!temporary.exists());
        assert!(!final_path.exists());

        publication::write_new(&final_path, b"final\n").unwrap();
        let redundant = acceptances.join("00000001.json.tmp-acceptance-123-789");
        publication::write_new(&redundant, b"partial").unwrap();
        recover_acceptance_temporary(&final_path).unwrap();
        assert!(!redundant.exists());
        assert_eq!(fs::read(final_path).unwrap(), b"final\n");
    }

    #[test]
    fn live_xtask_runtime_and_build_closure_is_zero_python_and_native_link_free() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let mut recorder = Recorder::default();
        require_zero_python_boundary(&repository, &mut recorder).unwrap();
        let metadata = recorder.commands().last().unwrap();
        assert_eq!(
            metadata.argv,
            [
                "cargo",
                "metadata",
                "--locked",
                "--offline",
                "--format-version",
                "1"
            ]
        );
        assert_eq!(metadata.status, "PASS");
    }

    #[test]
    fn zero_python_closure_rejects_python_packages_native_links_and_python_builds() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let manifest = repository.join("xtask/Cargo.toml");
        let mut metadata = json!({
            "packages": [
                {
                    "id": "path+xtask#0.1.0",
                    "name": "xtask",
                    "manifest_path": manifest,
                    "links": null,
                    "targets": []
                },
                {
                    "id": "registry+canary#1.0.0",
                    "name": "python-runtime-canary",
                    "manifest_path": "unused",
                    "links": null,
                    "targets": []
                }
            ],
            "resolve": {
                "nodes": [
                    {
                        "id": "path+xtask#0.1.0",
                        "deps": [{
                            "pkg": "registry+canary#1.0.0",
                            "dep_kinds": [{"kind": null}]
                        }]
                    },
                    {"id": "registry+canary#1.0.0", "deps": []}
                ]
            }
        });
        assert_eq!(
            require_python_free_xtask_closure(&repository, &metadata)
                .unwrap_err()
                .code,
            "PYTHON_DEPENDENCY_BOUNDARY_VIOLATION"
        );
        metadata["packages"][1]["name"] = json!("native-canary");
        metadata["packages"][1]["links"] = json!("native_canary");
        assert_eq!(
            require_python_free_xtask_closure(&repository, &metadata)
                .unwrap_err()
                .code,
            "XTASK_NATIVE_MODULE_BOUNDARY_VIOLATION"
        );

        let temp = tempfile::tempdir().unwrap();
        publication::write_new(&temp.path().join("Cargo.toml"), b"[package]\nname='x'\n").unwrap();
        publication::write_new(
            &temp.path().join("build.rs"),
            b"fn main() { Command::new(\"python\"); }\n",
        )
        .unwrap();
        let package = json!({"manifest_path": temp.path().join("Cargo.toml")});
        assert_eq!(
            require_python_free_build_script(&package, &temp.path().join("build.rs"))
                .unwrap_err()
                .code,
            "PYTHON_BUILD_SCRIPT_BOUNDARY_VIOLATION"
        );
    }

    #[test]
    fn approval_publication_and_checkbox_history_survive_later_head() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path();
        let mut setup = Recorder::default();
        git_ok(&mut setup, repository, &["init"]);
        git_ok(&mut setup, repository, &["config", "user.name", "P0A Test"]);
        git_ok(
            &mut setup,
            repository,
            &["config", "user.email", "p0a@example.invalid"],
        );
        publication::write_new(&repository.join("TODO.md"), b"- [ ] P0A complete\n").unwrap();
        git_ok(&mut setup, repository, &["add", "TODO.md"]);
        git_ok(&mut setup, repository, &["commit", "-m", "preapproval"]);
        let preapproval = git_head(&mut setup, repository);

        let output_root = repository.join(OUTPUT_ROOT);
        let approvals = output_root.join("approvals");
        publication::create_dir_all(&approvals).unwrap();
        let technical = approvals.join("technical-00000001.json");
        let governance = approvals.join("data-governance-00000001.json");
        publication::write_new(&technical, b"{\"role\":\"technical\"}\n").unwrap();
        publication::write_new(&governance, b"{\"role\":\"data_governance\"}\n").unwrap();
        git_ok(
            &mut setup,
            repository,
            &[
                "add",
                "docs/receipts/P0A/approvals/technical-00000001.json",
                "docs/receipts/P0A/approvals/data-governance-00000001.json",
            ],
        );
        git_ok(&mut setup, repository, &["commit", "-m", "approvals"]);
        let approval_commit = git_head(&mut setup, repository);

        let mut acceptance = sample_acceptance(1);
        acceptance.approval_commit = approval_commit.clone();
        acceptance.preapproval_commit = preapproval.clone();
        acceptance.todo_preapproval_sha256 = hash::bytes(b"- [ ] P0A complete\n");
        let acceptance_bytes = canonical_json_bytes(
            &acceptance,
            "TEST_ACCEPTANCE_SERIALIZATION",
            "could not serialize test acceptance",
        )
        .unwrap();
        let acceptances = output_root.join("acceptances");
        publication::create_dir_all(&acceptances).unwrap();
        let acceptance_path = acceptances.join("00000001.json");
        publication::write_new(&acceptance_path, &acceptance_bytes).unwrap();
        let pointer = Pointer {
            schema: "python-slm-p0a-phase-pointer-v1".to_owned(),
            phase_id: "P0A".to_owned(),
            interface_id: INTERFACE_ID.to_owned(),
            profile_id: PROFILE_ID.to_owned(),
            acceptance_path: "acceptances/00000001.json".to_owned(),
            acceptance_sha256: hash::bytes(&acceptance_bytes),
            updated_at: acceptance.created_at.clone(),
        };
        replace_pointer_atomically(&output_root.join("evidence.json"), &pointer).unwrap();
        git_ok(
            &mut setup,
            repository,
            &[
                "add",
                "docs/receipts/P0A/acceptances/00000001.json",
                "docs/receipts/P0A/evidence.json",
            ],
        );
        git_ok(&mut setup, repository, &["commit", "-m", "publication"]);
        let publication_commit = git_head(&mut setup, repository);

        fs::write(repository.join("TODO.md"), b"- [x] P0A complete\n").unwrap();
        git_ok(&mut setup, repository, &["add", "TODO.md"]);
        git_ok(&mut setup, repository, &["commit", "-m", "checkbox"]);
        publication::write_new(&repository.join("later.txt"), b"later\n").unwrap();
        git_ok(&mut setup, repository, &["add", "later.txt"]);
        git_ok(&mut setup, repository, &["commit", "-m", "later"]);

        let mut recorder = Recorder::default();
        require_approval_commit(
            repository,
            &mut recorder,
            &approval_commit,
            &technical,
            &governance,
            &preapproval,
        )
        .unwrap();
        require_committed_publication_pair(
            repository,
            &mut recorder,
            1,
            &approval_commit,
            &pointer,
        )
        .unwrap();
        validate_checkbox_commit(
            repository,
            &mut recorder,
            &output_root,
            &pointer,
            &acceptance,
        )
        .unwrap();

        let parents = recorder
            .git_text(
                repository,
                &["rev-list", "--parents", "-n", "1", &publication_commit],
                "TEST_GIT",
            )
            .unwrap();
        assert!(parents.trim_end().ends_with(&approval_commit));
    }

    #[test]
    fn committed_acceptance_without_pointer_in_same_commit_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path();
        let mut recorder = Recorder::default();
        git_ok(&mut recorder, repository, &["init"]);
        git_ok(
            &mut recorder,
            repository,
            &["config", "user.name", "P0A Test"],
        );
        git_ok(
            &mut recorder,
            repository,
            &["config", "user.email", "p0a@example.invalid"],
        );
        publication::write_new(&repository.join("base"), b"base\n").unwrap();
        git_ok(&mut recorder, repository, &["add", "base"]);
        git_ok(&mut recorder, repository, &["commit", "-m", "approval"]);
        let approval_commit = git_head(&mut recorder, repository);
        let output_root = repository.join(OUTPUT_ROOT);
        publication::create_dir_all(&output_root.join("acceptances")).unwrap();
        let acceptance_bytes = b"{\"status\":\"PASS\"}\n";
        publication::write_new(
            &output_root.join("acceptances/00000001.json"),
            acceptance_bytes,
        )
        .unwrap();
        git_ok(
            &mut recorder,
            repository,
            &["add", "docs/receipts/P0A/acceptances/00000001.json"],
        );
        git_ok(
            &mut recorder,
            repository,
            &["commit", "-m", "acceptance only"],
        );
        let pointer = Pointer {
            schema: "python-slm-p0a-phase-pointer-v1".to_owned(),
            phase_id: "P0A".to_owned(),
            interface_id: INTERFACE_ID.to_owned(),
            profile_id: PROFILE_ID.to_owned(),
            acceptance_path: "acceptances/00000001.json".to_owned(),
            acceptance_sha256: hash::bytes(acceptance_bytes),
            updated_at: "2026-08-13T12:34:56Z".to_owned(),
        };
        replace_pointer_atomically(&output_root.join("evidence.json"), &pointer).unwrap();
        assert_eq!(
            require_committed_publication_pair(
                repository,
                &mut recorder,
                1,
                &approval_commit,
                &pointer,
            )
            .unwrap_err()
            .code,
            "P0A_PUBLICATION_SPLIT_COMMIT"
        );
    }

    #[test]
    fn publication_recovery_rejects_unrelated_commit_after_approval_head() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path();
        let mut recorder = Recorder::default();
        git_ok(&mut recorder, repository, &["init"]);
        git_ok(
            &mut recorder,
            repository,
            &["config", "user.name", "P0A Test"],
        );
        git_ok(
            &mut recorder,
            repository,
            &["config", "user.email", "p0a@example.invalid"],
        );
        publication::write_new(&repository.join("approval"), b"approval\n").unwrap();
        git_ok(&mut recorder, repository, &["add", "approval"]);
        git_ok(&mut recorder, repository, &["commit", "-m", "approval"]);
        let approval_commit = git_head(&mut recorder, repository);
        require_publication_recovery_head(repository, &mut recorder, &approval_commit).unwrap();

        publication::write_new(&repository.join("unrelated"), b"later\n").unwrap();
        git_ok(&mut recorder, repository, &["add", "unrelated"]);
        git_ok(&mut recorder, repository, &["commit", "-m", "later"]);
        assert_eq!(
            require_publication_recovery_head(repository, &mut recorder, &approval_commit)
                .unwrap_err()
                .code,
            "P0A_PUBLICATION_RECOVERY_HEAD_INVALID"
        );
    }

    fn git_ok(recorder: &mut Recorder, repository: &Path, args: &[&str]) {
        recorder
            .git_success(repository, args, "TEST_GIT_FAILED")
            .unwrap();
    }

    fn git_head(recorder: &mut Recorder, repository: &Path) -> String {
        git_line(
            recorder,
            repository,
            &["rev-parse", "HEAD"],
            "TEST_GIT_FAILED",
        )
        .unwrap()
    }

    #[test]
    fn pointer_paths_and_latest_sequence_are_exact() {
        assert_eq!(
            parse_acceptance_path("acceptances/00000001.json").unwrap(),
            1
        );
        for invalid in [
            "acceptances/00000000.json",
            "acceptances/abcdefgh.json",
            "acceptances/../00001.json",
            "acceptances/00000001.json/extra",
        ] {
            assert!(parse_acceptance_path(invalid).is_err(), "{invalid}");
        }

        let temp = tempfile::tempdir().unwrap();
        publication::create_dir_all(&temp.path().join("acceptances")).unwrap();
        publication::write_new(&temp.path().join("acceptances/00000001.json"), b"{}\n").unwrap();
        publication::write_new(&temp.path().join("acceptances/00000002.json"), b"{}\n").unwrap();
        let pointer = Pointer {
            schema: "python-slm-p0a-phase-pointer-v1".to_owned(),
            phase_id: "P0A".to_owned(),
            interface_id: INTERFACE_ID.to_owned(),
            profile_id: PROFILE_ID.to_owned(),
            acceptance_path: "acceptances/00000001.json".to_owned(),
            acceptance_sha256: "a".repeat(64),
            updated_at: "2026-08-13T12:34:56Z".to_owned(),
        };
        let acceptance = sample_acceptance(1);
        assert_eq!(
            validate_selected_acceptance(temp.path(), &pointer, &acceptance)
                .unwrap_err()
                .code,
            "P0A_POINTER_ROLLBACK"
        );
        fs::remove_file(temp.path().join("acceptances/00000002.json")).unwrap();
        validate_selected_acceptance(temp.path(), &pointer, &acceptance).unwrap();
        publication::write_new(&temp.path().join("acceptances/00000003.json"), b"{}\n").unwrap();
        assert_eq!(
            next_acceptance_sequence(temp.path()).unwrap_err().code,
            "P0A_ACCEPTANCE_SEQUENCE_GAP"
        );
    }

    #[test]
    fn acceptance_paths_are_bound_to_role_and_approval_attempt() {
        let mut acceptance = sample_acceptance(1);
        validate_acceptance_shape(&acceptance).unwrap();
        acceptance.approvals[0].path = "approvals/technical-00000002.json".to_owned();
        acceptance.approvals[1].path = "approvals/data-governance-00000002.json".to_owned();
        validate_acceptance_shape(&acceptance).unwrap();
        acceptance.approvals[1].path = "approvals/data-governance-00000003.json".to_owned();
        assert_eq!(
            validate_acceptance_shape(&acceptance).unwrap_err().code,
            "P0A_ACCEPTANCE_INVALID"
        );
        acceptance = sample_acceptance(1);
        acceptance.approvals[0].path = "approvals/technical-00000000.json".to_owned();
        acceptance.approvals[1].path = "approvals/data-governance-00000000.json".to_owned();
        assert_eq!(
            validate_acceptance_shape(&acceptance).unwrap_err().code,
            "P0A_ACCEPTANCE_INVALID"
        );
        acceptance = sample_acceptance(1);
        acceptance.run_path = "runs/../elsewhere".to_owned();
        assert_eq!(
            validate_acceptance_shape(&acceptance).unwrap_err().code,
            "P0A_ACCEPTANCE_INVALID"
        );
    }

    #[test]
    fn approval_attempt_sequence_must_advance_across_acceptances() {
        assert_eq!(
            require_approval_sequence_successor(2, 2).unwrap_err().code,
            "P0A_APPROVAL_SEQUENCE_REPLAY"
        );
        assert_eq!(
            require_approval_sequence_successor(3, 2).unwrap_err().code,
            "P0A_APPROVAL_SEQUENCE_REPLAY"
        );
        require_approval_sequence_successor(1, 2).unwrap();
    }

    #[test]
    fn retained_acceptance_selection_is_lazy_for_genesis_and_mismatch() {
        assert!(select_matching_retained_acceptance(None, false).is_none());
        assert!(select_matching_retained_acceptance(None, true).is_none());

        let acceptance = sample_acceptance(1);
        assert!(select_matching_retained_acceptance(Some(&acceptance), false).is_none());
        assert_eq!(
            select_matching_retained_acceptance(Some(&acceptance), true),
            Some(&acceptance)
        );
    }

    #[test]
    fn new_acceptance_uses_selected_predecessor_but_genesis_can_follow_failed_approvals() {
        let genesis = tempfile::tempdir().unwrap();
        require_selected_predecessor_for_new_acceptance(genesis.path(), None, 1, 2).unwrap();

        let temp = tempfile::tempdir().unwrap();
        let acceptances = temp.path().join("acceptances");
        fs::create_dir(&acceptances).unwrap();
        let predecessor = sample_acceptance(1);
        let predecessor_path = acceptances.join("00000001.json");
        publication::write_json_new(&predecessor_path, &predecessor).unwrap();
        let predecessor_hash = hash::file(&predecessor_path).unwrap();
        let pointer = Pointer {
            schema: "python-slm-p0a-phase-pointer-v1".to_owned(),
            phase_id: "P0A".to_owned(),
            interface_id: INTERFACE_ID.to_owned(),
            profile_id: PROFILE_ID.to_owned(),
            acceptance_path: "acceptances/00000001.json".to_owned(),
            acceptance_sha256: predecessor_hash,
            updated_at: predecessor.created_at.clone(),
        };
        publication::write_json_new(&temp.path().join("evidence.json"), &pointer).unwrap();
        require_selected_predecessor_for_new_acceptance(temp.path(), Some(&predecessor), 2, 2)
            .unwrap();
        assert_eq!(
            require_selected_predecessor_for_new_acceptance(temp.path(), Some(&predecessor), 2, 1,)
                .unwrap_err()
                .code,
            "P0A_APPROVAL_SEQUENCE_REPLAY"
        );
    }

    #[test]
    fn acceptance_temp_recovery_precedes_sequence_enumeration() {
        let temp = tempfile::tempdir().unwrap();
        let acceptances = temp.path().join("acceptances");
        fs::create_dir(&acceptances).unwrap();
        let partial = acceptances.join("00000001.json.tmp-acceptance-123-456");
        publication::write_new(&partial, b"partial").unwrap();
        recover_all_acceptance_temporaries(temp.path()).unwrap();
        assert!(!partial.exists());
        assert_eq!(next_acceptance_sequence(temp.path()).unwrap(), 1);

        let final_path = acceptances.join("00000001.json");
        publication::write_new(&final_path, b"{}\n").unwrap();
        let redundant = acceptances.join("00000001.json.tmp-acceptance-123-789");
        publication::write_new(&redundant, b"{}\n").unwrap();
        recover_all_acceptance_temporaries(temp.path()).unwrap();
        assert!(final_path.exists());
        assert!(!redundant.exists());
        assert_eq!(next_acceptance_sequence(temp.path()).unwrap(), 2);
    }

    #[test]
    fn slash_relative_canonicalizes_both_sides_before_containment() {
        let temp = tempfile::tempdir().unwrap();
        let inside = temp.path().join("nested");
        fs::create_dir(&inside).unwrap();
        let retained = inside.join("approval.json");
        publication::write_new(&retained, b"{}\n").unwrap();
        let canonical_retained = fs::canonicalize(&retained).unwrap();
        assert_eq!(
            slash_relative(temp.path(), &canonical_retained).unwrap(),
            "nested/approval.json"
        );

        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("approval.json");
        publication::write_new(&outside_file, b"{}\n").unwrap();
        assert_eq!(
            slash_relative(temp.path(), &outside_file).unwrap_err().code,
            "P0A_PATH_CONTAINMENT_INVALID"
        );
    }

    #[test]
    fn live_portable_policy_and_parser_bundle_are_exact() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        require_portable_policy(&repository).unwrap();
        let bundle = build_schema_bundle(&repository).unwrap();
        assert_eq!(bundle.entries.len(), 12);
        let parser = build_parser_boundary(&repository).unwrap();
        assert_eq!(parser["python_codegen"], false);
        assert_eq!(parser["native_role"], "parser_only");
        assert_eq!(parser["runtime_sources"].as_array().unwrap().len(), 3);
        assert_eq!(parser["generated_sources"].as_array().unwrap().len(), 5);
        let retained = expected_parser_boundary(
            hash::file(
                &repository
                    .join("docs/schemas/portable-v2/tree-sitter-python-compatibility-v1.json"),
            )
            .unwrap(),
        );
        assert_eq!(parser, retained);
        let mut mutated = retained.clone();
        mutated["python_codegen"] = json!(true);
        assert_ne!(mutated, parser);
    }

    #[test]
    fn locked_parser_packages_are_exact_and_python_free() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let lock = fs::read(repository.join("Cargo.lock")).unwrap();
        require_locked_parser_packages(&lock).unwrap();
        let mut wrong = String::from_utf8(lock).unwrap();
        wrong = wrong.replacen(
            "name = \"tree-sitter-python\"\nversion = \"0.25.0\"",
            "name = \"tree-sitter-python\"\nversion = \"0.25.1\"",
            1,
        );
        assert_eq!(
            require_locked_parser_packages(wrong.as_bytes())
                .unwrap_err()
                .code,
            "PARSER_LOCK_INVALID"
        );
        let registry_root = cargo_registry_source_root().unwrap();
        let parser = find_registry_package(&registry_root, "tree-sitter-python-0.25.0").unwrap();
        let runtime = find_registry_package(&registry_root, "tree-sitter-0.25.8").unwrap();
        require_python_free_native_build(&parser, &runtime).unwrap();
    }

    #[test]
    fn approval_timestamp_and_decision_are_fail_closed() {
        let mut approval = Approval {
            schema: "python-slm-p0a-approval-v1".to_owned(),
            phase_id: "P0A".to_owned(),
            run_id: "20260813T123456789Z-0123456789abcdef01234567".to_owned(),
            role: "technical".to_owned(),
            decision: "APPROVE".to_owned(),
            owner_identity: "owner".to_owned(),
            review_reference: "signed review".to_owned(),
            utc_timestamp: "2026-08-13T12:34:56Z".to_owned(),
            run_evidence_sha256: "a".repeat(64),
            seal_sha256: "b".repeat(64),
            explicit_dual_role_authority: false,
        };
        validate_approval(&approval, "technical").unwrap();
        approval.utc_timestamp = "2026-08-13T12:34:56+00:00".to_owned();
        assert_eq!(
            validate_approval(&approval, "technical").unwrap_err().code,
            "P0A_APPROVAL_INVALID"
        );
        approval.utc_timestamp = "2026-08-13T12:34:56Z".to_owned();
        approval.decision = "REJECT".to_owned();
        assert_eq!(
            validate_approval(&approval, "technical").unwrap_err().code,
            "P0A_APPROVAL_INVALID"
        );
    }

    fn sample_acceptance(sequence: u32) -> Acceptance {
        let run_id = "20260813T123456789Z-0123456789abcdef01234567";
        Acceptance {
            schema: "python-slm-p0a-phase-acceptance-v1".to_owned(),
            phase_id: "P0A".to_owned(),
            interface_id: INTERFACE_ID.to_owned(),
            profile_id: PROFILE_ID.to_owned(),
            sequence,
            status: "PASS".to_owned(),
            acceptance_kind: "owner_approved_contract_amendment".to_owned(),
            run_path: format!("runs/{run_id}"),
            run_evidence_sha256: "a".repeat(64),
            seal_path: format!("runs/{run_id}/SHA256SUMS"),
            seal_sha256: "b".repeat(64),
            approvals: vec![
                ApprovalRef {
                    role: "technical".to_owned(),
                    decision: "APPROVE".to_owned(),
                    path: format!("approvals/technical-{sequence:08}.json"),
                    sha256: "c".repeat(64),
                },
                ApprovalRef {
                    role: "data_governance".to_owned(),
                    decision: "APPROVE".to_owned(),
                    path: format!("approvals/data-governance-{sequence:08}.json"),
                    sha256: "d".repeat(64),
                },
            ],
            approval_commit: "e".repeat(40),
            preapproval_commit: "f".repeat(40),
            todo_preapproval_sha256: "0".repeat(64),
            previous_acceptance_sha256: (sequence > 1).then(|| "1".repeat(64)),
            created_at: "2026-08-13T12:34:56Z".to_owned(),
        }
    }

    fn sample_amendment_inputs() -> AmendmentInputs {
        let snapshots = SNAPSHOTS
            .iter()
            .map(|(_, destination)| ((*destination).to_owned(), destination.as_bytes().to_vec()))
            .collect();
        AmendmentInputs {
            source: SourceIdentity {
                schema: "python-slm-p0a-source-identity-v1",
                commit: "a".repeat(40),
                tree: "b".repeat(40),
                branch: "main".to_owned(),
                dirty: false,
                cargo_lock_sha256: "c".repeat(64),
                verifier_source_sha256: "d".repeat(64),
                schema_bundle_sha256: "e".repeat(64),
            },
            p0_dependency: P0Dependency {
                schema: "python-slm-p0-dependency-v1",
                status: "PASS",
                baseline_commit: p0::BASELINE_COMMIT,
                receipt_commit: p0::RECEIPT_COMMIT,
                receipt_sha256: p0::RECEIPT_SHA256,
                contract_sha256: p0::CONTRACT_SHA256,
                decision_ledger_sha256: p0::LEDGER_SHA256,
                reconciliation_commit: p0::RECONCILIATION_COMMIT,
                reconciliation_tree: p0::RECONCILIATION_TREE,
                run_id: p0::RUN_ID,
                seal_sha256: p0::SEAL_SHA256,
                historical_cargo_lock_sha256: p0::HISTORICAL_CARGO_LOCK_SHA256,
                oracle_commit: p0::ORACLE_COMMIT,
                oracle_normalized_sha256: p0::ORACLE_NORMALIZED_SHA256,
                verified_at_source_commit: "a".repeat(40),
            },
            schema_bundle: SchemaBundle {
                schema: "python-slm-p0a-schema-bundle-v1",
                entries: SCHEMA_PATHS
                    .iter()
                    .map(|path| SchemaEntry {
                        path: (*path).to_owned(),
                        sha256: "f".repeat(64),
                    })
                    .collect(),
                bundle_sha256: "e".repeat(64),
            },
            parser_boundary: expected_parser_boundary("0".repeat(64)),
            snapshots,
        }
    }

    #[test]
    fn pinned_tree_sitter_reproduces_the_frozen_compatibility_corpus() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let fixture: Value = read_json_closed(
            &repository.join("docs/schemas/portable-v2/tree-sitter-python-compatibility-v1.json"),
            "TEST_FIXTURE",
        )
        .unwrap();
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let source = base64::engine::general_purpose::STANDARD
                .decode(case["source_base64"].as_str().unwrap())
                .unwrap();
            assert_eq!(hash::bytes(&source), case["source_sha256"]);
            let tree = parser.parse(&source, None).unwrap();
            let root = tree.root_node();
            assert_eq!(root.has_error(), case["expected_has_error"]);
            assert_eq!(root.to_sexp(), case["expected_cst_sexp"]);
            let mut comments = Vec::new();
            let mut lexical = Vec::new();
            collect_parser_outputs(root, &source, &mut comments, &mut lexical);
            assert_eq!(json!(comments), case["comment_ranges"]);
            assert_eq!(json!(lexical), case["expected_lexical_tokens"]);
            let mut cursor = root.walk();
            let header_generated = root
                .children(&mut cursor)
                .take_while(|node| node.kind() == "comment")
                .any(|node| {
                    String::from_utf8_lossy(&source[node.byte_range()])
                        .to_ascii_lowercase()
                        .contains("generated by")
                });
            assert_eq!(
                json!(header_generated),
                case["generated_marker_in_comment_header"]
            );
            let mut string_ranges = Vec::new();
            collect_kind_ranges(root, "string", &mut string_ranges);
            let string_is_comment = string_ranges.iter().any(|range| comments.contains(range));
            assert_eq!(json!(string_is_comment), case["string_literal_is_comment"]);
        }
    }

    fn collect_kind_ranges(node: Node<'_>, kind: &str, output: &mut Vec<[usize; 2]>) {
        if node.kind() == kind {
            output.push([node.start_byte(), node.end_byte()]);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_kind_ranges(child, kind, output);
        }
    }

    fn collect_parser_outputs(
        node: Node<'_>,
        source: &[u8],
        comments: &mut Vec<[usize; 2]>,
        lexical: &mut Vec<Value>,
    ) {
        if node.kind() == "comment" {
            comments.push([node.start_byte(), node.end_byte()]);
            return;
        }
        if node.kind() == "string" {
            lexical.push(json!({
                "kind": "string",
                "text_hex": hex::encode(&source[node.byte_range()])
            }));
            return;
        }
        if node.child_count() == 0 {
            if !node.is_extra() && !node.kind().trim().is_empty() {
                lexical.push(json!({
                    "kind": node.kind(),
                    "text_hex": hex::encode(&source[node.byte_range()])
                }));
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_parser_outputs(child, source, comments, lexical);
        }
    }
}
