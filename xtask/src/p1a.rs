use crate::error::{Category, IoContext, Result, XtaskError};
use crate::hash;
use crate::json_schema;
use crate::p1a_process::{AuditedOutput, DirectCommand, ProcessAudit, QualifiedPersistentFile};
use crate::p1a_receipt::ReceiptSchemas;
use crate::p1a_windows::{
    CpuIsolationPolicy, EXPECTED_CPU_BRAND, EXPECTED_CPU_VENDOR, PrototypeWindowsHostReport,
    WindowsHostPolicy,
};
use crate::process::FileRef;
use crate::publication;
use crate::time;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

const PHASE_ID: &str = "P1A";
const INTERFACE_ID: &str = "portable-interface-v2";
const PROFILE_ID: &str = "prototype-windows-5090-v1";
const SUPPORT_TIER: &str = "implemented";
const QUALIFICATION_SCOPE: &str = "host_toolchain_only";
const OUTPUT_ROOT: &str = "docs/receipts/P1A-prototype-v2";
const P0A_POINTER_SHA256: &str = "c3ac49fdd2aecddb677bd315678d0f0d72bba3fbd92f773289ef98871505e15d";
const P0A_ACCEPTANCE_SHA256: &str =
    "31434001719f50de1abfa06dd84200ffb8e1f0947e08c8673f6413c08dbc4fb4";
const P0A_RUN_ID: &str = "20260813T111427138Z-594485fab70714426a7a3870";
const P0A_EVIDENCE_SHA256: &str =
    "c9de84843e74771fa675514f552301757b8fbadb2ea16e7afa13a0b4c40e2cd1";
const P0A_SEAL_SHA256: &str = "5248bbd6ef45e42480676a978a683a8883e68466804b0cce572af6b368044900";
const P0A_TECHNICAL_APPROVAL_SHA256: &str =
    "ea9eae24d1763e8ac257a35b63f6df3648bc5ed7cb7199863d6161b3142a9765";
const P0A_DATA_GOVERNANCE_APPROVAL_SHA256: &str =
    "8273e1fb6fb8aa812253f825dd38632770fa43ed4f8246907be622bf0b0783a3";

const SCHEMA_PATHS: &[&str] = &[
    "docs/schemas/P1A-prototype-v2/python-slm-p0a-dependency-v1.schema.json",
    "docs/schemas/P1A-prototype-v2/python-slm-p1a-cpu-isolation-v1.schema.json",
    "docs/schemas/P1A-prototype-v2/python-slm-p1a-host-environment-v1.schema.json",
    "docs/schemas/P1A-prototype-v2/python-slm-p1a-native-abi-probe-v1.schema.json",
    "docs/schemas/P1A-prototype-v2/python-slm-p1a-phase-acceptance-v1.schema.json",
    "docs/schemas/P1A-prototype-v2/python-slm-p1a-phase-evidence-v1.schema.json",
    "docs/schemas/P1A-prototype-v2/python-slm-p1a-phase-pointer-v1.schema.json",
    "docs/schemas/P1A-prototype-v2/python-slm-p1a-schema-bundle-v1.schema.json",
    "docs/schemas/P1A-prototype-v2/python-slm-p1a-source-identity-v1.schema.json",
    "docs/schemas/portable-v2/python-slm-prototype-profile-v1.schema.json",
    "docs/schemas/portable-v2/python-slm-prototype-sla-v1.schema.json",
    "docs/schemas/portable-v2/python-slm-training-memory-policy-v1.schema.json",
    "docs/schemas/portable-v2/tree-sitter-python-compatibility-v1.json",
];

const ARTIFACT_PATHS: &[&str] = &[
    "artifacts/cpu-isolation.json",
    "artifacts/host-environment.json",
    "artifacts/native-abi-probe.json",
    "artifacts/p0a-dependency.json",
    "artifacts/schema-bundle.json",
    "artifacts/source-identity.json",
];

const VERIFIER_PATHS: &[&str] = &[
    "Cargo.lock",
    "Cargo.toml",
    "xtask/Cargo.toml",
    "xtask/probes/p1a_abi.c",
    "xtask/probes/p1a_abi.cpp",
    "xtask/probes/p1a_abi.rs",
    "xtask/src/cli.rs",
    "xtask/src/error.rs",
    "xtask/src/hash.rs",
    "xtask/src/json_schema.rs",
    "xtask/src/lib.rs",
    "xtask/src/main.rs",
    "xtask/src/p0.rs",
    "xtask/src/p0a.rs",
    "xtask/src/p1a.rs",
    "xtask/src/p1a_artifacts.rs",
    "xtask/src/p1a_process.rs",
    "xtask/src/p1a_receipt.rs",
    "xtask/src/p1a_windows.rs",
    "xtask/src/process.rs",
    "xtask/src/publication.rs",
    "xtask/src/time.rs",
];

const HISTORICAL_P1A_BASELINE: &str = "9531898eccb4ff87322717d47cc4eefd376d6b95";
const HISTORICAL_P1A_TREE: &str = "4136924a6137ba0c06b2f6e663bc79fda7fa5511";
const HISTORICAL_P1A_POINTER_SHA256: &str =
    "056978d93a11ff1ca92456de9a588c79d5ae5fd2db3f5d3753a9b85b12680753";
const HISTORICAL_P1A_ACCEPTANCE_SHA256: &str =
    "8c47159f9b06224e4092a8a97568fc29bd8d3d28ac09a17387941fb34852e685";
const HISTORICAL_P1A_INVENTORY_SHA256: &str =
    "9ba7131368de56c8a1ce8ab8bf70e1b7dfd2b1bbc9f091c18a97675535e2c190";

const BANNED_BUILD_ENVIRONMENT: &[&str] = &[
    "RUSTC",
    "RUSTDOC",
    "RUSTUP_TOOLCHAIN",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_TARGET_DIR",
    "CC",
    "CXX",
    "AR",
    "CFLAGS",
    "CXXFLAGS",
    "LDFLAGS",
    "CARGO_BUILD_TARGET",
    "CARGO_BUILD_RUSTC_WRAPPER",
];

const ADMITTED_PYTHON_SOURCE_PACKAGE: &str = "tree-sitter-python@0.25.0";

const EXPLICIT_PROVIDER_SOURCE_PACKAGE_NAMES: &[&str] = &[
    "burn-cubecl",
    "burn-cubecl-fusion",
    "burn-cuda",
    "burn-rocm",
    "burn-tch",
    "burn-wgpu",
    "cubecl",
    "cubecl-core",
    "cubecl-cpp",
    "cubecl-cpu",
    "cubecl-cuda",
    "cubecl-hip",
    "cubecl-hip-sys",
    "cubecl-wgpu",
    "cudarc",
    "cuda-sys",
    "hip-sys",
    "hipblas",
    "libtorch",
    "metal",
    "metal-rs",
    "objc2-metal",
    "onnxruntime",
    "raw-window-metal",
    "rocblas",
    "tch",
    "tensorflow",
    "torch-sys",
    "wgpu",
];

const ABI_EXPECTED_STDOUT: &[u8] = b"P1A_ABI_PASS c=3137 cpp=150\n";
const ABI_EXPECTED_SHA256: &str =
    "89d1a4fea39a52632f01e703e0b97c6462edbd6971b43c969e692a8da66f7ab5";
const ABI_ALLOWED_IMPORTS: &[&str] = &[
    "advapi32.dll",
    "api-ms-win-core-synch-l1-2-0.dll",
    "api-ms-win-crt-convert-l1-1-0.dll",
    "api-ms-win-crt-environment-l1-1-0.dll",
    "api-ms-win-crt-filesystem-l1-1-0.dll",
    "api-ms-win-crt-heap-l1-1-0.dll",
    "api-ms-win-crt-locale-l1-1-0.dll",
    "api-ms-win-crt-math-l1-1-0.dll",
    "api-ms-win-crt-multibyte-l1-1-0.dll",
    "api-ms-win-crt-private-l1-1-0.dll",
    "api-ms-win-crt-process-l1-1-0.dll",
    "api-ms-win-crt-runtime-l1-1-0.dll",
    "api-ms-win-crt-stdio-l1-1-0.dll",
    "api-ms-win-crt-string-l1-1-0.dll",
    "api-ms-win-crt-time-l1-1-0.dll",
    "api-ms-win-crt-utility-l1-1-0.dll",
    "bcrypt.dll",
    "kernel32.dll",
    "kernelbase.dll",
    "ntdll.dll",
    "shell32.dll",
    "userenv.dll",
    "vcruntime140.dll",
    "ws2_32.dll",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceIdentity {
    schema: String,
    phase_id: String,
    interface_id: String,
    profile_id: String,
    support_tier: String,
    commit: String,
    tree: String,
    branch: String,
    dirty: bool,
    cargo_lock_sha256: String,
    verifier_source_sha256: String,
    schema_bundle_sha256: String,
    p0a_pointer_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct P0aDependency {
    schema: String,
    phase_id: String,
    interface_id: String,
    profile_id: String,
    status: String,
    pointer_path: String,
    pointer_sha256: String,
    acceptance_path: String,
    acceptance_sha256: String,
    acceptance_sequence: u32,
    run_id: String,
    run_evidence_sha256: String,
    seal_sha256: String,
    preapproval_commit: String,
    receipt_commit: String,
    approval_commit: String,
    publication_commit: String,
    closure_commit: String,
    technical_approval_sha256: String,
    data_governance_approval_sha256: String,
    verified_at_source_commit: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SchemaEntry {
    path: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SchemaBundle {
    schema: String,
    phase_id: String,
    interface_id: String,
    profile_id: String,
    entries: Vec<SchemaEntry>,
    bundle_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandEvidence {
    id: String,
    command_kind: String,
    argv: Vec<String>,
    cwd: String,
    exit_code: i32,
    status: String,
    started_at_utc: String,
    finished_at_utc: String,
    duration_ns: u64,
    network_mode: String,
    stdout: FileRef,
    stderr: FileRef,
    process_audit: Option<ProcessAudit>,
    execution_error_code: Option<String>,
}

#[derive(Clone, Debug)]
struct CapturedCommand {
    id: String,
    command_kind: String,
    argv: Vec<String>,
    cwd: String,
    exit_code: i32,
    started_at_utc: String,
    finished_at_utc: String,
    duration_ns: u64,
    network_mode: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    process_audit: Option<ProcessAudit>,
    execution_error_code: Option<String>,
}

#[derive(Default)]
struct P1aRecorder {
    commands: Vec<CapturedCommand>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Acceptance {
    schema: String,
    phase_id: String,
    interface_id: String,
    profile_id: String,
    support_tier: String,
    qualification_scope: String,
    qualification_tuple: Value,
    sequence: u32,
    acceptance_path: String,
    status: String,
    acceptance_kind: String,
    required_approvals: Vec<Value>,
    approvals: Vec<Value>,
    human_checkbox_review: String,
    run_path: String,
    run_evidence_sha256: String,
    seal_path: String,
    seal_sha256: String,
    source_commit: String,
    source_tree: String,
    p0a_acceptance_sha256: String,
    host_environment_sha256: String,
    cpu_isolation_sha256: String,
    native_abi_probe_sha256: String,
    run_id: String,
    previous_acceptance_path: Option<String>,
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
    support_tier: String,
    qualification_scope: String,
    sequence: u32,
    run_id: String,
    run_path: String,
    run_evidence_sha256: String,
    seal_path: String,
    seal_sha256: String,
    source_commit: String,
    source_tree: String,
    host_environment_sha256: String,
    cpu_isolation_sha256: String,
    native_abi_probe_sha256: String,
    previous_acceptance_sha256: Option<String>,
    host: Value,
    accelerator_provider: Option<Value>,
    accelerator_device: Option<Value>,
    memory_model: Option<Value>,
    backend_identity: Option<Value>,
    native_ml_library_identity: Option<Value>,
    sla: Option<Value>,
    acceptance_path: String,
    acceptance_sha256: String,
    updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AttemptMetadata {
    schema: String,
    run_id: String,
    stage_container_name: String,
    generated_at_utc: String,
    source: SourceIdentity,
    p0a_dependency: P0aDependency,
}

struct Admission {
    output_root: PathBuf,
    source: SourceIdentity,
    p0a_dependency: P0aDependency,
    schema_bundle: SchemaBundle,
    input_manifest_sha256: String,
    recorder: P1aRecorder,
}

struct WorkPaths {
    stage_container: PathBuf,
    stage_run: PathBuf,
    work_root: PathBuf,
    stage_container_name: String,
    stage_entropy: String,
    run_id: String,
    generated_at_utc: String,
}

enum BeginAttempt {
    Recovered(Value),
    New(WorkPaths),
}

#[cfg(windows)]
struct QualificationLock(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for QualificationLock {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::ReleaseMutex;
        // SAFETY: this guard owns the acquired mutex handle until drop.
        unsafe {
            let _ = ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn acquire_qualification_lock(repository: &Path) -> Result<QualificationLock> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    let repository = fs::canonicalize(repository).io_context(
        "P1A_LOCK_FAILED",
        "could not canonicalize the repository for the P1A qualification lock",
    )?;
    let name = format!(
        "Local\\python-slm-p1a-{}",
        hash::bytes(repository.as_os_str().to_string_lossy().as_bytes())
    );
    let mut wide = OsStr::new(&name).encode_wide().collect::<Vec<_>>();
    wide.push(0);
    // SAFETY: the name buffer is NUL-terminated and valid for this call.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
    if handle.is_null() {
        return Err(XtaskError::environment(
            "P1A_LOCK_FAILED",
            format!(
                "could not create the P1A qualification mutex: {}",
                std::io::Error::last_os_error()
            ),
        ));
    }
    // SAFETY: handle is a valid mutex handle and the zero timeout never blocks.
    let wait = unsafe { WaitForSingleObject(handle, 0) };
    if !matches!(wait, WAIT_OBJECT_0 | WAIT_ABANDONED) {
        // SAFETY: handle is valid and not owned by this process on this branch.
        unsafe { CloseHandle(handle) };
        return Err(XtaskError::gate(
            "P1A_QUALIFICATION_ALREADY_RUNNING",
            "another process already owns the exact P1A qualification namespace",
            "Wait for the active P1A qualification to finish, then retry.",
        ));
    }
    Ok(QualificationLock(handle))
}

pub fn qualify(
    repository: &Path,
    supplied_root: &Path,
    visual_studio_instance_id: Option<&str>,
) -> Result<Value> {
    require_native_prototype_dispatch()?;
    #[cfg(windows)]
    let _qualification_lock = acquire_qualification_lock(repository)?;
    let mut admission = preflight(repository, supplied_root)?;
    let work = match begin_attempt(repository, &admission)? {
        BeginAttempt::Recovered(result) => return Ok(result),
        BeginAttempt::New(work) => work,
    };
    let result =
        execute_qualification(repository, &mut admission, &work, visual_studio_instance_id);
    match result {
        Ok(result) => Ok(result),
        Err(error) => {
            if admission
                .output_root
                .join("runs")
                .join(&work.run_id)
                .exists()
            {
                // Publication crossed the immutable-run boundary. The durable attempt
                // journal is intentionally retained so the next invocation can finish
                // cleanup and, for PASS, acceptance/pointer publication without ever
                // creating a contradictory second terminal record for this run ID.
            } else {
                close_failed_attempt(repository, &admission, &work, &error)?;
            }
            Err(error)
        }
    }
}

pub fn check_selected(repository: &Path, supplied_root: &Path) -> Result<Value> {
    let output_root = publication::require_exact_output_root(
        repository,
        supplied_root,
        Path::new(OUTPUT_ROOT),
        PHASE_ID,
    )?;
    publication::require_no_follow_tree(&output_root)?;
    let mut historical_recorder = P1aRecorder::default();
    require_historical_p1a_immutable(repository, &mut historical_recorder)?;
    inspect_existing_namespace_read_only(&output_root)?;
    require_no_unfinished_publication(&output_root)?;
    let bundle = build_schema_bundle(repository)?;
    require_schema_contracts(repository, &bundle)?;
    let staging = output_root.join(".staging");
    if staging.exists()
        && fs::read_dir(&staging)
            .io_context(
                "P1A_STAGING_ENUMERATION_FAILED",
                "could not inspect P1A staging during selected verification",
            )?
            .next()
            .transpose()
            .io_context(
                "P1A_STAGING_ENUMERATION_FAILED",
                "could not read P1A staging during selected verification",
            )?
            .is_some()
    {
        return Err(XtaskError::integrity(
            "P1A_PUBLICATION_INCOMPLETE",
            "selected P1A verification found unfinished publication state",
        ));
    }
    validate_all_terminal_runs(repository, &output_root)?;
    validate_selected_receipt(repository, &output_root)?;
    validate_selected_artifact_bindings(repository, &output_root)?;
    require_namespace_committed(repository, &output_root)?;
    let pointer: Pointer = read_json(
        &output_root.join("evidence.json"),
        "P1A_POINTER_JSON_INVALID",
    )?;
    Ok(json!({
        "schema": "python-slm-p1a-selected-result-v1",
        "phase_id": PHASE_ID,
        "status": "PASS",
        "profile_id": PROFILE_ID,
        "support_tier": SUPPORT_TIER,
        "sequence": pointer.sequence,
        "acceptance_path": pointer.acceptance_path,
        "acceptance_sha256": pointer.acceptance_sha256,
        "run_id": pointer.run_id,
        "run_evidence_sha256": pointer.run_evidence_sha256,
        "seal_sha256": pointer.seal_sha256
    }))
}

fn require_no_unfinished_publication(output_root: &Path) -> Result<()> {
    for entry in fs::read_dir(output_root).io_context(
        "P1A_NAMESPACE_ENUMERATION_FAILED",
        "could not inspect P1A root publication state",
    )? {
        let entry = entry.io_context(
            "P1A_NAMESPACE_ENUMERATION_FAILED",
            "could not read P1A root publication state",
        )?;
        let file_name = entry.file_name();
        if file_name.to_str().is_some_and(valid_pointer_temp_name) {
            return Err(XtaskError::integrity(
                "P1A_PUBLICATION_INCOMPLETE",
                "selected P1A verification found an unfinished pointer publication",
            ));
        }
    }
    for entry in fs::read_dir(output_root.join("acceptances")).io_context(
        "P1A_ACCEPTANCE_ENUMERATION_FAILED",
        "could not inspect P1A acceptance publication state",
    )? {
        let entry = entry.io_context(
            "P1A_ACCEPTANCE_ENUMERATION_FAILED",
            "could not read P1A acceptance publication state",
        )?;
        let file_name = entry.file_name();
        if file_name
            .to_str()
            .and_then(acceptance_temp_target)
            .is_some()
        {
            return Err(XtaskError::integrity(
                "P1A_PUBLICATION_INCOMPLETE",
                "selected P1A verification found an unfinished acceptance publication",
            ));
        }
    }
    Ok(())
}

fn preflight(repository: &Path, supplied_root: &Path) -> Result<Admission> {
    let output_root = publication::require_exact_output_root(
        repository,
        supplied_root,
        Path::new(OUTPUT_ROOT),
        PHASE_ID,
    )?;
    publication::require_no_follow_tree(&output_root)?;
    inspect_existing_namespace_read_only(&output_root)?;
    require_native_prototype_dispatch()?;
    let schema_bundle = build_schema_bundle(repository)?;
    require_schema_contracts(repository, &schema_bundle)?;
    require_probe_sources(repository)?;
    require_build_policy(repository)?;
    let mut recorder = P1aRecorder::default();
    require_historical_p1a_immutable(repository, &mut recorder)?;
    let commit = git_line(
        &mut recorder,
        repository,
        &["rev-parse", "HEAD"],
        "P1A_SOURCE_COMMIT_INVALID",
    )?;
    let tree = git_line(
        &mut recorder,
        repository,
        &["rev-parse", "HEAD^{tree}"],
        "P1A_SOURCE_TREE_INVALID",
    )?;
    let branch = git_line(
        &mut recorder,
        repository,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        "P1A_SOURCE_BRANCH_INVALID",
    )?;
    require_source_clean(repository, &mut recorder)?;
    let input_manifest_sha256 = source_manifest_hash(repository, &mut recorder)?;
    let p0a_dependency = selected_p0a_dependency(repository, &commit, &mut recorder)?;
    let source = SourceIdentity {
        schema: "python-slm-p1a-source-identity-v1".to_owned(),
        phase_id: PHASE_ID.to_owned(),
        interface_id: INTERFACE_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        support_tier: SUPPORT_TIER.to_owned(),
        commit,
        tree,
        branch,
        dirty: false,
        cargo_lock_sha256: hash::file(&repository.join("Cargo.lock"))?,
        verifier_source_sha256: verifier_bundle_hash(repository)?,
        schema_bundle_sha256: schema_bundle.bundle_sha256.clone(),
        p0a_pointer_sha256: P0A_POINTER_SHA256.to_owned(),
    };
    Ok(Admission {
        output_root,
        source,
        p0a_dependency,
        schema_bundle,
        input_manifest_sha256,
        recorder,
    })
}

fn require_native_prototype_dispatch() -> Result<()> {
    if !cfg!(all(windows, target_arch = "x86_64"))
        || std::env::consts::OS != "windows"
        || std::env::consts::ARCH != "x86_64"
    {
        return Err(XtaskError::gate(
            "DEFERRED_POST_P16",
            "P1A is implemented only for the native Windows x86_64 prototype host",
            "Use prototype-windows-5090-v1 on Windows or wait for P17.",
        ));
    }
    Ok(())
}

fn qualification_tuple() -> Value {
    json!({
        "host": {
            "os_family": "windows",
            "architecture": "x86_64",
            "cpu_vendor": EXPECTED_CPU_VENDOR,
            "cpu_model": EXPECTED_CPU_BRAND,
            "rust_target": "x86_64-pc-windows-msvc",
            "c_cpp_toolchain": "msvc"
        },
        "accelerator_provider": null,
        "accelerator_device": null,
        "memory_model": null,
        "backend_identity": null,
        "native_ml_library_identity": null,
        "sla": null
    })
}

fn minimal_native_environment() -> Result<BTreeMap<String, Option<OsString>>> {
    let path = system32()?;
    let mut environment = BTreeMap::from([("PATH".to_owned(), Some(path.as_os_str().to_owned()))]);
    for name in [
        "CUDA_PATH",
        "CUDA_HOME",
        "CUDNN_PATH",
        "HIP_PATH",
        "ROCM_PATH",
        "LIBTORCH",
        "PYTHONHOME",
        "PYTHONPATH",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CC",
        "CXX",
        "AR",
        "CFLAGS",
        "CXXFLAGS",
        "LDFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_BUILD_RUSTC_WRAPPER",
    ] {
        environment.insert(name.to_owned(), None);
    }
    Ok(environment)
}

fn parse_stage_container_name(name: &str) -> Result<(&str, &str)> {
    let (run_id, entropy) = name.split_once(".work-").ok_or_else(|| {
        XtaskError::integrity(
            "P1A_STAGING_OWNERSHIP_INVALID",
            "P1A stage container does not use the exact <run_id>.work-<hex> grammar",
        )
    })?;
    if name.matches(".work-").count() != 1
        || !valid_run_id(run_id)
        || !(16..=32).contains(&entropy.len())
        || !entropy
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(XtaskError::integrity(
            "P1A_STAGING_OWNERSHIP_INVALID",
            "P1A stage container does not use the exact <run_id>.work-<hex> grammar",
        ));
    }
    Ok((run_id, entropy))
}

fn parse_finalization_marker_name(name: &str) -> Result<(&str, &str)> {
    let stem = name.strip_suffix(".json").ok_or_else(|| {
        XtaskError::integrity(
            "P1A_FINALIZATION_MARKER_INVALID",
            "P1A finalization marker has an invalid suffix",
        )
    })?;
    let (run_id, entropy) = stem.split_once(".finalizing-").ok_or_else(|| {
        XtaskError::integrity(
            "P1A_FINALIZATION_MARKER_INVALID",
            "P1A finalization marker does not use the exact run-bound grammar",
        )
    })?;
    if stem.matches(".finalizing-").count() != 1
        || !valid_run_id(run_id)
        || !(16..=32).contains(&entropy.len())
        || !entropy
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(XtaskError::integrity(
            "P1A_FINALIZATION_MARKER_INVALID",
            "P1A finalization marker does not use the exact run-bound grammar",
        ));
    }
    Ok((run_id, entropy))
}

fn generated_timestamp_binds_run_id(generated_at_utc: &str, run_id: &str) -> bool {
    let bytes = generated_at_utc.as_bytes();
    if bytes.len() != 30
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[29] != b'Z'
        || !bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 29) || byte.is_ascii_digit()
        })
    {
        return false;
    }
    let prefix = format!(
        "{}{}{}T{}{}{}{}Z",
        &generated_at_utc[0..4],
        &generated_at_utc[5..7],
        &generated_at_utc[8..10],
        &generated_at_utc[11..13],
        &generated_at_utc[14..16],
        &generated_at_utc[17..19],
        &generated_at_utc[20..23]
    );
    run_id
        .strip_prefix(&prefix)
        .is_some_and(|suffix| suffix.len() == 25 && suffix.starts_with('-'))
}

fn inspect_existing_namespace_read_only(output_root: &Path) -> Result<()> {
    if !output_root.exists() {
        return Ok(());
    }
    publication::require_no_follow_tree(output_root)?;
    let mut root_names = BTreeSet::new();
    for entry in fs::read_dir(output_root).io_context(
        "P1A_NAMESPACE_ENUMERATION_FAILED",
        "could not enumerate the P1A receipt namespace",
    )? {
        let entry = entry.io_context(
            "P1A_NAMESPACE_ENUMERATION_FAILED",
            "could not read a P1A receipt namespace entry",
        )?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_NAMESPACE_ENTRY_INVALID",
                    "P1A receipt namespace contains a non-UTF-8 name",
                )
            })?
            .to_owned();
        if !root_names.insert(name.to_ascii_lowercase()) {
            return Err(XtaskError::integrity(
                "P1A_NAMESPACE_ENTRY_DUPLICATE",
                "P1A receipt namespace contains a case-colliding duplicate entry",
            ));
        }
        let metadata = fs::symlink_metadata(entry.path()).io_context(
            "P1A_NAMESPACE_ENTRY_INVALID",
            "could not inspect a P1A receipt namespace entry",
        )?;
        let valid = match name.as_str() {
            ".staging" | "acceptances" | "runs" => metadata.is_dir(),
            "evidence.json" => metadata.is_file(),
            _ => metadata.is_file() && valid_pointer_temp_name(&name),
        };
        if !valid {
            return Err(XtaskError::integrity(
                "P1A_NAMESPACE_ENTRY_INVALID",
                format!("unexpected P1A receipt namespace entry: {name}"),
            ));
        }
    }

    let staging = output_root.join(".staging");
    if staging.exists() {
        let mut stage_names = BTreeSet::new();
        for entry in fs::read_dir(&staging).io_context(
            "P1A_STAGING_ENUMERATION_FAILED",
            "could not enumerate P1A staging",
        )? {
            let entry = entry.io_context(
                "P1A_STAGING_ENUMERATION_FAILED",
                "could not read P1A staging entry",
            )?;
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| {
                    XtaskError::integrity(
                        "P1A_STAGING_OWNERSHIP_INVALID",
                        "P1A staging contains a non-UTF-8 name",
                    )
                })?
                .to_owned();
            if !stage_names.insert(name.to_ascii_lowercase()) {
                return Err(XtaskError::integrity(
                    "P1A_STAGING_OWNERSHIP_INVALID",
                    "P1A staging contains a case-colliding duplicate entry",
                ));
            }
            let metadata = fs::symlink_metadata(entry.path()).io_context(
                "P1A_STAGING_INSPECTION_FAILED",
                "could not inspect P1A staging entry",
            )?;
            if metadata.is_dir() {
                parse_stage_container_name(&name)?;
            } else if metadata.is_file() {
                parse_finalization_marker_name(&name)?;
            } else {
                return Err(XtaskError::integrity(
                    "P1A_STAGING_OWNERSHIP_INVALID",
                    "P1A staging contains a non-regular entry",
                ));
            }
        }
    }

    let runs = output_root.join("runs");
    if runs.exists() {
        let mut run_ids = BTreeSet::new();
        for entry in fs::read_dir(&runs)
            .io_context("P1A_RUN_ENUMERATION_FAILED", "could not enumerate P1A runs")?
        {
            let entry =
                entry.io_context("P1A_RUN_ENUMERATION_FAILED", "could not read P1A run entry")?;
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| {
                    XtaskError::integrity("P1A_RUN_ID_INVALID", "P1A run name is not UTF-8")
                })?
                .to_owned();
            let metadata = fs::symlink_metadata(entry.path()).io_context(
                "P1A_RUN_INSPECTION_FAILED",
                "could not inspect P1A run entry",
            )?;
            if !valid_run_id(&name) || !metadata.is_dir() || !run_ids.insert(name) {
                return Err(XtaskError::integrity(
                    "P1A_RUN_ID_INVALID",
                    "P1A runs contain an invalid or duplicate run directory",
                ));
            }
        }
    }
    let acceptances = output_root.join("acceptances");
    if acceptances.exists() {
        let mut has_temporary = false;
        for entry in fs::read_dir(&acceptances).io_context(
            "P1A_ACCEPTANCE_ENUMERATION_FAILED",
            "could not inspect P1A acceptances read-only",
        )? {
            let entry = entry.io_context(
                "P1A_ACCEPTANCE_ENUMERATION_FAILED",
                "could not read P1A acceptance read-only entry",
            )?;
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| {
                    XtaskError::integrity(
                        "P1A_ACCEPTANCE_SEQUENCE_INVALID",
                        "P1A acceptance name is not UTF-8",
                    )
                })?
                .to_owned();
            if acceptance_temp_target(&name).is_some() {
                has_temporary = true;
                let metadata = fs::symlink_metadata(entry.path()).io_context(
                    "P1A_ACCEPTANCE_INSPECTION_FAILED",
                    "could not inspect P1A acceptance temporary",
                )?;
                if !metadata.is_file() {
                    return Err(XtaskError::integrity(
                        "P1A_ACCEPTANCE_SEQUENCE_INVALID",
                        "P1A acceptance temporary is not a regular file",
                    ));
                }
                let _: Acceptance = read_json(&entry.path(), "P1A_ACCEPTANCE_JSON_INVALID")?;
            }
        }
        if !has_temporary {
            let _ = acceptance_inventory(output_root)?;
        }
    }
    Ok(())
}

fn read_recovery_attempt(admission: &Admission) -> Result<Option<AttemptMetadata>> {
    let staging = admission.output_root.join(".staging");
    let mut entries = fs::read_dir(&staging)
        .io_context(
            "P1A_STAGING_ENUMERATION_FAILED",
            "could not enumerate P1A staging before recovery",
        )?
        .collect::<std::io::Result<Vec<_>>>()
        .io_context(
            "P1A_STAGING_ENUMERATION_FAILED",
            "could not read P1A staging before recovery",
        )?;
    entries.sort_by_key(|entry| entry.file_name());
    if entries.len() > 1 {
        return Err(XtaskError::integrity(
            "P1A_STAGING_RECOVERY_AMBIGUOUS",
            "P1A staging contains more than one unfinished attempt or finalization journal",
        ));
    }
    let Some(entry) = entries.pop() else {
        return Ok(None);
    };
    let metadata = fs::symlink_metadata(entry.path()).io_context(
        "P1A_STAGING_INSPECTION_FAILED",
        "could not inspect P1A staging before recovery",
    )?;
    let name = entry
        .file_name()
        .to_str()
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_STAGING_OWNERSHIP_INVALID",
                "P1A staging contains a non-UTF-8 entry",
            )
        })?
        .to_owned();
    if metadata.is_file() {
        let (run_id, entropy) = parse_finalization_marker_name(&name)?;
        let attempt: AttemptMetadata = read_json(&entry.path(), "P1A_ATTEMPT_JSON_INVALID")?;
        validate_attempt_binding(&attempt, admission, &format!("{run_id}.work-{entropy}"))?;
        return Ok(Some(attempt));
    }
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(XtaskError::integrity(
            "P1A_STAGING_OWNERSHIP_INVALID",
            "P1A staging recovery entry is not a regular owned directory or journal",
        ));
    }
    parse_stage_container_name(&name)?;
    let attempt_path = entry.path().join("attempt.json");
    if !attempt_path.exists() {
        return Ok(None);
    }
    let attempt: AttemptMetadata = read_json(&attempt_path, "P1A_ATTEMPT_JSON_INVALID")?;
    validate_attempt_binding(&attempt, admission, &name)?;
    Ok(Some(attempt))
}

fn recovery_temporary_paths(output_root: &Path) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    let mut acceptances = Vec::new();
    for entry in fs::read_dir(output_root.join("acceptances")).io_context(
        "P1A_ACCEPTANCE_ENUMERATION_FAILED",
        "could not enumerate acceptance temporaries before recovery",
    )? {
        let entry = entry.io_context(
            "P1A_ACCEPTANCE_ENUMERATION_FAILED",
            "could not read acceptance publication state before recovery",
        )?;
        if entry
            .file_name()
            .to_str()
            .and_then(acceptance_temp_target)
            .is_some()
        {
            acceptances.push(entry.path());
        }
    }
    let mut pointers = Vec::new();
    for entry in fs::read_dir(output_root).io_context(
        "P1A_NAMESPACE_ENUMERATION_FAILED",
        "could not enumerate pointer temporaries before recovery",
    )? {
        let entry = entry.io_context(
            "P1A_NAMESPACE_ENUMERATION_FAILED",
            "could not read pointer publication state before recovery",
        )?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(valid_pointer_temp_name)
        {
            pointers.push(entry.path());
        }
    }
    if acceptances.len() > 1 || pointers.len() > 1 {
        return Err(XtaskError::integrity(
            "P1A_PUBLICATION_RECOVERY_AMBIGUOUS",
            "P1A contains multiple unfinished acceptance or pointer publications",
        ));
    }
    Ok((acceptances.pop(), pointers.pop()))
}

fn pointer_equals_inventory_item(
    pointer: &Pointer,
    item: &(u32, PathBuf, Acceptance, String),
) -> bool {
    pointer == &pointer_for_acceptance(&item.2, item.3.clone())
}

fn validate_recovery_pointer_state(
    output_root: &Path,
    active_run_id: Option<&str>,
    acceptance_temporary: Option<&Path>,
    pointer_temporary: Option<&Path>,
) -> Result<Vec<(u32, PathBuf, Acceptance, String)>> {
    if active_run_id.is_none() && (acceptance_temporary.is_some() || pointer_temporary.is_some()) {
        return Err(XtaskError::integrity(
            "P1A_PUBLICATION_JOURNAL_MISSING",
            "unfinished acceptance or pointer publication has no durable attempt journal",
        ));
    }
    let inventory = acceptance_inventory_excluding(output_root, acceptance_temporary)?;
    let active_items = inventory
        .iter()
        .filter(|(_, _, acceptance, _)| Some(acceptance.run_id.as_str()) == active_run_id)
        .collect::<Vec<_>>();
    if active_items.len() > 1
        || active_items
            .first()
            .is_some_and(|item| inventory.last().map(|last| last.0) != Some(item.0))
    {
        return Err(XtaskError::integrity(
            "P1A_ACCEPTANCE_CHAIN_INVALID",
            "the journal-bound recovery acceptance is not the unique latest acceptance",
        ));
    }
    if let Some(temporary) = acceptance_temporary {
        let acceptance: Acceptance = read_json(temporary, "P1A_ACCEPTANCE_JSON_INVALID")?;
        if Some(acceptance.run_id.as_str()) != active_run_id {
            return Err(XtaskError::integrity(
                "P1A_ACCEPTANCE_RECOVERY_INVALID",
                "acceptance temporary does not bind the active durable attempt journal",
            ));
        }
    }

    let current_pointer = if output_root.join("evidence.json").exists() {
        Some(read_json::<Pointer>(
            &output_root.join("evidence.json"),
            "P1A_POINTER_JSON_INVALID",
        )?)
    } else {
        None
    };
    if active_run_id.is_none() {
        match (current_pointer.as_ref(), inventory.last()) {
            (None, None) => {}
            (Some(pointer), Some(item)) if pointer_equals_inventory_item(pointer, item) => {}
            _ => {
                return Err(XtaskError::integrity(
                    "P1A_POINTER_RECOVERY_INVALID",
                    "finalized P1A pointer is not the exact latest acceptance projection",
                ));
            }
        }
        return Ok(inventory);
    }

    let target = active_items.first().copied();
    let predecessor = match target {
        Some(item) => inventory
            .iter()
            .rev()
            .find(|candidate| candidate.0 < item.0),
        None => inventory.last(),
    };
    let current_is_allowed = match current_pointer.as_ref() {
        None => predecessor.is_none(),
        Some(pointer) => {
            predecessor.is_some_and(|item| pointer_equals_inventory_item(pointer, item))
                || target.is_some_and(|item| pointer_equals_inventory_item(pointer, item))
        }
    };
    if !current_is_allowed {
        return Err(XtaskError::integrity(
            "P1A_POINTER_RECOVERY_INVALID",
            "recovery pointer is neither the exact immediate predecessor nor the journal-bound target",
        ));
    }
    if let Some(temporary) = pointer_temporary {
        let pointer: Pointer = read_json(temporary, "P1A_POINTER_JSON_INVALID")?;
        if !target.is_some_and(|item| pointer_equals_inventory_item(&pointer, item)) {
            return Err(XtaskError::integrity(
                "P1A_POINTER_RECOVERY_INVALID",
                "pointer temporary is not the exact journal-bound latest acceptance projection",
            ));
        }
    }
    Ok(inventory)
}

fn parse_status_paths(bytes: &[u8]) -> Result<Vec<String>> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        XtaskError::integrity(
            "P1A_SELECTED_NAMESPACE_DIRTY",
            "Git status returned non-UTF-8 P1A paths",
        )
    })?;
    text.lines()
        .map(|line| {
            if line.len() < 4
                || line.as_bytes().get(2) != Some(&b' ')
                || line[3..].starts_with('"')
                || line[3..].contains(" -> ")
            {
                return Err(XtaskError::integrity(
                    "P1A_SELECTED_NAMESPACE_DIRTY",
                    "Git status returned a noncanonical P1A path record",
                ));
            }
            Ok(line[3..].replace('\\', "/"))
        })
        .collect()
}

fn validate_committed_prefix_before_recovery(
    repository: &Path,
    admission: &Admission,
) -> Result<()> {
    let attempt = read_recovery_attempt(admission)?;
    let active_run_id = attempt.as_ref().map(|attempt| attempt.run_id.as_str());
    let (acceptance_temporary, pointer_temporary) =
        recovery_temporary_paths(&admission.output_root)?;
    let inventory = validate_recovery_pointer_state(
        &admission.output_root,
        active_run_id,
        acceptance_temporary.as_deref(),
        pointer_temporary.as_deref(),
    )?;
    validate_terminal_runs_except(
        repository,
        &admission.output_root,
        active_run_id,
        acceptance_temporary.as_deref(),
        false,
    )?;

    let active_acceptance_path = inventory
        .iter()
        .find(|(_, _, acceptance, _)| Some(acceptance.run_id.as_str()) == active_run_id)
        .map(|(_, path, _, _)| path.clone());
    let mut allowed_dirty = vec![format!("{OUTPUT_ROOT}/.staging/")];
    if let Some(run_id) = active_run_id {
        allowed_dirty.push(format!("{OUTPUT_ROOT}/runs/{run_id}/"));
        allowed_dirty.push(format!("{OUTPUT_ROOT}/evidence.json"));
    }
    for path in [
        acceptance_temporary.as_deref(),
        pointer_temporary.as_deref(),
        active_acceptance_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let relative = path.strip_prefix(&admission.output_root).map_err(|_| {
            XtaskError::integrity(
                "P1A_PUBLICATION_RECOVERY_INVALID",
                "recovery publication path escaped the P1A receipt root",
            )
        })?;
        allowed_dirty.push(format!(
            "{OUTPUT_ROOT}/{}",
            relative.to_string_lossy().replace('\\', "/")
        ));
    }

    let mut recorder = P1aRecorder::default();
    let status = recorder.run_git(
        repository,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            OUTPUT_ROOT,
        ],
    )?;
    if status.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_SELECTED_NAMESPACE_DIRTY",
            "could not inspect the committed P1A recovery prefix",
        ));
    }
    for path in parse_status_paths(&status.stdout)? {
        if !allowed_dirty.iter().any(|allowed| {
            path == *allowed || (allowed.ends_with('/') && path.starts_with(allowed))
        }) {
            return Err(XtaskError::integrity(
                "P1A_SELECTED_NAMESPACE_DIRTY",
                format!("committed P1A recovery prefix changed at {path}"),
            ));
        }
    }

    let tracked = recorder.run_git(repository, &["ls-files", "--", OUTPUT_ROOT])?;
    if tracked.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_SELECTED_NAMESPACE_UNTRACKED",
            "could not enumerate tracked P1A recovery-prefix files",
        ));
    }
    let tracked = std::str::from_utf8(&tracked.stdout)
        .map_err(|_| {
            XtaskError::integrity(
                "P1A_SELECTED_NAMESPACE_UNTRACKED",
                "tracked P1A recovery-prefix inventory is not UTF-8",
            )
        })?
        .lines()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut required_relative = Vec::new();
    for entry in fs::read_dir(admission.output_root.join("runs")).io_context(
        "P1A_RUN_ENUMERATION_FAILED",
        "could not enumerate P1A runs for recovery-prefix tracking",
    )? {
        let entry = entry.io_context(
            "P1A_RUN_ENUMERATION_FAILED",
            "could not read P1A run for recovery-prefix tracking",
        )?;
        if entry.file_name().to_str() == active_run_id {
            continue;
        }
        collect_namespace_files(
            &admission.output_root,
            &entry.path(),
            &mut required_relative,
        )?;
    }
    for (_, path, acceptance, _) in &inventory {
        if Some(acceptance.run_id.as_str()) == active_run_id {
            continue;
        }
        required_relative.push(
            path.strip_prefix(&admission.output_root)
                .expect("acceptance is below output root")
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    let prior_acceptance_exists = inventory
        .iter()
        .any(|(_, _, acceptance, _)| Some(acceptance.run_id.as_str()) != active_run_id);
    if admission.output_root.join("evidence.json").exists()
        && (active_run_id.is_none() || prior_acceptance_exists)
    {
        required_relative.push("evidence.json".to_owned());
    }
    for relative in required_relative {
        let path = format!("{OUTPUT_ROOT}/{relative}");
        if !tracked.contains(&path) {
            return Err(XtaskError::integrity(
                "P1A_SELECTED_NAMESPACE_UNTRACKED",
                format!("committed P1A recovery-prefix file is not tracked: {path}"),
            ));
        }
    }
    Ok(())
}

fn begin_attempt(repository: &Path, admission: &Admission) -> Result<BeginAttempt> {
    let resolved = publication::require_exact_output_root(
        repository,
        Path::new(OUTPUT_ROOT),
        Path::new(OUTPUT_ROOT),
        PHASE_ID,
    )?;
    if resolved != admission.output_root {
        return Err(XtaskError::integrity(
            "P1A_OUTPUT_ROOT_CHANGED",
            "P1A output-root identity changed between admission and mutation",
        ));
    }
    publication::create_dir_all(&admission.output_root)?;
    publication::require_no_follow_tree(&admission.output_root)?;
    publication::create_dir_all(&admission.output_root.join("runs"))?;
    publication::create_dir_all(&admission.output_root.join("acceptances"))?;
    publication::create_dir_all(&admission.output_root.join(".staging"))?;
    validate_committed_prefix_before_recovery(repository, admission)?;
    recover_publication_temporaries(admission)?;
    if let Some(result) = recover_interrupted_attempts(repository, admission)? {
        return Ok(BeginAttempt::Recovered(result));
    }
    let pointer_exists = admission.output_root.join("evidence.json").exists();
    let acceptance_exists = fs::read_dir(admission.output_root.join("acceptances"))
        .io_context(
            "P1A_ACCEPTANCE_ENUMERATION_FAILED",
            "could not inspect P1A acceptances before a new attempt",
        )?
        .next()
        .transpose()
        .io_context(
            "P1A_ACCEPTANCE_ENUMERATION_FAILED",
            "could not read P1A acceptance before a new attempt",
        )?
        .is_some();
    if pointer_exists || acceptance_exists {
        if !(pointer_exists && acceptance_exists) {
            return Err(XtaskError::integrity(
                "P1A_SELECTION_PUBLICATION_INCOMPLETE",
                "P1A contains a pointer or acceptance without its required counterpart",
            ));
        }
        validate_selected_receipt(repository, &admission.output_root)?;
        validate_selected_artifact_bindings(repository, &admission.output_root)?;
    }
    let runs_present = fs::read_dir(admission.output_root.join("runs"))
        .io_context(
            "P1A_RUN_ENUMERATION_FAILED",
            "could not inspect P1A terminal runs before a new attempt",
        )?
        .next()
        .transpose()
        .io_context(
            "P1A_RUN_ENUMERATION_FAILED",
            "could not read P1A terminal runs before a new attempt",
        )?
        .is_some();
    if runs_present {
        validate_all_terminal_runs(repository, &admission.output_root)?;
        require_namespace_committed(repository, &admission.output_root)?;
    }
    let (generated_at_utc, prefix, entropy) = time::now();
    let suffix = hash::bytes(
        format!(
            "{}:{entropy}:{}",
            std::process::id(),
            admission.source.commit
        )
        .as_bytes(),
    );
    let run_id = format!("{prefix}-{}", &suffix[..24]);
    let stage_entropy = format!("{entropy:x}");
    let stage_container_name = format!("{run_id}.work-{stage_entropy}");
    let stage_container = admission
        .output_root
        .join(".staging")
        .join(&stage_container_name);
    publication::create_dir(&stage_container)?;
    let attempt = AttemptMetadata {
        schema: "python-slm-p1a-attempt-v1".to_owned(),
        run_id: run_id.clone(),
        stage_container_name: stage_container_name.clone(),
        generated_at_utc: generated_at_utc.clone(),
        source: admission.source.clone(),
        p0a_dependency: admission.p0a_dependency.clone(),
    };
    publication::write_json_new_via_owned_temp(
        &stage_container.join("attempt.json"),
        &attempt,
        "p1a-attempt",
    )?;
    let work_root = stage_container.join("private-work");
    publication::create_dir(&work_root)?;
    Ok(BeginAttempt::New(WorkPaths {
        stage_run: stage_container.join(&run_id),
        stage_container,
        work_root,
        stage_container_name,
        stage_entropy,
        run_id,
        generated_at_utc,
    }))
}

fn recover_interrupted_attempts(repository: &Path, admission: &Admission) -> Result<Option<Value>> {
    let staging = admission.output_root.join(".staging");
    if !staging.exists() {
        return Ok(None);
    }
    publication::require_no_follow_tree(&staging)?;
    let mut entries = fs::read_dir(&staging)
        .io_context(
            "P1A_STAGING_ENUMERATION_FAILED",
            "could not enumerate P1A staging",
        )?
        .collect::<std::io::Result<Vec<_>>>()
        .io_context(
            "P1A_STAGING_ENUMERATION_FAILED",
            "could not read P1A staging entry",
        )?;
    entries.sort_by_key(|entry| entry.file_name());
    if entries.len() > 1 {
        return Err(XtaskError::integrity(
            "P1A_STAGING_RECOVERY_AMBIGUOUS",
            "P1A staging contains more than one unfinished attempt or finalization journal",
        ));
    }
    for entry in entries {
        let metadata = fs::symlink_metadata(entry.path()).io_context(
            "P1A_STAGING_INSPECTION_FAILED",
            "could not inspect P1A staging entry",
        )?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_STAGING_OWNERSHIP_INVALID",
                    "P1A staging contains a non-UTF-8 entry",
                )
            })?
            .to_owned();
        if metadata.is_file() {
            let (marker_run_id, marker_entropy) = parse_finalization_marker_name(&name)?;
            let attempt: AttemptMetadata = read_json(&entry.path(), "P1A_ATTEMPT_JSON_INVALID")?;
            validate_attempt_binding(
                &attempt,
                admission,
                &format!("{marker_run_id}.work-{marker_entropy}"),
            )?;
            let status = recover_published_attempt(repository, admission, &attempt)?;
            finish_stage_cleanup(&entry.path())?;
            if status == "PASS" {
                return Ok(Some(recovered_selected_result(
                    repository,
                    &admission.output_root,
                )?));
            }
            return Err(recovered_fail_error());
        }
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(XtaskError::integrity(
                "P1A_STAGING_OWNERSHIP_INVALID",
                "P1A staging contains an unowned or non-directory entry",
            ));
        }
        let (stage_run_id, stage_entropy) = parse_stage_container_name(&name)?;
        let stage_run_id = stage_run_id.to_owned();
        let stage_entropy = stage_entropy.to_owned();
        let attempt_path = entry.path().join("attempt.json");
        if !attempt_path.is_file() {
            let children = fs::read_dir(entry.path())
                .io_context(
                    "P1A_STAGING_INSPECTION_FAILED",
                    "could not inspect orphan stage",
                )?
                .collect::<std::io::Result<Vec<_>>>()
                .io_context(
                    "P1A_STAGING_INSPECTION_FAILED",
                    "could not read orphan stage",
                )?;
            if children.is_empty()
                || children.iter().all(|child| {
                    child
                        .file_name()
                        .to_str()
                        .is_some_and(valid_attempt_temp_name)
                        && child.file_type().is_ok_and(|kind| kind.is_file())
                })
            {
                for child in children {
                    fs::remove_file(child.path()).io_context(
                        "P1A_STAGING_CLEANUP_FAILED",
                        "could not remove an exact pre-identity attempt temporary",
                    )?;
                }
                fs::remove_dir(entry.path()).io_context(
                    "P1A_STAGING_CLEANUP_FAILED",
                    "could not remove empty pre-identity P1A staging directory",
                )?;
                continue;
            }
            return Err(XtaskError::integrity(
                "P1A_STAGING_IDENTITY_MISSING",
                "interrupted P1A staging has no recoverable attempt identity",
            ));
        }
        let attempt: AttemptMetadata = read_json(&attempt_path, "P1A_ATTEMPT_JSON_INVALID")?;
        validate_attempt_binding(&attempt, admission, &name)?;
        if attempt.run_id != stage_run_id || attempt.stage_container_name != name {
            return Err(XtaskError::integrity(
                "P1A_INTERRUPTED_ATTEMPT_BINDING_INVALID",
                "interrupted P1A attempt does not bind its exact stage container",
            ));
        }
        let recovery_error = XtaskError::new(
            "P1A_ATTEMPT_INTERRUPTED",
            Category::Internal,
            "an admitted P1A attempt was interrupted before terminal publication",
            "Review the sealed FAIL receipt, then rerun from the same clean source.",
        );
        let work = WorkPaths {
            stage_container: entry.path(),
            stage_run: entry.path().join(&attempt.run_id),
            work_root: entry.path().join("private-work"),
            stage_container_name: name,
            stage_entropy,
            run_id: attempt.run_id.clone(),
            generated_at_utc: attempt.generated_at_utc.clone(),
        };
        let recovered = Admission {
            output_root: admission.output_root.clone(),
            source: attempt.source.clone(),
            p0a_dependency: attempt.p0a_dependency.clone(),
            schema_bundle: admission.schema_bundle.clone(),
            input_manifest_sha256: admission.input_manifest_sha256.clone(),
            recorder: P1aRecorder::default(),
        };
        if recovered
            .output_root
            .join("runs")
            .join(&work.run_id)
            .exists()
        {
            let status = recover_published_attempt(repository, &recovered, &attempt)?;
            let marker = cleanup_stage_after_publication(&work)?;
            finish_stage_cleanup(&marker)?;
            if status == "PASS" {
                return Ok(Some(recovered_selected_result(
                    repository,
                    &admission.output_root,
                )?));
            }
            return Err(recovered_fail_error());
        } else {
            if work.stage_run.exists() {
                discard_unpublished_stage_run(&work)?;
            }
            close_failed_attempt(repository, &recovered, &work, &recovery_error)?;
            return Err(recovered_fail_error());
        }
    }
    Ok(None)
}

fn recovered_fail_error() -> XtaskError {
    XtaskError::gate(
        "P1A_RECOVERED_FAILED_ATTEMPT",
        "the interrupted P1A attempt is now durably closed as FAIL",
        "Review and commit the immutable FAIL run before starting another P1A attempt.",
    )
}

fn recovered_selected_result(repository: &Path, output_root: &Path) -> Result<Value> {
    validate_all_terminal_runs(repository, output_root)?;
    validate_selected_receipt(repository, output_root)?;
    let pointer: Pointer = read_json(
        &output_root.join("evidence.json"),
        "P1A_POINTER_JSON_INVALID",
    )?;
    Ok(json!({
        "schema": "python-slm-p1a-qualification-result-v1",
        "phase_id": PHASE_ID,
        "status": "PASS",
        "profile_id": PROFILE_ID,
        "support_tier": SUPPORT_TIER,
        "qualification_scope": QUALIFICATION_SCOPE,
        "run_path": pointer.run_path,
        "run_evidence_sha256": pointer.run_evidence_sha256,
        "seal_sha256": pointer.seal_sha256,
        "acceptance_path": pointer.acceptance_path,
        "acceptance_sha256": pointer.acceptance_sha256,
        "human_checkbox_review": "PENDING",
        "recovered": true
    }))
}

fn valid_attempt_temp_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix("attempt.json.tmp-p1a-attempt-") else {
        return false;
    };
    let Some((pid, entropy)) = suffix.split_once('-') else {
        return false;
    };
    !pid.is_empty()
        && !entropy.is_empty()
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && entropy.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_pointer_temp_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix("evidence.json.tmp-pointer-") else {
        return false;
    };
    let Some((pid, entropy)) = suffix.split_once('-') else {
        return false;
    };
    !pid.is_empty()
        && !entropy.is_empty()
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && entropy.bytes().all(|byte| byte.is_ascii_digit())
}

fn acceptance_temp_target(name: &str) -> Option<&str> {
    let (target, suffix) = name.split_once(".tmp-p1a-acceptance-")?;
    let (pid, entropy) = suffix.split_once('-')?;
    if target.len() != 13
        || !target.ends_with(".json")
        || !target[..8].bytes().all(|byte| byte.is_ascii_digit())
        || pid.is_empty()
        || entropy.is_empty()
        || !pid.bytes().all(|byte| byte.is_ascii_digit())
        || !entropy.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some(target)
}

fn finalization_attempt_for_run(admission: &Admission, run_id: &str) -> Result<AttemptMetadata> {
    let staging = admission.output_root.join(".staging");
    let mut matches = Vec::new();
    for entry in fs::read_dir(&staging).io_context(
        "P1A_STAGING_ENUMERATION_FAILED",
        "could not enumerate finalization journals",
    )? {
        let entry = entry.io_context(
            "P1A_STAGING_ENUMERATION_FAILED",
            "could not read finalization journal entry",
        )?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if entry.file_type().is_ok_and(|kind| kind.is_file())
            && parse_finalization_marker_name(name)
                .is_ok_and(|(marker_run_id, _)| marker_run_id == run_id)
        {
            matches.push(entry.path());
        }
    }
    if matches.len() != 1 {
        return Err(XtaskError::integrity(
            "P1A_PUBLICATION_JOURNAL_INVALID",
            "unfinished P1A acceptance or pointer publication lacks one exact finalization journal",
        ));
    }
    let marker = &matches[0];
    let marker_name = marker.file_name().and_then(OsStr::to_str).ok_or_else(|| {
        XtaskError::integrity(
            "P1A_FINALIZATION_MARKER_INVALID",
            "P1A finalization journal name is not UTF-8",
        )
    })?;
    let (_, entropy) = parse_finalization_marker_name(marker_name)?;
    let attempt: AttemptMetadata = read_json(marker, "P1A_ATTEMPT_JSON_INVALID")?;
    validate_attempt_binding(&attempt, admission, &format!("{run_id}.work-{entropy}"))?;
    Ok(attempt)
}

fn recover_publication_temporaries(admission: &Admission) -> Result<()> {
    let output_root = &admission.output_root;
    let acceptance_directory = output_root.join("acceptances");
    let mut acceptance_temporaries = Vec::new();
    for entry in fs::read_dir(&acceptance_directory).io_context(
        "P1A_ACCEPTANCE_ENUMERATION_FAILED",
        "could not enumerate acceptance publication recovery",
    )? {
        let entry = entry.io_context(
            "P1A_ACCEPTANCE_ENUMERATION_FAILED",
            "could not read acceptance publication recovery entry",
        )?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_ACCEPTANCE_SEQUENCE_INVALID",
                    "acceptance publication recovery name is not UTF-8",
                )
            })?
            .to_owned();
        if acceptance_temp_target(&name).is_some() {
            acceptance_temporaries.push((entry.path(), name));
        }
    }
    if acceptance_temporaries.len() > 1 {
        return Err(XtaskError::integrity(
            "P1A_ACCEPTANCE_RECOVERY_AMBIGUOUS",
            "multiple unfinished P1A acceptance publications are present",
        ));
    }
    if let Some((temporary, name)) = acceptance_temporaries.pop() {
        let target_name = acceptance_temp_target(&name).expect("classified temporary");
        let target = acceptance_directory.join(target_name);
        let metadata = fs::symlink_metadata(&temporary).io_context(
            "P1A_ACCEPTANCE_INSPECTION_FAILED",
            "could not inspect acceptance publication temporary",
        )?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(XtaskError::integrity(
                "P1A_ACCEPTANCE_RECOVERY_INVALID",
                "acceptance publication temporary is not a regular file",
            ));
        }
        let acceptance: Acceptance = read_json(&temporary, "P1A_ACCEPTANCE_JSON_INVALID")?;
        let journal = finalization_attempt_for_run(admission, &acceptance.run_id)?;
        if journal.source.commit != acceptance.source_commit
            || journal.source.tree != acceptance.source_tree
        {
            return Err(XtaskError::integrity(
                "P1A_ACCEPTANCE_RECOVERY_INVALID",
                "acceptance publication temporary does not bind its finalization journal",
            ));
        }
        if target.exists() {
            if fs::read(&target).io_context(
                "P1A_ACCEPTANCE_READ_FAILED",
                "could not read recovered acceptance",
            )? != fs::read(&temporary).io_context(
                "P1A_ACCEPTANCE_READ_FAILED",
                "could not read acceptance publication temporary",
            )? {
                return Err(XtaskError::integrity(
                    "P1A_ACCEPTANCE_RECOVERY_CONFLICT",
                    "acceptance temporary conflicts with its create-new destination",
                ));
            }
        } else {
            let inventory = acceptance_inventory_excluding(output_root, Some(&temporary))?;
            let sequence = u32::try_from(inventory.len() + 1).map_err(|_| {
                XtaskError::integrity(
                    "P1A_ACCEPTANCE_SEQUENCE_EXHAUSTED",
                    "too many P1A acceptances during recovery",
                )
            })?;
            if target_name != format!("{sequence:08}.json") {
                return Err(XtaskError::integrity(
                    "P1A_ACCEPTANCE_RECOVERY_INVALID",
                    "acceptance temporary is not the unique next create-new sequence",
                ));
            }
            validate_acceptance(&acceptance, sequence, inventory.last())?;
            validate_acceptance_run(output_root, &acceptance)?;
            fs::hard_link(&temporary, &target).io_context(
                "P1A_ACCEPTANCE_RECOVERY_FAILED",
                "could not atomically finish create-new acceptance publication",
            )?;
            if fs::read(&target).ok() != fs::read(&temporary).ok() {
                return Err(XtaskError::integrity(
                    "P1A_ACCEPTANCE_RECOVERY_INVALID",
                    "recovered acceptance bytes changed during create-new publication",
                ));
            }
        }
        fs::remove_file(&temporary).io_context(
            "P1A_ACCEPTANCE_RECOVERY_FAILED",
            "could not remove completed acceptance publication temporary",
        )?;
        let _ = acceptance_inventory(output_root)?;
    }

    let mut pointer_temporaries = Vec::new();
    for entry in fs::read_dir(output_root).io_context(
        "P1A_NAMESPACE_ENUMERATION_FAILED",
        "could not enumerate pointer publication recovery",
    )? {
        let entry = entry.io_context(
            "P1A_NAMESPACE_ENUMERATION_FAILED",
            "could not read pointer publication recovery entry",
        )?;
        let file_name = entry.file_name();
        let name = file_name.to_str().unwrap_or_default();
        if valid_pointer_temp_name(name) {
            pointer_temporaries.push(entry.path());
        }
    }
    if pointer_temporaries.len() > 1 {
        return Err(XtaskError::integrity(
            "P1A_POINTER_RECOVERY_AMBIGUOUS",
            "multiple unfinished P1A pointer publications are present",
        ));
    }
    if let Some(temporary) = pointer_temporaries.pop() {
        let metadata = fs::symlink_metadata(&temporary).io_context(
            "P1A_POINTER_READ_FAILED",
            "could not inspect pointer publication temporary",
        )?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(XtaskError::integrity(
                "P1A_POINTER_RECOVERY_INVALID",
                "pointer publication temporary is not a regular file",
            ));
        }
        let pointer: Pointer = read_json(&temporary, "P1A_POINTER_JSON_INVALID")?;
        let journal = finalization_attempt_for_run(admission, &pointer.run_id)?;
        if journal.source.commit != pointer.source_commit
            || journal.source.tree != pointer.source_tree
        {
            return Err(XtaskError::integrity(
                "P1A_POINTER_RECOVERY_INVALID",
                "pointer publication temporary does not bind its finalization journal",
            ));
        }
        let inventory = acceptance_inventory(output_root)?;
        let (sequence, _, acceptance, acceptance_sha256) = inventory.last().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_POINTER_RECOVERY_INVALID",
                "pointer publication temporary has no acceptance",
            )
        })?;
        validate_pointer_projection(&pointer, *sequence, acceptance, acceptance_sha256)?;
        validate_acceptance_run(output_root, acceptance)?;
        replace_pointer_atomically(&output_root.join("evidence.json"), &pointer)?;
        fs::remove_file(&temporary).io_context(
            "P1A_POINTER_RECOVERY_FAILED",
            "could not remove completed pointer publication temporary",
        )?;
    }
    Ok(())
}

fn validate_attempt_binding(
    attempt: &AttemptMetadata,
    admission: &Admission,
    stage_container_name: &str,
) -> Result<()> {
    let (stage_run_id, _) = parse_stage_container_name(stage_container_name)?;
    if attempt.schema != "python-slm-p1a-attempt-v1"
        || attempt.run_id != stage_run_id
        || attempt.stage_container_name != stage_container_name
        || !generated_timestamp_binds_run_id(&attempt.generated_at_utc, &attempt.run_id)
        || attempt.source != admission.source
        || attempt.p0a_dependency != admission.p0a_dependency
    {
        return Err(XtaskError::integrity(
            "P1A_INTERRUPTED_ATTEMPT_BINDING_INVALID",
            "interrupted P1A attempt does not exactly bind its stage, time, source, and P0A dependency",
        ));
    }
    Ok(())
}

fn discard_unpublished_stage_run(work: &WorkPaths) -> Result<()> {
    validate_work_paths(work)?;
    if !work.stage_run.exists() {
        return Ok(());
    }
    publication::require_no_follow_tree(&work.stage_run)?;
    fs::remove_dir_all(&work.stage_run).io_context(
        "P1A_STAGING_CLEANUP_FAILED",
        "could not discard a strictly owned, unpublished partial terminal run",
    )
}

fn execute_qualification(
    repository: &Path,
    admission: &mut Admission,
    work: &WorkPaths,
    visual_studio_instance_id: Option<&str>,
) -> Result<Value> {
    let vswhere_runtime = crate::p1a_windows::bind_vswhere_runtime()?;
    let vswhere_path = crate::p1a_windows::discover_vswhere_path()?;
    let vswhere_args: Vec<OsString> = crate::p1a_windows::VSWHERE_ARGS
        .iter()
        .map(OsString::from)
        .collect();
    let setup_configuration = vswhere_runtime.setup_configuration_identity();
    let vswhere_output = admission.recorder.run_audited_with_files(
        repository,
        &work.work_root,
        &vswhere_path,
        vswhere_args,
        std::iter::once("${VSWHERE}".to_owned())
            .chain(
                crate::p1a_windows::VSWHERE_ARGS
                    .iter()
                    .map(|value| (*value).to_owned()),
            )
            .collect(),
        "${REPO}",
        minimal_native_environment()?,
        Duration::from_secs(30),
        Vec::new(),
        vec![QualifiedPersistentFile {
            path: setup_configuration.path.clone(),
            sha256: setup_configuration.sha256.clone(),
            bytes: setup_configuration.bytes,
        }],
        "not_applicable",
    )?;
    require_audited_pass(&vswhere_output, "P1A_VSWHERE_FAILED")?;
    let (selected_group, selected_affinity_mask) = crate::p1a_windows::current_affinity_policy()?;
    let policy = WindowsHostPolicy {
        isolation: CpuIsolationPolicy {
            selected_group,
            selected_affinity_mask,
            verifier_ancestry_and_contained_process_identities:
                crate::p1a_windows::current_verifier_ancestry()?,
        },
        visual_studio_instance_id: visual_studio_instance_id.map(str::to_owned),
    };
    let host = crate::p1a_windows::probe_prototype_windows_host(&policy, &vswhere_output.stdout)?;
    if !host.qualified {
        let codes = host
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect::<Vec<_>>()
            .join(",");
        return Err(XtaskError::gate(
            "P1A_HOST_NOT_QUALIFIED",
            format!("prototype host qualification failed: {codes}"),
            "Run from an unconstrained ordinary native Windows shell after removing competing compute work.",
        ));
    }
    require_tool_paths_contained(&host)?;
    bind_vswhere_transcript(&mut admission.recorder, &vswhere_output.stdout, &host)?;
    let toolchain_before = crate::p1a_windows::snapshot_host_toolchain(&host)?;
    let rust = discover_rust_toolchain(repository, &mut admission.recorder, &work.work_root)?;
    let cargo_cache = retain_locked_resolution_cargo_home(
        &rust.cargo_home,
        &work.work_root,
        &repository.join("Cargo.lock"),
    )?;
    let graph = audit_cpu_graph(
        repository,
        &mut admission.recorder,
        &work.work_root,
        &host,
        &rust,
    )?;
    validate_resolution_cache_against_graph(&cargo_cache, &graph)?;
    let native_probe = run_native_probe(
        repository,
        &mut admission.recorder,
        &work.work_root,
        &host,
        &rust,
        &admission.input_manifest_sha256,
    )?;
    let quality = run_quality_gate(
        repository,
        &mut admission.recorder,
        &work.work_root,
        &host,
        &rust,
    )?;
    validate_owned_cargo_cache_inventory(
        &owned_persistent_root(&work.work_root).join("cargo-home"),
        &cargo_cache.manifest_entries,
    )?;
    require_source_clean(repository, &mut admission.recorder)?;
    let after_manifest = source_manifest_hash(repository, &mut admission.recorder)?;
    if after_manifest != admission.input_manifest_sha256 {
        return Err(XtaskError::integrity(
            "P1A_INPUT_MUTATION_DETECTED",
            "tracked source inputs changed during host qualification",
        ));
    }
    let toolchain_after = crate::p1a_windows::revalidate_host_toolchain(&host, &toolchain_before)?;
    revalidate_rust_toolchain(&rust)?;
    cleanup_owned_work(&work.work_root)?;

    let host_environment = build_host_environment(
        &host,
        &rust,
        &quality,
        &graph,
        &admission.input_manifest_sha256,
        &after_manifest,
        (&toolchain_before, &toolchain_after),
        &cargo_cache,
    )?;
    let cpu_isolation = build_cpu_isolation(&host)?;
    let artifacts = ArtifactValues {
        source_identity: serde_json::to_value(&admission.source).expect("source serializes"),
        p0a_dependency: serde_json::to_value(&admission.p0a_dependency)
            .expect("dependency serializes"),
        schema_bundle: serde_json::to_value(&admission.schema_bundle).expect("bundle serializes"),
        host_environment,
        cpu_isolation,
        native_probe,
    };
    let emitted = emit_terminal_run(
        repository,
        admission,
        work,
        "PASS",
        Some(artifacts),
        Vec::new(),
    )?;
    let finalization_marker = cleanup_stage_after_publication(work)?;
    let (acceptance_path, acceptance_sha256) = publish_automatic_acceptance(
        repository,
        &admission.output_root,
        &work.run_id,
        &work.generated_at_utc,
        &admission.source,
        &emitted,
    )?;
    finish_stage_cleanup(&finalization_marker)?;
    Ok(json!({
        "schema": "python-slm-p1a-qualification-result-v1",
        "phase_id": PHASE_ID,
        "status": "PASS",
        "profile_id": PROFILE_ID,
        "support_tier": SUPPORT_TIER,
        "qualification_scope": QUALIFICATION_SCOPE,
        "run_path": format!("runs/{}", work.run_id),
        "run_evidence_sha256": emitted.evidence_sha256,
        "seal_sha256": emitted.seal_sha256,
        "acceptance_path": acceptance_path,
        "acceptance_sha256": acceptance_sha256,
        "human_checkbox_review": "PENDING"
    }))
}

fn require_audited_pass(output: &AuditedOutput, code: &'static str) -> Result<()> {
    if !audited_output_passed(output) {
        return Err(XtaskError::gate(
            code,
            format!(
                "audited command failed or violated process closure (exit {}, timed_out {}, forbidden processes {}, forbidden modules {})",
                output.exit_code,
                output.audit.timed_out,
                output.audit.forbidden_processes.len(),
                output.audit.forbidden_modules.len()
            ),
            "Correct the command environment without weakening the audit and retry.",
        ));
    }
    Ok(())
}

fn process_audit_passed(audit: &ProcessAudit) -> bool {
    !audit.timed_out
        && audit.atomic_job_assignment
        && audit.audited_process_count > 0
        && audit.audited_process_count == audit.covered_process_count
        && audit.successful_snapshots > 0
        && audit.exit_races == 0
        && audit.process_tree_terminated
        && !audit.unexpected_descendants
        && audit.forbidden_processes.is_empty()
        && audit.forbidden_modules.is_empty()
}

fn audited_output_passed(output: &AuditedOutput) -> bool {
    output.exit_code == 0 && process_audit_passed(&output.audit)
}

impl P1aRecorder {
    fn next_id(&self) -> String {
        format!("C{:03}", self.commands.len() + 1)
    }

    fn run_git(&mut self, repository: &Path, args: &[&str]) -> Result<AuditedOutput> {
        require_fixed_git_argv(args)?;
        let (git, git_root) = crate::p1a_windows::discover_git_path()?;
        let id = self.next_id();
        let capture_directory = repository.join("target").join(format!(
            ".p1a-git-capture-{}-{}-{}",
            std::process::id(),
            time::now().2,
            id
        ));
        if capture_directory.exists() {
            return Err(XtaskError::integrity(
                "P1A_GIT_CAPTURE_COLLISION",
                "the unique audited Git capture directory already exists",
            ));
        }
        fs::create_dir_all(capture_directory.parent().expect("capture has parent")).io_context(
            "P1A_GIT_CAPTURE_ROOT_FAILED",
            "could not create the ignored P1A Git capture root",
        )?;
        let mut environment = minimal_native_environment()?;
        for (key, value) in [
            ("GIT_OPTIONAL_LOCKS", "0"),
            ("GIT_NO_REPLACE_OBJECTS", "1"),
            ("GIT_NO_LAZY_FETCH", "1"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "NUL"),
            ("GIT_CONFIG_SYSTEM", "NUL"),
            ("GIT_CONFIG_COUNT", "4"),
            ("GIT_CONFIG_KEY_0", "core.fsmonitor"),
            ("GIT_CONFIG_VALUE_0", "false"),
            ("GIT_CONFIG_KEY_1", "core.hooksPath"),
            ("GIT_CONFIG_VALUE_1", "NUL"),
            ("GIT_CONFIG_KEY_2", "credential.helper"),
            ("GIT_CONFIG_VALUE_2", ""),
            ("GIT_CONFIG_KEY_3", "core.pager"),
            ("GIT_CONFIG_VALUE_3", "cat"),
        ] {
            environment.insert(key.to_owned(), Some(OsString::from(value)));
        }
        isolate_persistent_environment(&mut environment, &capture_directory)?;
        let display_argv = std::iter::once("${GIT}".to_owned())
            .chain(args.iter().map(|value| (*value).to_owned()))
            .collect::<Vec<_>>();
        let (started_at_utc, _, _) = time::now();
        let started = Instant::now();
        let output = crate::p1a_process::run(&DirectCommand {
            program: git,
            args: args.iter().map(OsString::from).collect(),
            display_argv: display_argv.clone(),
            cwd: repository.to_path_buf(),
            environment,
            timeout: Duration::from_secs(120),
            capture_directory: capture_directory.clone(),
            capture_stem: id.clone(),
            qualified_persistent_roots: vec![git_root],
            qualified_persistent_files: Vec::new(),
        });
        let duration_ns = duration_ns(started.elapsed());
        let (finished_at_utc, _, _) = time::now();
        match output {
            Ok(output) => {
                let audit_passed = audited_output_passed(&output);
                self.commands.push(CapturedCommand {
                    id,
                    command_kind: "git".to_owned(),
                    argv: display_argv,
                    cwd: "${REPO}".to_owned(),
                    exit_code: output.exit_code,
                    started_at_utc,
                    finished_at_utc,
                    duration_ns,
                    network_mode: "not_applicable".to_owned(),
                    stdout: redact_output(&output.stdout, repository, None),
                    stderr: redact_output(&output.stderr, repository, None),
                    process_audit: Some(output.audit.clone()),
                    execution_error_code: None,
                });
                remove_owned_persistent_environment(&capture_directory)?;
                cleanup_capture_directory(&capture_directory)?;
                if !audit_passed {
                    Err(XtaskError::gate(
                        "P1A_GIT_PROCESS_AUDIT_FAILED",
                        "Git completed without satisfying the closed process audit",
                        "Correct the Git runtime closure without weakening the audit.",
                    ))
                } else {
                    Ok(output)
                }
            }
            Err(error) => {
                self.commands.push(CapturedCommand {
                    id,
                    command_kind: "git".to_owned(),
                    argv: display_argv,
                    cwd: "${REPO}".to_owned(),
                    exit_code: -1,
                    started_at_utc,
                    finished_at_utc,
                    duration_ns,
                    network_mode: "not_applicable".to_owned(),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    process_audit: None,
                    execution_error_code: Some(error.code.to_owned()),
                });
                remove_owned_persistent_environment_best_effort(&capture_directory);
                cleanup_capture_directory_best_effort(&capture_directory);
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_audited(
        &mut self,
        repository: &Path,
        work_root: &Path,
        program: &Path,
        args: Vec<OsString>,
        display_argv: Vec<String>,
        cwd_token: &str,
        environment: BTreeMap<String, Option<OsString>>,
        timeout: Duration,
        qualified_persistent_roots: Vec<PathBuf>,
        network_mode: &str,
    ) -> Result<AuditedOutput> {
        self.run_audited_with_files(
            repository,
            work_root,
            program,
            args,
            display_argv,
            cwd_token,
            environment,
            timeout,
            qualified_persistent_roots,
            Vec::new(),
            network_mode,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_audited_with_files(
        &mut self,
        repository: &Path,
        work_root: &Path,
        program: &Path,
        args: Vec<OsString>,
        display_argv: Vec<String>,
        cwd_token: &str,
        mut environment: BTreeMap<String, Option<OsString>>,
        timeout: Duration,
        qualified_persistent_roots: Vec<PathBuf>,
        qualified_persistent_files: Vec<QualifiedPersistentFile>,
        network_mode: &str,
    ) -> Result<AuditedOutput> {
        if !matches!(cwd_token, "${REPO}" | "${P1A_TEMP}")
            || !matches!(network_mode, "not_applicable" | "cargo_offline_enforced")
        {
            return Err(XtaskError::integrity(
                "P1A_COMMAND_RECORD_INVALID",
                "audited command uses an unapproved cwd or network mode token",
            ));
        }
        if !environment.contains_key("PATH") {
            return Err(XtaskError::integrity(
                "P1A_COMMAND_PATH_UNSEALED",
                "audited non-Git command does not replace the inherited PATH",
            ));
        }
        let id = self.next_id();
        let capture_directory = work_root.join("command-captures");
        let cwd = if cwd_token == "${REPO}" {
            repository.to_path_buf()
        } else {
            work_root.to_path_buf()
        };
        isolate_persistent_environment(&mut environment, work_root)?;
        environment.insert("TEMP".to_owned(), Some(work_root.as_os_str().to_owned()));
        environment.insert("TMP".to_owned(), Some(work_root.as_os_str().to_owned()));
        let (started_at_utc, _, _) = time::now();
        let started = Instant::now();
        let output = crate::p1a_process::run(&DirectCommand {
            program: program.to_path_buf(),
            args,
            display_argv: display_argv.clone(),
            cwd,
            environment,
            timeout,
            capture_directory,
            capture_stem: id.clone(),
            qualified_persistent_roots,
            qualified_persistent_files,
        });
        let duration_ns = duration_ns(started.elapsed());
        let (finished_at_utc, _, _) = time::now();
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                self.commands.push(CapturedCommand {
                    id,
                    command_kind: "non_git".to_owned(),
                    argv: display_argv,
                    cwd: cwd_token.to_owned(),
                    exit_code: -1,
                    started_at_utc,
                    finished_at_utc,
                    duration_ns,
                    network_mode: network_mode.to_owned(),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    process_audit: None,
                    execution_error_code: Some(error.code.to_owned()),
                });
                return Err(error);
            }
        };
        let mut audit = output.audit.clone();
        audit.audit_method = "windows_job_object_toolhelp32_v1".to_owned();
        let retained = AuditedOutput {
            exit_code: output.exit_code,
            stdout: output.stdout.clone(),
            stderr: output.stderr.clone(),
            audit: audit.clone(),
        };
        self.commands.push(CapturedCommand {
            id,
            command_kind: "non_git".to_owned(),
            argv: display_argv,
            cwd: cwd_token.to_owned(),
            exit_code: output.exit_code,
            started_at_utc,
            finished_at_utc,
            duration_ns,
            network_mode: network_mode.to_owned(),
            stdout: redact_output(&output.stdout, repository, Some(work_root)),
            stderr: redact_output(&output.stderr, repository, Some(work_root)),
            process_audit: Some(audit),
            execution_error_code: None,
        });
        Ok(retained)
    }
}

fn cleanup_capture_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(path).io_context(
        "P1A_GIT_CAPTURE_CLEANUP_FAILED",
        "could not inspect the audited Git capture directory",
    )?;
    if entries
        .next()
        .transpose()
        .io_context(
            "P1A_GIT_CAPTURE_CLEANUP_FAILED",
            "could not read the audited Git capture directory",
        )?
        .is_some()
    {
        return Err(XtaskError::integrity(
            "P1A_GIT_CAPTURE_CLEANUP_FAILED",
            "audited Git left unexpected private capture bytes",
        ));
    }
    fs::remove_dir(path).io_context(
        "P1A_GIT_CAPTURE_CLEANUP_FAILED",
        "could not remove the audited Git capture directory",
    )
}

fn cleanup_capture_directory_best_effort(path: &Path) {
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let _ = fs::remove_file(entry.path());
        }
        let _ = fs::remove_dir(path);
    }
}

fn require_fixed_git_argv(args: &[&str]) -> Result<()> {
    let fixed = matches!(
        args,
        ["rev-parse", "HEAD"]
            | ["rev-parse", "HEAD^{tree}"]
            | ["symbolic-ref", "--quiet", "--short", "HEAD"]
            | [
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--",
                ".",
                _
            ]
            | ["ls-files", "-z", "--", ".", _]
            | ["rev-parse", "HEAD:docs/receipts/P1A"]
            | [
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--",
                "docs/receipts/P1A"
            ]
            | [
                "ls-tree",
                "-r",
                "--name-only",
                "HEAD",
                "--",
                "docs/receipts/P1A"
            ]
            | [
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--",
                OUTPUT_ROOT
            ]
            | ["ls-files", "--", OUTPUT_ROOT]
    );
    let dynamic_tree = matches!(args, ["rev-parse", revspec]
        if revspec.strip_suffix("^{tree}").is_some_and(is_lower_git_sha));
    let dynamic_ancestor = matches!(args, ["merge-base", "--is-ancestor", commit, "HEAD"]
        if is_lower_git_sha(commit))
        || matches!(args, ["merge-base", "--is-ancestor", ancestor, descendant]
            if is_lower_git_sha(ancestor) && is_lower_git_sha(descendant));
    let dynamic_blob = matches!(args, ["show", spec] if valid_git_blob_spec(spec));
    let dynamic_history = matches!(args, ["log", "--full-history", "-m", "--format=commit:%H", "--name-status", "--no-renames", "--", path]
        if valid_immutable_history_path(path) || *path == format!("{OUTPUT_ROOT}/evidence.json"));
    let dynamic_commit_scope = matches!(args, ["diff-tree", "--root", "--no-commit-id", "--name-status", "-r", "--no-renames", commit]
        if is_lower_git_sha(commit));
    let dynamic_commit_parents = matches!(args, ["rev-list", "--parents", "-n", "1", commit]
        if is_lower_git_sha(commit));
    let allowed = fixed
        || dynamic_tree
        || dynamic_ancestor
        || dynamic_blob
        || dynamic_history
        || dynamic_commit_scope
        || dynamic_commit_parents;
    if !allowed {
        return Err(XtaskError::integrity(
            "P1A_GIT_COMMAND_NOT_ALLOWED",
            format!("rejected non-allowlisted Git argv: {}", args.join(" ")),
        ));
    }
    Ok(())
}

fn valid_immutable_history_path(path: &str) -> bool {
    let run_prefix = format!("{OUTPUT_ROOT}/runs/");
    let acceptance_prefix = format!("{OUTPUT_ROOT}/acceptances/");
    if let Some(run_id) = path.strip_prefix(&run_prefix) {
        return valid_run_id(run_id);
    }
    path.strip_prefix(&acceptance_prefix).is_some_and(|name| {
        name.len() == 13
            && name.ends_with(".json")
            && name[..8].bytes().all(|byte| byte.is_ascii_digit())
            && &name[..8] != "00000000"
    })
}

fn valid_git_blob_spec(spec: &str) -> bool {
    let Some((commit, path)) = spec.split_once(':') else {
        return false;
    };
    is_lower_git_sha(commit) && valid_git_blob_path(path)
}

fn valid_git_blob_path(path: &str) -> bool {
    SCHEMA_PATHS.contains(&path)
        || VERIFIER_PATHS.contains(&path)
        || path == format!("{OUTPUT_ROOT}/evidence.json")
        || matches!(
            path,
            "docs/receipts/P0A/evidence.json" | "docs/receipts/P0A/acceptances/00000001.json"
        )
}

fn git_blob(
    recorder: &mut P1aRecorder,
    repository: &Path,
    commit: &str,
    relative: &str,
) -> Result<Vec<u8>> {
    if !is_lower_git_sha(commit) || !valid_git_blob_path(relative) {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_BLOB_PATH_INVALID",
            "refused to read a source blob outside the frozen verifier/schema/authority inventory",
        ));
    }
    let spec = format!("{commit}:{relative}");
    let output = recorder.run_git(repository, &["show", &spec])?;
    if output.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_BLOB_MISSING",
            format!("source commit does not contain {relative}"),
        ));
    }
    Ok(output.stdout)
}

fn receipt_authority_at_source(
    repository: &Path,
    source: &SourceIdentity,
    recorded_bundle: &SchemaBundle,
) -> Result<ReceiptSchemas> {
    let mut recorder = P1aRecorder::default();
    if !is_lower_git_sha(&source.commit) || !is_lower_git_sha(&source.tree) || source.dirty {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_IDENTITY_INVALID",
            "receipt source identity is not a clean canonical Git identity",
        ));
    }
    let ancestor = recorder.run_git(
        repository,
        &["merge-base", "--is-ancestor", &source.commit, "HEAD"],
    )?;
    if ancestor.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_ANCESTRY_INVALID",
            "receipt source commit is not an ancestor of the verifying commit",
        ));
    }
    let tree_spec = format!("{}^{{tree}}", source.commit);
    let actual_tree = git_line(
        &mut recorder,
        repository,
        &["rev-parse", &tree_spec],
        "P1A_SOURCE_TREE_INVALID",
    )?;
    if actual_tree != source.tree {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_TREE_INVALID",
            "receipt source tree does not match its source commit",
        ));
    }
    let mut schema_bytes = BTreeMap::new();
    let mut schema_manifest = String::new();
    let mut expected_entries = Vec::new();
    let mut sorted_paths = SCHEMA_PATHS.to_vec();
    sorted_paths.sort_unstable();
    for relative in sorted_paths {
        let bytes = git_blob(&mut recorder, repository, &source.commit, relative)?;
        if bytes.contains(&b'\r') || !bytes.ends_with(b"\n") {
            return Err(XtaskError::integrity(
                "P1A_SOURCE_SCHEMA_ENCODING_INVALID",
                format!("source schema {relative} is not canonical LF JSON"),
            ));
        }
        let schema: Value = serde_json::from_slice(&bytes).map_err(|error| {
            XtaskError::integrity(
                "P1A_SOURCE_SCHEMA_JSON_INVALID",
                format!("source schema {relative} is invalid JSON: {error}"),
            )
        })?;
        let digest = hash::bytes(&bytes);
        schema_manifest.push_str(&digest);
        schema_manifest.push_str("  ");
        schema_manifest.push_str(relative);
        schema_manifest.push('\n');
        expected_entries.push(SchemaEntry {
            path: relative.to_owned(),
            sha256: digest,
        });
        schema_bytes.insert(relative.to_owned(), schema);
    }
    if recorded_bundle.entries.len() != expected_entries.len()
        || recorded_bundle
            .entries
            .iter()
            .zip(&expected_entries)
            .any(|(recorded, expected)| {
                recorded.path != expected.path || recorded.sha256 != expected.sha256
            })
        || recorded_bundle.bundle_sha256 != hash::bytes(schema_manifest.as_bytes())
        || source.schema_bundle_sha256 != recorded_bundle.bundle_sha256
    {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_SCHEMA_BUNDLE_MISMATCH",
            "recorded schema bundle does not reproduce source-commit schema bytes",
        ));
    }
    let cargo_lock = git_blob(&mut recorder, repository, &source.commit, "Cargo.lock")?;
    if source.cargo_lock_sha256 != hash::bytes(&cargo_lock) {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_CARGO_LOCK_MISMATCH",
            "recorded Cargo.lock hash does not reproduce source-commit bytes",
        ));
    }
    let mut verifier_manifest = String::new();
    let mut verifier_paths = VERIFIER_PATHS.to_vec();
    verifier_paths.sort_unstable();
    for relative in verifier_paths {
        let bytes = if relative == "Cargo.lock" {
            cargo_lock.clone()
        } else {
            git_blob(&mut recorder, repository, &source.commit, relative)?
        };
        verifier_manifest.push_str(&hash::bytes(&bytes));
        verifier_manifest.push_str("  ");
        verifier_manifest.push_str(relative);
        verifier_manifest.push('\n');
    }
    if source.verifier_source_sha256 != hash::bytes(verifier_manifest.as_bytes()) {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_VERIFIER_BUNDLE_MISMATCH",
            "recorded verifier hash does not reproduce source-commit bytes",
        ));
    }
    let required = |path: &str| -> Result<Value> {
        schema_bytes.get(path).cloned().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_SOURCE_SCHEMA_MISSING",
                format!("source schema inventory omits {path}"),
            )
        })
    };
    Ok(ReceiptSchemas {
        evidence: required(
            "docs/schemas/P1A-prototype-v2/python-slm-p1a-phase-evidence-v1.schema.json",
        )?,
        artifacts: BTreeMap::from([
            (
                "artifacts/source-identity.json".to_owned(),
                required(
                    "docs/schemas/P1A-prototype-v2/python-slm-p1a-source-identity-v1.schema.json",
                )?,
            ),
            (
                "artifacts/p0a-dependency.json".to_owned(),
                required("docs/schemas/P1A-prototype-v2/python-slm-p0a-dependency-v1.schema.json")?,
            ),
            (
                "artifacts/schema-bundle.json".to_owned(),
                required(
                    "docs/schemas/P1A-prototype-v2/python-slm-p1a-schema-bundle-v1.schema.json",
                )?,
            ),
            (
                "artifacts/host-environment.json".to_owned(),
                required(
                    "docs/schemas/P1A-prototype-v2/python-slm-p1a-host-environment-v1.schema.json",
                )?,
            ),
            (
                "artifacts/cpu-isolation.json".to_owned(),
                required(
                    "docs/schemas/P1A-prototype-v2/python-slm-p1a-cpu-isolation-v1.schema.json",
                )?,
            ),
            (
                "artifacts/native-abi-probe.json".to_owned(),
                required(
                    "docs/schemas/P1A-prototype-v2/python-slm-p1a-native-abi-probe-v1.schema.json",
                )?,
            ),
        ]),
        acceptance: required(
            "docs/schemas/P1A-prototype-v2/python-slm-p1a-phase-acceptance-v1.schema.json",
        )?,
        pointer: required(
            "docs/schemas/P1A-prototype-v2/python-slm-p1a-phase-pointer-v1.schema.json",
        )?,
    })
}

fn receipt_authority_for_run(repository: &Path, run_root: &Path) -> Result<ReceiptSchemas> {
    let source: SourceIdentity = read_json(
        &run_root.join("artifacts/source-identity.json"),
        "P1A_SOURCE_IDENTITY_INVALID",
    )?;
    let bundle: SchemaBundle = read_json(
        &run_root.join("artifacts/schema-bundle.json"),
        "P1A_SCHEMA_BUNDLE_INVALID",
    )?;
    receipt_authority_at_source(repository, &source, &bundle)
}

fn git_line(
    recorder: &mut P1aRecorder,
    repository: &Path,
    args: &[&str],
    code: &'static str,
) -> Result<String> {
    let output = recorder.run_git(repository, args)?;
    if output.exit_code != 0 {
        return Err(XtaskError::integrity(
            code,
            format!(
                "git {} failed with exit {}",
                args.join(" "),
                output.exit_code
            ),
        ));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| XtaskError::integrity(code, "Git returned non-UTF-8 identity output"))?;
    let value = text.trim();
    if value.is_empty() || value.lines().count() != 1 {
        return Err(XtaskError::integrity(
            code,
            "Git identity output was empty or multiline",
        ));
    }
    Ok(value.to_owned())
}

fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().try_into().unwrap_or(u64::MAX).max(1)
}

fn redact_output(bytes: &[u8], repository: &Path, work_root: Option<&Path>) -> Vec<u8> {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    let mut replacements = vec![(repository.to_path_buf(), "${REPO}")];
    if let Some(work_root) = work_root {
        replacements.push((work_root.to_path_buf(), "${P1A_TEMP}"));
    }
    for name in ["USERPROFILE", "HOME"] {
        if let Some(value) = std::env::var_os(name) {
            replacements.push((PathBuf::from(value), "${HOME}"));
        }
    }
    for name in ["TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            replacements.push((PathBuf::from(value), "${TEMP}"));
        }
    }
    replacements.sort_by_key(|(path, _)| std::cmp::Reverse(path.as_os_str().len()));
    for (path, token) in replacements {
        let native = path.to_string_lossy();
        if native.is_empty() {
            continue;
        }
        text = text
            .replace(&native.replace('\\', "\\\\"), token)
            .replace(native.as_ref(), token)
            .replace(&native.replace('\\', "/"), token);
    }
    if let Ok(mut value) = serde_json::from_str::<Value>(&text) {
        redact_absolute_paths_in_json(&mut value);
        if let Ok(mut bytes) = serde_json::to_vec_pretty(&value) {
            bytes.push(b'\n');
            return bytes;
        }
    }
    redact_absolute_windows_paths(&text).into_bytes()
}

fn bind_vswhere_transcript(
    recorder: &mut P1aRecorder,
    raw_stdout: &[u8],
    host: &PrototypeWindowsHostReport,
) -> Result<()> {
    let mut transcript: Value = serde_json::from_slice(raw_stdout).map_err(|error| {
        XtaskError::integrity(
            "P1A_VSWHERE_TRANSCRIPT_INVALID",
            format!("could not parse the audited vswhere transcript: {error}"),
        )
    })?;
    let candidates = transcript.as_array_mut().ok_or_else(|| {
        XtaskError::integrity(
            "P1A_VSWHERE_TRANSCRIPT_INVALID",
            "audited vswhere transcript is not a candidate array",
        )
    })?;
    let (program_files, program_files_x86) = crate::p1a_windows::native_program_files_roots()?;
    let roots = [
        (
            host.visual_studio.installation_path.as_path(),
            "${VS_INSTALL}",
        ),
        (program_files.as_path(), "${PROGRAM_FILES}"),
        (program_files_x86.as_path(), "${PROGRAM_FILES_X86}"),
    ];
    for candidate in candidates {
        let instance_id = candidate
            .pointer("/instanceId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_VSWHERE_TRANSCRIPT_INVALID",
                    "audited vswhere candidate omits instanceId",
                )
            })?;
        let qualified = host
            .visual_studio
            .candidates
            .iter()
            .find(|qualified| qualified.instance_id == instance_id)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_VSWHERE_TRANSCRIPT_INVALID",
                    format!("audited vswhere candidate {instance_id} was not qualified"),
                )
            })?;
        candidate["installationPath"] =
            Value::String(path_with_token(&qualified.installation_path, &roots)?);
    }
    redact_absolute_paths_in_json(&mut transcript);
    let mut bytes = serde_json::to_vec_pretty(&transcript).map_err(|error| {
        XtaskError::integrity(
            "P1A_VSWHERE_TRANSCRIPT_INVALID",
            format!("could not serialize the receipt-bound vswhere transcript: {error}"),
        )
    })?;
    bytes.push(b'\n');
    let matches = recorder
        .commands
        .iter_mut()
        .filter(|command| command.id == "C013")
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(XtaskError::integrity(
            "P1A_VSWHERE_TRANSCRIPT_INVALID",
            "recorder does not contain exactly one C013 vswhere command",
        ));
    }
    matches.into_iter().next().expect("one match").stdout = bytes;
    Ok(())
}

fn redact_absolute_paths_in_json(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(redact_absolute_paths_in_json),
        Value::Object(values) => values.values_mut().for_each(redact_absolute_paths_in_json),
        Value::String(text) if absolute_windows_path_start(text).is_some() => {
            *text = "${ABSOLUTE_WINDOWS_PATH}".to_owned();
        }
        _ => {}
    }
}

fn redact_absolute_windows_paths(text: &str) -> String {
    const TOKEN: &str = "${ABSOLUTE_WINDOWS_PATH}";
    let mut redacted = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(relative_start) = absolute_windows_path_start(&text[cursor..]) {
        let start = cursor + relative_start;
        redacted.push_str(&text[cursor..start]);
        redacted.push_str(TOKEN);
        cursor = text[start..]
            .find(['\r', '\n'])
            .map_or(text.len(), |relative_end| start + relative_end);
    }
    redacted.push_str(&text[cursor..]);
    redacted
}

fn absolute_windows_path_start(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    (0..bytes.len()).find(|&index| {
        let boundary =
            index == 0 || !bytes[index - 1].is_ascii_alphanumeric() && bytes[index - 1] != b'_';
        if !boundary {
            return false;
        }
        let tail = &bytes[index..];
        let drive_path = |value: &[u8]| {
            value.len() >= 3
                && value[0].is_ascii_alphabetic()
                && value[1] == b':'
                && matches!(value[2], b'\\' | b'/')
        };
        let unc_path = |value: &[u8]| {
            let mut parts = value
                .split(|byte| matches!(byte, b'\\' | b'/'))
                .filter(|part| !part.is_empty());
            parts.next().is_some() && parts.next().is_some()
        };
        drive_path(tail)
            || tail.strip_prefix(b"\\\\?\\").is_some_and(drive_path)
            || tail.strip_prefix(b"\\\\?\\UNC\\").is_some_and(unc_path)
            || tail.strip_prefix(b"\\\\.\\").is_some_and(drive_path)
            || tail.strip_prefix(b"\\\\").is_some_and(unc_path)
            || tail.strip_prefix(b"//?/").is_some_and(drive_path)
            || tail.strip_prefix(b"//").is_some_and(unc_path)
            || tail.strip_prefix(b"file:///").is_some_and(drive_path)
            || tail.strip_prefix(b"file://").is_some_and(drive_path)
    })
}

fn require_source_clean(repository: &Path, recorder: &mut P1aRecorder) -> Result<()> {
    let output = recorder.run_git(
        repository,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
            ":(exclude)docs/receipts/P1A-prototype-v2",
        ],
    )?;
    if output.exit_code != 0 || !output.stdout.is_empty() {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_DIRTY",
            "P1A requires a committed clean source outside its contained receipt namespace",
        ));
    }
    Ok(())
}

fn source_manifest_hash(repository: &Path, recorder: &mut P1aRecorder) -> Result<String> {
    let output = recorder.run_git(
        repository,
        &[
            "ls-files",
            "-z",
            "--",
            ".",
            ":(exclude)docs/receipts/P1A-prototype-v2",
        ],
    )?;
    if output.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_MANIFEST_FAILED",
            "could not enumerate the tracked P1A source manifest",
        ));
    }
    if output.stdout.is_empty() || !output.stdout.ends_with(&[0]) {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_MANIFEST_INVALID",
            "tracked source path inventory is empty or lacks its NUL terminator",
        ));
    }
    let mut paths = output.stdout[..output.stdout.len() - 1]
        .split(|byte| *byte == 0)
        .map(|bytes| {
            std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| {
                XtaskError::integrity(
                    "P1A_SOURCE_MANIFEST_INVALID",
                    "tracked source path inventory contains a non-UTF-8 path",
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if paths.is_empty()
        || paths.iter().any(|path| !valid_tracked_path(path))
        || paths.windows(2).any(|window| window[0] >= window[1])
    {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_MANIFEST_INVALID",
            "tracked source path inventory is not unique, sorted, and portable",
        ));
    }
    let mut manifest = String::new();
    for relative in paths.drain(..) {
        let (sha256, bytes) = stable_tracked_file_hash(repository, &relative)?;
        manifest.push_str(&sha256);
        manifest.push_str("  ");
        manifest.push_str(&bytes.to_string());
        manifest.push_str("  ");
        manifest.push_str(&relative);
        manifest.push('\n');
    }
    Ok(hash::bytes(manifest.as_bytes()))
}

fn valid_tracked_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn stable_tracked_file_hash(repository: &Path, relative: &str) -> Result<(String, u64)> {
    let path = repository.join(relative);
    let mut current = repository.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(component) = component else {
            return Err(XtaskError::integrity(
                "P1A_SOURCE_PATH_INVALID",
                "tracked source path contains a non-normal component",
            ));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current).io_context(
            "P1A_SOURCE_FILE_INSPECTION_FAILED",
            format!("could not inspect tracked input {relative}"),
        )?;
        if metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
            return Err(XtaskError::integrity(
                "P1A_SOURCE_REPARSE_REJECTED",
                format!("tracked input crosses a link or reparse point: {relative}"),
            ));
        }
    }
    #[cfg(windows)]
    let mut file = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .io_context(
                "P1A_SOURCE_FILE_LOCK_FAILED",
                format!("could not deny write/delete access to tracked input {relative}"),
            )?
    };
    #[cfg(not(windows))]
    let mut file = std::fs::File::open(&path).io_context(
        "P1A_SOURCE_FILE_LOCK_FAILED",
        format!("could not open tracked input {relative}"),
    )?;
    let before = file.metadata().io_context(
        "P1A_SOURCE_FILE_INSPECTION_FAILED",
        format!("could not bind tracked input {relative}"),
    )?;
    if !before.is_file() {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_FILE_INVALID",
            format!("tracked input is not an allowed regular file: {relative}"),
        ));
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).io_context(
            "P1A_SOURCE_FILE_READ_FAILED",
            format!("could not hash tracked input {relative}"),
        )?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let after = file.metadata().io_context(
        "P1A_SOURCE_FILE_INSPECTION_FAILED",
        format!("could not revalidate tracked input {relative}"),
    )?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || is_windows_reparse(&after)
    {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_FILE_MUTATED",
            format!("tracked input changed while hashing: {relative}"),
        ));
    }
    Ok((hex::encode(hasher.finalize()), before.len()))
}

fn is_windows_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

fn build_schema_bundle(repository: &Path) -> Result<SchemaBundle> {
    let mut paths = SCHEMA_PATHS.to_vec();
    paths.sort_unstable();
    if paths.len() != 13 || paths.windows(2).any(|window| window[0] == window[1]) {
        return Err(XtaskError::new(
            "P1A_SCHEMA_LIST_INVALID",
            Category::Internal,
            "the fixed P1A schema path list is not unique and complete",
            "Inspect the P1A verifier constants.",
        ));
    }
    let mut entries = Vec::new();
    let mut manifest = String::new();
    for path in paths {
        let bytes = fs::read(repository.join(path))
            .io_context("P1A_SCHEMA_READ_FAILED", format!("could not read {path}"))?;
        let parsed = serde_json::from_slice::<Value>(&bytes).map_err(|error| {
            XtaskError::integrity(
                "P1A_SCHEMA_JSON_INVALID",
                format!("{path} is not valid JSON: {error}"),
            )
        })?;
        if path.ends_with(".schema.json") {
            json_schema::validate_schema_document(&parsed, "P1A_SCHEMA_DOCUMENT_INVALID")?;
        }
        if bytes.contains(&b'\r') || !bytes.ends_with(b"\n") {
            return Err(XtaskError::integrity(
                "P1A_SCHEMA_ENCODING_INVALID",
                format!("{path} is not canonical LF JSON"),
            ));
        }
        let digest = hash::bytes(&bytes);
        manifest.push_str(&digest);
        manifest.push_str("  ");
        manifest.push_str(path);
        manifest.push('\n');
        entries.push(SchemaEntry {
            path: path.to_owned(),
            sha256: digest,
        });
    }
    Ok(SchemaBundle {
        schema: "python-slm-p1a-schema-bundle-v1".to_owned(),
        phase_id: PHASE_ID.to_owned(),
        interface_id: INTERFACE_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        entries,
        bundle_sha256: hash::bytes(manifest.as_bytes()),
    })
}

fn require_schema_contracts(repository: &Path, bundle: &SchemaBundle) -> Result<()> {
    if bundle.entries.len() != 13 {
        return Err(XtaskError::integrity(
            "P1A_SCHEMA_BUNDLE_INVALID",
            "P1A schema bundle does not contain exactly 13 entries",
        ));
    }
    let dependency: Value = read_json(
        &repository.join("docs/schemas/P1A-prototype-v2/python-slm-p0a-dependency-v1.schema.json"),
        "P1A_DEPENDENCY_SCHEMA_INVALID",
    )?;
    for (pointer, expected) in [
        (
            "/properties/pointer_sha256/const",
            json!(P0A_POINTER_SHA256),
        ),
        (
            "/properties/acceptance_sha256/const",
            json!(P0A_ACCEPTANCE_SHA256),
        ),
        ("/properties/run_id/const", json!(P0A_RUN_ID)),
        (
            "/properties/run_evidence_sha256/const",
            json!(P0A_EVIDENCE_SHA256),
        ),
        ("/properties/seal_sha256/const", json!(P0A_SEAL_SHA256)),
    ] {
        if dependency.pointer(pointer) != Some(&expected) {
            return Err(XtaskError::integrity(
                "P1A_DEPENDENCY_SCHEMA_INVALID",
                format!("P1A dependency schema has the wrong value at {pointer}"),
            ));
        }
    }
    let isolation: Value = read_json(
        &repository
            .join("docs/schemas/P1A-prototype-v2/python-slm-p1a-cpu-isolation-v1.schema.json"),
        "P1A_ISOLATION_SCHEMA_INVALID",
    )?;
    for (pointer, expected) in [
        (
            "/definitions/policy/properties/sample_duration_ms/const",
            json!(2000),
        ),
        (
            "/definitions/policy/properties/minimum_system_idle_fraction/const",
            json!(0.5),
        ),
        (
            "/definitions/policy/properties/maximum_foreign_process_single_core_fraction/const",
            json!(0.5),
        ),
    ] {
        if isolation.pointer(pointer) != Some(&expected) {
            return Err(XtaskError::integrity(
                "P1A_ISOLATION_SCHEMA_INVALID",
                format!("P1A isolation policy is not frozen at {pointer}"),
            ));
        }
    }
    Ok(())
}

fn require_probe_sources(repository: &Path) -> Result<()> {
    for relative in [
        "xtask/probes/p1a_abi.c",
        "xtask/probes/p1a_abi.cpp",
        "xtask/probes/p1a_abi.rs",
    ] {
        let path = repository.join(relative);
        let metadata = fs::symlink_metadata(&path).io_context(
            "P1A_PROBE_SOURCE_MISSING",
            format!("could not inspect {relative}"),
        )?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
            return Err(XtaskError::integrity(
                "P1A_PROBE_SOURCE_INVALID",
                format!("{relative} is not a nonempty regular source file"),
            ));
        }
    }
    Ok(())
}

fn require_build_policy(repository: &Path) -> Result<()> {
    for name in BANNED_BUILD_ENVIRONMENT {
        if std::env::var_os(name).is_some() {
            return Err(XtaskError::gate(
                "P1A_BUILD_OVERRIDE_REJECTED",
                format!("build-affecting environment variable {name} is set"),
                "Unset compiler, Cargo, and Rust wrapper/flag overrides in the calling shell.",
            ));
        }
    }
    for relative in ["xtask/.cargo/config", "xtask/.cargo/config.toml"] {
        if repository.join(relative).exists() {
            return Err(XtaskError::gate(
                "P1A_BUILD_CONFIG_REJECTED",
                format!("build-affecting Cargo config is present at {relative}"),
                "Remove the active Cargo config from the P1A source epoch.",
            ));
        }
    }
    for ancestor in repository.ancestors() {
        for name in ["config", "config.toml"] {
            let candidate = ancestor.join(".cargo").join(name);
            if candidate.exists() {
                return Err(XtaskError::gate(
                    "P1A_BUILD_CONFIG_REJECTED",
                    format!(
                        "build-affecting Cargo config is present in repository ancestry at {}",
                        candidate.display()
                    ),
                    "Remove the active ancestor Cargo config before host qualification.",
                ));
            }
        }
    }
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".cargo")));
    if let Some(home) = cargo_home {
        for name in ["config", "config.toml"] {
            if home.join(name).exists() {
                return Err(XtaskError::gate(
                    "P1A_BUILD_CONFIG_REJECTED",
                    "the caller Cargo home contains a build-affecting config",
                    "Run from a Cargo home without wrappers, aliases, target, linker, or registry overrides.",
                ));
            }
        }
    }
    Ok(())
}

fn require_historical_p1a_immutable(repository: &Path, recorder: &mut P1aRecorder) -> Result<()> {
    let ancestor = recorder.run_git(
        repository,
        &[
            "merge-base",
            "--is-ancestor",
            HISTORICAL_P1A_BASELINE,
            "HEAD",
        ],
    )?;
    if ancestor.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_HISTORICAL_BASELINE_MISSING",
            "the complete historical P1A evidence baseline is not an ancestor",
        ));
    }
    let tree = git_line(
        recorder,
        repository,
        &["rev-parse", "HEAD:docs/receipts/P1A"],
        "P1A_HISTORICAL_TREE_INVALID",
    )?;
    if tree != HISTORICAL_P1A_TREE {
        return Err(XtaskError::integrity(
            "P1A_HISTORICAL_TREE_CHANGED",
            "historical P1A receipt bytes differ from the frozen Git tree",
        ));
    }
    let status = recorder.run_git(
        repository,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            "docs/receipts/P1A",
        ],
    )?;
    if status.exit_code != 0 || !status.stdout.is_empty() {
        return Err(XtaskError::integrity(
            "P1A_HISTORICAL_TREE_DIRTY",
            "historical P1A evidence contains tracked or untracked changes",
        ));
    }
    hash::require_file(
        &repository.join("docs/receipts/P1A/evidence.json"),
        HISTORICAL_P1A_POINTER_SHA256,
        "P1A_HISTORICAL_POINTER_CHANGED",
    )?;
    hash::require_file(
        &repository.join("docs/receipts/P1A/acceptances/00000007.json"),
        HISTORICAL_P1A_ACCEPTANCE_SHA256,
        "P1A_HISTORICAL_ACCEPTANCE_CHANGED",
    )?;
    let listing = recorder.run_git(
        repository,
        &[
            "ls-tree",
            "-r",
            "--name-only",
            "HEAD",
            "--",
            "docs/receipts/P1A",
        ],
    )?;
    if listing.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_HISTORICAL_INVENTORY_FAILED",
            "could not enumerate the frozen historical P1A inventory",
        ));
    }
    let paths = std::str::from_utf8(&listing.stdout)
        .map_err(|_| {
            XtaskError::integrity(
                "P1A_HISTORICAL_INVENTORY_INVALID",
                "historical P1A path inventory is not UTF-8",
            )
        })?
        .lines()
        .collect::<Vec<_>>();
    if paths.len() != 4_665 || paths.windows(2).any(|window| window[0] >= window[1]) {
        return Err(XtaskError::integrity(
            "P1A_HISTORICAL_INVENTORY_INVALID",
            format!(
                "historical P1A inventory has {} entries instead of 4665 ordered entries",
                paths.len()
            ),
        ));
    }
    let mut manifest = String::new();
    for relative in paths {
        let path = repository.join(relative);
        let metadata = fs::metadata(&path).io_context(
            "P1A_HISTORICAL_INVENTORY_FAILED",
            format!("could not inspect {relative}"),
        )?;
        if !metadata.is_file() {
            return Err(XtaskError::integrity(
                "P1A_HISTORICAL_INVENTORY_INVALID",
                format!("historical inventory entry is not a regular file: {relative}"),
            ));
        }
        manifest.push_str(&hash::file(&path)?);
        manifest.push_str("  ");
        manifest.push_str(&metadata.len().to_string());
        manifest.push_str("  ");
        manifest.push_str(relative);
        manifest.push('\n');
    }
    if hash::bytes(manifest.as_bytes()) != HISTORICAL_P1A_INVENTORY_SHA256 {
        return Err(XtaskError::integrity(
            "P1A_HISTORICAL_INVENTORY_CHANGED",
            "historical P1A file inventory differs from the frozen 4665-entry manifest",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct RustToolchain {
    rustup_home: PathBuf,
    cargo_home: PathBuf,
    rustc: PathBuf,
    cargo: PathBuf,
    rustc_identity: crate::p1a_windows::ToolFileIdentity,
    cargo_identity: crate::p1a_windows::ToolFileIdentity,
    rustc_version: String,
    cargo_version: String,
    release: String,
    release_major: u32,
    release_minor: u32,
    release_patch: u32,
    host: String,
    llvm_version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnedCargoCacheSnapshot {
    source: String,
    resolution_input_role: String,
    copied_to_owned_root: bool,
    cargo_lock_package_count: usize,
    locked_crates_io_package_count: usize,
    archive_file_count: usize,
    sparse_index_record_count: usize,
    registry_config_file_count: usize,
    admitted_python_source_package: String,
    resolver_only_provider_source_packages: Vec<String>,
    file_count: usize,
    total_bytes: u64,
    manifest_sha256: String,
    #[serde(skip)]
    manifest_entries: Vec<(String, String, u64)>,
}

#[derive(Clone, Debug)]
struct CargoLockPackage {
    name: String,
    version: String,
    identity: String,
    checksum: Option<String>,
}

#[derive(Clone, Debug)]
struct CargoLockInventory {
    packages: Vec<CargoLockPackage>,
    identities: BTreeSet<String>,
    registry_packages: Vec<CargoLockPackage>,
    sparse_index_names: BTreeSet<String>,
    provider_source_packages: Vec<String>,
}

#[derive(Clone, Debug)]
struct QualityGateResult {
    commands: Vec<Value>,
    target_scans: Vec<crate::p1a_artifacts::TargetArtifactScan>,
}

#[derive(Clone, Debug)]
struct GraphAudit {
    activated_packages: Vec<String>,
}

#[derive(Clone, Debug)]
struct ArtifactValues {
    source_identity: Value,
    p0a_dependency: Value,
    schema_bundle: Value,
    host_environment: Value,
    cpu_isolation: Value,
    native_probe: Value,
}

#[derive(Clone, Debug)]
struct EmittedRun {
    evidence_sha256: String,
    seal_sha256: String,
    artifact_refs: BTreeMap<String, FileRef>,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path, code: &'static str) -> Result<T> {
    let bytes = fs::read(path).io_context(code, format!("could not read {}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        XtaskError::integrity(code, format!("invalid JSON at {}: {error}", path.display()))
    })
}

fn valid_run_id(value: &str) -> bool {
    let Some((stamp, suffix)) = value.split_once('-') else {
        return false;
    };
    stamp.len() == 19
        && stamp.as_bytes().get(8) == Some(&b'T')
        && stamp.as_bytes().get(18) == Some(&b'Z')
        && stamp[..8].bytes().all(|byte| byte.is_ascii_digit())
        && stamp[9..18].bytes().all(|byte| byte.is_ascii_digit())
        && suffix.len() == 24
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_lower_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn selected_p0a_dependency(
    repository: &Path,
    source_commit: &str,
    recorder: &mut P1aRecorder,
) -> Result<P0aDependency> {
    let pointer_path = repository.join("docs/receipts/P0A/evidence.json");
    let acceptance_path = repository.join("docs/receipts/P0A/acceptances/00000001.json");
    let run_root = repository.join("docs/receipts/P0A/runs").join(P0A_RUN_ID);
    let run_evidence_path = run_root.join("evidence.json");
    let technical_approval_path =
        repository.join("docs/receipts/P0A/approvals/technical-00000003.json");
    let governance_approval_path =
        repository.join("docs/receipts/P0A/approvals/data-governance-00000003.json");
    hash::require_file(
        &pointer_path,
        P0A_POINTER_SHA256,
        "P1A_P0A_POINTER_HASH_MISMATCH",
    )?;
    hash::require_file(
        &acceptance_path,
        P0A_ACCEPTANCE_SHA256,
        "P1A_P0A_ACCEPTANCE_HASH_MISMATCH",
    )?;
    hash::require_file(
        &run_evidence_path,
        P0A_EVIDENCE_SHA256,
        "P1A_P0A_EVIDENCE_HASH_MISMATCH",
    )?;
    hash::require_file(
        &technical_approval_path,
        P0A_TECHNICAL_APPROVAL_SHA256,
        "P1A_P0A_APPROVAL_HASH_MISMATCH",
    )?;
    hash::require_file(
        &governance_approval_path,
        P0A_DATA_GOVERNANCE_APPROVAL_SHA256,
        "P1A_P0A_APPROVAL_HASH_MISMATCH",
    )?;
    publication::verify_seal(&run_root, P0A_SEAL_SHA256)?;
    let pointer: Value = read_json(&pointer_path, "P1A_P0A_POINTER_INVALID")?;
    let acceptance: Value = read_json(&acceptance_path, "P1A_P0A_ACCEPTANCE_INVALID")?;
    let run_evidence: Value = read_json(&run_evidence_path, "P1A_P0A_EVIDENCE_INVALID")?;
    let technical_approval: Value =
        read_json(&technical_approval_path, "P1A_P0A_APPROVAL_INVALID")?;
    let governance_approval: Value =
        read_json(&governance_approval_path, "P1A_P0A_APPROVAL_INVALID")?;
    let todo = fs::read_to_string(repository.join("TODO.md")).io_context(
        "P1A_P0A_CHECKBOX_READ_FAILED",
        "could not read the live P0A closure marker",
    )?;
    let closure = recorder.run_git(
        repository,
        &[
            "merge-base",
            "--is-ancestor",
            "203b3580005ce59228c319d517134f910759c7bc",
            source_commit,
        ],
    )?;
    if closure.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_P0A_SOURCE_ANCESTRY_INVALID",
            "the selected P0A closure is not an ancestor of the P1A source commit",
        ));
    }
    let committed_pointer = git_blob(
        recorder,
        repository,
        source_commit,
        "docs/receipts/P0A/evidence.json",
    )?;
    let committed_acceptance = git_blob(
        recorder,
        repository,
        source_commit,
        "docs/receipts/P0A/acceptances/00000001.json",
    )?;
    if hash::bytes(&committed_pointer) != P0A_POINTER_SHA256
        || hash::bytes(&committed_acceptance) != P0A_ACCEPTANCE_SHA256
        || committed_pointer
            != fs::read(&pointer_path).io_context(
                "P1A_P0A_POINTER_READ_FAILED",
                "could not re-read the selected P0A pointer",
            )?
        || committed_acceptance
            != fs::read(&acceptance_path).io_context(
                "P1A_P0A_ACCEPTANCE_READ_FAILED",
                "could not re-read the selected P0A acceptance",
            )?
    {
        return Err(XtaskError::integrity(
            "P1A_P0A_SOURCE_BINDING_INVALID",
            "the P1A source commit does not contain the exact selected P0A authority bytes",
        ));
    }
    if pointer.pointer("/acceptance_path").and_then(Value::as_str)
        != Some("acceptances/00000001.json")
        || pointer
            .pointer("/acceptance_sha256")
            .and_then(Value::as_str)
            != Some(P0A_ACCEPTANCE_SHA256)
        || acceptance.pointer("/status").and_then(Value::as_str) != Some("PASS")
        || acceptance.pointer("/run_path").and_then(Value::as_str)
            != Some("runs/20260813T111427138Z-594485fab70714426a7a3870")
        || acceptance
            .pointer("/run_evidence_sha256")
            .and_then(Value::as_str)
            != Some(P0A_EVIDENCE_SHA256)
        || acceptance.pointer("/seal_sha256").and_then(Value::as_str) != Some(P0A_SEAL_SHA256)
        || acceptance
            .pointer("/approvals/0/role")
            .and_then(Value::as_str)
            != Some("technical")
        || acceptance
            .pointer("/approvals/0/path")
            .and_then(Value::as_str)
            != Some("approvals/technical-00000003.json")
        || acceptance
            .pointer("/approvals/0/sha256")
            .and_then(Value::as_str)
            != Some(P0A_TECHNICAL_APPROVAL_SHA256)
        || acceptance
            .pointer("/approvals/1/role")
            .and_then(Value::as_str)
            != Some("data_governance")
        || acceptance
            .pointer("/approvals/1/path")
            .and_then(Value::as_str)
            != Some("approvals/data-governance-00000003.json")
        || acceptance
            .pointer("/approvals/1/sha256")
            .and_then(Value::as_str)
            != Some(P0A_DATA_GOVERNANCE_APPROVAL_SHA256)
        || acceptance
            .pointer("/approval_commit")
            .and_then(Value::as_str)
            != Some("3b36781ea5cc4eb6316249d6c2fc342c61ef220e")
        || acceptance
            .pointer("/preapproval_commit")
            .and_then(Value::as_str)
            != Some("f49abcca7f0b110759606c0bf9beffc911cc6635")
        || run_evidence.pointer("/run_id").and_then(Value::as_str) != Some(P0A_RUN_ID)
        || run_evidence.pointer("/status").and_then(Value::as_str) != Some("AWAITING_REVIEW")
        || run_evidence
            .pointer("/authority/machine_evidence")
            .and_then(Value::as_str)
            != Some("PASS")
        || technical_approval.pointer("/role").and_then(Value::as_str) != Some("technical")
        || governance_approval.pointer("/role").and_then(Value::as_str) != Some("data_governance")
        || technical_approval
            .pointer("/decision")
            .and_then(Value::as_str)
            != Some("APPROVE")
        || governance_approval
            .pointer("/decision")
            .and_then(Value::as_str)
            != Some("APPROVE")
        || technical_approval
            .pointer("/run_id")
            .and_then(Value::as_str)
            != Some(P0A_RUN_ID)
        || governance_approval
            .pointer("/run_id")
            .and_then(Value::as_str)
            != Some(P0A_RUN_ID)
        || technical_approval
            .pointer("/explicit_dual_role_authority")
            .and_then(Value::as_bool)
            != Some(true)
        || governance_approval
            .pointer("/explicit_dual_role_authority")
            .and_then(Value::as_bool)
            != Some(true)
        || todo.matches("- [x] P0A complete").count() != 1
        || todo.contains("- [ ] P0A complete")
    {
        return Err(XtaskError::integrity(
            "P1A_P0A_SELECTION_INVALID",
            "the selected P0A pointer or acceptance does not match the frozen dependency",
        ));
    }
    Ok(build_p0a_dependency(source_commit))
}

fn build_p0a_dependency(source_commit: &str) -> P0aDependency {
    P0aDependency {
        schema: "python-slm-p0a-dependency-v1".to_owned(),
        phase_id: "P0A".to_owned(),
        interface_id: INTERFACE_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        status: "PASS".to_owned(),
        pointer_path: "docs/receipts/P0A/evidence.json".to_owned(),
        pointer_sha256: P0A_POINTER_SHA256.to_owned(),
        acceptance_path: "docs/receipts/P0A/acceptances/00000001.json".to_owned(),
        acceptance_sha256: P0A_ACCEPTANCE_SHA256.to_owned(),
        acceptance_sequence: 1,
        run_id: P0A_RUN_ID.to_owned(),
        run_evidence_sha256: P0A_EVIDENCE_SHA256.to_owned(),
        seal_sha256: P0A_SEAL_SHA256.to_owned(),
        preapproval_commit: "f49abcca7f0b110759606c0bf9beffc911cc6635".to_owned(),
        receipt_commit: "b32651bcccf285b256acfcf5f7ca19d3dd35c947".to_owned(),
        approval_commit: "3b36781ea5cc4eb6316249d6c2fc342c61ef220e".to_owned(),
        publication_commit: "f221a638f65509ee28457a0ccc03a23bc134956b".to_owned(),
        closure_commit: "203b3580005ce59228c319d517134f910759c7bc".to_owned(),
        technical_approval_sha256:
            "ea9eae24d1763e8ac257a35b63f6df3648bc5ed7cb7199863d6161b3142a9765".to_owned(),
        data_governance_approval_sha256:
            "8273e1fb6fb8aa812253f825dd38632770fa43ed4f8246907be622bf0b0783a3".to_owned(),
        verified_at_source_commit: source_commit.to_owned(),
    }
}

fn verifier_bundle_hash(repository: &Path) -> Result<String> {
    let mut paths = VERIFIER_PATHS.to_vec();
    paths.sort_unstable();
    let mut manifest = String::new();
    for relative in paths {
        manifest.push_str(&hash::file(&repository.join(relative))?);
        manifest.push_str("  ");
        manifest.push_str(relative);
        manifest.push('\n');
    }
    Ok(hash::bytes(manifest.as_bytes()))
}

fn home_directory() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| {
            XtaskError::environment(
                "P1A_HOME_UNAVAILABLE",
                "neither USERPROFILE nor HOME identifies the current user directory",
            )
        })
}

fn require_regular_file(path: &Path, code: &'static str) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)
        .io_context(code, format!("could not inspect {}", path.display()))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || is_windows_reparse(&metadata)
        || metadata.len() == 0
    {
        return Err(XtaskError::environment(
            code,
            format!(
                "required nonempty regular file is absent: {}",
                path.display()
            ),
        ));
    }
    fs::canonicalize(path).io_context(code, format!("could not canonicalize {}", path.display()))
}

fn discover_rust_toolchain(
    repository: &Path,
    recorder: &mut P1aRecorder,
    work_root: &Path,
) -> Result<RustToolchain> {
    let home = home_directory()?;
    let rustup_home = fs::canonicalize(
        std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".rustup")),
    )
    .io_context(
        "P1A_RUSTUP_HOME_INVALID",
        "could not canonicalize the Rustup home",
    )?;
    let cargo_home = fs::canonicalize(
        std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".cargo")),
    )
    .io_context(
        "P1A_CARGO_HOME_INVALID",
        "could not canonicalize the Cargo home",
    )?;
    let toolchain_root = rustup_home
        .join("toolchains")
        .join("stable-x86_64-pc-windows-msvc");
    let rustc = require_regular_file(&toolchain_root.join("bin/rustc.exe"), "P1A_RUSTC_NOT_FOUND")?;
    let cargo = require_regular_file(&toolchain_root.join("bin/cargo.exe"), "P1A_CARGO_NOT_FOUND")?;
    let rustc_output = recorder.run_audited(
        repository,
        work_root,
        &rustc,
        vec![OsString::from("-vV")],
        vec!["${RUSTC}".to_owned(), "-vV".to_owned()],
        "${REPO}",
        minimal_native_environment()?,
        Duration::from_secs(30),
        Vec::new(),
        "not_applicable",
    )?;
    require_audited_pass(&rustc_output, "P1A_RUSTC_VERSION_FAILED")?;
    let cargo_output = recorder.run_audited(
        repository,
        work_root,
        &cargo,
        vec![OsString::from("-Vv")],
        vec!["${CARGO}".to_owned(), "-Vv".to_owned()],
        "${REPO}",
        minimal_native_environment()?,
        Duration::from_secs(30),
        Vec::new(),
        "not_applicable",
    )?;
    require_audited_pass(&cargo_output, "P1A_CARGO_VERSION_FAILED")?;
    let rustc_identity = crate::p1a_windows::native_file_identity(&rustc)?;
    let cargo_identity = crate::p1a_windows::native_file_identity(&cargo)?;
    require_executed_tool_identity(&rustc_output, &rustc_identity, "rustc.exe")?;
    require_executed_tool_identity(&cargo_output, &cargo_identity, "cargo.exe")?;
    let rustc_text = std::str::from_utf8(&rustc_output.stdout).map_err(|_| {
        XtaskError::integrity("P1A_RUSTC_VERSION_INVALID", "rustc -vV output is not UTF-8")
    })?;
    let cargo_text = std::str::from_utf8(&cargo_output.stdout).map_err(|_| {
        XtaskError::integrity("P1A_CARGO_VERSION_INVALID", "cargo -Vv output is not UTF-8")
    })?;
    let release = version_field(rustc_text, "release")?;
    let host = version_field(rustc_text, "host")?;
    let llvm_version = version_field(rustc_text, "LLVM version")?;
    let (release_major, release_minor, release_patch) = parse_three_part_version(&release)?;
    if !rust_release_satisfies_minimum(release_major, release_minor)
        || host != "x86_64-pc-windows-msvc"
    {
        return Err(XtaskError::gate(
            "P1A_RUST_TOOLCHAIN_UNSUPPORTED",
            format!("Rust {release} for {host} does not satisfy Rust >=1.96 MSVC"),
            "Install the stable x86_64-pc-windows-msvc Rust toolchain at version 1.96 or newer.",
        ));
    }
    let cargo_release = cargo_text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_CARGO_VERSION_INVALID",
                "cargo -Vv omitted its release version",
            )
        })?;
    if cargo_release != release {
        return Err(XtaskError::gate(
            "P1A_RUST_CARGO_VERSION_MISMATCH",
            format!("rustc is {release} but cargo is {cargo_release}"),
            "Install a coherent stable Rust toolchain.",
        ));
    }
    Ok(RustToolchain {
        rustup_home,
        cargo_home,
        rustc,
        cargo,
        rustc_identity,
        cargo_identity,
        rustc_version: release.clone(),
        cargo_version: cargo_release.to_owned(),
        release,
        release_major,
        release_minor,
        release_patch,
        host,
        llvm_version,
    })
}

fn rust_release_satisfies_minimum(major: u32, minor: u32) -> bool {
    (major, minor) >= (1, 96)
}

fn require_executed_tool_identity(
    output: &AuditedOutput,
    expected: &crate::p1a_windows::ToolFileIdentity,
    executable_name: &str,
) -> Result<()> {
    let matching = output
        .audit
        .process_identities
        .iter()
        .filter(|identity| {
            identity
                .executable_name
                .eq_ignore_ascii_case(executable_name)
                && identity.executable_sha256 == expected.sha256
                && identity.executable_bytes == expected.bytes
        })
        .count();
    if matching != 1 {
        return Err(XtaskError::integrity(
            "P1A_EXECUTED_TOOL_IDENTITY_MISMATCH",
            format!("the audited {executable_name} process does not match its receipt identity"),
        ));
    }
    Ok(())
}

fn revalidate_rust_toolchain(rust: &RustToolchain) -> Result<()> {
    if crate::p1a_windows::native_file_identity(&rust.rustc)? != rust.rustc_identity
        || crate::p1a_windows::native_file_identity(&rust.cargo)? != rust.cargo_identity
    {
        return Err(XtaskError::integrity(
            "P1A_RUST_TOOLCHAIN_IDENTITY_DRIFT",
            "the selected Rust compiler or Cargo executable changed during qualification",
        ));
    }
    Ok(())
}

fn owned_persistent_root(work_root: &Path) -> PathBuf {
    work_root.join("persistent")
}

fn isolate_persistent_environment(
    environment: &mut BTreeMap<String, Option<OsString>>,
    work_root: &Path,
) -> Result<()> {
    let persistent = owned_persistent_root(work_root);
    let user_profile = persistent.join("user-profile");
    let local_app_data = persistent.join("local-app-data");
    let roaming_app_data = persistent.join("roaming-app-data");
    let program_data = persistent.join("program-data");
    let cargo_home = persistent.join("cargo-home");
    let rustup_home = persistent.join("rustup-home");
    for directory in [
        &persistent,
        &user_profile,
        &local_app_data,
        &roaming_app_data,
        &program_data,
        &cargo_home,
        &rustup_home,
    ] {
        if directory.exists() {
            let metadata = fs::symlink_metadata(directory).io_context(
                "P1A_PERSISTENT_ROOT_INVALID",
                "could not inspect an owned persistent-state directory",
            )?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || is_windows_reparse(&metadata)
            {
                return Err(XtaskError::integrity(
                    "P1A_PERSISTENT_ROOT_INVALID",
                    "an owned persistent-state path is not a plain directory",
                ));
            }
        } else {
            fs::create_dir_all(directory).io_context(
                "P1A_PERSISTENT_ROOT_CREATE_FAILED",
                "could not create an owned persistent-state directory",
            )?;
        }
    }
    publication::require_no_follow_tree(&persistent)?;
    for (name, path) in [
        ("USERPROFILE", &user_profile),
        ("HOME", &user_profile),
        ("LOCALAPPDATA", &local_app_data),
        ("APPDATA", &roaming_app_data),
        ("PROGRAMDATA", &program_data),
        ("CARGO_HOME", &cargo_home),
        ("RUSTUP_HOME", &rustup_home),
    ] {
        environment.insert(name.to_owned(), Some(path.as_os_str().to_owned()));
    }
    environment.insert("HOMEDRIVE".to_owned(), None);
    environment.insert("HOMEPATH".to_owned(), None);
    Ok(())
}

fn remove_owned_persistent_environment(work_root: &Path) -> Result<()> {
    let persistent = owned_persistent_root(work_root);
    if !persistent.exists() {
        return Ok(());
    }
    publication::require_no_follow_tree(&persistent)?;
    fs::remove_dir_all(&persistent).io_context(
        "P1A_PERSISTENT_ROOT_CLEANUP_FAILED",
        "could not remove the owned persistent-state tree",
    )
}

fn remove_owned_persistent_environment_best_effort(work_root: &Path) {
    let _ = remove_owned_persistent_environment(work_root);
}

fn retain_locked_resolution_cargo_home(
    source_cargo_home: &Path,
    work_root: &Path,
    cargo_lock: &Path,
) -> Result<OwnedCargoCacheSnapshot> {
    let cargo_home = owned_persistent_root(work_root).join("cargo-home");
    publication::require_no_follow_tree(&cargo_home)?;
    fs::remove_dir_all(&cargo_home).io_context(
        "P1A_OWNED_CARGO_HOME_RESET_FAILED",
        "could not reset the fresh owned Cargo home before exact lock population",
    )?;
    fs::create_dir(&cargo_home).io_context(
        "P1A_ACTIVE_CARGO_CACHE_CREATE_FAILED",
        "could not create the lock-exact Cargo resolution home",
    )?;
    let lock = cargo_lock_inventory(cargo_lock)?;

    let source_registry = source_cargo_home.join("registry");
    let source_cache_roots = plain_child_directories(&source_registry.join("cache"))?;
    let source_index_roots = plain_child_directories(&source_registry.join("index"))?;
    if source_cache_roots.len() != 1
        || source_index_roots.len() != 1
        || source_cache_roots[0].file_name() != source_index_roots[0].file_name()
    {
        return Err(XtaskError::gate(
            "P1A_CARGO_REGISTRY_IDENTITY_AMBIGUOUS",
            "the caller Cargo home does not contain exactly one matching cache/index registry identity",
            "Use the pinned crates.io registry cache without alternate registries.",
        ));
    }
    let registry_name = source_cache_roots[0]
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_CARGO_REGISTRY_IDENTITY_INVALID",
                "Cargo registry directory identity is not Unicode",
            )
        })?;
    if !registry_name.starts_with("index.crates.io-")
        || !registry_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(XtaskError::gate(
            "P1A_CARGO_REGISTRY_IDENTITY_INVALID",
            "the admitted Cargo registry is not the canonical crates.io cache identity",
            "Use the pinned crates.io registry without replacement sources.",
        ));
    }
    let destination_cache = cargo_home.join("registry/cache").join(registry_name);
    let destination_index = cargo_home.join("registry/index").join(registry_name);
    fs::create_dir_all(&destination_cache).io_context(
        "P1A_ACTIVE_CARGO_CACHE_CREATE_FAILED",
        "could not create the lock-exact Cargo archive cache",
    )?;
    fs::create_dir_all(destination_index.join(".cache")).io_context(
        "P1A_ACTIVE_CARGO_CACHE_CREATE_FAILED",
        "could not create the lock-exact Cargo sparse index",
    )?;

    let mut manifest_entries = Vec::<(String, String, u64)>::new();
    copy_selected_cache_file(
        &source_index_roots[0].join("config.json"),
        &destination_index.join("config.json"),
        &format!("index/{registry_name}/config.json"),
        &mut manifest_entries,
    )?;
    for package in &lock.registry_packages {
        let archive_name = format!("{}-{}.crate", package.name, package.version);
        let source_archive = source_cache_roots[0].join(&archive_name);
        let (archive_sha256, _) = stable_cache_file_hash(&source_archive).map_err(|error| {
            XtaskError::integrity(
                "P1A_LOCKED_CARGO_ARCHIVE_MISSING",
                format!(
                    "hash-pinned Cargo.lock archive {} is unavailable: {}",
                    package.identity, error.message
                ),
            )
        })?;
        if package.checksum.as_deref() != Some(archive_sha256.as_str()) {
            return Err(XtaskError::integrity(
                "P1A_LOCKED_CARGO_ARCHIVE_CHECKSUM_MISMATCH",
                format!(
                    "Cargo archive {} does not match its Cargo.lock checksum",
                    package.identity
                ),
            ));
        }
        copy_selected_cache_file(
            &source_archive,
            &destination_cache.join(&archive_name),
            &format!("cache/{registry_name}/{archive_name}"),
            &mut manifest_entries,
        )?;
        let copied_sha256 = manifest_entries
            .last()
            .map(|(_, sha256, _)| sha256.as_str())
            .expect("archive copy appends exactly one manifest entry");
        if package.checksum.as_deref() != Some(copied_sha256) {
            return Err(XtaskError::integrity(
                "P1A_LOCKED_CARGO_ARCHIVE_CHECKSUM_MISMATCH",
                format!(
                    "copied Cargo archive {} does not match its Cargo.lock checksum",
                    package.identity
                ),
            ));
        }
    }
    for name in &lock.sparse_index_names {
        let index_relative = crates_io_sparse_relative(name)?;
        let source = source_index_roots[0].join(".cache").join(&index_relative);
        let destination = destination_index.join(".cache").join(&index_relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).io_context(
                "P1A_ACTIVE_CARGO_CACHE_CREATE_FAILED",
                "could not create a lock-exact sparse-index parent",
            )?;
        }
        copy_selected_cache_file(
            &source,
            &destination,
            &format!(
                "index/{registry_name}/.cache/{}",
                slash_path(&index_relative)?
            ),
            &mut manifest_entries,
        )?;
    }
    manifest_entries.sort_by(|left, right| left.0.cmp(&right.0));
    validate_owned_cargo_cache_inventory(&cargo_home, &manifest_entries)?;
    let total_bytes = manifest_entries
        .iter()
        .try_fold(0_u64, |total, (_, _, bytes)| total.checked_add(*bytes))
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_ACTIVE_CARGO_CACHE_INVENTORY_INVALID",
                "lock-exact Cargo resolution cache byte count overflowed",
            )
        })?;
    let mut manifest = String::new();
    for (relative, sha256, bytes) in &manifest_entries {
        manifest.push_str(sha256);
        manifest.push_str("  ");
        manifest.push_str(&bytes.to_string());
        manifest.push_str("  ");
        manifest.push_str(relative);
        manifest.push('\n');
    }
    publication::require_no_follow_tree(&cargo_home)?;
    Ok(OwnedCargoCacheSnapshot {
        source: "cargo-lock-v4-crates-io-resolution-input-v1".to_owned(),
        resolution_input_role:
            "immutable-resolution-source-input-not-activated-compiled-or-linked-v1".to_owned(),
        copied_to_owned_root: true,
        cargo_lock_package_count: lock.packages.len(),
        locked_crates_io_package_count: lock.registry_packages.len(),
        archive_file_count: lock.registry_packages.len(),
        sparse_index_record_count: lock.sparse_index_names.len(),
        registry_config_file_count: 1,
        admitted_python_source_package: ADMITTED_PYTHON_SOURCE_PACKAGE.to_owned(),
        resolver_only_provider_source_packages: lock.provider_source_packages,
        file_count: manifest_entries.len(),
        total_bytes,
        manifest_sha256: hash::bytes(manifest.as_bytes()),
        manifest_entries,
    })
}

fn cargo_lock_inventory(cargo_lock: &Path) -> Result<CargoLockInventory> {
    const CARGO_LOCK_V4_PREAMBLE: &str = concat!(
        "# This file is automatically @generated by Cargo.\n",
        "# It is not intended for manual editing.\n",
        "version = 4\n\n",
    );
    let (stable_sha256, stable_bytes) = stable_cache_file_hash(cargo_lock)?;
    let bytes = fs::read(cargo_lock).io_context(
        "P1A_CARGO_LOCK_READ_FAILED",
        "could not read Cargo.lock for its registry-name closure",
    )?;
    if bytes.len() as u64 != stable_bytes || hash::bytes(&bytes) != stable_sha256 {
        return Err(XtaskError::integrity(
            "P1A_CARGO_LOCK_MUTATED",
            "Cargo.lock changed between its stable identity and parser read",
        ));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        XtaskError::integrity("P1A_CARGO_LOCK_INVALID", "Cargo.lock is not strict UTF-8")
    })?;
    if !text.starts_with(CARGO_LOCK_V4_PREAMBLE) {
        return Err(XtaskError::integrity(
            "P1A_CARGO_LOCK_INVALID",
            "Cargo.lock does not use the frozen version-4 preamble",
        ));
    }
    let marker = "[[package]]\n";
    let mut sections = text.split(marker);
    let preamble = sections.next().expect("split always yields preamble");
    if preamble != CARGO_LOCK_V4_PREAMBLE {
        return Err(XtaskError::integrity(
            "P1A_CARGO_LOCK_INVALID",
            "Cargo.lock preamble differs from the frozen version-4 preamble",
        ));
    }
    let mut packages = Vec::new();
    let mut identities = BTreeSet::new();
    let mut registry_packages = Vec::new();
    let mut sparse_index_names = BTreeSet::new();
    let mut provider_source_packages = Vec::new();
    let mut package_count = 0usize;
    for section in sections {
        package_count += 1;
        let name = exact_lock_field(section, "name")?;
        let version = exact_lock_field(section, "version")?;
        let source = optional_exact_lock_field(section, "source")?;
        let checksum = optional_exact_lock_field(section, "checksum")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || version.is_empty()
            || !version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
        {
            return Err(XtaskError::integrity(
                "P1A_CARGO_LOCK_INVALID",
                "Cargo.lock contains a nonportable package identity",
            ));
        }
        let identity = format!("{name}@{version}");
        if !identities.insert(identity.clone()) {
            return Err(XtaskError::integrity(
                "P1A_CARGO_LOCK_INVALID",
                format!("Cargo.lock repeats package identity {identity}"),
            ));
        }
        if is_forbidden_python_source_package(&name, &version) {
            return Err(XtaskError::gate(
                "P1A_CARGO_LOCK_PYTHON_PACKAGE_REJECTED",
                format!("Cargo.lock contains forbidden Python package {identity}"),
                "Remove Python packages; only tree-sitter-python 0.25.0 is admitted as generated-C source input.",
            ));
        }
        let package = CargoLockPackage {
            name: name.clone(),
            version,
            identity: identity.clone(),
            checksum: checksum.clone(),
        };
        match (source, checksum) {
            (Some(source), Some(checksum)) => {
                if source != "registry+https://github.com/rust-lang/crates.io-index"
                    || !hash::is_lower_sha256(&checksum)
                {
                    return Err(XtaskError::gate(
                        "P1A_CARGO_LOCK_SOURCE_REJECTED",
                        "Cargo.lock contains a non-crates.io or unhashed external package",
                        "Use only hash-pinned crates.io packages for the P1A CPU graph.",
                    ));
                }
                sparse_index_names.insert(name.clone());
                if is_provider_source_package_name(&name) {
                    provider_source_packages.push(identity);
                }
                registry_packages.push(package.clone());
            }
            (None, None) => {}
            _ => {
                return Err(XtaskError::integrity(
                    "P1A_CARGO_LOCK_INVALID",
                    "Cargo.lock package source and checksum are not paired",
                ));
            }
        }
        packages.push(package);
    }
    if package_count == 0 || registry_packages.is_empty() {
        return Err(XtaskError::integrity(
            "P1A_CARGO_LOCK_INVALID",
            "Cargo.lock contains no package or registry identity",
        ));
    }
    provider_source_packages.sort();
    registry_packages.sort_by(|left, right| left.identity.cmp(&right.identity));
    Ok(CargoLockInventory {
        packages,
        identities,
        registry_packages,
        sparse_index_names,
        provider_source_packages,
    })
}

fn audit_active_packages_against_lock(
    active_packages: &[String],
    lock: &CargoLockInventory,
) -> Result<()> {
    if active_packages.is_empty() {
        return Err(XtaskError::integrity(
            "P1A_ACTIVE_PACKAGE_INVENTORY_EMPTY",
            "the activated Cargo package inventory is empty",
        ));
    }
    let active = active_packages.iter().cloned().collect::<BTreeSet<_>>();
    if active.len() != active_packages.len() {
        return Err(XtaskError::integrity(
            "P1A_ACTIVE_PACKAGE_IDENTITY_INVALID",
            "the activated Cargo package inventory repeats an identity",
        ));
    }
    let outside_lock = active
        .difference(&lock.identities)
        .cloned()
        .collect::<Vec<_>>();
    if !outside_lock.is_empty() {
        return Err(XtaskError::integrity(
            "P1A_ACTIVE_PACKAGE_OUTSIDE_LOCK",
            format!(
                "activated packages are absent from Cargo.lock: {}",
                outside_lock.join(",")
            ),
        ));
    }
    let forbidden = active
        .iter()
        .filter_map(|identity| {
            let (name, version) = identity.split_once('@')?;
            (is_provider_source_package_name(name)
                || is_forbidden_python_source_package(name, version))
            .then(|| identity.clone())
        })
        .collect::<Vec<_>>();
    if !forbidden.is_empty() {
        return Err(XtaskError::gate(
            "P1A_CPU_GRAPH_BOUNDARY_VIOLATED",
            format!(
                "activated CPU graph contains provider or Python packages: {}",
                forbidden.join(",")
            ),
            "Remove provider and Python packages from the activated CPU graph.",
        ));
    }
    Ok(())
}

fn validate_resolution_cache_against_graph(
    cache: &OwnedCargoCacheSnapshot,
    graph: &GraphAudit,
) -> Result<()> {
    let active = graph
        .activated_packages
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let activated_provider_sources = cache
        .resolver_only_provider_source_packages
        .iter()
        .filter(|identity| active.contains(identity.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !activated_provider_sources.is_empty() {
        return Err(XtaskError::integrity(
            "P1A_RESOLVER_ONLY_PROVIDER_ACTIVATED",
            format!(
                "provider sources classified as resolver-only were activated: {}",
                activated_provider_sources.join(",")
            ),
        ));
    }
    if cache.file_count
        != cache.archive_file_count
            + cache.sparse_index_record_count
            + cache.registry_config_file_count
        || cache.archive_file_count != cache.locked_crates_io_package_count
        || cache.registry_config_file_count != 1
    {
        return Err(XtaskError::integrity(
            "P1A_LOCKED_CARGO_CACHE_COUNT_INVALID",
            "Cargo resolution-input package, archive, index, and file counts disagree",
        ));
    }
    Ok(())
}

fn is_forbidden_python_source_package(name: &str, version: &str) -> bool {
    let identity = format!("{}@{}", name.to_ascii_lowercase(), version);
    if identity == ADMITTED_PYTHON_SOURCE_PACKAGE {
        return false;
    }
    let name = name.to_ascii_lowercase();
    name.contains("python")
        || name == "cpython"
        || name == "maturin"
        || name == "pyo3"
        || name.starts_with("pyo3-")
        || name == "setuptools-rust"
}

fn is_provider_source_package_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    EXPLICIT_PROVIDER_SOURCE_PACKAGE_NAMES.contains(&name.as_str()) || name.starts_with("wgpu-")
}

fn validate_owned_cargo_cache_inventory(
    cargo_home: &Path,
    expected: &[(String, String, u64)],
) -> Result<()> {
    let mut actual = Vec::new();
    collect_cache_inventory(
        &cargo_home.join("registry/cache"),
        "cache",
        false,
        &mut actual,
    )?;
    collect_cache_inventory(
        &cargo_home.join("registry/index"),
        "index",
        false,
        &mut actual,
    )?;
    actual.sort_by(|left, right| left.0.cmp(&right.0));
    if expected != actual {
        return Err(XtaskError::integrity(
            "P1A_LOCKED_CARGO_CACHE_INVENTORY_INVALID",
            "the lock-exact Cargo resolution input contains a missing, changed, or extra file",
        ));
    }
    Ok(())
}

fn exact_lock_field(section: &str, field: &str) -> Result<String> {
    optional_exact_lock_field(section, field)?.ok_or_else(|| {
        XtaskError::integrity(
            "P1A_CARGO_LOCK_INVALID",
            format!("Cargo.lock package omitted {field}"),
        )
    })
}

fn optional_exact_lock_field(section: &str, field: &str) -> Result<Option<String>> {
    let prefix = format!("{field} = \"");
    let mut values = section
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .map(|tail| {
            tail.strip_suffix('"').ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_CARGO_LOCK_INVALID",
                    format!("Cargo.lock {field} is not an exact quoted scalar"),
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if values.len() > 1 {
        return Err(XtaskError::integrity(
            "P1A_CARGO_LOCK_INVALID",
            format!("Cargo.lock package repeats {field}"),
        ));
    }
    Ok(values.pop().map(str::to_owned))
}

fn plain_child_directories(root: &Path) -> Result<Vec<PathBuf>> {
    let metadata = fs::symlink_metadata(root).io_context(
        "P1A_CARGO_REGISTRY_INVENTORY_INVALID",
        "could not inspect a Cargo registry inventory root",
    )?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
        return Err(XtaskError::integrity(
            "P1A_CARGO_REGISTRY_INVENTORY_INVALID",
            "Cargo registry inventory root is not a plain directory",
        ));
    }
    let mut directories = Vec::new();
    for entry in fs::read_dir(root).io_context(
        "P1A_CARGO_REGISTRY_INVENTORY_INVALID",
        "could not enumerate Cargo registry identities",
    )? {
        let entry = entry.io_context(
            "P1A_CARGO_REGISTRY_INVENTORY_INVALID",
            "could not read a Cargo registry identity",
        )?;
        let metadata = fs::symlink_metadata(entry.path()).io_context(
            "P1A_CARGO_REGISTRY_INVENTORY_INVALID",
            "could not inspect a Cargo registry identity",
        )?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata)
        {
            return Err(XtaskError::integrity(
                "P1A_CARGO_REGISTRY_INVENTORY_INVALID",
                "Cargo registry inventory contains a non-directory or reparse entry",
            ));
        }
        directories.push(entry.path());
    }
    directories.sort();
    Ok(directories)
}

fn crates_io_sparse_relative(name: &str) -> Result<PathBuf> {
    let name = name.to_ascii_lowercase();
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(XtaskError::integrity(
            "P1A_ACTIVE_PACKAGE_IDENTITY_INVALID",
            "active package name cannot map to the crates.io sparse index",
        ));
    }
    Ok(match bytes.len() {
        1 => PathBuf::from("1").join(&name),
        2 => PathBuf::from("2").join(&name),
        3 => PathBuf::from("3")
            .join(String::from_utf8(vec![bytes[0]]).expect("ASCII package"))
            .join(&name),
        _ => PathBuf::from(&name[..2]).join(&name[2..4]).join(&name),
    })
}

fn copy_selected_cache_file(
    source: &Path,
    destination: &Path,
    relative: &str,
    manifest: &mut Vec<(String, String, u64)>,
) -> Result<()> {
    let (sha256, bytes) = copy_owned_cache_file(source, destination)?;
    let (source_sha256, source_bytes) = stable_cache_file_hash(source)?;
    if sha256 != source_sha256 || bytes != source_bytes || hash::file(destination)? != sha256 {
        return Err(XtaskError::integrity(
            "P1A_ACTIVE_CARGO_CACHE_DRIFT",
            "an active Cargo cache file differs across source, copy, and revalidation",
        ));
    }
    manifest.push((relative.to_owned(), sha256, bytes));
    Ok(())
}

fn slash_path(path: &Path) -> Result<String> {
    let mut output = String::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(XtaskError::integrity(
                "P1A_CARGO_CACHE_PATH_INVALID",
                "Cargo cache path contains a non-normal component",
            ));
        };
        let component = component.to_str().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_CARGO_CACHE_PATH_INVALID",
                "Cargo cache path is not Unicode",
            )
        })?;
        if !output.is_empty() {
            output.push('/');
        }
        output.push_str(component);
    }
    Ok(output)
}

fn collect_cache_inventory(
    root: &Path,
    relative_prefix: &str,
    lock_against_mutation: bool,
    manifest: &mut Vec<(String, String, u64)>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(root).io_context(
        "P1A_CARGO_CACHE_INVENTORY_INVALID",
        "could not inspect a Cargo cache inventory directory",
    )?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
        return Err(XtaskError::integrity(
            "P1A_CARGO_CACHE_INVENTORY_INVALID",
            "Cargo cache inventory crosses a non-directory or reparse point",
        ));
    }
    let mut entries = fs::read_dir(root)
        .io_context(
            "P1A_CARGO_CACHE_INVENTORY_INVALID",
            "could not enumerate a Cargo cache inventory",
        )?
        .collect::<std::io::Result<Vec<_>>>()
        .io_context(
            "P1A_CARGO_CACHE_INVENTORY_INVALID",
            "could not enumerate a Cargo cache inventory",
        )?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_CARGO_CACHE_PATH_INVALID",
                "Cargo cache inventory path is not Unicode",
            )
        })?;
        if name.is_empty() || matches!(name, "." | "..") || name.contains(['/', '\\']) {
            return Err(XtaskError::integrity(
                "P1A_CARGO_CACHE_PATH_INVALID",
                "Cargo cache inventory contains an invalid path component",
            ));
        }
        let path = entry.path();
        let relative = format!("{relative_prefix}/{name}");
        let metadata = fs::symlink_metadata(&path).io_context(
            "P1A_CARGO_CACHE_INVENTORY_INVALID",
            "could not inspect a Cargo cache inventory entry",
        )?;
        if metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
            return Err(XtaskError::integrity(
                "P1A_CARGO_CACHE_REPARSE_REJECTED",
                "Cargo cache inventory crosses a link or reparse point",
            ));
        }
        if metadata.is_dir() {
            collect_cache_inventory(&path, &relative, lock_against_mutation, manifest)?;
        } else if metadata.is_file() {
            let (sha256, bytes) = if lock_against_mutation {
                stable_cache_file_hash(&path)?
            } else {
                (hash::file(&path)?, metadata.len())
            };
            manifest.push((relative, sha256, bytes));
        } else {
            return Err(XtaskError::integrity(
                "P1A_CARGO_CACHE_SPECIAL_REJECTED",
                "Cargo cache inventory contains a special filesystem entry",
            ));
        }
        if manifest.len() > 100_000 {
            return Err(XtaskError::gate(
                "P1A_OWNED_CARGO_CACHE_INVALID",
                "the admitted Cargo cache exceeds 100000 files",
                "Retain only registry content required by Cargo.lock.",
            ));
        }
    }
    Ok(())
}

fn stable_cache_file_hash(path: &Path) -> Result<(String, u64)> {
    #[cfg(windows)]
    let mut input = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
            .io_context(
                "P1A_CARGO_CACHE_LOCK_FAILED",
                "could not deny write/delete access to a Cargo cache inventory file",
            )?
    };
    #[cfg(not(windows))]
    let mut input = fs::File::open(path).io_context(
        "P1A_CARGO_CACHE_LOCK_FAILED",
        "could not open a Cargo cache inventory file",
    )?;
    let before = input.metadata().io_context(
        "P1A_CARGO_CACHE_INVENTORY_INVALID",
        "could not bind a Cargo cache inventory file",
    )?;
    if !before.is_file() || before.len() == 0 || before.len() > 1024 * 1024 * 1024 {
        return Err(XtaskError::integrity(
            "P1A_CARGO_CACHE_INVENTORY_INVALID",
            "Cargo cache inventory file is empty, nonregular, or over one GiB",
        ));
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).io_context(
            "P1A_CARGO_CACHE_READ_FAILED",
            "could not read a Cargo cache inventory file",
        )?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let after = input.metadata().io_context(
        "P1A_CARGO_CACHE_INVENTORY_INVALID",
        "could not revalidate a Cargo cache inventory file",
    )?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || is_windows_reparse(&after)
    {
        return Err(XtaskError::integrity(
            "P1A_CARGO_CACHE_SOURCE_MUTATED",
            "a Cargo cache inventory file changed during stable hashing",
        ));
    }
    Ok((hex::encode(hasher.finalize()), before.len()))
}

fn copy_owned_cache_file(source: &Path, destination: &Path) -> Result<(String, u64)> {
    #[cfg(windows)]
    let mut input = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(source)
            .io_context(
                "P1A_CARGO_CACHE_LOCK_FAILED",
                "could not deny write/delete access to a Cargo cache source file",
            )?
    };
    #[cfg(not(windows))]
    let mut input = fs::File::open(source).io_context(
        "P1A_CARGO_CACHE_LOCK_FAILED",
        "could not open a Cargo cache source file",
    )?;
    let before = input.metadata().io_context(
        "P1A_CARGO_CACHE_SOURCE_INVALID",
        "could not bind a Cargo cache source file",
    )?;
    if !before.is_file() || before.len() == 0 || before.len() > 1024 * 1024 * 1024 {
        return Err(XtaskError::integrity(
            "P1A_CARGO_CACHE_SOURCE_INVALID",
            "Cargo cache source file is empty, nonregular, or over one GiB",
        ));
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .io_context(
            "P1A_OWNED_CARGO_CACHE_WRITE_FAILED",
            "could not create an owned Cargo cache file",
        )?;
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).io_context(
            "P1A_CARGO_CACHE_READ_FAILED",
            "could not read a Cargo cache source file",
        )?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).io_context(
            "P1A_OWNED_CARGO_CACHE_WRITE_FAILED",
            "could not write an owned Cargo cache file",
        )?;
        hasher.update(&buffer[..read]);
        copied = copied.checked_add(read as u64).ok_or_else(|| {
            XtaskError::integrity(
                "P1A_OWNED_CARGO_CACHE_INVALID",
                "Cargo cache copy byte count overflowed",
            )
        })?;
    }
    output.sync_all().io_context(
        "P1A_OWNED_CARGO_CACHE_WRITE_FAILED",
        "could not flush an owned Cargo cache file",
    )?;
    let after = input.metadata().io_context(
        "P1A_CARGO_CACHE_SOURCE_INVALID",
        "could not revalidate a Cargo cache source file",
    )?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || copied != before.len()
        || is_windows_reparse(&after)
    {
        return Err(XtaskError::integrity(
            "P1A_CARGO_CACHE_SOURCE_MUTATED",
            "a Cargo cache source file changed during its stable copy",
        ));
    }
    Ok((hex::encode(hasher.finalize()), copied))
}

fn version_field(text: &str, name: &str) -> Result<String> {
    let prefix = format!("{name}: ");
    let values = text
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect::<Vec<_>>();
    if values.len() != 1 || values[0].is_empty() {
        return Err(XtaskError::integrity(
            "P1A_RUST_VERSION_INVALID",
            format!("Rust version output did not contain exactly one {name} field"),
        ));
    }
    Ok(values[0].to_owned())
}

fn parse_three_part_version(value: &str) -> Result<(u32, u32, u32)> {
    let parts = value
        .split('.')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| {
            XtaskError::integrity(
                "P1A_VERSION_INVALID",
                format!("version is not numeric major.minor.patch: {value}"),
            )
        })?;
    if parts.len() != 3 {
        return Err(XtaskError::integrity(
            "P1A_VERSION_INVALID",
            format!("version is not exactly major.minor.patch: {value}"),
        ));
    }
    Ok((parts[0], parts[1], parts[2]))
}

fn native_build_environment(
    host: &PrototypeWindowsHostReport,
) -> Result<BTreeMap<String, Option<OsString>>> {
    let sdk = &host.windows_sdk;
    let vs = &host.visual_studio;
    let include_root = sdk.kits_root.join("Include").join(&sdk.version);
    let lib_root = sdk.kits_root.join("Lib").join(&sdk.version);
    let includes = [
        vs.msvc_include.clone(),
        include_root.join("ucrt"),
        include_root.join("shared"),
        include_root.join("um"),
        include_root.join("winrt"),
        include_root.join("cppwinrt"),
    ];
    let libraries = [
        vs.msvc_x64_lib.clone(),
        lib_root.join("ucrt/x64"),
        lib_root.join("um/x64"),
    ];
    for path in includes.iter().chain(libraries.iter()) {
        if !path.is_dir() {
            return Err(XtaskError::environment(
                "P1A_NATIVE_SEARCH_PATH_MISSING",
                format!("required native search path is absent: {}", path.display()),
            ));
        }
    }
    let mut path_entries = vec![
        vs.cl
            .path
            .parent()
            .unwrap_or(&vs.msvc_tools_root)
            .to_path_buf(),
        sdk.rc.path.parent().unwrap_or(&sdk.kits_root).to_path_buf(),
    ];
    path_entries.push(system32()?);
    let joined_path = std::env::join_paths(path_entries).map_err(|error| {
        XtaskError::environment(
            "P1A_NATIVE_PATH_INVALID",
            format!("could not construct the native tool PATH: {error}"),
        )
    })?;
    let include = std::env::join_paths(includes).map_err(|error| {
        XtaskError::environment(
            "P1A_NATIVE_INCLUDE_INVALID",
            format!("could not construct INCLUDE: {error}"),
        )
    })?;
    let lib = std::env::join_paths(libraries).map_err(|error| {
        XtaskError::environment(
            "P1A_NATIVE_LIB_INVALID",
            format!("could not construct LIB: {error}"),
        )
    })?;
    let mut environment = BTreeMap::from([
        ("PATH".to_owned(), Some(joined_path)),
        ("INCLUDE".to_owned(), Some(include)),
        ("LIB".to_owned(), Some(lib)),
    ]);
    for name in [
        "CUDA_PATH",
        "CUDA_HOME",
        "CUDNN_PATH",
        "HIP_PATH",
        "ROCM_PATH",
        "LIBTORCH",
        "PYTHONHOME",
        "PYTHONPATH",
    ] {
        environment.insert(name.to_owned(), None);
    }
    Ok(environment)
}

fn run_native_probe(
    repository: &Path,
    recorder: &mut P1aRecorder,
    work_root: &Path,
    host: &PrototypeWindowsHostReport,
    rust: &RustToolchain,
    before_manifest: &str,
) -> Result<Value> {
    let sources = work_root.join("sources");
    let native = work_root.join("native");
    let rust_target = work_root.join("rust-target");
    publication::create_dir(&sources)?;
    publication::create_dir(&native)?;
    publication::create_dir(&rust_target)?;
    for name in ["p1a_abi.c", "p1a_abi.cpp", "p1a_abi.rs"] {
        let source = repository.join("xtask/probes").join(name);
        let bytes = fs::read(&source).io_context(
            "P1A_PROBE_SOURCE_READ_FAILED",
            format!("could not read {}", source.display()),
        )?;
        publication::write_new(&sources.join(name), &bytes)?;
    }
    let environment = native_build_environment(host)?;
    let roots = vec![host.visual_studio.msvc_tools_root.clone()];
    let c_obj = native.join("p1a_c.obj");
    let cpp_obj = native.join("p1a_cpp.obj");
    let c_lib = native.join("p1a_c.lib");
    let cpp_lib = native.join("p1a_cpp.lib");
    let executable = rust_target.join("p1a_abi_probe.exe");

    let c_args = vec![
        OsString::from("/nologo"),
        OsString::from("/TC"),
        OsString::from("/std:c17"),
        OsString::from("/W4"),
        OsString::from("/WX"),
        OsString::from("/MD"),
        OsString::from("/c"),
        sources.join("p1a_abi.c").into_os_string(),
        OsString::from(format!("/Fo{}", c_obj.display())),
    ];
    let c = recorder.run_audited(
        repository,
        work_root,
        &host.visual_studio.cl.path,
        c_args,
        vec![
            "${CL}".to_owned(),
            "/nologo".to_owned(),
            "/TC".to_owned(),
            "/std:c17".to_owned(),
            "/W4".to_owned(),
            "/WX".to_owned(),
            "/MD".to_owned(),
            "/c".to_owned(),
            "${P1A_TEMP}/sources/p1a_abi.c".to_owned(),
            "/Fo${P1A_TEMP}/native/p1a_c.obj".to_owned(),
        ],
        "${P1A_TEMP}",
        environment.clone(),
        Duration::from_secs(120),
        roots.clone(),
        "not_applicable",
    )?;
    require_audited_pass(&c, "P1A_C_COMPILE_FAILED")?;

    let cpp_args = vec![
        OsString::from("/nologo"),
        OsString::from("/TP"),
        OsString::from("/std:c++20"),
        OsString::from("/EHsc"),
        OsString::from("/W4"),
        OsString::from("/WX"),
        OsString::from("/MD"),
        OsString::from("/c"),
        sources.join("p1a_abi.cpp").into_os_string(),
        OsString::from(format!("/Fo{}", cpp_obj.display())),
    ];
    let cpp = recorder.run_audited(
        repository,
        work_root,
        &host.visual_studio.cl.path,
        cpp_args,
        vec![
            "${CL}".to_owned(),
            "/nologo".to_owned(),
            "/TP".to_owned(),
            "/std:c++20".to_owned(),
            "/EHsc".to_owned(),
            "/W4".to_owned(),
            "/WX".to_owned(),
            "/MD".to_owned(),
            "/c".to_owned(),
            "${P1A_TEMP}/sources/p1a_abi.cpp".to_owned(),
            "/Fo${P1A_TEMP}/native/p1a_cpp.obj".to_owned(),
        ],
        "${P1A_TEMP}",
        environment.clone(),
        Duration::from_secs(120),
        roots.clone(),
        "not_applicable",
    )?;
    require_audited_pass(&cpp, "P1A_CPP_COMPILE_FAILED")?;

    for (object, archive, display_object, display_archive) in [
        (&c_obj, &c_lib, "p1a_c.obj", "p1a_c.lib"),
        (&cpp_obj, &cpp_lib, "p1a_cpp.obj", "p1a_cpp.lib"),
    ] {
        let output = recorder.run_audited(
            repository,
            work_root,
            &host.visual_studio.lib.path,
            vec![
                OsString::from("/nologo"),
                OsString::from(format!("/OUT:{}", archive.display())),
                object.as_os_str().to_owned(),
            ],
            vec![
                "${LIB}".to_owned(),
                "/nologo".to_owned(),
                format!("/OUT:${{P1A_TEMP}}/native/{display_archive}"),
                format!("${{P1A_TEMP}}/native/{display_object}"),
            ],
            "${P1A_TEMP}",
            environment.clone(),
            Duration::from_secs(120),
            roots.clone(),
            "not_applicable",
        )?;
        require_audited_pass(&output, "P1A_NATIVE_ARCHIVE_FAILED")?;
    }

    let rust_args = vec![
        OsString::from("--edition=2024"),
        sources.join("p1a_abi.rs").into_os_string(),
        OsString::from("--target=x86_64-pc-windows-msvc"),
        OsString::from("-C"),
        OsString::from(format!("linker={}", host.visual_studio.link.path.display())),
        OsString::from("-L"),
        OsString::from(format!("native={}", native.display())),
        OsString::from("-l"),
        OsString::from("static=p1a_c"),
        OsString::from("-l"),
        OsString::from("static=p1a_cpp"),
        OsString::from("-o"),
        executable.as_os_str().to_owned(),
    ];
    let rust_link = recorder.run_audited(
        repository,
        work_root,
        &rust.rustc,
        rust_args,
        vec![
            "${RUSTC}".to_owned(),
            "--edition=2024".to_owned(),
            "${P1A_TEMP}/sources/p1a_abi.rs".to_owned(),
            "--target=x86_64-pc-windows-msvc".to_owned(),
            "-C".to_owned(),
            "linker=${LINK}".to_owned(),
            "-L".to_owned(),
            "native=${P1A_TEMP}/native".to_owned(),
            "-l".to_owned(),
            "static=p1a_c".to_owned(),
            "-l".to_owned(),
            "static=p1a_cpp".to_owned(),
            "-o".to_owned(),
            "${P1A_TEMP}/rust-target/p1a_abi_probe.exe".to_owned(),
        ],
        "${P1A_TEMP}",
        environment.clone(),
        Duration::from_secs(180),
        roots.clone(),
        "not_applicable",
    )?;
    require_audited_pass(&rust_link, "P1A_RUST_NATIVE_LINK_FAILED")?;
    let probe = recorder.run_audited(
        repository,
        work_root,
        &executable,
        Vec::new(),
        vec!["${P1A_TEMP}/rust-target/p1a_abi_probe.exe".to_owned()],
        "${P1A_TEMP}",
        environment.clone(),
        Duration::from_secs(30),
        Vec::new(),
        "not_applicable",
    )?;
    require_audited_pass(&probe, "P1A_ABI_PROBE_FAILED")?;
    if probe.stdout != ABI_EXPECTED_STDOUT || hash::bytes(&probe.stdout) != ABI_EXPECTED_SHA256 {
        return Err(XtaskError::gate(
            "P1A_ABI_OUTPUT_MISMATCH",
            format!(
                "ABI probe returned {} unexpected output bytes",
                probe.stdout.len()
            ),
            "Correct the qualified native ABI without changing the frozen probe.",
        ));
    }
    let dumpbin = recorder.run_audited(
        repository,
        work_root,
        &host.visual_studio.dumpbin.path,
        vec![
            OsString::from("/HEADERS"),
            OsString::from("/DEPENDENTS"),
            executable.as_os_str().to_owned(),
        ],
        vec![
            "${DUMPBIN}".to_owned(),
            "/HEADERS".to_owned(),
            "/DEPENDENTS".to_owned(),
            "${P1A_TEMP}/rust-target/p1a_abi_probe.exe".to_owned(),
        ],
        "${P1A_TEMP}",
        environment,
        Duration::from_secs(120),
        roots,
        "not_applicable",
    )?;
    require_audited_pass(&dumpbin, "P1A_BINARY_AUDIT_FAILED")?;
    let imports = parse_dumpbin_imports(&dumpbin.stdout)?;
    let unexpected_imports = imports
        .iter()
        .filter(|import| {
            !ABI_ALLOWED_IMPORTS
                .iter()
                .any(|allowed| import.eq_ignore_ascii_case(allowed))
        })
        .cloned()
        .collect::<Vec<_>>();
    let lower_imports = imports
        .iter()
        .map(|item| item.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let python_imports = matching_items(&imports, &lower_imports, &["python", "pypy"]);
    let accelerator_imports = matching_items(
        &imports,
        &lower_imports,
        &[
            "cuda", "cudart", "cublas", "cudnn", "nvcuda", "hip", "roc", "metal",
        ],
    );
    let native_ml_imports = matching_items(
        &imports,
        &lower_imports,
        &["torch", "onnxruntime", "tensorflow"],
    );
    let debug_runtime_imports = imports
        .iter()
        .filter(|item| item.to_ascii_lowercase().ends_with("d.dll"))
        .cloned()
        .collect::<Vec<_>>();
    if !python_imports.is_empty()
        || !accelerator_imports.is_empty()
        || !native_ml_imports.is_empty()
        || !debug_runtime_imports.is_empty()
        || !unexpected_imports.is_empty()
    {
        return Err(XtaskError::gate(
            "P1A_BINARY_IMPORT_BOUNDARY_VIOLATED",
            "ABI probe imports Python, accelerator, native ML, or debug runtime libraries",
            "Restore the CPU-only release-runtime native link boundary.",
        ));
    }
    require_exact_probe_artifact_inventory(work_root)?;
    let after_manifest = source_manifest_hash(repository, recorder)?;
    if after_manifest != before_manifest {
        return Err(XtaskError::integrity(
            "P1A_INPUT_MUTATION_DETECTED",
            "tracked source inputs changed during the native ABI probe",
        ));
    }
    let source_inputs = [
        ("p1a_abi.c", "c17"),
        ("p1a_abi.cpp", "c++20"),
        ("p1a_abi.rs", "rust2024"),
    ]
    .into_iter()
    .map(|(name, language)| {
        file_value(
            &sources.join(name),
            &format!("sources/{name}"),
            Some(language),
        )
    })
    .collect::<Result<Vec<_>>>()?;
    let native_outputs = [
        (&c_obj, "native/p1a_c.obj"),
        (&cpp_obj, "native/p1a_cpp.obj"),
        (&c_lib, "native/p1a_c.lib"),
        (&cpp_lib, "native/p1a_cpp.lib"),
    ]
    .into_iter()
    .map(|(path, relative)| file_value(path, relative, None))
    .collect::<Result<Vec<_>>>()?;
    let executable_ref = file_value(&executable, "rust-target/p1a_abi_probe.exe", None)?;
    let import_classifications = imports
        .iter()
        .map(|name| {
            let lower = name.to_ascii_lowercase();
            let class = if lower == "vcruntime140.dll" {
                "msvc_release_runtime"
            } else if lower.starts_with("api-ms-win-crt-") {
                "ucrt_release"
            } else {
                "windows_system"
            };
            json!({"name": name, "class": class})
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "python-slm-p1a-native-abi-probe-v1",
        "phase_id": PHASE_ID, "interface_id": INTERFACE_ID, "profile_id": PROFILE_ID,
        "support_tier": SUPPORT_TIER, "status": "PASS",
        "result": {
            "kind": "complete",
            "purpose": "permitted_future_accelerator_or_native_ml_ffi_shape_only",
            "temp_root": "${P1A_TEMP}",
            "fresh_native_build_directory": true,
            "fresh_rust_target_directory": true,
            "source_inputs": source_inputs,
            "native_outputs": native_outputs,
            "probe_executable": executable_ref,
            "rust_c_probe": probe_result("p1a_c_probe", 3137),
            "rust_cpp_probe": probe_result("p1a_cpp_probe", 150),
            "target_abi": target_abi(false),
            "binary_audit": {
                "pe_machine": "AMD64", "imports": imports,
                "allowed_imports": ABI_ALLOWED_IMPORTS,
                "import_classifications": import_classifications,
                "python_imports": [], "accelerator_imports": [],
                "native_ml_backend_imports": [], "debug_runtime_imports": [],
                "unexpected_imports": [],
                "generated_artifact_scan": {
                    "python_artifacts": [], "accelerator_artifacts": [],
                    "native_ml_backend_artifacts": []
                }
            },
            "input_stability": {"before_sha256": before_manifest, "after_sha256": after_manifest, "stable": true},
            "cleanup": {"owned_temp_root_removed": true, "process_job_empty": true, "persistent_state_mutation_requested": false}
        },
        "errors": []
    }))
}

fn require_exact_probe_artifact_inventory(work_root: &Path) -> Result<()> {
    publication::require_no_follow_tree(work_root)?;
    let expected_top_level_directories = BTreeSet::from([
        "command-captures".to_owned(),
        "native".to_owned(),
        "persistent".to_owned(),
        "rust-target".to_owned(),
        "sources".to_owned(),
    ]);
    let mut actual_top_level_directories = BTreeSet::new();
    for entry in fs::read_dir(work_root).io_context(
        "P1A_ARTIFACT_SCAN_FAILED",
        "could not enumerate the native probe owned root",
    )? {
        let entry = entry.io_context(
            "P1A_ARTIFACT_SCAN_FAILED",
            "could not read a native probe owned-root entry",
        )?;
        let metadata = fs::symlink_metadata(entry.path()).io_context(
            "P1A_ARTIFACT_SCAN_FAILED",
            "could not inspect a native probe owned-root entry",
        )?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata)
        {
            return Err(XtaskError::integrity(
                "P1A_ARTIFACT_INVENTORY_UNEXPECTED",
                "native probe owned root contains a non-directory or linked top-level entry",
            ));
        }
        let name = entry.file_name().into_string().map_err(|_| {
            XtaskError::integrity(
                "P1A_ARTIFACT_INVENTORY_UNEXPECTED",
                "native probe owned root contains a non-Unicode top-level entry",
            )
        })?;
        actual_top_level_directories.insert(name);
    }
    if actual_top_level_directories != expected_top_level_directories {
        return Err(XtaskError::integrity(
            "P1A_ARTIFACT_INVENTORY_UNEXPECTED",
            "native probe owned root differs from the closed top-level directory inventory",
        ));
    }

    let expected_directories = BTreeSet::from([
        "native".to_owned(),
        "rust-target".to_owned(),
        "sources".to_owned(),
    ]);
    let expected_files = BTreeSet::from([
        "native/p1a_c.lib".to_owned(),
        "native/p1a_c.obj".to_owned(),
        "native/p1a_cpp.lib".to_owned(),
        "native/p1a_cpp.obj".to_owned(),
        "rust-target/p1a_abi_probe.exe".to_owned(),
        "sources/p1a_abi.c".to_owned(),
        "sources/p1a_abi.cpp".to_owned(),
        "sources/p1a_abi.rs".to_owned(),
    ]);
    let mut actual_directories = expected_directories.clone();
    let mut actual_files = BTreeSet::new();
    let mut stack = expected_directories
        .iter()
        .map(|relative| work_root.join(relative))
        .collect::<Vec<_>>();
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).io_context(
            "P1A_ARTIFACT_SCAN_FAILED",
            "could not enumerate the native probe artifact tree",
        )? {
            let entry = entry.io_context(
                "P1A_ARTIFACT_SCAN_FAILED",
                "could not read a native probe artifact entry",
            )?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).io_context(
                "P1A_ARTIFACT_SCAN_FAILED",
                "could not inspect a native probe artifact entry",
            )?;
            if metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
                return Err(XtaskError::integrity(
                    "P1A_ARTIFACT_SCAN_REPARSE_REJECTED",
                    "native probe artifact inventory contains a link or reparse point",
                ));
            }
            let relative = path
                .strip_prefix(work_root)
                .map_err(|_| {
                    XtaskError::integrity(
                        "P1A_ARTIFACT_SCAN_ESCAPED",
                        "native probe artifact escaped its owned root",
                    )
                })?
                .to_string_lossy()
                .replace('\\', "/");
            if metadata.is_dir() {
                actual_directories.insert(relative);
                stack.push(path);
            } else if metadata.is_file() {
                if metadata.len() == 0 {
                    return Err(XtaskError::integrity(
                        "P1A_ARTIFACT_SCAN_EMPTY_FILE",
                        "native probe artifact inventory contains an empty file",
                    ));
                }
                actual_files.insert(relative);
            } else {
                return Err(XtaskError::integrity(
                    "P1A_ARTIFACT_SCAN_NONREGULAR_REJECTED",
                    "native probe artifact inventory contains a special entry",
                ));
            }
        }
    }
    if actual_directories != expected_directories || actual_files != expected_files {
        return Err(XtaskError::integrity(
            "P1A_ARTIFACT_INVENTORY_UNEXPECTED",
            "native probe generated an artifact outside the exact frozen inventory",
        ));
    }
    Ok(())
}

fn file_value(path: &Path, relative: &str, language: Option<&str>) -> Result<Value> {
    let metadata = fs::metadata(path).io_context(
        "P1A_PROBE_OUTPUT_MISSING",
        format!("could not inspect {}", path.display()),
    )?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(XtaskError::gate(
            "P1A_PROBE_OUTPUT_INVALID",
            format!("probe output is absent or empty: {}", path.display()),
            "Correct the native probe toolchain and retry.",
        ));
    }
    let mut value = json!({"path": relative, "sha256": hash::file(path)?, "bytes": metadata.len()});
    if let Some(language) = language {
        value["language"] = json!(language);
    }
    Ok(value)
}

fn probe_result(symbol: &str, expected_value: u64) -> Value {
    json!({
        "symbol": symbol, "arguments": [40, 2],
        "expected_value": expected_value, "observed_value": expected_value,
        "compiled": true, "linked": true, "executed": true, "exit_code": 0,
        "expected_stdout": "P1A_ABI_PASS c=3137 cpp=150\n",
        "observed_stdout": "P1A_ABI_PASS c=3137 cpp=150\n",
        "expected_output_sha256": ABI_EXPECTED_SHA256,
        "output_sha256": ABI_EXPECTED_SHA256, "output_matches_expected": true
    })
}

fn parse_dumpbin_imports(bytes: &[u8]) -> Result<Vec<String>> {
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    if !(lower.contains("machine (x64)") || lower.contains("8664 machine")) {
        return Err(XtaskError::gate(
            "P1A_PE_MACHINE_MISMATCH",
            "dumpbin did not identify the ABI probe as AMD64 PE-COFF",
            "Use the qualified x64 MSVC linker and Rust target.",
        ));
    }
    let mut imports = text
        .lines()
        .filter_map(|line| {
            let value = line.trim();
            if value.len() > 4
                && value.to_ascii_lowercase().ends_with(".dll")
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            {
                Some(value.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    imports.sort();
    imports.dedup();
    if imports.is_empty() {
        return Err(XtaskError::gate(
            "P1A_PE_IMPORTS_MISSING",
            "dumpbin reported no imported DLLs for the ABI probe",
            "Inspect the qualified binary audit output.",
        ));
    }
    Ok(imports)
}

fn matching_items(imports: &[String], lower: &[String], tokens: &[&str]) -> Vec<String> {
    imports
        .iter()
        .zip(lower)
        .filter(|(_, normalized)| tokens.iter().any(|token| normalized.contains(token)))
        .map(|(original, _)| original.clone())
        .collect()
}

fn target_abi(include_runtime: bool) -> Value {
    let mut value = json!({
        "rust_target": "x86_64-pc-windows-msvc", "object_format": "PE-COFF",
        "machine": "AMD64", "calling_convention": "C", "pointer_width": 64,
        "endianness": "little"
    });
    if include_runtime {
        value["c_runtime_linkage"] = json!("dynamic_release");
    }
    value
}

fn cargo_environment(
    host: &PrototypeWindowsHostReport,
    rust: &RustToolchain,
    target_directory: Option<&Path>,
) -> Result<BTreeMap<String, Option<OsString>>> {
    let mut environment = native_build_environment(host)?;
    let mut path_entries = vec![
        rust.rustc
            .parent()
            .ok_or_else(|| {
                XtaskError::integrity("P1A_RUST_PATH_INVALID", "rustc has no parent directory")
            })?
            .to_path_buf(),
    ];
    if let Some(existing) = environment.get("PATH").and_then(Option::as_ref) {
        path_entries.extend(std::env::split_paths(existing));
    }
    environment.insert(
        "PATH".to_owned(),
        Some(std::env::join_paths(path_entries).map_err(|error| {
            XtaskError::environment(
                "P1A_CARGO_PATH_INVALID",
                format!("could not construct the closed Cargo PATH: {error}"),
            )
        })?),
    );
    environment.insert("CARGO_NET_OFFLINE".to_owned(), Some(OsString::from("true")));
    environment.insert("RUSTC".to_owned(), Some(rust.rustc.as_os_str().to_owned()));
    environment.insert("RUSTDOC".to_owned(), None);
    environment.insert("RUSTUP_TOOLCHAIN".to_owned(), None);
    if let Some(target) = target_directory {
        environment.insert(
            "CARGO_TARGET_DIR".to_owned(),
            Some(target.as_os_str().to_owned()),
        );
    } else {
        environment.insert("CARGO_TARGET_DIR".to_owned(), None);
    }
    for name in [
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CC",
        "CXX",
        "AR",
        "CFLAGS",
        "CXXFLAGS",
        "LDFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_BUILD_RUSTC_WRAPPER",
    ] {
        environment.insert(name.to_owned(), None);
    }
    Ok(environment)
}

fn audit_cpu_graph(
    repository: &Path,
    recorder: &mut P1aRecorder,
    work_root: &Path,
    host: &PrototypeWindowsHostReport,
    rust: &RustToolchain,
) -> Result<GraphAudit> {
    let args = [
        "tree",
        "--locked",
        "--offline",
        "--no-default-features",
        "--features",
        "cpu-reference",
        "--edges",
        "normal,build,dev",
        "--prefix",
        "none",
    ];
    let output = recorder.run_audited(
        repository,
        work_root,
        &rust.cargo,
        args.iter().map(OsString::from).collect(),
        std::iter::once("cargo".to_owned())
            .chain(args.iter().map(|value| (*value).to_owned()))
            .collect(),
        "${REPO}",
        cargo_environment(host, rust, None)?,
        Duration::from_secs(300),
        vec![host.visual_studio.msvc_tools_root.clone()],
        "cargo_offline_enforced",
    )?;
    require_audited_pass(&output, "P1A_CPU_GRAPH_AUDIT_FAILED")?;
    let text = std::str::from_utf8(&output.stdout).map_err(|_| {
        XtaskError::integrity(
            "P1A_CPU_GRAPH_INVALID",
            "activated Cargo graph output is not UTF-8",
        )
    })?;
    let mut packages = parse_cargo_tree_packages(text)?;
    packages.sort();
    packages.dedup();
    let lock = cargo_lock_inventory(&repository.join("Cargo.lock"))?;
    audit_active_packages_against_lock(&packages, &lock)?;
    for required in ["tree-sitter@0.25.8", ADMITTED_PYTHON_SOURCE_PACKAGE] {
        if packages.binary_search(&required.to_owned()).is_err() {
            return Err(XtaskError::gate(
                "P1A_TREE_SITTER_BOUNDARY_MISSING",
                format!("activated CPU graph does not contain pinned {required}"),
                "Restore the pinned generated-C parser and C runtime dependencies.",
            ));
        }
    }
    Ok(GraphAudit {
        activated_packages: packages,
    })
}

fn parse_cargo_tree_packages(text: &str) -> Result<Vec<String>> {
    let mut packages = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let name = fields.next().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_CPU_GRAPH_INVALID",
                "activated Cargo graph contains an empty package line",
            )
        })?;
        let version = fields.next().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_CPU_GRAPH_INVALID",
                "activated Cargo graph package line omitted its version",
            )
        })?;
        let version = version.strip_prefix('v').ok_or_else(|| {
            XtaskError::integrity(
                "P1A_CPU_GRAPH_INVALID",
                "activated Cargo graph was not emitted with the exact prefix-free package format",
            )
        })?;
        if name.is_empty()
            || version.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
        {
            return Err(XtaskError::integrity(
                "P1A_CPU_GRAPH_INVALID",
                "activated Cargo graph contains a nonportable package identity",
            ));
        }
        packages.push(format!("{name}@{version}"));
    }
    if packages.is_empty()
        || packages.first().map(String::as_str) != Some("rust-llm-pretrain@0.1.0")
    {
        return Err(XtaskError::integrity(
            "P1A_CPU_GRAPH_INVALID",
            "activated Cargo graph is empty or does not begin with the qualified root package",
        ));
    }
    Ok(packages)
}

fn run_quality_gate(
    repository: &Path,
    recorder: &mut P1aRecorder,
    work_root: &Path,
    host: &PrototypeWindowsHostReport,
    rust: &RustToolchain,
) -> Result<QualityGateResult> {
    let commands: &[&[&str]] = &[
        &["fmt", "--all", "--", "--check"],
        &[
            "clippy",
            "--locked",
            "--all-targets",
            "--features",
            "cpu-reference",
            "--",
            "-D",
            "warnings",
        ],
        &["test", "--locked", "--features", "cpu-reference"],
    ];
    let mut result = Vec::new();
    let mut target_scans = Vec::new();
    for (index, args) in commands.iter().enumerate() {
        let target = work_root.join(format!("quality-target-{}", index + 1));
        if target.exists() {
            return Err(XtaskError::integrity(
                "P1A_QUALITY_TARGET_NOT_FRESH",
                "quality command target directory existed before its command",
            ));
        }
        let output = recorder.run_audited(
            repository,
            work_root,
            &rust.cargo,
            args.iter().map(OsString::from).collect(),
            std::iter::once("cargo".to_owned())
                .chain(args.iter().map(|value| (*value).to_owned()))
                .collect(),
            "${REPO}",
            cargo_environment(host, rust, Some(&target))?,
            Duration::from_secs(30 * 60),
            vec![host.visual_studio.msvc_tools_root.clone()],
            "cargo_offline_enforced",
        )?;
        require_audited_pass(&output, "P1A_QUALITY_GATE_FAILED")?;
        target_scans.push(crate::p1a_artifacts::scan_target(&target, args[0])?);
        if target.exists() {
            publication::require_no_follow_tree(&target)?;
            fs::remove_dir_all(&target).io_context(
                "P1A_QUALITY_TARGET_CLEANUP_FAILED",
                "could not remove an isolated quality target directory",
            )?;
        }
        result.push(json!({
            "argv": std::iter::once("cargo")
                .chain(args.iter().copied())
                .collect::<Vec<_>>(),
            "exit_code": 0
        }));
    }
    Ok(QualityGateResult {
        commands: result,
        target_scans,
    })
}

fn require_tool_paths_contained(host: &PrototypeWindowsHostReport) -> Result<()> {
    let vs_root = fs::canonicalize(&host.visual_studio.installation_path).io_context(
        "P1A_VS_PATH_INVALID",
        "could not canonicalize the selected Visual Studio root",
    )?;
    let kits_root = fs::canonicalize(&host.windows_sdk.kits_root).io_context(
        "P1A_SDK_PATH_INVALID",
        "could not canonicalize the selected Windows Kits root",
    )?;
    for path in [
        &host.visual_studio.cl.path,
        &host.visual_studio.link.path,
        &host.visual_studio.lib.path,
        &host.visual_studio.dumpbin.path,
        &host.visual_studio.c_frontend.path,
        &host.visual_studio.cpp_frontend.path,
        &host.visual_studio.code_generator.path,
        &host.visual_studio.msvc_include,
        &host.visual_studio.msvc_x64_lib,
        &host.visual_studio.vcruntime_lib.path,
        &host.visual_studio.vcruntime_dll.path,
    ] {
        let canonical = fs::canonicalize(path).io_context(
            "P1A_VS_PATH_INVALID",
            format!("could not canonicalize {}", path.display()),
        )?;
        if !canonical.starts_with(&vs_root) {
            return Err(XtaskError::integrity(
                "P1A_VS_PATH_ESCAPED",
                format!(
                    "qualified Visual Studio path escaped its installation: {}",
                    path.display()
                ),
            ));
        }
    }
    for path in [
        &host.windows_sdk.windows_header.path,
        &host.windows_sdk.ucrt_header.path,
        &host.windows_sdk.kernel32_lib.path,
        &host.windows_sdk.ucrt_lib.path,
        &host.windows_sdk.rc.path,
        &host.windows_sdk.mt.path,
    ] {
        let canonical = fs::canonicalize(path).io_context(
            "P1A_SDK_PATH_INVALID",
            format!("could not canonicalize {}", path.display()),
        )?;
        if !canonical.starts_with(&kits_root) {
            return Err(XtaskError::integrity(
                "P1A_SDK_PATH_ESCAPED",
                format!(
                    "qualified Windows SDK path escaped its root: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn path_with_token(path: &Path, roots: &[(&Path, &str)]) -> Result<String> {
    let canonical = fs::canonicalize(path).io_context(
        "P1A_PATH_TOKENIZATION_FAILED",
        format!("could not canonicalize {}", path.display()),
    )?;
    for (root, token) in roots {
        let canonical_root = fs::canonicalize(root).io_context(
            "P1A_PATH_TOKENIZATION_FAILED",
            format!("could not canonicalize {}", root.display()),
        )?;
        if let Ok(relative) = canonical.strip_prefix(&canonical_root) {
            let components = relative
                .components()
                .filter_map(|component| match component {
                    Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if components.iter().any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b" ._()+-".contains(&byte))
            }) {
                return Err(XtaskError::integrity(
                    "P1A_PATH_TOKENIZATION_FAILED",
                    "tool path has a component outside the closed tokenized-path alphabet",
                ));
            }
            return if components.is_empty() {
                Ok((*token).to_owned())
            } else {
                Ok(format!("{token}/{}", components.join("/")))
            };
        }
    }
    Err(XtaskError::integrity(
        "P1A_PATH_TOKENIZATION_FAILED",
        format!("path is outside every qualified root: {}", path.display()),
    ))
}

fn tool_value(
    logical_name: &str,
    identity: &crate::p1a_windows::ToolFileIdentity,
    version: &str,
    invocation_mode: &str,
    roots: &[(&Path, &str)],
) -> Result<Value> {
    Ok(json!({
        "logical_name": logical_name,
        "path": path_with_token(&identity.path, roots)?,
        "version": version,
        "sha256": identity.sha256,
        "bytes": identity.bytes,
        "invocation_mode": invocation_mode
    }))
}

fn plain_file_identity_value(
    logical_name: &str,
    identity: &crate::p1a_windows::ToolFileIdentity,
    roots: &[(&Path, &str)],
) -> Result<Value> {
    Ok(json!({
        "logical_name": logical_name,
        "path": path_with_token(&identity.path, roots)?,
        "version": identity.file_version,
        "sha256": identity.sha256,
        "bytes": identity.bytes
    }))
}

fn system32() -> Result<PathBuf> {
    Ok(crate::p1a_windows::loader_resolved_system_runtime()?.system_directory)
}

#[allow(clippy::too_many_arguments)]
fn build_host_environment(
    host: &PrototypeWindowsHostReport,
    rust: &RustToolchain,
    quality: &QualityGateResult,
    graph: &GraphAudit,
    before_manifest: &str,
    after_manifest: &str,
    toolchain_stability: (
        &crate::p1a_windows::HostToolchainStabilitySnapshot,
        &crate::p1a_windows::HostToolchainStabilitySnapshot,
    ),
    cargo_cache: &OwnedCargoCacheSnapshot,
) -> Result<Value> {
    let (toolchain_before, toolchain_after) = toolchain_stability;
    let vs = &host.visual_studio;
    let sdk = &host.windows_sdk;
    let system32 = host.system_runtime.system_directory.clone();
    let (program_files, program_files_x86) = crate::p1a_windows::native_program_files_roots()?;
    let tool_roots = [
        (rust.rustup_home.as_path(), "${RUSTUP_HOME}"),
        (rust.cargo_home.as_path(), "${CARGO_HOME}"),
        (vs.installation_path.as_path(), "${VS_INSTALL}"),
        (vs.msvc_tools_root.as_path(), "${VC_TOOLS}"),
        (sdk.kits_root.as_path(), "${WINDOWS_KITS}"),
        (system32.as_path(), "${SYSTEM32}"),
        (program_files.as_path(), "${PROGRAM_FILES}"),
        (program_files_x86.as_path(), "${PROGRAM_FILES_X86}"),
    ];
    let selected = vs
        .candidates
        .iter()
        .find(|candidate| candidate.instance_id == vs.selected_instance_id)
        .ok_or_else(|| {
            XtaskError::integrity("P1A_VS_SELECTION_INVALID", "selected VS candidate vanished")
        })?;
    let rustc = tool_value(
        "rustc",
        &rust.rustc_identity,
        &rust.rustc_version,
        "native",
        &tool_roots,
    )?;
    let cargo = tool_value(
        "cargo",
        &rust.cargo_identity,
        &rust.cargo_version,
        "native",
        &tool_roots,
    )?;
    let c_compiler = tool_value(
        "msvc_c_compiler",
        &vs.cl,
        &vs.cl.file_version,
        "c17",
        &tool_roots,
    )?;
    let cpp_compiler = tool_value(
        "msvc_cpp_compiler",
        &vs.cl,
        &vs.cl.file_version,
        "c++20",
        &tool_roots,
    )?;
    let linker = tool_value(
        "msvc_linker",
        &vs.link,
        &vs.link.file_version,
        "native",
        &tool_roots,
    )?;
    let archiver = tool_value(
        "msvc_archiver",
        &vs.lib,
        &vs.lib.file_version,
        "native",
        &tool_roots,
    )?;
    let inspector = tool_value(
        "msvc_binary_inspector",
        &vs.dumpbin,
        &vs.dumpbin.file_version,
        "native",
        &tool_roots,
    )?;
    let vswhere = tool_value(
        "vswhere",
        &vs.vswhere,
        &vs.vswhere.file_version,
        "native",
        &tool_roots,
    )?;
    let c_frontend = plain_file_identity_value("msvc_c_frontend_c1", &vs.c_frontend, &tool_roots)?;
    let cpp_frontend =
        plain_file_identity_value("msvc_cpp_frontend_c1xx", &vs.cpp_frontend, &tool_roots)?;
    let optimizer_codegen =
        plain_file_identity_value("msvc_optimizer_codegen_c2", &vs.code_generator, &tool_roots)?;
    let rc = tool_value(
        "windows_resource_compiler",
        &sdk.rc,
        &sdk.rc.file_version,
        "native",
        &tool_roots,
    )?;
    let mt = tool_value(
        "windows_manifest_tool",
        &sdk.mt,
        &sdk.mt.file_version,
        "native",
        &tool_roots,
    )?;
    let ucrtbase_value = plain_file_identity_value(
        "loader_resolved_ucrtbase",
        &host.system_runtime.ucrtbase,
        &tool_roots,
    )?;
    let vcruntime_value = plain_file_identity_value(
        "loader_resolved_vcruntime",
        &host.system_runtime.vcruntime,
        &tool_roots,
    )?;
    let compiler_redist_vcruntime =
        plain_file_identity_value("compiler_redist_vcruntime", &vs.vcruntime_dll, &tool_roots)?;
    let activated_graph_digest = hash::bytes(graph.activated_packages.join("\n").as_bytes());
    let core_topology = crate::p1a_windows::canonical_core_topology(&host.topology)?;
    let logical_processor_union_mask = format!(
        "0x{:016X}",
        crate::p1a_windows::processor_group_union_mask(&host.topology, 0)?
    );
    let core_topology_sha256 = crate::p1a_windows::processor_topology_sha256(&host.topology)?;
    Ok(json!({
        "schema": "python-slm-p1a-host-environment-v1", "phase_id": PHASE_ID,
        "interface_id": INTERFACE_ID, "profile_id": PROFILE_ID,
        "support_tier": SUPPORT_TIER, "qualification_scope": QUALIFICATION_SCOPE,
        "qualification_tuple": qualification_tuple(), "status": "PASS",
        "result": {
            "kind": "complete",
            "operating_system": {
                "family": "windows", "architecture": "x86_64",
                "native_architecture": "AMD64",
                "version": format!("{}.{}.{}", host.os.major, host.os.minor, host.os.build),
                "build": host.os.build,
                "service_pack_major": host.os.service_pack_major,
                "service_pack_minor": host.os.service_pack_minor,
                "product_type": host.os.product_type,
                "native_windows_process": host.architecture.process_is_native
            },
            "cpu": {
                "vendor_id": host.cpu.vendor, "normalized_model": host.cpu.brand,
                "family": host.cpu.family, "model": host.cpu.model, "stepping": host.cpu.stepping,
                "physical_packages": host.topology.package_count,
                "physical_cores": host.topology.physical_core_count,
                "logical_processors": host.topology.active_logical_processors,
                "processor_groups": host.topology.active_group_count,
                "logical_processor_union_mask": logical_processor_union_mask,
                "core_topology": core_topology,
                "core_topology_sha256": core_topology_sha256,
                "instruction_sets": {
                    "sse2": host.isa.sse2, "sse3": host.isa.sse3,
                    "ssse3": host.isa.ssse3, "sse4_1": host.isa.sse41,
                    "sse4_2": host.isa.sse42, "popcnt": host.isa.popcnt,
                    "pclmulqdq": host.isa.pclmulqdq, "aes": host.isa.aes,
                    "avx": host.isa.avx_os_enabled, "avx2": host.isa.avx2_os_enabled,
                    "fma": host.isa.fma, "bmi1": host.isa.bmi1, "bmi2": host.isa.bmi2,
                    "avx512f": host.isa.avx512f_hardware && host.isa.avx512_os_enabled,
                    "avx512dq": host.isa.avx512dq && host.isa.avx512_os_enabled,
                    "avx512cd": host.isa.avx512cd && host.isa.avx512_os_enabled,
                    "avx512bw": host.isa.avx512bw && host.isa.avx512_os_enabled,
                    "avx512vl": host.isa.avx512vl && host.isa.avx512_os_enabled,
                    "sha": host.isa.sha, "vaes": host.isa.vaes,
                    "vpclmulqdq": host.isa.vpclmulqdq
                }
            },
            "rust_toolchain": {
                "rustc": rustc, "cargo": cargo, "release": rust.release,
                "release_semver": {"major": rust.release_major, "minor": rust.release_minor, "patch": rust.release_patch},
                "host": rust.host, "llvm_version": rust.llvm_version
            },
            "native_toolchain": {
                "visual_studio_discovery_method": vs.discovery_method,
                "vswhere": vswhere,
                "vswhere_query": vs.query,
                "visual_studio_candidates": vs.candidates.iter().map(|candidate| {
                    Ok(json!({
                        "instance_id": candidate.instance_id, "product_id": candidate.product_id,
                        "installation_version": candidate.installation_version,
                        "installation_path": path_with_token(&candidate.installation_path, &tool_roots)?,
                        "complete": candidate.complete, "launchable": candidate.launchable,
                        "reboot_required": candidate.reboot_required
                    }))
                }).collect::<Result<Vec<_>>>()?,
                "selected_visual_studio_instance_id": vs.selected_instance_id,
                "selected_visual_studio_product_id": vs.product_id,
                "visual_studio_installation_path": path_with_token(&vs.installation_path, &tool_roots)?,
                "visual_studio_edition": vs.product_id,
                "visual_studio_version": vs.installation_version,
                "visual_studio_complete": selected.complete,
                "visual_studio_launchable": selected.launchable,
                "msvc_tools_version": vs.msvc_tools_version,
                "msvc_runtime_redist_version": vs.vcruntime_redist_version,
                "c_compiler": c_compiler, "cpp_compiler": cpp_compiler,
                "c_frontend": c_frontend, "cpp_frontend": cpp_frontend,
                "optimizer_codegen": optimizer_codegen,
                "linker": linker, "archiver": archiver, "binary_inspector": inspector
            },
            "windows_sdk": {
                "version": sdk.version, "ucrt_version": sdk.ucrt_version,
                "resource_compiler": rc, "manifest_tool": mt,
                "include_tree_sha256": toolchain_before.windows_sdk_include_tree.sha256,
                "x64_lib_tree_sha256": toolchain_before.windows_sdk_x64_lib_tree.sha256
            },
            "runtime": {
                "loader_resolution_policy": "windows-system32-safe-search-v1",
                "ucrtbase": ucrtbase_value,
                "vcruntime": vcruntime_value,
                "compiler_redist_vcruntime": compiler_redist_vcruntime,
                "debug_runtime_linked": false
            },
            "toolchain_identity_stability": {
                "algorithm": "sha256-qualified-files-and-selected-trees-v1",
                "before": toolchain_before,
                "after": toolchain_after,
                "stable": true
            },
            "target_abi": target_abi(true),
            "cargo_build_policy": {
                "locked": true, "cargo_offline": true, "fresh_target_directory": true,
                "enabled_features": ["cpu-reference"], "accelerator_features_enabled": [],
                "compiler_wrappers": [], "build_affecting_cargo_configs": [],
                "python_processes": [], "python_modules_or_packages": [], "python_linked_imports": [],
                "accelerator_tools_executed": [], "accelerator_artifacts": [], "accelerator_linked_imports": [],
                "native_ml_backend_imports": [],
                "cargo_tree_edges": ["normal", "build", "dev"],
                "activated_packages": graph.activated_packages,
                "activated_packages_sha256": activated_graph_digest,
                "owned_cargo_cache": cargo_cache,
                "quality_target_scans": quality.target_scans,
                "permitted_generated_c_boundaries": [
                    "tree-sitter-python@0.25.0-generated-c-parser", "tree-sitter@0.25.8-c-runtime"
                ]
            },
            "quality_gate": {"status": "PASS", "commands": quality.commands},
            "input_stability": {"before_sha256": before_manifest, "after_sha256": after_manifest, "stable": true},
            "cleanup": {
                "owned_temp_root_removed": true,
                "fresh_target_removed": true,
                "process_job_empty": true,
                "persistent_state_mutation_requested": false,
                "persistent_environment_redirected": true,
                "persistent_environment_removed": true
            },
            "deferred_adapters": [
                {"adapter": "linux-host", "status": "DEFERRED_POST_P16", "probed": false},
                {"adapter": "macos-host", "status": "DEFERRED_POST_P16", "probed": false},
                {"adapter": "gcc", "status": "DEFERRED_POST_P16", "probed": false},
                {"adapter": "clang", "status": "DEFERRED_POST_P16", "probed": false},
                {"adapter": "apple-clang", "status": "DEFERRED_POST_P16", "probed": false}
            ]
        },
        "errors": []
    }))
}

fn isolation_snapshot(host: &PrototypeWindowsHostReport, before: bool) -> Result<Value> {
    let affinity = if before {
        &host.isolation.affinity_before
    } else {
        &host.isolation.affinity_after
    };
    Ok(json!({
        "processor_group_count": host.topology.active_group_count,
        "active_logical_processors": host.topology.active_logical_processors,
        "logical_processor_union_mask": format!(
            "0x{:016X}",
            crate::p1a_windows::processor_group_union_mask(&host.topology, 0)?
        ),
        "core_topology_sha256": crate::p1a_windows::processor_topology_sha256(&host.topology)?,
        "process_group": affinity.thread_group,
        "thread_group_mask": format!("0x{:X}", affinity.thread_group_mask),
        "process_affinity_mask": format!("0x{:X}", affinity.process_mask),
        "system_affinity_mask": format!("0x{:X}", affinity.system_mask),
        "selected_logical_processor_count": affinity.process_mask.count_ones(),
        "power_scheme_guid": host.power_policy.active_scheme_guid,
        "power_scheme_name": host.power_policy.active_scheme_name,
        "ac_line_status": host.power_policy.ac_line_status,
        "power_value_source": host.power_policy.value_source,
        "clock_policy": {
            "minimum_processor_percent_ac": host.power_policy.processor_minimum_percent,
            "maximum_processor_percent_ac": host.power_policy.processor_maximum_percent,
            "boost_mode_ac": host.power_policy.processor_boost_mode,
            "energy_performance_preference_ac": host.power_policy.energy_performance_preference
        }
    }))
}

fn build_cpu_isolation(host: &PrototypeWindowsHostReport) -> Result<Value> {
    let competing = host
        .isolation
        .foreign_process_loads
        .iter()
        .filter_map(|load| {
            let reason = if crate::p1a_windows::is_competing_foreign_load(load) {
                Some("cpu_fraction_exceeded")
            } else {
                None
            };
            reason.map(|reason| {
                json!({
                    "process_id": load.process_id,
                    "creation_time_100ns": load.creation_time_100ns,
                    "image_name": load.image_name,
                    "reason": reason
                })
            })
        })
        .collect::<Vec<_>>();
    if !host.isolation.passed || !competing.is_empty() {
        return Err(XtaskError::gate(
            "P1A_CPU_ISOLATION_FAILED",
            "the frozen CPU isolation policy did not pass",
            "Stop competing compute processes and rerun from an unconstrained host shell.",
        ));
    }
    Ok(json!({
        "schema": "python-slm-p1a-cpu-isolation-v1", "phase_id": PHASE_ID,
        "interface_id": INTERFACE_ID, "profile_id": PROFILE_ID, "support_tier": SUPPORT_TIER,
        "status": "PASS",
        "policy": {
            "policy_id": "p1a-cpu-isolation-policy-v1", "sample_duration_ms": 2000,
            "minimum_system_idle_fraction": 0.5,
            "maximum_foreign_process_single_core_fraction": 0.5,
            "approved_process_basis": "verifier_process_ancestry_and_contained_descendants_only",
            "selected_processor_group": host.isolation.affinity_before.thread_group,
            "selected_affinity_mask": format!("0x{:X}", host.isolation.affinity_before.process_mask),
            "approved_identity_basis": "native_pid_creation_time_ancestry_plus_windows_job_containment",
            "competing_compute_names": crate::p1a_windows::COMPETING_COMPUTE_NAMES,
            "require_topology_stability": true, "require_affinity_stability": true,
            "require_power_policy_stability": true
        },
        "result": {
            "kind": "complete", "before": isolation_snapshot(host, true)?,
            "after": isolation_snapshot(host, false)?,
            "baseline": {
                "sample_duration_ms": 2000,
                "actual_elapsed_ns": host.isolation.actual_elapsed_nanoseconds,
                "logical_processor_capacity": host.isolation.logical_processor_capacity,
                "system_idle_delta_100ns": host.isolation.system_idle_delta_100ns,
                "system_kernel_delta_100ns": host.isolation.system_kernel_delta_100ns,
                "system_user_delta_100ns": host.isolation.system_user_delta_100ns,
                "system_busy_basis_points": host.isolation.system_busy_basis_points,
                "system_idle_fraction": f64::from(10_000 - host.isolation.system_busy_basis_points) / 10_000.0,
                "largest_unapproved_process_single_core_basis_points": host.isolation.largest_unapproved_process_single_core_basis_points,
                "maximum_foreign_process_single_core_fraction": f64::from(host.isolation.largest_unapproved_process_single_core_basis_points) / 10_000.0,
                "ordinary_os_activity_process_count": host.isolation.ordinary_os_activity_process_count,
                "ordinary_os_activity_total_cpu_100ns": host.isolation.ordinary_os_activity_total_cpu_100ns,
                "approved_verifier_cpu_100ns": host.isolation.approved_verifier_cpu_100ns,
                "inaccessible_processes_at_start": host.isolation.inaccessible_processes_at_start,
                "inaccessible_processes_at_end": host.isolation.inaccessible_processes_at_end,
                "new_processes": host.isolation.new_processes,
                "ended_processes": host.isolation.ended_processes,
                "approved_process_basis": "verifier_process_ancestry_and_contained_descendants_only",
                "foreign_process_loads": host.isolation.foreign_process_loads.iter().map(|load| json!({
                    "process_id": load.process_id,
                    "creation_time_100ns": load.creation_time_100ns,
                    "image_name": load.image_name,
                    "cpu_time_100ns": load.cpu_time_100ns,
                    "single_core_basis_points": load.single_core_basis_points,
                    "single_core_fraction": f64::from(load.single_core_basis_points) / 10_000.0,
                    "approved": load.approved, "known_compute_name": load.known_compute_name
                })).collect::<Vec<_>>(),
                "competing_workloads": competing
            },
            "topology_stable": host.isolation.topology_stable,
            "affinity_stable": host.isolation.affinity_stable,
            "power_policy_stable": host.isolation.power_policy_stable
        },
        "errors": []
    }))
}

fn cleanup_owned_work(work_root: &Path) -> Result<()> {
    if !work_root.exists() {
        return Ok(());
    }
    let name = work_root.file_name().and_then(OsStr::to_str);
    let parent_name = work_root
        .parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str);
    if name != Some("private-work")
        || work_root
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            != Some(OsStr::new(".staging"))
        || parent_name
            .map(parse_stage_container_name)
            .transpose()?
            .is_none()
    {
        return Err(XtaskError::integrity(
            "P1A_TEMP_OWNERSHIP_INVALID",
            "refused to remove a directory outside the exact P1A private-work shape",
        ));
    }
    publication::require_no_follow_tree(work_root)?;
    fs::remove_dir_all(work_root).io_context(
        "P1A_TEMP_CLEANUP_FAILED",
        format!("could not remove {}", work_root.display()),
    )
}

fn artifact_values(
    values: Option<&ArtifactValues>,
    admission: &Admission,
) -> Vec<(&'static str, Value)> {
    match values {
        Some(values) => vec![
            ("artifacts/cpu-isolation.json", values.cpu_isolation.clone()),
            (
                "artifacts/host-environment.json",
                values.host_environment.clone(),
            ),
            (
                "artifacts/native-abi-probe.json",
                values.native_probe.clone(),
            ),
            (
                "artifacts/p0a-dependency.json",
                values.p0a_dependency.clone(),
            ),
            ("artifacts/schema-bundle.json", values.schema_bundle.clone()),
            (
                "artifacts/source-identity.json",
                values.source_identity.clone(),
            ),
        ],
        None => vec![
            (
                "artifacts/p0a-dependency.json",
                serde_json::to_value(&admission.p0a_dependency).expect("dependency serializes"),
            ),
            (
                "artifacts/schema-bundle.json",
                serde_json::to_value(&admission.schema_bundle).expect("bundle serializes"),
            ),
            (
                "artifacts/source-identity.json",
                serde_json::to_value(&admission.source).expect("source serializes"),
            ),
        ],
    }
}

fn validate_planned_p0a_source_binding(planned: &[(&str, Value)]) -> Result<()> {
    let artifact = |path: &str| {
        planned
            .iter()
            .find_map(|(candidate, value)| (*candidate == path).then_some(value))
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_P0A_SOURCE_RELATION_INVALID",
                    format!("terminal artifact plan omits {path}"),
                )
            })
    };
    let source = artifact("artifacts/source-identity.json")?;
    let dependency = artifact("artifacts/p0a-dependency.json")?;
    let source_commit = source.pointer("/commit").and_then(Value::as_str);
    let verified_commit = dependency
        .pointer("/verified_at_source_commit")
        .and_then(Value::as_str);
    if source_commit != verified_commit || !source_commit.is_some_and(is_lower_git_sha) {
        return Err(XtaskError::integrity(
            "P1A_P0A_SOURCE_RELATION_INVALID",
            "P0A verification source commit differs from the P1A source identity",
        ));
    }
    Ok(())
}

fn schema_path_for_artifact(relative: &str) -> Result<&'static str> {
    match relative {
        "artifacts/source-identity.json" => {
            Ok("docs/schemas/P1A-prototype-v2/python-slm-p1a-source-identity-v1.schema.json")
        }
        "artifacts/p0a-dependency.json" => {
            Ok("docs/schemas/P1A-prototype-v2/python-slm-p0a-dependency-v1.schema.json")
        }
        "artifacts/schema-bundle.json" => {
            Ok("docs/schemas/P1A-prototype-v2/python-slm-p1a-schema-bundle-v1.schema.json")
        }
        "artifacts/host-environment.json" => {
            Ok("docs/schemas/P1A-prototype-v2/python-slm-p1a-host-environment-v1.schema.json")
        }
        "artifacts/cpu-isolation.json" => {
            Ok("docs/schemas/P1A-prototype-v2/python-slm-p1a-cpu-isolation-v1.schema.json")
        }
        "artifacts/native-abi-probe.json" => {
            Ok("docs/schemas/P1A-prototype-v2/python-slm-p1a-native-abi-probe-v1.schema.json")
        }
        _ => Err(XtaskError::new(
            "P1A_ARTIFACT_PATH_INTERNAL_INVALID",
            Category::Internal,
            format!("unknown P1A artifact path {relative}"),
            "Inspect the fixed P1A artifact plan.",
        )),
    }
}

fn read_live_schema(repository: &Path, relative: &str) -> Result<Value> {
    let bytes = fs::read(repository.join(relative)).io_context(
        "P1A_SCHEMA_READ_FAILED",
        format!("could not read {relative}"),
    )?;
    serde_json::from_slice(&bytes).map_err(|error| {
        XtaskError::integrity(
            "P1A_SCHEMA_JSON_INVALID",
            format!("{relative} is not valid JSON: {error}"),
        )
    })
}

fn validate_live_schema(
    repository: &Path,
    schema_relative: &str,
    value: &Value,
    code: &'static str,
) -> Result<()> {
    let schema = read_live_schema(repository, schema_relative)?;
    json_schema::validate(&schema, value, code)
}

fn validate_artifact_root(repository: &Path, relative: &str, value: &Value) -> Result<()> {
    let expected: &[&str] = match relative {
        "artifacts/source-identity.json" => &[
            "schema",
            "phase_id",
            "interface_id",
            "profile_id",
            "support_tier",
            "commit",
            "tree",
            "branch",
            "dirty",
            "cargo_lock_sha256",
            "verifier_source_sha256",
            "schema_bundle_sha256",
            "p0a_pointer_sha256",
        ],
        "artifacts/p0a-dependency.json" => &[
            "schema",
            "phase_id",
            "interface_id",
            "profile_id",
            "status",
            "pointer_path",
            "pointer_sha256",
            "acceptance_path",
            "acceptance_sha256",
            "acceptance_sequence",
            "run_id",
            "run_evidence_sha256",
            "seal_sha256",
            "preapproval_commit",
            "receipt_commit",
            "approval_commit",
            "publication_commit",
            "closure_commit",
            "technical_approval_sha256",
            "data_governance_approval_sha256",
            "verified_at_source_commit",
        ],
        "artifacts/schema-bundle.json" => &[
            "schema",
            "phase_id",
            "interface_id",
            "profile_id",
            "entries",
            "bundle_sha256",
        ],
        "artifacts/host-environment.json" => &[
            "schema",
            "phase_id",
            "interface_id",
            "profile_id",
            "support_tier",
            "qualification_scope",
            "qualification_tuple",
            "status",
            "result",
            "errors",
        ],
        "artifacts/cpu-isolation.json" => &[
            "schema",
            "phase_id",
            "interface_id",
            "profile_id",
            "support_tier",
            "status",
            "policy",
            "result",
            "errors",
        ],
        "artifacts/native-abi-probe.json" => &[
            "schema",
            "phase_id",
            "interface_id",
            "profile_id",
            "support_tier",
            "status",
            "result",
            "errors",
        ],
        _ => {
            return Err(XtaskError::new(
                "P1A_ARTIFACT_PATH_INTERNAL_INVALID",
                Category::Internal,
                format!("unknown P1A artifact path {relative}"),
                "Inspect the fixed P1A artifact plan.",
            ));
        }
    };
    let object = value.as_object().ok_or_else(|| {
        XtaskError::integrity(
            "P1A_ARTIFACT_SHAPE_INVALID",
            format!("{relative} is not an object"),
        )
    })?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(XtaskError::integrity(
            "P1A_ARTIFACT_SHAPE_INVALID",
            format!("{relative} does not have the exact closed root keys"),
        ));
    }
    validate_live_schema(
        repository,
        schema_path_for_artifact(relative)?,
        value,
        "P1A_ARTIFACT_SCHEMA_INVALID",
    )
}

fn expected_command_plan(source_commit: &str) -> Vec<Vec<String>> {
    let command = |values: &[&str]| values.iter().map(|value| (*value).to_owned()).collect();
    let mut plan = vec![
        command(&[
            "${GIT}",
            "merge-base",
            "--is-ancestor",
            HISTORICAL_P1A_BASELINE,
            "HEAD",
        ]),
        command(&["${GIT}", "rev-parse", "HEAD:docs/receipts/P1A"]),
        command(&[
            "${GIT}",
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            "docs/receipts/P1A",
        ]),
        command(&[
            "${GIT}",
            "ls-tree",
            "-r",
            "--name-only",
            "HEAD",
            "--",
            "docs/receipts/P1A",
        ]),
        command(&["${GIT}", "rev-parse", "HEAD"]),
        command(&["${GIT}", "rev-parse", "HEAD^{tree}"]),
        command(&["${GIT}", "symbolic-ref", "--quiet", "--short", "HEAD"]),
        command(&[
            "${GIT}",
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
            ":(exclude)docs/receipts/P1A-prototype-v2",
        ]),
        command(&[
            "${GIT}",
            "ls-files",
            "-z",
            "--",
            ".",
            ":(exclude)docs/receipts/P1A-prototype-v2",
        ]),
        command(&[
            "${GIT}",
            "merge-base",
            "--is-ancestor",
            "203b3580005ce59228c319d517134f910759c7bc",
            source_commit,
        ]),
        command(&[
            "${GIT}",
            "show",
            &format!("{source_commit}:docs/receipts/P0A/evidence.json"),
        ]),
        command(&[
            "${GIT}",
            "show",
            &format!("{source_commit}:docs/receipts/P0A/acceptances/00000001.json"),
        ]),
    ];
    plan.push(
        std::iter::once("${VSWHERE}".to_owned())
            .chain(
                crate::p1a_windows::VSWHERE_ARGS
                    .iter()
                    .map(|value| (*value).to_owned()),
            )
            .collect(),
    );
    plan.extend([
        command(&["${RUSTC}", "-vV"]),
        command(&["${CARGO}", "-Vv"]),
        command(&[
            "cargo",
            "tree",
            "--locked",
            "--offline",
            "--no-default-features",
            "--features",
            "cpu-reference",
            "--edges",
            "normal,build,dev",
            "--prefix",
            "none",
        ]),
        command(&[
            "${CL}",
            "/nologo",
            "/TC",
            "/std:c17",
            "/W4",
            "/WX",
            "/MD",
            "/c",
            "${P1A_TEMP}/sources/p1a_abi.c",
            "/Fo${P1A_TEMP}/native/p1a_c.obj",
        ]),
        command(&[
            "${CL}",
            "/nologo",
            "/TP",
            "/std:c++20",
            "/EHsc",
            "/W4",
            "/WX",
            "/MD",
            "/c",
            "${P1A_TEMP}/sources/p1a_abi.cpp",
            "/Fo${P1A_TEMP}/native/p1a_cpp.obj",
        ]),
        command(&[
            "${LIB}",
            "/nologo",
            "/OUT:${P1A_TEMP}/native/p1a_c.lib",
            "${P1A_TEMP}/native/p1a_c.obj",
        ]),
        command(&[
            "${LIB}",
            "/nologo",
            "/OUT:${P1A_TEMP}/native/p1a_cpp.lib",
            "${P1A_TEMP}/native/p1a_cpp.obj",
        ]),
        command(&[
            "${RUSTC}",
            "--edition=2024",
            "${P1A_TEMP}/sources/p1a_abi.rs",
            "--target=x86_64-pc-windows-msvc",
            "-C",
            "linker=${LINK}",
            "-L",
            "native=${P1A_TEMP}/native",
            "-l",
            "static=p1a_c",
            "-l",
            "static=p1a_cpp",
            "-o",
            "${P1A_TEMP}/rust-target/p1a_abi_probe.exe",
        ]),
        command(&["${P1A_TEMP}/rust-target/p1a_abi_probe.exe"]),
        command(&[
            "${DUMPBIN}",
            "/HEADERS",
            "/DEPENDENTS",
            "${P1A_TEMP}/rust-target/p1a_abi_probe.exe",
        ]),
        command(&[
            "${GIT}",
            "ls-files",
            "-z",
            "--",
            ".",
            ":(exclude)docs/receipts/P1A-prototype-v2",
        ]),
        command(&["cargo", "fmt", "--all", "--", "--check"]),
        command(&[
            "cargo",
            "clippy",
            "--locked",
            "--all-targets",
            "--features",
            "cpu-reference",
            "--",
            "-D",
            "warnings",
        ]),
        command(&["cargo", "test", "--locked", "--features", "cpu-reference"]),
        command(&[
            "${GIT}",
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
            ":(exclude)docs/receipts/P1A-prototype-v2",
        ]),
        command(&[
            "${GIT}",
            "ls-files",
            "-z",
            "--",
            ".",
            ":(exclude)docs/receipts/P1A-prototype-v2",
        ]),
    ]);
    plan
}

fn validate_captured_command_plan(
    recorder: &P1aRecorder,
    status: &str,
    source_commit: &str,
) -> Result<()> {
    let expected = expected_command_plan(source_commit);
    if recorder.commands.len() > expected.len()
        || (status == "PASS" && recorder.commands.len() != expected.len())
        || (status == "PASS" && recorder.commands.is_empty())
        || recorder
            .commands
            .iter()
            .zip(&expected)
            .any(|(actual, planned)| {
                let metadata = crate::p1a_receipt::expected_command_metadata(planned);
                actual.argv != *planned
                    || metadata.is_err()
                    || metadata.is_ok_and(|(kind, cwd, network)| {
                        actual.command_kind != kind
                            || actual.cwd != cwd
                            || actual.network_mode != network
                    })
            })
    {
        return Err(XtaskError::integrity(
            "P1A_COMMAND_PLAN_MISMATCH",
            "captured commands are not the exact consecutive prefix of the frozen P1A plan",
        ));
    }
    Ok(())
}

fn write_command_files(run_root: &Path, recorder: &P1aRecorder) -> Result<Vec<CommandEvidence>> {
    publication::create_dir(&run_root.join("commands"))?;
    let mut evidence = Vec::with_capacity(recorder.commands.len());
    for (index, command) in recorder.commands.iter().enumerate() {
        let expected_id = format!("C{:03}", index + 1);
        if command.id != expected_id {
            return Err(XtaskError::integrity(
                "P1A_COMMAND_SEQUENCE_INVALID",
                "captured command IDs are not unique consecutive C001 identifiers",
            ));
        }
        let stdout_path = format!("commands/{}.stdout.txt", command.id);
        let stderr_path = format!("commands/{}.stderr.txt", command.id);
        publication::write_new(&run_root.join(&stdout_path), &command.stdout)?;
        publication::write_new(&run_root.join(&stderr_path), &command.stderr)?;
        evidence.push(CommandEvidence {
            id: command.id.clone(),
            command_kind: command.command_kind.clone(),
            argv: command.argv.clone(),
            cwd: command.cwd.clone(),
            exit_code: command.exit_code,
            status: if command.exit_code == 0
                && command
                    .process_audit
                    .as_ref()
                    .is_some_and(process_audit_passed)
                && command.execution_error_code.is_none()
            {
                "PASS"
            } else {
                "FAIL"
            }
            .to_owned(),
            started_at_utc: command.started_at_utc.clone(),
            finished_at_utc: command.finished_at_utc.clone(),
            duration_ns: command.duration_ns,
            network_mode: command.network_mode.clone(),
            stdout: publication::file_ref_from(run_root, &stdout_path)?,
            stderr: publication::file_ref_from(run_root, &stderr_path)?,
            process_audit: command.process_audit.clone(),
            execution_error_code: command.execution_error_code.clone(),
        });
    }
    Ok(evidence)
}

fn sanitize_error(error: &XtaskError, repository: &Path, work_root: &Path) -> Value {
    let message = String::from_utf8_lossy(&redact_output(
        error.message.as_bytes(),
        repository,
        Some(work_root),
    ))
    .into_owned();
    let remediation = String::from_utf8_lossy(&redact_output(
        error.remediation.as_bytes(),
        repository,
        Some(work_root),
    ))
    .into_owned();
    json!({
        "code": error.code,
        "category": error.category,
        "message": message,
        "remediation": remediation
    })
}

fn validate_public_bytes(run_root: &Path) -> Result<()> {
    let mut stack = vec![run_root.to_path_buf()];
    let mut needles = vec![
        "C:\\".to_owned(),
        "\\\\?\\".to_owned(),
        "file://".to_owned(),
    ];
    for name in ["USERNAME", "USERPROFILE", "HOME", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            let value = value.to_string_lossy();
            if value.len() >= 3 {
                needles.push(value.into_owned());
            }
        }
    }
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).io_context(
            "P1A_REDACTION_SCAN_FAILED",
            "could not enumerate public receipt",
        )? {
            let entry =
                entry.io_context("P1A_REDACTION_SCAN_FAILED", "could not read receipt entry")?;
            let metadata = fs::symlink_metadata(entry.path()).io_context(
                "P1A_REDACTION_SCAN_FAILED",
                "could not inspect receipt entry",
            )?;
            if metadata.is_dir() {
                stack.push(entry.path());
            } else if metadata.is_file() {
                let bytes = fs::read(entry.path())
                    .io_context("P1A_REDACTION_SCAN_FAILED", "could not scan receipt file")?;
                let text = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
                if needles
                    .iter()
                    .any(|needle| text.contains(&needle.to_ascii_lowercase()))
                    || absolute_windows_path_start(&text).is_some()
                    || [
                        "aws_secret_access_key",
                        "github_token",
                        "authorization: bearer",
                    ]
                    .iter()
                    .any(|needle| text.contains(needle))
                {
                    return Err(XtaskError::integrity(
                        "P1A_PUBLIC_RECEIPT_LEAK_DETECTED",
                        "public P1A receipt contains a private path, username, temp path, or secret marker",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn emit_terminal_run(
    repository: &Path,
    admission: &Admission,
    work: &WorkPaths,
    status: &str,
    artifacts: Option<ArtifactValues>,
    errors: Vec<Value>,
) -> Result<EmittedRun> {
    if !matches!(status, "PASS" | "FAIL") || (status == "PASS") != errors.is_empty() {
        return Err(XtaskError::new(
            "P1A_TERMINAL_STATUS_INTERNAL_INVALID",
            Category::Internal,
            "terminal run status and error list are inconsistent",
            "Inspect the P1A terminal publication call.",
        ));
    }
    validate_captured_command_plan(&admission.recorder, status, &admission.source.commit)?;
    let planned_artifacts = artifact_values(artifacts.as_ref(), admission);
    validate_planned_p0a_source_binding(&planned_artifacts)?;
    if status == "PASS"
        && planned_artifacts
            .iter()
            .map(|(path, _)| *path)
            .collect::<Vec<_>>()
            != ARTIFACT_PATHS
    {
        return Err(XtaskError::new(
            "P1A_ARTIFACT_PLAN_INTERNAL_INVALID",
            Category::Internal,
            "PASS artifact plan differs from the fixed six-file contract",
            "Inspect the P1A artifact plan constants.",
        ));
    }
    publication::create_dir(&work.stage_run)?;
    publication::create_dir(&work.stage_run.join("artifacts"))?;
    let mut artifact_refs = BTreeMap::new();
    for (relative, value) in planned_artifacts {
        validate_artifact_root(repository, relative, &value)?;
        publication::write_json_new(&work.stage_run.join(relative), &value)?;
        artifact_refs.insert(
            relative.to_owned(),
            publication::file_ref_from(&work.stage_run, relative)?,
        );
    }
    let commands = write_command_files(&work.stage_run, &admission.recorder)?;
    let source_ref = artifact_refs
        .get("artifacts/source-identity.json")
        .cloned()
        .ok_or_else(|| {
            XtaskError::integrity("P1A_SOURCE_REF_MISSING", "source artifact is missing")
        })?;
    let dependency_ref = artifact_refs
        .get("artifacts/p0a-dependency.json")
        .cloned()
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_DEPENDENCY_REF_MISSING",
                "dependency artifact is missing",
            )
        })?;
    let command_count = commands.len();
    let entries = 1 + artifact_refs.len() + (2 * command_count);
    let evidence = json!({
        "schema": "python-slm-p1a-phase-evidence-v1",
        "phase_id": PHASE_ID, "interface_id": INTERFACE_ID, "profile_id": PROFILE_ID,
        "support_tier": SUPPORT_TIER, "qualification_scope": QUALIFICATION_SCOPE,
        "qualification_tuple": qualification_tuple(), "run_id": work.run_id,
        "status": status, "generated_at_utc": work.generated_at_utc,
        "source": {
            "commit": admission.source.commit, "tree": admission.source.tree,
            "branch": admission.source.branch, "dirty": false,
            "cargo_lock_sha256": admission.source.cargo_lock_sha256,
            "identity_ref": source_ref
        },
        "p0a_dependency": {
            "status": "PASS", "acceptance_sha256": P0A_ACCEPTANCE_SHA256,
            "run_id": P0A_RUN_ID, "seal_sha256": P0A_SEAL_SHA256,
            "reference": dependency_ref
        },
        "artifacts": artifact_refs.values().cloned().collect::<Vec<_>>(),
        "command_plan": {
            "policy_id": "p1a-command-plan-v1", "id_format": "C%03u", "first_id": "C001",
            "command_count": command_count, "consecutive_ids": true, "unique_ids": true,
            "exact_argv_enforced": true
        },
        "commands": commands,
        "redaction": {
            "private_path_leaks": [], "username_leaks": [], "temporary_path_leaks": [],
            "environment_secret_leaks": [], "validated": true
        },
        "containment": {
            "output_root": OUTPUT_ROOT,
            "owned_staging_root": format!("{OUTPUT_ROOT}/.staging"),
            "reparse_points_followed": false,
            "write_scope_enforcement": "fixed-argv-cargo-offline-redirected-standard-write-roots-v1",
            "owned_temp_root_removed": !work.work_root.exists(), "staging_root_removed": false,
            "process_jobs_empty": admission.recorder.commands.iter().all(|command| {
                command.process_audit.as_ref().is_some_and(|audit| audit.process_tree_terminated)
            })
        },
        "authority": {
            "machine_evidence": status,
            "automatic_acceptance_eligible": status == "PASS",
            "required_approvals": [], "human_checkbox_review": "PENDING"
        },
        "errors": errors,
        "seal": {"path": "SHA256SUMS", "entries": entries, "coverage_rule": "all_run_files_except_seal"}
    });
    validate_live_schema(
        repository,
        "docs/schemas/P1A-prototype-v2/python-slm-p1a-phase-evidence-v1.schema.json",
        &evidence,
        "P1A_EVIDENCE_SCHEMA_INVALID",
    )?;
    publication::write_json_new(&work.stage_run.join("evidence.json"), &evidence)?;
    validate_public_bytes(&work.stage_run)?;
    let (actual_entries, seal_sha256) = publication::seal(&work.stage_run)?;
    if actual_entries != entries {
        return Err(XtaskError::integrity(
            "P1A_SEAL_ENTRY_COUNT_INVALID",
            format!("planned {entries} sealed files but observed {actual_entries}"),
        ));
    }
    publication::verify_seal(&work.stage_run, &seal_sha256)?;
    let evidence_sha256 = hash::file(&work.stage_run.join("evidence.json"))?;
    let final_run = admission.output_root.join("runs").join(&work.run_id);
    if final_run.exists() {
        return Err(XtaskError::integrity(
            "P1A_RUN_ALREADY_EXISTS",
            "refused to overwrite an immutable P1A run",
        ));
    }
    move_file_write_through(&work.stage_run, &final_run, false).map_err(|error| {
        XtaskError::new(
            "P1A_RUN_PUBLICATION_FAILED",
            error.category,
            format!(
                "could not durably publish the sealed P1A run: {}",
                error.message
            ),
            error.remediation,
        )
    })?;
    publication::verify_seal(&final_run, &seal_sha256)?;
    hash::require_file(
        &final_run.join("evidence.json"),
        &evidence_sha256,
        "P1A_EVIDENCE_CHANGED_AFTER_PUBLICATION",
    )?;
    Ok(EmittedRun {
        evidence_sha256,
        seal_sha256,
        artifact_refs,
    })
}

fn close_failed_attempt(
    repository: &Path,
    admission: &Admission,
    work: &WorkPaths,
    error: &XtaskError,
) -> Result<()> {
    if work.work_root.exists() {
        cleanup_owned_work(&work.work_root)?;
    }
    if work.stage_run.exists() {
        // A PASS or FAIL emission can fail after create-new staging has begun. The
        // stage path is still private and strictly bound to this attempt, so discard
        // that incomplete candidate before creating the one immutable FAIL terminal.
        discard_unpublished_stage_run(work)?;
    }
    let error_value = sanitize_error(error, repository, &work.work_root);
    emit_terminal_run(repository, admission, work, "FAIL", None, vec![error_value])?;
    let marker = cleanup_stage_after_publication(work)?;
    finish_stage_cleanup(&marker)
}

fn validate_work_paths(work: &WorkPaths) -> Result<()> {
    let (run_id, entropy) = parse_stage_container_name(&work.stage_container_name)?;
    if run_id != work.run_id
        || entropy != work.stage_entropy
        || work.stage_container.file_name() != Some(OsStr::new(&work.stage_container_name))
        || work.stage_container.parent().and_then(Path::file_name) != Some(OsStr::new(".staging"))
        || work.stage_run != work.stage_container.join(&work.run_id)
        || work.work_root != work.stage_container.join("private-work")
        || !generated_timestamp_binds_run_id(&work.generated_at_utc, &work.run_id)
    {
        return Err(XtaskError::integrity(
            "P1A_STAGING_OWNERSHIP_INVALID",
            "P1A work paths do not exactly bind the admitted run and stage grammar",
        ));
    }
    Ok(())
}

fn cleanup_stage_after_publication(work: &WorkPaths) -> Result<PathBuf> {
    validate_work_paths(work)?;
    if !work.stage_container.exists() {
        return Err(XtaskError::integrity(
            "P1A_STAGING_IDENTITY_MISSING",
            "terminal cleanup requires its durable stage attempt journal",
        ));
    }
    publication::require_no_follow_tree(&work.stage_container)?;
    if work.work_root.exists() {
        cleanup_owned_work(&work.work_root)?;
    }
    let attempt = work.stage_container.join("attempt.json");
    if !attempt.is_file() {
        return Err(XtaskError::integrity(
            "P1A_STAGING_IDENTITY_MISSING",
            "terminal cleanup requires its durable attempt.json identity",
        ));
    }
    let attempt_metadata: AttemptMetadata = read_json(&attempt, "P1A_ATTEMPT_JSON_INVALID")?;
    if attempt_metadata.run_id != work.run_id
        || attempt_metadata.stage_container_name != work.stage_container_name
        || attempt_metadata.generated_at_utc != work.generated_at_utc
    {
        return Err(XtaskError::integrity(
            "P1A_STAGING_OWNERSHIP_INVALID",
            "terminal cleanup journal does not bind the exact work paths",
        ));
    }
    let leftovers = fs::read_dir(&work.stage_container)
        .io_context(
            "P1A_STAGING_CLEANUP_FAILED",
            "could not inspect terminal staging",
        )?
        .collect::<std::io::Result<Vec<_>>>()
        .io_context(
            "P1A_STAGING_CLEANUP_FAILED",
            "could not read terminal staging entry",
        )?;
    let mut owned_temporaries = Vec::new();
    for entry in leftovers {
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_STAGING_OWNERSHIP_INVALID",
                    "terminal staging contains a non-UTF-8 entry",
                )
            })?
            .to_owned();
        let metadata = fs::symlink_metadata(entry.path()).io_context(
            "P1A_STAGING_CLEANUP_FAILED",
            "could not inspect terminal staging entry",
        )?;
        if name == "attempt.json" && metadata.is_file() {
            continue;
        }
        if valid_attempt_temp_name(&name) && metadata.is_file() {
            owned_temporaries.push(entry.path());
        } else {
            // Validate the entire cleanup inventory before moving or deleting the
            // durable attempt journal. Unexpected bytes remain intact for review.
            return Err(XtaskError::integrity(
                "P1A_STAGING_OWNERSHIP_INVALID",
                format!("terminal staging contains an unexpected entry: {name}"),
            ));
        }
    }
    for temporary in owned_temporaries {
        fs::remove_file(temporary).io_context(
            "P1A_STAGING_CLEANUP_FAILED",
            "could not remove an exact owned attempt temporary",
        )?;
    }
    let marker = work
        .stage_container
        .parent()
        .expect("validated staging parent")
        .join(format!(
            "{}.finalizing-{}.json",
            work.run_id, work.stage_entropy
        ));
    if marker.exists() {
        return Err(XtaskError::integrity(
            "P1A_FINALIZATION_MARKER_EXISTS",
            "refused to overwrite an existing P1A finalization marker",
        ));
    }
    move_file_write_through(&attempt, &marker, false)?;
    if work.stage_container.exists() {
        let remaining = fs::read_dir(&work.stage_container)
            .io_context(
                "P1A_STAGING_CLEANUP_FAILED",
                "could not inspect the emptied terminal stage",
            )?
            .next()
            .transpose()
            .io_context(
                "P1A_STAGING_CLEANUP_FAILED",
                "could not read the emptied terminal stage",
            )?;
        if remaining.is_some() {
            return Err(XtaskError::integrity(
                "P1A_STAGING_OWNERSHIP_INVALID",
                "terminal stage changed after cleanup inventory validation",
            ));
        }
        fs::remove_dir(&work.stage_container).io_context(
            "P1A_STAGING_CLEANUP_FAILED",
            "could not remove terminal P1A staging directory",
        )?;
    }
    Ok(marker)
}

fn finish_stage_cleanup(marker: &Path) -> Result<()> {
    let name = marker.file_name().and_then(OsStr::to_str).ok_or_else(|| {
        XtaskError::integrity(
            "P1A_FINALIZATION_MARKER_INVALID",
            "P1A finalization marker name is not UTF-8",
        )
    })?;
    parse_finalization_marker_name(name)?;
    if marker.parent().and_then(Path::file_name) != Some(OsStr::new(".staging")) {
        return Err(XtaskError::integrity(
            "P1A_FINALIZATION_MARKER_INVALID",
            "P1A finalization marker is outside the exact staging root",
        ));
    }
    let metadata = fs::symlink_metadata(marker).io_context(
        "P1A_STAGING_CLEANUP_FAILED",
        "could not inspect P1A finalization marker",
    )?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(XtaskError::integrity(
            "P1A_FINALIZATION_MARKER_INVALID",
            "P1A finalization marker is not a regular file",
        ));
    }
    require_finalization_publication_closed(marker)?;
    fs::remove_file(marker).io_context(
        "P1A_STAGING_CLEANUP_FAILED",
        "could not remove completed P1A finalization marker",
    )
}

fn require_finalization_publication_closed(marker: &Path) -> Result<()> {
    let staging = marker.parent().ok_or_else(|| {
        XtaskError::integrity(
            "P1A_FINALIZATION_MARKER_INVALID",
            "P1A finalization marker has no staging parent",
        )
    })?;
    let output_root = staging.parent().ok_or_else(|| {
        XtaskError::integrity(
            "P1A_FINALIZATION_MARKER_INVALID",
            "P1A staging has no receipt-root parent",
        )
    })?;
    for entry in fs::read_dir(staging).io_context(
        "P1A_STAGING_ENUMERATION_FAILED",
        "could not inspect staging before deleting the finalization journal",
    )? {
        let entry = entry.io_context(
            "P1A_STAGING_ENUMERATION_FAILED",
            "could not read staging before deleting the finalization journal",
        )?;
        if entry.path() != marker {
            return Err(XtaskError::integrity(
                "P1A_PUBLICATION_INCOMPLETE",
                "refused to delete the finalization journal while another staging entry remains",
            ));
        }
    }
    for entry in fs::read_dir(output_root.join("acceptances")).io_context(
        "P1A_ACCEPTANCE_ENUMERATION_FAILED",
        "could not inspect acceptance temporaries before deleting the finalization journal",
    )? {
        let entry = entry.io_context(
            "P1A_ACCEPTANCE_ENUMERATION_FAILED",
            "could not read acceptance publication state before deleting the finalization journal",
        )?;
        if entry
            .file_name()
            .to_str()
            .and_then(acceptance_temp_target)
            .is_some()
        {
            return Err(XtaskError::integrity(
                "P1A_PUBLICATION_INCOMPLETE",
                "refused to delete the finalization journal while an acceptance temporary remains",
            ));
        }
    }
    for entry in fs::read_dir(output_root).io_context(
        "P1A_NAMESPACE_ENUMERATION_FAILED",
        "could not inspect pointer temporaries before deleting the finalization journal",
    )? {
        let entry = entry.io_context(
            "P1A_NAMESPACE_ENUMERATION_FAILED",
            "could not read pointer publication state before deleting the finalization journal",
        )?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(valid_pointer_temp_name)
        {
            return Err(XtaskError::integrity(
                "P1A_PUBLICATION_INCOMPLETE",
                "refused to delete the finalization journal while a pointer temporary remains",
            ));
        }
    }
    Ok(())
}

fn move_file_write_through(source: &Path, destination: &Path, replace: bool) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let mut source_wide = source.as_os_str().encode_wide().collect::<Vec<_>>();
        source_wide.push(0);
        let mut destination_wide = destination.as_os_str().encode_wide().collect::<Vec<_>>();
        destination_wide.push(0);
        let mut flags = MOVEFILE_WRITE_THROUGH;
        if replace {
            flags |= MOVEFILE_REPLACE_EXISTING;
        }
        // SAFETY: both UTF-16 buffers are NUL-terminated and remain live for the call.
        if unsafe { MoveFileExW(source_wide.as_ptr(), destination_wide.as_ptr(), flags) } == 0 {
            return Err(XtaskError::environment(
                "P1A_ATOMIC_MOVE_FAILED",
                format!(
                    "could not durably move {} to {}: {}",
                    source.display(),
                    destination.display(),
                    std::io::Error::last_os_error()
                ),
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        if !replace && destination.exists() {
            return Err(XtaskError::integrity(
                "P1A_ATOMIC_MOVE_FAILED",
                "refused to replace an existing destination",
            ));
        }
        fs::rename(source, destination).io_context(
            "P1A_ATOMIC_MOVE_FAILED",
            "could not atomically move P1A publication state",
        )
    }
}

fn acceptance_inventory(root: &Path) -> Result<Vec<(u32, PathBuf, Acceptance, String)>> {
    acceptance_inventory_excluding(root, None)
}

fn acceptance_inventory_excluding(
    root: &Path,
    excluded_temporary: Option<&Path>,
) -> Result<Vec<(u32, PathBuf, Acceptance, String)>> {
    let directory = root.join("acceptances");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let directory_metadata = fs::symlink_metadata(&directory).io_context(
        "P1A_ACCEPTANCE_INSPECTION_FAILED",
        "could not inspect P1A acceptance directory",
    )?;
    if !directory_metadata.is_dir() || directory_metadata.file_type().is_symlink() {
        return Err(XtaskError::integrity(
            "P1A_ACCEPTANCE_DIRECTORY_INVALID",
            "P1A acceptances path is not a regular directory",
        ));
    }
    let mut files = fs::read_dir(&directory)
        .io_context(
            "P1A_ACCEPTANCE_ENUMERATION_FAILED",
            "could not enumerate P1A acceptances",
        )?
        .collect::<std::io::Result<Vec<_>>>()
        .io_context(
            "P1A_ACCEPTANCE_ENUMERATION_FAILED",
            "could not read P1A acceptance entry",
        )?;
    if let Some(excluded) = excluded_temporary {
        files.retain(|entry| entry.path() != excluded);
    }
    files.sort_by_key(|entry| entry.file_name());
    let mut inventory = Vec::new();
    let mut accepted_runs = BTreeSet::new();
    for (index, entry) in files.into_iter().enumerate() {
        let metadata = fs::symlink_metadata(entry.path()).io_context(
            "P1A_ACCEPTANCE_INSPECTION_FAILED",
            "could not inspect P1A acceptance",
        )?;
        let expected_sequence = (index + 1) as u32;
        let expected_name = format!("{expected_sequence:08}.json");
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || entry.file_name() != OsStr::new(&expected_name)
        {
            return Err(XtaskError::integrity(
                "P1A_ACCEPTANCE_SEQUENCE_INVALID",
                "P1A acceptances are not a contiguous create-new sequence",
            ));
        }
        let acceptance: Acceptance = read_json(&entry.path(), "P1A_ACCEPTANCE_JSON_INVALID")?;
        let digest = hash::file(&entry.path())?;
        validate_acceptance(&acceptance, expected_sequence, inventory.last())?;
        if !accepted_runs.insert(acceptance.run_id.clone()) {
            return Err(XtaskError::integrity(
                "P1A_ACCEPTANCE_RUN_DUPLICATE",
                "multiple P1A acceptances bind the same immutable run",
            ));
        }
        inventory.push((expected_sequence, entry.path(), acceptance, digest));
    }
    Ok(inventory)
}

fn validate_acceptance(
    acceptance: &Acceptance,
    sequence: u32,
    previous: Option<&(u32, PathBuf, Acceptance, String)>,
) -> Result<()> {
    let expected_path = format!("acceptances/{sequence:08}.json");
    if acceptance.schema != "python-slm-p1a-phase-acceptance-v1"
        || acceptance.phase_id != PHASE_ID
        || acceptance.interface_id != INTERFACE_ID
        || acceptance.profile_id != PROFILE_ID
        || acceptance.support_tier != SUPPORT_TIER
        || acceptance.qualification_scope != QUALIFICATION_SCOPE
        || acceptance.qualification_tuple != qualification_tuple()
        || acceptance.sequence != sequence
        || acceptance.acceptance_path != expected_path
        || acceptance.status != "PASS"
        || acceptance.acceptance_kind != "automatic_machine_qualification"
        || !acceptance.required_approvals.is_empty()
        || !acceptance.approvals.is_empty()
        || acceptance.human_checkbox_review != "PENDING"
        || !valid_run_id(&acceptance.run_id)
        || acceptance.run_path != format!("runs/{}", acceptance.run_id)
        || acceptance.seal_path != format!("runs/{}/SHA256SUMS", acceptance.run_id)
        || acceptance.p0a_acceptance_sha256 != P0A_ACCEPTANCE_SHA256
        || !is_lower_git_sha(&acceptance.source_commit)
        || !is_lower_git_sha(&acceptance.source_tree)
        || !hash::is_lower_sha256(&acceptance.run_evidence_sha256)
        || !hash::is_lower_sha256(&acceptance.seal_sha256)
        || !hash::is_lower_sha256(&acceptance.host_environment_sha256)
        || !hash::is_lower_sha256(&acceptance.cpu_isolation_sha256)
        || !hash::is_lower_sha256(&acceptance.native_abi_probe_sha256)
        || !generated_timestamp_binds_run_id(&acceptance.created_at, &acceptance.run_id)
    {
        return Err(XtaskError::integrity(
            "P1A_ACCEPTANCE_SHAPE_INVALID",
            format!("acceptance {sequence} violates the closed P1A contract"),
        ));
    }
    match previous {
        None if acceptance.previous_acceptance_path.is_none()
            && acceptance.previous_acceptance_sha256.is_none() => {}
        Some((prior_sequence, _, _, prior_hash))
            if acceptance.previous_acceptance_path.as_deref()
                == Some(format!("acceptances/{prior_sequence:08}.json").as_str())
                && acceptance.previous_acceptance_sha256.as_deref() == Some(prior_hash) => {}
        _ => {
            return Err(XtaskError::integrity(
                "P1A_ACCEPTANCE_CHAIN_INVALID",
                "P1A acceptance does not bind exactly its immediate predecessor",
            ));
        }
    }
    Ok(())
}

fn replace_pointer_atomically(path: &Path, pointer: &Pointer) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(pointer).map_err(|error| {
        XtaskError::new(
            "P1A_POINTER_SERIALIZATION_FAILED",
            Category::Internal,
            format!("could not serialize P1A pointer: {error}"),
            "Inspect the closed P1A pointer model.",
        )
    })?;
    bytes.push(b'\n');
    let prior = if path.exists() {
        Some(fs::read(path).io_context(
            "P1A_POINTER_READ_FAILED",
            "could not capture the previously selected P1A pointer",
        )?)
    } else {
        None
    };
    let output_root = path.parent().ok_or_else(|| {
        XtaskError::integrity(
            "P1A_POINTER_RELATION_INVALID",
            "selected P1A pointer has no receipt-root parent",
        )
    })?;
    let inventory = acceptance_inventory(output_root)?;
    let latest = inventory.last().ok_or_else(|| {
        XtaskError::integrity(
            "P1A_POINTER_RELATION_INVALID",
            "cannot publish a selected pointer without an acceptance",
        )
    })?;
    validate_pointer_projection(pointer, latest.0, &latest.2, &latest.3)?;
    if prior.as_deref() == Some(bytes.as_slice()) {
        return Ok(());
    }
    let expected_prior = if pointer.sequence == 1 {
        None
    } else {
        let predecessor = inventory
            .get(pointer.sequence.saturating_sub(2) as usize)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_POINTER_RELATION_INVALID",
                    "selected pointer has no exact immediate predecessor acceptance",
                )
            })?;
        let predecessor = pointer_for_acceptance(&predecessor.2, predecessor.3.clone());
        let mut predecessor_bytes = serde_json::to_vec_pretty(&predecessor).map_err(|error| {
            XtaskError::new(
                "P1A_POINTER_SERIALIZATION_FAILED",
                Category::Internal,
                format!("could not serialize predecessor P1A pointer: {error}"),
                "Inspect the closed P1A pointer model.",
            )
        })?;
        predecessor_bytes.push(b'\n');
        Some(predecessor_bytes)
    };
    if prior.as_deref() != expected_prior.as_deref() {
        return Err(XtaskError::integrity(
            "P1A_POINTER_PREDECESSOR_INVALID",
            "selected pointer is not absent for sequence one or the exact immediate predecessor",
        ));
    }
    let temporary = path.with_extension(format!(
        "json.tmp-pointer-{}-{}",
        std::process::id(),
        time::now().2
    ));
    publication::write_new(&temporary, &bytes)?;
    let current = if path.exists() {
        Some(fs::read(path).io_context(
            "P1A_POINTER_READ_FAILED",
            "could not revalidate the selected P1A pointer before replacement",
        )?)
    } else {
        None
    };
    if current != prior {
        let _ = fs::remove_file(&temporary);
        return Err(XtaskError::integrity(
            "P1A_POINTER_CONCURRENT_CHANGE",
            "selected P1A pointer changed during compare-before-replace publication",
        ));
    }
    if let Err(error) = move_file_write_through(&temporary, path, prior.is_some()) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if fs::read(path).ok().as_deref() != Some(bytes.as_slice()) {
        return Err(XtaskError::integrity(
            "P1A_POINTER_REREAD_FAILED",
            "selected P1A pointer changed during atomic replacement",
        ));
    }
    Ok(())
}

fn publish_automatic_acceptance(
    repository: &Path,
    output_root: &Path,
    run_id: &str,
    created_at: &str,
    source: &SourceIdentity,
    emitted: &EmittedRun,
) -> Result<(String, String)> {
    let inventory = acceptance_inventory(output_root)?;
    let sequence = u32::try_from(inventory.len() + 1).map_err(|_| {
        XtaskError::integrity(
            "P1A_ACCEPTANCE_SEQUENCE_EXHAUSTED",
            "too many P1A acceptances",
        )
    })?;
    if sequence > 99_999_999 {
        return Err(XtaskError::integrity(
            "P1A_ACCEPTANCE_SEQUENCE_EXHAUSTED",
            "P1A acceptance sequence exceeds eight digits",
        ));
    }
    let acceptance_path = format!("acceptances/{sequence:08}.json");
    let previous_path = inventory
        .last()
        .map(|(prior, _, _, _)| format!("acceptances/{prior:08}.json"));
    let previous_hash = inventory.last().map(|(_, _, _, digest)| digest.clone());
    let required_ref = |path: &str| -> Result<&FileRef> {
        emitted.artifact_refs.get(path).ok_or_else(|| {
            XtaskError::integrity("P1A_ACCEPTANCE_ARTIFACT_MISSING", format!("missing {path}"))
        })
    };
    let acceptance = Acceptance {
        schema: "python-slm-p1a-phase-acceptance-v1".to_owned(),
        phase_id: PHASE_ID.to_owned(),
        interface_id: INTERFACE_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        support_tier: SUPPORT_TIER.to_owned(),
        qualification_scope: QUALIFICATION_SCOPE.to_owned(),
        qualification_tuple: qualification_tuple(),
        sequence,
        acceptance_path: acceptance_path.clone(),
        status: "PASS".to_owned(),
        acceptance_kind: "automatic_machine_qualification".to_owned(),
        required_approvals: Vec::new(),
        approvals: Vec::new(),
        human_checkbox_review: "PENDING".to_owned(),
        run_id: run_id.to_owned(),
        run_path: format!("runs/{run_id}"),
        run_evidence_sha256: emitted.evidence_sha256.clone(),
        seal_path: format!("runs/{run_id}/SHA256SUMS"),
        seal_sha256: emitted.seal_sha256.clone(),
        source_commit: source.commit.clone(),
        source_tree: source.tree.clone(),
        p0a_acceptance_sha256: P0A_ACCEPTANCE_SHA256.to_owned(),
        host_environment_sha256: required_ref("artifacts/host-environment.json")?
            .sha256
            .clone(),
        cpu_isolation_sha256: required_ref("artifacts/cpu-isolation.json")?.sha256.clone(),
        native_abi_probe_sha256: required_ref("artifacts/native-abi-probe.json")?
            .sha256
            .clone(),
        previous_acceptance_path: previous_path,
        previous_acceptance_sha256: previous_hash,
        created_at: created_at.to_owned(),
    };
    validate_acceptance(&acceptance, sequence, inventory.last())?;
    validate_live_schema(
        repository,
        "docs/schemas/P1A-prototype-v2/python-slm-p1a-phase-acceptance-v1.schema.json",
        &serde_json::to_value(&acceptance).expect("acceptance serializes"),
        "P1A_ACCEPTANCE_SCHEMA_INVALID",
    )?;
    let final_path = output_root.join(&acceptance_path);
    publication::write_json_new_via_owned_temp(&final_path, &acceptance, "p1a-acceptance")?;
    let acceptance_sha256 = hash::file(&final_path)?;
    let host = qualification_tuple()["host"].clone();
    let pointer = Pointer {
        schema: "python-slm-p1a-phase-pointer-v1".to_owned(),
        phase_id: PHASE_ID.to_owned(),
        interface_id: INTERFACE_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        support_tier: SUPPORT_TIER.to_owned(),
        qualification_scope: QUALIFICATION_SCOPE.to_owned(),
        sequence,
        run_id: run_id.to_owned(),
        run_path: acceptance.run_path.clone(),
        run_evidence_sha256: acceptance.run_evidence_sha256.clone(),
        seal_path: acceptance.seal_path.clone(),
        seal_sha256: acceptance.seal_sha256.clone(),
        source_commit: source.commit.clone(),
        source_tree: source.tree.clone(),
        host_environment_sha256: acceptance.host_environment_sha256.clone(),
        cpu_isolation_sha256: acceptance.cpu_isolation_sha256.clone(),
        native_abi_probe_sha256: acceptance.native_abi_probe_sha256.clone(),
        previous_acceptance_sha256: acceptance.previous_acceptance_sha256.clone(),
        host,
        accelerator_provider: None,
        accelerator_device: None,
        memory_model: None,
        backend_identity: None,
        native_ml_library_identity: None,
        sla: None,
        acceptance_path: acceptance_path.clone(),
        acceptance_sha256: acceptance_sha256.clone(),
        updated_at: created_at.to_owned(),
    };
    validate_live_schema(
        repository,
        "docs/schemas/P1A-prototype-v2/python-slm-p1a-phase-pointer-v1.schema.json",
        &serde_json::to_value(&pointer).expect("pointer serializes"),
        "P1A_POINTER_SCHEMA_INVALID",
    )?;
    replace_pointer_atomically(&output_root.join("evidence.json"), &pointer)?;
    let reread: Pointer = read_json(
        &output_root.join("evidence.json"),
        "P1A_POINTER_JSON_INVALID",
    )?;
    if reread != pointer
        || reread.acceptance_sha256 != acceptance_sha256
        || reread.run_evidence_sha256 != emitted.evidence_sha256
        || reread.seal_sha256 != emitted.seal_sha256
    {
        return Err(XtaskError::integrity(
            "P1A_POINTER_RELATION_INVALID",
            "selected pointer does not reproduce the accepted run and artifact identities",
        ));
    }
    validate_selected_receipt(repository, output_root)?;
    Ok((acceptance_path, acceptance_sha256))
}

fn validate_acceptance_run(output_root: &Path, acceptance: &Acceptance) -> Result<()> {
    let run_root = output_root.join(&acceptance.run_path);
    publication::verify_seal(&run_root, &acceptance.seal_sha256)?;
    hash::require_file(
        &run_root.join("evidence.json"),
        &acceptance.run_evidence_sha256,
        "P1A_RUN_EVIDENCE_HASH_MISMATCH",
    )?;
    for (relative, expected) in [
        (
            "artifacts/host-environment.json",
            &acceptance.host_environment_sha256,
        ),
        (
            "artifacts/cpu-isolation.json",
            &acceptance.cpu_isolation_sha256,
        ),
        (
            "artifacts/native-abi-probe.json",
            &acceptance.native_abi_probe_sha256,
        ),
    ] {
        hash::require_file(
            &run_root.join(relative),
            expected,
            "P1A_ARTIFACT_HASH_MISMATCH",
        )?;
    }
    Ok(())
}

fn pointer_for_acceptance(acceptance: &Acceptance, acceptance_sha256: String) -> Pointer {
    Pointer {
        schema: "python-slm-p1a-phase-pointer-v1".to_owned(),
        phase_id: PHASE_ID.to_owned(),
        interface_id: INTERFACE_ID.to_owned(),
        profile_id: PROFILE_ID.to_owned(),
        support_tier: SUPPORT_TIER.to_owned(),
        qualification_scope: QUALIFICATION_SCOPE.to_owned(),
        sequence: acceptance.sequence,
        run_id: acceptance.run_id.clone(),
        run_path: acceptance.run_path.clone(),
        run_evidence_sha256: acceptance.run_evidence_sha256.clone(),
        seal_path: acceptance.seal_path.clone(),
        seal_sha256: acceptance.seal_sha256.clone(),
        source_commit: acceptance.source_commit.clone(),
        source_tree: acceptance.source_tree.clone(),
        host_environment_sha256: acceptance.host_environment_sha256.clone(),
        cpu_isolation_sha256: acceptance.cpu_isolation_sha256.clone(),
        native_abi_probe_sha256: acceptance.native_abi_probe_sha256.clone(),
        previous_acceptance_sha256: acceptance.previous_acceptance_sha256.clone(),
        host: qualification_tuple()["host"].clone(),
        accelerator_provider: None,
        accelerator_device: None,
        memory_model: None,
        backend_identity: None,
        native_ml_library_identity: None,
        sla: None,
        acceptance_path: acceptance.acceptance_path.clone(),
        acceptance_sha256,
        updated_at: acceptance.created_at.clone(),
    }
}

fn validate_pointer_projection(
    pointer: &Pointer,
    sequence: u32,
    acceptance: &Acceptance,
    acceptance_sha256: &str,
) -> Result<()> {
    let expected = pointer_for_acceptance(acceptance, acceptance_sha256.to_owned());
    if pointer != &expected || pointer.sequence != sequence {
        return Err(XtaskError::integrity(
            "P1A_POINTER_RELATION_INVALID",
            "selected P1A pointer is not an exact projection of the latest acceptance",
        ));
    }
    Ok(())
}

fn recover_published_attempt(
    repository: &Path,
    admission: &Admission,
    attempt: &AttemptMetadata,
) -> Result<String> {
    let run_root = admission.output_root.join("runs").join(&attempt.run_id);
    publication::require_no_follow_tree(&run_root)?;
    let schemas = receipt_authority_for_run(repository, &run_root)?;
    let evidence = crate::p1a_receipt::validate_terminal_run(
        &run_root,
        &schemas,
        ARTIFACT_PATHS,
        &expected_command_plan(&attempt.source.commit),
    )?;
    let status = evidence
        .pointer("/status")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_EVIDENCE_RELATION_INVALID",
                "published terminal run has no closed PASS or FAIL status",
            )
        })?;
    require_evidence_timestamp(&evidence, &attempt.run_id)?;
    if !matches!(status, "PASS" | "FAIL")
        || evidence.pointer("/run_id").and_then(Value::as_str) != Some(&attempt.run_id)
        || evidence.pointer("/source/commit").and_then(Value::as_str)
            != Some(&attempt.source.commit)
        || evidence.pointer("/source/tree").and_then(Value::as_str) != Some(&attempt.source.tree)
        || evidence
            .pointer("/p0a_dependency/acceptance_sha256")
            .and_then(Value::as_str)
            != Some(P0A_ACCEPTANCE_SHA256)
    {
        return Err(XtaskError::integrity(
            "P1A_EVIDENCE_RELATION_INVALID",
            "published terminal run does not bind its durable attempt journal",
        ));
    }
    let seal_sha256 = hash::file(&run_root.join("SHA256SUMS"))?;
    publication::verify_seal(&run_root, &seal_sha256)?;
    let evidence_sha256 = hash::file(&run_root.join("evidence.json"))?;
    let inventory = acceptance_inventory(&admission.output_root)?;
    let matching = inventory
        .iter()
        .filter(|(_, _, acceptance, _)| acceptance.run_id == attempt.run_id)
        .collect::<Vec<_>>();
    if status == "FAIL" {
        if !matching.is_empty() {
            return Err(XtaskError::integrity(
                "P1A_FAIL_RUN_ACCEPTED",
                "a sealed FAIL run is referenced by an acceptance",
            ));
        }
        return Ok(status.to_owned());
    }

    if let Some((sequence, _, acceptance, acceptance_sha256)) = matching.first().copied() {
        if inventory.last().map(|item| item.0) != Some(*sequence) {
            return Err(XtaskError::integrity(
                "P1A_ACCEPTANCE_CHAIN_INVALID",
                "recovered PASS run is not the latest P1A acceptance",
            ));
        }
        validate_acceptance_run(&admission.output_root, acceptance)?;
        let pointer = pointer_for_acceptance(acceptance, acceptance_sha256.clone());
        replace_pointer_atomically(&admission.output_root.join("evidence.json"), &pointer)?;
        validate_selected_receipt(repository, &admission.output_root)?;
        return Ok(status.to_owned());
    }

    let mut artifact_refs = BTreeMap::new();
    for relative in ARTIFACT_PATHS {
        artifact_refs.insert(
            (*relative).to_owned(),
            publication::file_ref_from(&run_root, relative)?,
        );
    }
    let emitted = EmittedRun {
        evidence_sha256,
        seal_sha256,
        artifact_refs,
    };
    publish_automatic_acceptance(
        repository,
        &admission.output_root,
        &attempt.run_id,
        &attempt.generated_at_utc,
        &attempt.source,
        &emitted,
    )?;
    Ok(status.to_owned())
}

fn validate_all_terminal_runs(repository: &Path, output_root: &Path) -> Result<()> {
    validate_terminal_runs_except(repository, output_root, None, None, true)
}

fn validate_terminal_runs_except(
    repository: &Path,
    output_root: &Path,
    active_run_id: Option<&str>,
    excluded_acceptance_temporary: Option<&Path>,
    require_one: bool,
) -> Result<()> {
    let inventory = acceptance_inventory_excluding(output_root, excluded_acceptance_temporary)?;
    let accepted = inventory
        .iter()
        .map(|(_, _, acceptance, _)| (acceptance.run_id.as_str(), acceptance))
        .collect::<BTreeMap<_, _>>();
    for (_, acceptance_path, acceptance, _) in &inventory {
        validate_acceptance_run(output_root, acceptance)?;
        let run_root = output_root.join(&acceptance.run_path);
        let schemas = receipt_authority_for_run(repository, &run_root)?;
        let acceptance_value: Value = read_json(acceptance_path, "P1A_ACCEPTANCE_JSON_INVALID")?;
        crate::p1a_receipt::validate_json(
            &schemas.acceptance,
            &acceptance_value,
            "P1A_ACCEPTANCE_SCHEMA_INVALID",
        )?;
    }
    let runs = output_root.join("runs");
    let mut run_count = 0usize;
    for entry in fs::read_dir(&runs).io_context(
        "P1A_RUN_ENUMERATION_FAILED",
        "could not enumerate terminal P1A runs",
    )? {
        let entry = entry.io_context(
            "P1A_RUN_ENUMERATION_FAILED",
            "could not read terminal P1A run",
        )?;
        let run_id = entry
            .file_name()
            .to_str()
            .ok_or_else(|| XtaskError::integrity("P1A_RUN_ID_INVALID", "P1A run ID is not UTF-8"))?
            .to_owned();
        let metadata = fs::symlink_metadata(entry.path()).io_context(
            "P1A_RUN_INSPECTION_FAILED",
            "could not inspect terminal P1A run",
        )?;
        if !valid_run_id(&run_id) || !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(XtaskError::integrity(
                "P1A_RUN_ID_INVALID",
                "P1A runs contain a noncanonical terminal run entry",
            ));
        }
        if active_run_id == Some(run_id.as_str()) {
            continue;
        }
        let schemas = receipt_authority_for_run(repository, &entry.path())?;
        let raw_evidence: Value = read_json(
            &entry.path().join("evidence.json"),
            "P1A_EVIDENCE_JSON_INVALID",
        )?;
        let source_commit = raw_evidence
            .pointer("/source/commit")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_SOURCE_IDENTITY_INVALID",
                    "terminal evidence omits its source commit",
                )
            })?;
        let evidence = crate::p1a_receipt::validate_terminal_run(
            &entry.path(),
            &schemas,
            ARTIFACT_PATHS,
            &expected_command_plan(source_commit),
        )?;
        let status = evidence.pointer("/status").and_then(Value::as_str);
        require_evidence_timestamp(&evidence, &run_id)?;
        if evidence.pointer("/run_id").and_then(Value::as_str) != Some(&run_id)
            || !matches!(status, Some("PASS" | "FAIL"))
            || (status == Some("PASS")) != accepted.contains_key(run_id.as_str())
            || evidence
                .pointer("/p0a_dependency/acceptance_sha256")
                .and_then(Value::as_str)
                != Some(P0A_ACCEPTANCE_SHA256)
            || evidence
                .pointer("/authority/automatic_acceptance_eligible")
                .and_then(Value::as_bool)
                != Some(status == Some("PASS"))
        {
            return Err(XtaskError::integrity(
                "P1A_TERMINAL_RUN_RELATION_INVALID",
                "P1A terminal run status, acceptance, or dependency relation is invalid",
            ));
        }
        if let Some(acceptance) = accepted.get(run_id.as_str()) {
            validate_acceptance_evidence_relation(acceptance, &evidence)?;
        }
        validate_public_bytes(&entry.path())?;
        run_count += 1;
    }
    if require_one && run_count == 0 {
        return Err(XtaskError::integrity(
            "P1A_RUN_MISSING",
            "selected P1A namespace contains no terminal run",
        ));
    }
    Ok(())
}

fn validate_acceptance_evidence_relation(acceptance: &Acceptance, evidence: &Value) -> Result<()> {
    if evidence.pointer("/status").and_then(Value::as_str) != Some("PASS")
        || evidence.pointer("/run_id").and_then(Value::as_str) != Some(&acceptance.run_id)
        || evidence.pointer("/source/commit").and_then(Value::as_str)
            != Some(&acceptance.source_commit)
        || evidence.pointer("/source/tree").and_then(Value::as_str) != Some(&acceptance.source_tree)
        || evidence
            .pointer("/p0a_dependency/acceptance_sha256")
            .and_then(Value::as_str)
            != Some(&acceptance.p0a_acceptance_sha256)
        || evidence
            .pointer("/generated_at_utc")
            .and_then(Value::as_str)
            != Some(&acceptance.created_at)
    {
        return Err(XtaskError::integrity(
            "P1A_ACCEPTANCE_EVIDENCE_RELATION_INVALID",
            "automatic acceptance does not bind the PASS evidence source and dependency",
        ));
    }
    let artifacts = evidence
        .pointer("/artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_ACCEPTANCE_EVIDENCE_RELATION_INVALID",
                "accepted evidence has no artifact references",
            )
        })?;
    for (path, expected) in [
        (
            "artifacts/host-environment.json",
            &acceptance.host_environment_sha256,
        ),
        (
            "artifacts/cpu-isolation.json",
            &acceptance.cpu_isolation_sha256,
        ),
        (
            "artifacts/native-abi-probe.json",
            &acceptance.native_abi_probe_sha256,
        ),
    ] {
        let actual = artifacts.iter().find_map(|reference| {
            (reference.pointer("/path").and_then(Value::as_str) == Some(path))
                .then(|| reference.pointer("/sha256").and_then(Value::as_str))
                .flatten()
        });
        if actual != Some(expected.as_str()) {
            return Err(XtaskError::integrity(
                "P1A_ACCEPTANCE_EVIDENCE_RELATION_INVALID",
                format!("automatic acceptance does not bind {path}"),
            ));
        }
    }
    Ok(())
}

fn require_evidence_timestamp(evidence: &Value, run_id: &str) -> Result<()> {
    let generated = evidence
        .pointer("/generated_at_utc")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_EVIDENCE_TIMESTAMP_INVALID",
                "terminal evidence omits its generated-at timestamp",
            )
        })?;
    if !generated_timestamp_binds_run_id(generated, run_id) {
        return Err(XtaskError::integrity(
            "P1A_EVIDENCE_TIMESTAMP_INVALID",
            "terminal evidence timestamp does not bind its immutable run identity",
        ));
    }
    Ok(())
}

fn validate_selected_artifact_bindings(repository: &Path, output_root: &Path) -> Result<()> {
    let pointer: Pointer = read_json(
        &output_root.join("evidence.json"),
        "P1A_POINTER_JSON_INVALID",
    )?;
    let run_root = output_root.join(&pointer.run_path);
    let source: SourceIdentity = read_json(
        &run_root.join("artifacts/source-identity.json"),
        "P1A_SOURCE_IDENTITY_INVALID",
    )?;
    if source.schema != "python-slm-p1a-source-identity-v1"
        || source.phase_id != PHASE_ID
        || source.interface_id != INTERFACE_ID
        || source.profile_id != PROFILE_ID
        || source.support_tier != SUPPORT_TIER
        || source.commit != pointer.source_commit
        || source.tree != pointer.source_tree
        || source.dirty
        || !is_lower_git_sha(&source.commit)
        || !is_lower_git_sha(&source.tree)
        || !hash::is_lower_sha256(&source.cargo_lock_sha256)
        || !hash::is_lower_sha256(&source.verifier_source_sha256)
        || !hash::is_lower_sha256(&source.schema_bundle_sha256)
        || source.p0a_pointer_sha256 != P0A_POINTER_SHA256
    {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_IDENTITY_RELATION_INVALID",
            "selected source-identity artifact does not bind the accepted source",
        ));
    }
    let recorded_bundle: SchemaBundle = read_json(
        &run_root.join("artifacts/schema-bundle.json"),
        "P1A_SCHEMA_BUNDLE_INVALID",
    )?;
    let mut expected_paths = SCHEMA_PATHS
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<Vec<_>>();
    expected_paths.sort();
    let actual_paths = recorded_bundle
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
    let mut schema_manifest = String::new();
    for entry in &recorded_bundle.entries {
        if !hash::is_lower_sha256(&entry.sha256) {
            return Err(XtaskError::integrity(
                "P1A_SCHEMA_BUNDLE_INVALID",
                "selected schema bundle contains an invalid schema digest",
            ));
        }
        schema_manifest.push_str(&entry.sha256);
        schema_manifest.push_str("  ");
        schema_manifest.push_str(&entry.path);
        schema_manifest.push('\n');
    }
    if recorded_bundle.schema != "python-slm-p1a-schema-bundle-v1"
        || recorded_bundle.phase_id != PHASE_ID
        || recorded_bundle.interface_id != INTERFACE_ID
        || recorded_bundle.profile_id != PROFILE_ID
        || actual_paths != expected_paths
        || recorded_bundle.bundle_sha256 != hash::bytes(schema_manifest.as_bytes())
        || recorded_bundle.bundle_sha256 != source.schema_bundle_sha256
    {
        return Err(XtaskError::integrity(
            "P1A_SCHEMA_BUNDLE_INVALID",
            "selected schema-bundle artifact is not a closed self-consistent source binding",
        ));
    }
    let dependency: P0aDependency = read_json(
        &run_root.join("artifacts/p0a-dependency.json"),
        "P1A_P0A_DEPENDENCY_INVALID",
    )?;
    let mut dependency_recorder = P1aRecorder::default();
    let expected_dependency =
        selected_p0a_dependency(repository, &source.commit, &mut dependency_recorder)?;
    if dependency != expected_dependency {
        return Err(XtaskError::integrity(
            "P1A_P0A_DEPENDENCY_RELATION_INVALID",
            "selected P0A dependency artifact differs from the frozen selected dependency",
        ));
    }
    let mut recorder = P1aRecorder::default();
    let ancestor = recorder.run_git(
        repository,
        &["merge-base", "--is-ancestor", &source.commit, "HEAD"],
    )?;
    if ancestor.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_ANCESTRY_INVALID",
            "accepted P1A source commit is not an ancestor of the verifying commit",
        ));
    }
    let revspec = format!("{}^{{tree}}", source.commit);
    let actual_tree = git_line(
        &mut recorder,
        repository,
        &["rev-parse", &revspec],
        "P1A_SOURCE_TREE_INVALID",
    )?;
    if actual_tree != source.tree {
        return Err(XtaskError::integrity(
            "P1A_SOURCE_TREE_INVALID",
            "accepted P1A source tree does not match its Git commit",
        ));
    }
    Ok(())
}

fn collect_namespace_files(root: &Path, directory: &Path, files: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(directory).io_context(
        "P1A_NAMESPACE_ENUMERATION_FAILED",
        "could not enumerate committed P1A namespace",
    )? {
        let entry = entry.io_context(
            "P1A_NAMESPACE_ENUMERATION_FAILED",
            "could not read committed P1A namespace entry",
        )?;
        let metadata = fs::symlink_metadata(entry.path()).io_context(
            "P1A_NAMESPACE_ENTRY_INVALID",
            "could not inspect committed P1A namespace entry",
        )?;
        if metadata.is_dir() {
            collect_namespace_files(root, &entry.path(), files)?;
        } else if metadata.is_file() {
            files.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .expect("namespace descendant")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        } else {
            return Err(XtaskError::integrity(
                "P1A_NAMESPACE_ENTRY_INVALID",
                "committed P1A namespace contains a special file",
            ));
        }
    }
    Ok(())
}

fn require_create_new_history(
    repository: &Path,
    history_path: &str,
    expected_paths: &BTreeSet<String>,
) -> Result<String> {
    if !valid_immutable_history_path(history_path) || expected_paths.is_empty() {
        return Err(XtaskError::integrity(
            "P1A_IMMUTABLE_HISTORY_PATH_INVALID",
            "immutable P1A history query is not a canonical run or acceptance path",
        ));
    }
    let mut recorder = P1aRecorder::default();
    let output = recorder.run_git(
        repository,
        &[
            "log",
            "--full-history",
            "-m",
            "--format=commit:%H",
            "--name-status",
            "--no-renames",
            "--",
            history_path,
        ],
    )?;
    if output.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_IMMUTABLE_HISTORY_QUERY_FAILED",
            "could not inspect immutable P1A creation history",
        ));
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| {
        XtaskError::integrity(
            "P1A_IMMUTABLE_HISTORY_INVALID",
            "immutable P1A creation history is not UTF-8",
        )
    })?;
    let mut current_commit = None;
    let mut creation_commit = None;
    let mut observed = BTreeSet::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        if let Some(commit) = line.strip_prefix("commit:") {
            if !is_lower_git_sha(commit) {
                return Err(XtaskError::integrity(
                    "P1A_IMMUTABLE_HISTORY_INVALID",
                    "immutable P1A history contains a malformed commit identity",
                ));
            }
            current_commit = Some(commit.to_owned());
            continue;
        }
        let Some((status, path)) = line.split_once('\t') else {
            return Err(XtaskError::integrity(
                "P1A_IMMUTABLE_HISTORY_INVALID",
                "immutable P1A history contains a malformed path record",
            ));
        };
        let commit = current_commit.as_ref().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_IMMUTABLE_HISTORY_INVALID",
                "immutable P1A path record is not commit-bound",
            )
        })?;
        if status != "A" || !expected_paths.contains(path) || !observed.insert(path.to_owned()) {
            return Err(XtaskError::integrity(
                "P1A_IMMUTABLE_HISTORY_CHANGED",
                "an immutable P1A run or acceptance was modified, deleted, renamed, re-added, or duplicated",
            ));
        }
        if creation_commit
            .as_ref()
            .is_some_and(|created| created != commit)
        {
            return Err(XtaskError::integrity(
                "P1A_IMMUTABLE_HISTORY_CHANGED",
                "one immutable P1A object was created across multiple commits",
            ));
        }
        creation_commit = Some(commit.clone());
    }
    if &observed != expected_paths {
        return Err(XtaskError::integrity(
            "P1A_IMMUTABLE_HISTORY_CHANGED",
            "immutable P1A creation history does not exactly cover the live object",
        ));
    }
    creation_commit.ok_or_else(|| {
        XtaskError::integrity(
            "P1A_IMMUTABLE_HISTORY_MISSING",
            "immutable P1A object has no create-new commit",
        )
    })
}

fn require_receipt_parent(
    repository: &Path,
    receipt_commit: &str,
    source_commit: &str,
) -> Result<()> {
    let mut recorder = P1aRecorder::default();
    let output = recorder.run_git(
        repository,
        &["rev-list", "--parents", "-n", "1", receipt_commit],
    )?;
    if output.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_RECEIPT_PARENT_QUERY_FAILED",
            "could not inspect the immutable P1A receipt parent",
        ));
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| {
        XtaskError::integrity(
            "P1A_RECEIPT_PARENT_INVALID",
            "immutable P1A receipt parent record is not UTF-8",
        )
    })?;
    let fields = text.split_whitespace().collect::<Vec<_>>();
    if fields != [receipt_commit, source_commit] {
        return Err(XtaskError::integrity(
            "P1A_RECEIPT_PARENT_INVALID",
            "the P1A receipt commit is not the single-parent direct child of its recorded source commit",
        ));
    }
    Ok(())
}

fn require_receipt_commit_scope(
    repository: &Path,
    commit: &str,
    expected: &BTreeMap<String, String>,
) -> Result<()> {
    let mut recorder = P1aRecorder::default();
    let output = recorder.run_git(
        repository,
        &[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-status",
            "-r",
            "--no-renames",
            commit,
        ],
    )?;
    if output.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_RECEIPT_COMMIT_SCOPE_QUERY_FAILED",
            "could not inspect the immutable P1A receipt commit scope",
        ));
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| {
        XtaskError::integrity(
            "P1A_RECEIPT_COMMIT_SCOPE_INVALID",
            "P1A receipt commit scope is not UTF-8",
        )
    })?;
    let observed = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (status, path) = line.split_once('\t').ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_RECEIPT_COMMIT_SCOPE_INVALID",
                    "P1A receipt commit contains a malformed path record",
                )
            })?;
            if !matches!(status, "A" | "M") || path.is_empty() {
                return Err(XtaskError::integrity(
                    "P1A_RECEIPT_COMMIT_SCOPE_INVALID",
                    "P1A receipt commit contains a rename, deletion, or special status",
                ));
            }
            Ok((path.to_owned(), status.to_owned()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    if &observed != expected {
        return Err(XtaskError::integrity(
            "P1A_RECEIPT_COMMIT_SCOPE_INVALID",
            "P1A receipt commit is not exactly one immutable run plus its optional acceptance and pointer",
        ));
    }
    Ok(())
}

fn pointer_at_commit(repository: &Path, commit: &str) -> Result<Pointer> {
    let mut recorder = P1aRecorder::default();
    let spec = format!("{commit}:{OUTPUT_ROOT}/evidence.json");
    let bytes = git_blob(
        &mut recorder,
        repository,
        commit,
        &format!("{OUTPUT_ROOT}/evidence.json"),
    )?;
    serde_json::from_slice(&bytes).map_err(|error| {
        XtaskError::integrity(
            "P1A_RECEIPT_COMMIT_POINTER_INVALID",
            format!("selected pointer at {spec} is invalid JSON: {error}"),
        )
    })
}

fn require_pointer_history(
    repository: &Path,
    inventory: &[(u32, PathBuf, Acceptance, String)],
    acceptance_commits: &BTreeMap<u32, String>,
) -> Result<()> {
    if inventory.len() != acceptance_commits.len() {
        return Err(XtaskError::integrity(
            "P1A_POINTER_HISTORY_INVALID",
            "acceptance creation commits do not exactly cover the selected-pointer history",
        ));
    }
    let path = format!("{OUTPUT_ROOT}/evidence.json");
    let mut recorder = P1aRecorder::default();
    let output = recorder.run_git(
        repository,
        &[
            "log",
            "--full-history",
            "-m",
            "--format=commit:%H",
            "--name-status",
            "--no-renames",
            "--",
            &path,
        ],
    )?;
    if output.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_POINTER_HISTORY_QUERY_FAILED",
            "could not inspect selected P1A pointer history",
        ));
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| {
        XtaskError::integrity(
            "P1A_POINTER_HISTORY_INVALID",
            "selected P1A pointer history is not UTF-8",
        )
    })?;
    let mut current_commit = None;
    let mut records = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        if let Some(commit) = line.strip_prefix("commit:") {
            if !is_lower_git_sha(commit) {
                return Err(XtaskError::integrity(
                    "P1A_POINTER_HISTORY_INVALID",
                    "selected pointer history contains a malformed commit",
                ));
            }
            current_commit = Some(commit.to_owned());
            continue;
        }
        let Some((status, observed_path)) = line.split_once('\t') else {
            return Err(XtaskError::integrity(
                "P1A_POINTER_HISTORY_INVALID",
                "selected pointer history contains a malformed path record",
            ));
        };
        let commit = current_commit.clone().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_POINTER_HISTORY_INVALID",
                "selected pointer history path is not commit-bound",
            )
        })?;
        if observed_path != path || !matches!(status, "A" | "M") {
            return Err(XtaskError::integrity(
                "P1A_POINTER_HISTORY_CHANGED",
                "selected pointer was deleted, renamed, type-changed, or changed outside acceptance publication",
            ));
        }
        records.push((commit, status.to_owned()));
    }
    let expected = inventory
        .iter()
        .rev()
        .map(|(sequence, _, _, _)| {
            Ok((
                acceptance_commits.get(sequence).cloned().ok_or_else(|| {
                    XtaskError::integrity(
                        "P1A_POINTER_HISTORY_INVALID",
                        "selected pointer has no matching acceptance creation commit",
                    )
                })?,
                if *sequence == 1 { "A" } else { "M" }.to_owned(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    if records != expected {
        return Err(XtaskError::integrity(
            "P1A_POINTER_HISTORY_CHANGED",
            "selected pointer history is not exactly one monotonic publication per acceptance",
        ));
    }
    Ok(())
}

fn require_namespace_committed(repository: &Path, output_root: &Path) -> Result<()> {
    let mut recorder = P1aRecorder::default();
    let status = recorder.run_git(
        repository,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            OUTPUT_ROOT,
        ],
    )?;
    if status.exit_code != 0 || !status.stdout.is_empty() {
        return Err(XtaskError::integrity(
            "P1A_SELECTED_NAMESPACE_DIRTY",
            "existing P1A receipt namespace is not clean and committed",
        ));
    }
    let tracked = recorder.run_git(repository, &["ls-files", "--", OUTPUT_ROOT])?;
    if tracked.exit_code != 0 {
        return Err(XtaskError::integrity(
            "P1A_SELECTED_NAMESPACE_UNTRACKED",
            "could not enumerate tracked P1A receipt files",
        ));
    }
    let text = std::str::from_utf8(&tracked.stdout).map_err(|_| {
        XtaskError::integrity(
            "P1A_SELECTED_NAMESPACE_UNTRACKED",
            "tracked P1A receipt inventory is not UTF-8",
        )
    })?;
    let prefix = format!("{OUTPUT_ROOT}/");
    let mut tracked_relative = text
        .lines()
        .map(|path| {
            path.strip_prefix(&prefix)
                .ok_or_else(|| {
                    XtaskError::integrity(
                        "P1A_SELECTED_NAMESPACE_UNTRACKED",
                        "Git returned a path outside the P1A receipt namespace",
                    )
                })
                .map(str::to_owned)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut live_relative = Vec::new();
    collect_namespace_files(output_root, output_root, &mut live_relative)?;
    tracked_relative.sort();
    live_relative.sort();
    if tracked_relative != live_relative {
        return Err(XtaskError::integrity(
            "P1A_SELECTED_NAMESPACE_UNTRACKED",
            "live P1A receipt files differ from the exact tracked inventory",
        ));
    }
    let acceptance_inventory = acceptance_inventory(output_root)?;
    let acceptance_by_run = acceptance_inventory
        .iter()
        .map(|(_, _, acceptance, digest)| (acceptance.run_id.as_str(), (acceptance, digest)))
        .collect::<BTreeMap<_, _>>();
    let mut runs = BTreeMap::<String, BTreeSet<String>>::new();
    for relative in &live_relative {
        if let Some(rest) = relative.strip_prefix("runs/") {
            let (run_id, _) = rest.split_once('/').ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_IMMUTABLE_HISTORY_PATH_INVALID",
                    "terminal run contains a noncanonical tracked path",
                )
            })?;
            if !valid_run_id(run_id) {
                return Err(XtaskError::integrity(
                    "P1A_IMMUTABLE_HISTORY_PATH_INVALID",
                    "terminal run has a noncanonical identity",
                ));
            }
            runs.entry(run_id.to_owned())
                .or_default()
                .insert(format!("{OUTPUT_ROOT}/{relative}"));
        }
    }
    let mut run_commits = BTreeMap::new();
    let mut acceptance_commits = BTreeMap::new();
    for (run_id, expected) in runs {
        let commit = require_create_new_history(
            repository,
            &format!("{OUTPUT_ROOT}/runs/{run_id}"),
            &expected,
        )?;
        let source: SourceIdentity = read_json(
            &output_root
                .join("runs")
                .join(&run_id)
                .join("artifacts/source-identity.json"),
            "P1A_SOURCE_IDENTITY_INVALID",
        )?;
        require_receipt_parent(repository, &commit, &source.commit)?;
        let mut scope = expected
            .iter()
            .map(|path| (path.clone(), "A".to_owned()))
            .collect::<BTreeMap<_, _>>();
        if let Some((acceptance, digest)) = acceptance_by_run.get(run_id.as_str()) {
            let acceptance_path = format!("{OUTPUT_ROOT}/{}", acceptance.acceptance_path);
            let acceptance_commit = require_create_new_history(
                repository,
                &acceptance_path,
                &BTreeSet::from([acceptance_path.clone()]),
            )?;
            if acceptance_commit != commit {
                return Err(XtaskError::integrity(
                    "P1A_RECEIPT_COMMIT_SCOPE_INVALID",
                    "automatic acceptance was not committed with its immutable PASS run",
                ));
            }
            scope.insert(acceptance_path, "A".to_owned());
            scope.insert(
                format!("{OUTPUT_ROOT}/evidence.json"),
                if acceptance.sequence == 1 { "A" } else { "M" }.to_owned(),
            );
            let committed_pointer = pointer_at_commit(repository, &commit)?;
            validate_pointer_projection(
                &committed_pointer,
                acceptance.sequence,
                acceptance,
                digest,
            )?;
            acceptance_commits.insert(acceptance.sequence, commit.clone());
        }
        require_receipt_commit_scope(repository, &commit, &scope)?;
        run_commits.insert(run_id, commit);
    }
    if run_commits.len()
        != fs::read_dir(output_root.join("runs"))
            .io_context(
                "P1A_RUN_ENUMERATION_FAILED",
                "could not count committed terminal P1A runs",
            )?
            .count()
    {
        return Err(XtaskError::integrity(
            "P1A_RECEIPT_COMMIT_SCOPE_INVALID",
            "committed terminal run inventory is incomplete",
        ));
    }
    require_pointer_history(repository, &acceptance_inventory, &acceptance_commits)
}

fn validate_selected_receipt(repository: &Path, output_root: &Path) -> Result<()> {
    publication::require_no_follow_tree(output_root)?;
    let pointer_path = output_root.join("evidence.json");
    let pointer: Pointer = read_json(&pointer_path, "P1A_POINTER_JSON_INVALID")?;
    let inventory = acceptance_inventory(output_root)?;
    let (sequence, acceptance_path, acceptance, acceptance_sha256) =
        inventory.last().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_ACCEPTANCE_MISSING",
                "selected P1A pointer has no acceptance",
            )
        })?;
    validate_pointer_projection(&pointer, *sequence, acceptance, acceptance_sha256)?;
    hash::require_file(
        acceptance_path,
        acceptance_sha256,
        "P1A_ACCEPTANCE_HASH_MISMATCH",
    )?;
    validate_acceptance_run(output_root, acceptance)?;
    let run_root = output_root.join(&acceptance.run_path);
    let schemas = receipt_authority_for_run(repository, &run_root)?;
    let pointer_value: Value = read_json(&pointer_path, "P1A_POINTER_JSON_INVALID")?;
    crate::p1a_receipt::validate_json(
        &schemas.pointer,
        &pointer_value,
        "P1A_POINTER_SCHEMA_INVALID",
    )?;
    let acceptance_value: Value = read_json(acceptance_path, "P1A_ACCEPTANCE_JSON_INVALID")?;
    crate::p1a_receipt::validate_json(
        &schemas.acceptance,
        &acceptance_value,
        "P1A_ACCEPTANCE_SCHEMA_INVALID",
    )?;
    let evidence = crate::p1a_receipt::validate_terminal_run(
        &run_root,
        &schemas,
        ARTIFACT_PATHS,
        &expected_command_plan(&acceptance.source_commit),
    )?;
    if evidence.pointer("/status").and_then(Value::as_str) != Some("PASS")
        || evidence.pointer("/run_id").and_then(Value::as_str) != Some(&acceptance.run_id)
        || evidence.pointer("/source/commit").and_then(Value::as_str)
            != Some(&acceptance.source_commit)
        || evidence.pointer("/source/tree").and_then(Value::as_str) != Some(&acceptance.source_tree)
        || evidence
            .pointer("/p0a_dependency/acceptance_sha256")
            .and_then(Value::as_str)
            != Some(P0A_ACCEPTANCE_SHA256)
        || evidence
            .pointer("/authority/machine_evidence")
            .and_then(Value::as_str)
            != Some("PASS")
        || evidence
            .pointer("/authority/automatic_acceptance_eligible")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(XtaskError::integrity(
            "P1A_EVIDENCE_RELATION_INVALID",
            "selected PASS evidence does not bind its accepted source and P0A dependency",
        ));
    }
    let artifacts = evidence
        .pointer("/artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_EVIDENCE_ARTIFACTS_INVALID",
                "evidence artifacts are absent",
            )
        })?;
    if artifacts.len() != ARTIFACT_PATHS.len() {
        return Err(XtaskError::integrity(
            "P1A_EVIDENCE_ARTIFACTS_INVALID",
            "PASS evidence does not contain exactly six artifact references",
        ));
    }
    for relative in ARTIFACT_PATHS {
        let reference = artifacts
            .iter()
            .find(|item| item.pointer("/path").and_then(Value::as_str) == Some(relative))
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_EVIDENCE_ARTIFACTS_INVALID",
                    format!("missing {relative}"),
                )
            })?;
        let actual = hash::file(&run_root.join(relative))?;
        if reference.pointer("/sha256").and_then(Value::as_str) != Some(&actual) {
            return Err(XtaskError::integrity(
                "P1A_EVIDENCE_ARTIFACTS_INVALID",
                format!("evidence hash does not bind {relative}"),
            ));
        }
    }
    validate_public_bytes(&run_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RUN_ID: &str = "20260813T101021174Z-aaaaaaaaaaaaaaaaaaaaaaaa";
    const TEST_TIMESTAMP: &str = "2026-08-13T10:10:21.174000000Z";
    const TEST_ENTROPY: &str = "bbbbbbbbbbbbbbbb";

    fn test_source() -> SourceIdentity {
        SourceIdentity {
            schema: "python-slm-p1a-source-identity-v1".to_owned(),
            phase_id: PHASE_ID.to_owned(),
            interface_id: INTERFACE_ID.to_owned(),
            profile_id: PROFILE_ID.to_owned(),
            support_tier: SUPPORT_TIER.to_owned(),
            commit: "0".repeat(40),
            tree: "1".repeat(40),
            branch: "main".to_owned(),
            dirty: false,
            cargo_lock_sha256: "2".repeat(64),
            verifier_source_sha256: "3".repeat(64),
            schema_bundle_sha256: "4".repeat(64),
            p0a_pointer_sha256: P0A_POINTER_SHA256.to_owned(),
        }
    }

    fn test_dependency() -> P0aDependency {
        build_p0a_dependency(&"0".repeat(40))
    }

    #[test]
    fn emitted_p0a_dependency_satisfies_its_closed_schema() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is a workspace member");
        let dependency = serde_json::to_value(build_p0a_dependency(&"0".repeat(40))).unwrap();
        validate_live_schema(
            repository,
            "docs/schemas/P1A-prototype-v2/python-slm-p0a-dependency-v1.schema.json",
            &dependency,
            "P1A_TEST_DEPENDENCY_SCHEMA_INVALID",
        )
        .unwrap();
    }

    #[test]
    fn historical_p1a_status_git_argv_is_allowlisted_exactly() {
        require_fixed_git_argv(&[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            "docs/receipts/P1A",
        ])
        .unwrap();

        for rejected in [
            &[
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--",
                "docs/receipts/P1A/evidence.json",
            ][..],
            &[
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--",
                "docs/receipts/P1A",
                "unexpected",
            ][..],
        ] {
            assert_eq!(
                require_fixed_git_argv(rejected).unwrap_err().code,
                "P1A_GIT_COMMAND_NOT_ALLOWED"
            );
        }
    }

    fn test_admission(root: &Path) -> Admission {
        Admission {
            output_root: root.to_path_buf(),
            source: test_source(),
            p0a_dependency: test_dependency(),
            schema_bundle: SchemaBundle {
                schema: "python-slm-p1a-schema-bundle-v1".to_owned(),
                phase_id: PHASE_ID.to_owned(),
                interface_id: INTERFACE_ID.to_owned(),
                profile_id: PROFILE_ID.to_owned(),
                entries: Vec::new(),
                bundle_sha256: "c".repeat(64),
            },
            input_manifest_sha256: "d".repeat(64),
            recorder: P1aRecorder::default(),
        }
    }

    fn test_attempt() -> AttemptMetadata {
        AttemptMetadata {
            schema: "python-slm-p1a-attempt-v1".to_owned(),
            run_id: TEST_RUN_ID.to_owned(),
            stage_container_name: format!("{TEST_RUN_ID}.work-{TEST_ENTROPY}"),
            generated_at_utc: TEST_TIMESTAMP.to_owned(),
            source: test_source(),
            p0a_dependency: test_dependency(),
        }
    }

    fn test_work(root: &Path) -> WorkPaths {
        let stage_container_name = format!("{TEST_RUN_ID}.work-{TEST_ENTROPY}");
        let stage_container = root.join(".staging").join(&stage_container_name);
        WorkPaths {
            stage_run: stage_container.join(TEST_RUN_ID),
            work_root: stage_container.join("private-work"),
            stage_container,
            stage_container_name,
            stage_entropy: TEST_ENTROPY.to_owned(),
            run_id: TEST_RUN_ID.to_owned(),
            generated_at_utc: TEST_TIMESTAMP.to_owned(),
        }
    }

    fn test_acceptance() -> Acceptance {
        Acceptance {
            schema: "python-slm-p1a-phase-acceptance-v1".to_owned(),
            phase_id: PHASE_ID.to_owned(),
            interface_id: INTERFACE_ID.to_owned(),
            profile_id: PROFILE_ID.to_owned(),
            support_tier: SUPPORT_TIER.to_owned(),
            qualification_scope: QUALIFICATION_SCOPE.to_owned(),
            qualification_tuple: qualification_tuple(),
            sequence: 1,
            acceptance_path: "acceptances/00000001.json".to_owned(),
            status: "PASS".to_owned(),
            acceptance_kind: "automatic_machine_qualification".to_owned(),
            required_approvals: Vec::new(),
            approvals: Vec::new(),
            human_checkbox_review: "PENDING".to_owned(),
            run_id: TEST_RUN_ID.to_owned(),
            run_path: format!("runs/{TEST_RUN_ID}"),
            run_evidence_sha256: "1".repeat(64),
            seal_path: format!("runs/{TEST_RUN_ID}/SHA256SUMS"),
            seal_sha256: "2".repeat(64),
            source_commit: "3".repeat(40),
            source_tree: "4".repeat(40),
            p0a_acceptance_sha256: P0A_ACCEPTANCE_SHA256.to_owned(),
            host_environment_sha256: "5".repeat(64),
            cpu_isolation_sha256: "6".repeat(64),
            native_abi_probe_sha256: "7".repeat(64),
            previous_acceptance_path: None,
            previous_acceptance_sha256: None,
            created_at: TEST_TIMESTAMP.to_owned(),
        }
    }

    #[test]
    fn stage_container_grammar_is_closed() {
        let stage_name = format!("{TEST_RUN_ID}.work-{TEST_ENTROPY}");
        assert_eq!(
            parse_stage_container_name(&stage_name).unwrap(),
            (TEST_RUN_ID, TEST_ENTROPY)
        );
        for invalid in [
            format!("{TEST_RUN_ID}.work-{TEST_ENTROPY}.extra"),
            format!("{TEST_RUN_ID}.work-BBBBBBBBBBBBBBBB"),
            format!("{TEST_RUN_ID}.work-short"),
            format!("prefix.work-{TEST_ENTROPY}"),
        ] {
            assert_eq!(
                parse_stage_container_name(&invalid).unwrap_err().code,
                "P1A_STAGING_OWNERSHIP_INVALID"
            );
        }
    }

    #[test]
    fn attempt_binding_rejects_source_and_timestamp_mutation() {
        let temporary = tempfile::tempdir().unwrap();
        let admission = test_admission(temporary.path());
        let stage_name = format!("{TEST_RUN_ID}.work-{TEST_ENTROPY}");
        validate_attempt_binding(&test_attempt(), &admission, &stage_name).unwrap();

        let mut source_mutation = test_attempt();
        source_mutation.source.branch = "other".to_owned();
        assert_eq!(
            validate_attempt_binding(&source_mutation, &admission, &stage_name)
                .unwrap_err()
                .code,
            "P1A_INTERRUPTED_ATTEMPT_BINDING_INVALID"
        );

        let mut time_mutation = test_attempt();
        time_mutation.generated_at_utc = "2026-08-13T10:10:21.175000000Z".to_owned();
        assert_eq!(
            validate_attempt_binding(&time_mutation, &admission, &stage_name)
                .unwrap_err()
                .code,
            "P1A_INTERRUPTED_ATTEMPT_BINDING_INVALID"
        );
    }

    #[test]
    fn cleanup_validates_all_leftovers_before_moving_attempt_journal() {
        let temporary = tempfile::tempdir().unwrap();
        let work = test_work(temporary.path());
        publication::create_dir_all(&work.work_root).unwrap();
        publication::write_json_new(&work.stage_container.join("attempt.json"), &test_attempt())
            .unwrap();
        publication::write_new(&work.stage_container.join("unexpected.bin"), b"owned? no").unwrap();

        assert_eq!(
            cleanup_stage_after_publication(&work).unwrap_err().code,
            "P1A_STAGING_OWNERSHIP_INVALID"
        );
        assert!(work.stage_container.join("attempt.json").is_file());
        assert!(work.stage_container.join("unexpected.bin").is_file());
    }

    #[test]
    fn cleanup_moves_journal_until_terminal_publication_is_complete() {
        let temporary = tempfile::tempdir().unwrap();
        let work = test_work(temporary.path());
        publication::create_dir_all(&temporary.path().join("acceptances")).unwrap();
        publication::create_dir_all(&work.work_root).unwrap();
        publication::write_json_new(&work.stage_container.join("attempt.json"), &test_attempt())
            .unwrap();

        let marker = cleanup_stage_after_publication(&work).unwrap();
        assert!(!work.stage_container.exists());
        assert!(marker.is_file());
        finish_stage_cleanup(&marker).unwrap();
        assert!(!marker.exists());
    }

    #[test]
    fn partial_terminal_stage_is_discarded_before_fail_retry() {
        let temporary = tempfile::tempdir().unwrap();
        let work = test_work(temporary.path());
        publication::create_dir_all(&work.stage_run).unwrap();
        publication::write_new(&work.stage_run.join("partial.json"), b"partial\n").unwrap();

        discard_unpublished_stage_run(&work).unwrap();

        assert!(!work.stage_run.exists());
        assert!(work.stage_container.exists());
    }

    #[test]
    fn finalization_journal_is_retained_while_acceptance_temp_exists() {
        let temporary = tempfile::tempdir().unwrap();
        let work = test_work(temporary.path());
        let acceptances = temporary.path().join("acceptances");
        publication::create_dir_all(&acceptances).unwrap();
        publication::create_dir_all(&work.work_root).unwrap();
        publication::write_json_new(&work.stage_container.join("attempt.json"), &test_attempt())
            .unwrap();
        let marker = cleanup_stage_after_publication(&work).unwrap();
        let acceptance_temp = acceptances.join("00000001.json.tmp-p1a-acceptance-1-2");
        publication::write_new(&acceptance_temp, b"unfinished\n").unwrap();

        assert_eq!(
            finish_stage_cleanup(&marker).unwrap_err().code,
            "P1A_PUBLICATION_INCOMPLETE"
        );
        assert!(marker.is_file());
        fs::remove_file(acceptance_temp).unwrap();
        finish_stage_cleanup(&marker).unwrap();
        assert!(!marker.exists());
    }

    #[test]
    fn pointer_recovery_refuses_to_heal_arbitrary_prior_bytes() {
        let temporary = tempfile::tempdir().unwrap();
        let acceptances = temporary.path().join("acceptances");
        publication::create_dir_all(&acceptances).unwrap();
        let acceptance = test_acceptance();
        let acceptance_path = acceptances.join("00000001.json");
        publication::write_json_new(&acceptance_path, &acceptance).unwrap();
        let pointer = pointer_for_acceptance(&acceptance, hash::file(&acceptance_path).unwrap());
        let pointer_path = temporary.path().join("evidence.json");
        publication::write_new(&pointer_path, b"{}\n").unwrap();

        assert_eq!(
            replace_pointer_atomically(&pointer_path, &pointer)
                .unwrap_err()
                .code,
            "P1A_POINTER_PREDECESSOR_INVALID"
        );
        assert_eq!(fs::read(pointer_path).unwrap(), b"{}\n");
    }

    #[test]
    fn persistent_environment_is_redirected_under_owned_work_and_removed() {
        let temporary = tempfile::tempdir().unwrap();
        let work_root = temporary.path().join("private-work");
        fs::create_dir(&work_root).unwrap();
        let mut environment = minimal_native_environment().unwrap();

        isolate_persistent_environment(&mut environment, &work_root).unwrap();

        let persistent = work_root.join("persistent");
        for key in [
            "USERPROFILE",
            "HOME",
            "LOCALAPPDATA",
            "APPDATA",
            "PROGRAMDATA",
            "CARGO_HOME",
            "RUSTUP_HOME",
        ] {
            let value = environment
                .get(key)
                .and_then(Option::as_ref)
                .expect("redirected environment value");
            assert!(Path::new(value).starts_with(&persistent));
        }
        assert_eq!(environment.get("HOMEDRIVE"), Some(&None));
        assert_eq!(environment.get("HOMEPATH"), Some(&None));
        remove_owned_persistent_environment(&work_root).unwrap();
        assert!(!persistent.exists());
    }

    fn materialize_exact_native_probe_inventory(work_root: &Path) {
        for directory in [
            "command-captures",
            "native",
            "persistent/cargo-home/registry/cache/index.crates.io-test",
            "rust-target",
            "sources",
        ] {
            fs::create_dir_all(work_root.join(directory)).unwrap();
        }
        for relative in [
            "native/p1a_c.lib",
            "native/p1a_c.obj",
            "native/p1a_cpp.lib",
            "native/p1a_cpp.obj",
            "rust-target/p1a_abi_probe.exe",
            "sources/p1a_abi.c",
            "sources/p1a_abi.cpp",
            "sources/p1a_abi.rs",
        ] {
            fs::write(work_root.join(relative), b"nonempty").unwrap();
        }
        fs::write(
            work_root
                .join("persistent/cargo-home/registry/cache/index.crates.io-test/retained.crate"),
            b"retained cache bytes",
        )
        .unwrap();
    }

    #[test]
    fn native_probe_inventory_permits_owned_persistent_cache_but_rejects_native_extras() {
        let temporary = tempfile::tempdir().unwrap();
        let work_root = temporary.path().join("private-work");
        fs::create_dir(&work_root).unwrap();
        materialize_exact_native_probe_inventory(&work_root);

        require_exact_probe_artifact_inventory(&work_root).unwrap();

        fs::write(work_root.join("native/unexpected.obj"), b"unexpected").unwrap();
        assert_eq!(
            require_exact_probe_artifact_inventory(&work_root)
                .unwrap_err()
                .code,
            "P1A_ARTIFACT_INVENTORY_UNEXPECTED"
        );
    }

    #[test]
    fn fail_error_sanitization_redacts_generic_windows_tool_paths() {
        let error = XtaskError::new(
            "P1A_NATIVE_SEARCH_PATH_MISSING",
            Category::Environment,
            "Visual Studio path is C:\\Program Files\\Microsoft Visual Studio\\2022\\Community\\VC\\Tools\nWindows Kits path is C:/Program Files (x86)/Windows Kits/10/Include",
            "Retry after inspecting \\\\?\\C:\\Program Files (x86)\\Windows Kits\\10\\Lib\nFile URI file:///C:/Program Files/Microsoft Visual Studio/2022",
        );
        let sanitized = sanitize_error(
            &error,
            Path::new("D:\\repository"),
            Path::new("E:\\owned-temp"),
        );
        let serialized = serde_json::to_string(&sanitized).unwrap();

        assert!(serialized.contains("${ABSOLUTE_WINDOWS_PATH}"));
        assert!(absolute_windows_path_start(&serialized).is_none());
        assert!(!serialized.contains("Program Files"));

        let run_root = tempfile::tempdir().unwrap();
        fs::write(run_root.path().join("error.json"), serialized).unwrap();
        validate_public_bytes(run_root.path()).unwrap();
    }

    #[test]
    fn json_transcript_redaction_preserves_vswhere_structure() {
        let transcript = br#"[
          {
            "instanceId": "vs-instance",
            "installationPath": "C:\\Program Files\\Microsoft Visual Studio\\2022\\Community",
            "isComplete": true
          }
        ]"#;
        let redacted = redact_output(
            transcript,
            Path::new("D:\\repository"),
            Some(Path::new("E:\\owned-temp")),
        );
        let parsed: Value = serde_json::from_slice(&redacted).unwrap();
        assert_eq!(
            parsed
                .pointer("/0/installationPath")
                .and_then(Value::as_str),
            Some("${ABSOLUTE_WINDOWS_PATH}")
        );
        assert_eq!(
            parsed.pointer("/0/instanceId").and_then(Value::as_str),
            Some("vs-instance")
        );
        assert!(absolute_windows_path_start(&String::from_utf8(redacted).unwrap()).is_none());
    }

    #[test]
    fn planned_p0a_dependency_is_source_commit_bound_before_publication() {
        let source = json!({"commit": "a".repeat(40)});
        let dependency = json!({"verified_at_source_commit": "a".repeat(40)});
        let mut planned = vec![
            ("artifacts/source-identity.json", source),
            ("artifacts/p0a-dependency.json", dependency),
        ];
        validate_planned_p0a_source_binding(&planned).unwrap();

        planned[1].1["verified_at_source_commit"] = json!("b".repeat(40));
        assert_eq!(
            validate_planned_p0a_source_binding(&planned)
                .unwrap_err()
                .code,
            "P1A_P0A_SOURCE_RELATION_INVALID"
        );

        planned[1]
            .1
            .as_object_mut()
            .unwrap()
            .remove("verified_at_source_commit");
        assert_eq!(
            validate_planned_p0a_source_binding(&planned)
                .unwrap_err()
                .code,
            "P1A_P0A_SOURCE_RELATION_INVALID"
        );
    }

    #[test]
    fn rust_minimum_accepts_future_major_versions() {
        assert!(!rust_release_satisfies_minimum(1, 95));
        assert!(rust_release_satisfies_minimum(1, 96));
        assert!(rust_release_satisfies_minimum(2, 0));
    }

    fn write_test_lock(path: &Path, packages: &[(&str, &str, &str)]) {
        let mut lock = concat!(
            "# This file is automatically @generated by Cargo.\n",
            "# It is not intended for manual editing.\n",
            "version = 4\n\n",
        )
        .to_owned();
        for (name, version, checksum) in packages {
            lock.push_str("[[package]]\n");
            lock.push_str(&format!("name = \"{name}\"\n"));
            lock.push_str(&format!("version = \"{version}\"\n"));
            lock.push_str("source = \"registry+https://github.com/rust-lang/crates.io-index\"\n");
            lock.push_str(&format!("checksum = \"{checksum}\"\n\n"));
        }
        fs::write(path, lock).unwrap();
    }

    fn write_test_registry_file(root: &Path, relative: &Path, bytes: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn cargo_lock_rejects_inactive_pyo3_source_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let lock = temporary.path().join("Cargo.lock");
        write_test_lock(
            &lock,
            &[(
                "pyo3",
                "0.24.2",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )],
        );

        assert_eq!(
            cargo_lock_inventory(&lock).unwrap_err().code,
            "P1A_CARGO_LOCK_PYTHON_PACKAGE_REJECTED"
        );
    }

    #[test]
    fn lock_exact_cache_retains_and_discloses_inactive_cuda_source() {
        let temporary = tempfile::tempdir().unwrap();
        let source_home = temporary.path().join("caller-cargo");
        let registry = "index.crates.io-deadbeef";
        let cache_root = source_home.join("registry/cache").join(registry);
        let index_root = source_home.join("registry/index").join(registry);
        let crate_bytes = b"ordinary archive";
        let cuda_bytes = b"cuda resolver source archive";
        let crate_checksum = hash::bytes(crate_bytes);
        let cuda_checksum = hash::bytes(cuda_bytes);
        write_test_registry_file(&cache_root, Path::new("crate-1.0.0.crate"), crate_bytes);
        write_test_registry_file(&cache_root, Path::new("cudarc-0.17.8.crate"), cuda_bytes);
        write_test_registry_file(&index_root, Path::new("config.json"), b"{}\n");
        write_test_registry_file(
            &index_root,
            &Path::new(".cache").join(crates_io_sparse_relative("crate").unwrap()),
            b"crate index",
        );
        write_test_registry_file(
            &index_root,
            &Path::new(".cache").join(crates_io_sparse_relative("cudarc").unwrap()),
            b"cuda index",
        );
        let lock = temporary.path().join("Cargo.lock");
        write_test_lock(
            &lock,
            &[
                ("crate", "1.0.0", &crate_checksum),
                ("cudarc", "0.17.8", &cuda_checksum),
            ],
        );
        let work_root = temporary.path().join("private-work");
        fs::create_dir(&work_root).unwrap();
        let mut environment = minimal_native_environment().unwrap();
        isolate_persistent_environment(&mut environment, &work_root).unwrap();

        let snapshot =
            retain_locked_resolution_cargo_home(&source_home, &work_root, &lock).unwrap();

        assert_eq!(snapshot.cargo_lock_package_count, 2);
        assert_eq!(snapshot.locked_crates_io_package_count, 2);
        assert_eq!(snapshot.archive_file_count, 2);
        assert_eq!(snapshot.sparse_index_record_count, 2);
        assert_eq!(snapshot.registry_config_file_count, 1);
        assert_eq!(snapshot.file_count, 5);
        assert_eq!(
            snapshot.resolver_only_provider_source_packages,
            ["cudarc@0.17.8"]
        );
        assert!(
            work_root
                .join(format!(
                    "persistent/cargo-home/registry/cache/{registry}/cudarc-0.17.8.crate"
                ))
                .is_file()
        );
        let graph = GraphAudit {
            activated_packages: vec!["crate@1.0.0".to_owned()],
        };
        validate_resolution_cache_against_graph(&snapshot, &graph).unwrap();
        let activated_provider = GraphAudit {
            activated_packages: vec!["crate@1.0.0".to_owned(), "cudarc@0.17.8".to_owned()],
        };
        assert_eq!(
            validate_resolution_cache_against_graph(&snapshot, &activated_provider)
                .unwrap_err()
                .code,
            "P1A_RESOLVER_ONLY_PROVIDER_ACTIVATED"
        );
    }

    #[test]
    fn lock_exact_cache_rejects_missing_and_extra_archive_inventory() {
        let temporary = tempfile::tempdir().unwrap();
        let cargo_home = temporary.path().join("cargo-home");
        let registry = cargo_home.join("registry");
        write_test_registry_file(
            &registry,
            Path::new("cache/index.crates.io-test/a-1.0.0.crate"),
            b"a",
        );
        write_test_registry_file(
            &registry,
            Path::new("index/index.crates.io-test/config.json"),
            b"{}",
        );
        let mut expected = Vec::new();
        collect_cache_inventory(&registry.join("cache"), "cache", false, &mut expected).unwrap();
        collect_cache_inventory(&registry.join("index"), "index", false, &mut expected).unwrap();
        expected.sort_by(|left, right| left.0.cmp(&right.0));
        validate_owned_cargo_cache_inventory(&cargo_home, &expected).unwrap();

        fs::remove_file(registry.join("cache/index.crates.io-test/a-1.0.0.crate")).unwrap();
        assert_eq!(
            validate_owned_cargo_cache_inventory(&cargo_home, &expected)
                .unwrap_err()
                .code,
            "P1A_LOCKED_CARGO_CACHE_INVENTORY_INVALID"
        );
        fs::write(
            registry.join("cache/index.crates.io-test/a-1.0.0.crate"),
            b"a",
        )
        .unwrap();
        fs::write(
            registry.join("cache/index.crates.io-test/extra-1.0.0.crate"),
            b"extra",
        )
        .unwrap();
        assert_eq!(
            validate_owned_cargo_cache_inventory(&cargo_home, &expected)
                .unwrap_err()
                .code,
            "P1A_LOCKED_CARGO_CACHE_INVENTORY_INVALID"
        );
    }

    #[test]
    fn prefix_free_cargo_tree_parser_records_every_package_identity() {
        let parsed = parse_cargo_tree_packages(
            "rust-llm-pretrain v0.1.0 (workspace)\nserde v1.0.229\nserde v1.0.229 (*)\n",
        )
        .unwrap();
        assert_eq!(
            parsed,
            ["rust-llm-pretrain@0.1.0", "serde@1.0.229", "serde@1.0.229"]
        );
        assert_eq!(
            parse_cargo_tree_packages("├── serde v1.0.229\n")
                .unwrap_err()
                .code,
            "P1A_CPU_GRAPH_INVALID"
        );
    }
}
