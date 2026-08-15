use crate::error::{Category, Result, XtaskError};
use serde_json::Value;
use std::path::PathBuf;

const BURN_CUBECL_CUDA: &str = "burn-cubecl-cuda";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestedProvider {
    Cuda,
    Rocm,
    Metal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestedBackend {
    Auto,
    BurnCubeclCuda,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedRequest {
    provider: RequestedProvider,
    backend: RequestedBackend,
    device_uuid: Option<String>,
}
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
use crate::error::IoContext;
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
use crate::p1a_process::{DirectCommand, ProcessPolicy};
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
use crate::p1b::{self, ProbeOptions, ProbeReport};
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
use rust_llm_pretrain::backend::{
    BackendRequest, BackendRequestKind, CandidateResult, ProviderIdentity,
    burn_cubecl_cuda_capability, select_candidate,
};
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
use serde_json::json;
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
use std::collections::BTreeMap;
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
use std::ffi::OsString;
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
use std::fs;
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
use std::path::Path;
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
use std::time::Duration;
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
const PROFILE: &str = "prototype-windows-5090-v1";
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
const RESULT_SCHEMA: &str = "python-slm-p2-backend-selection-result-v1";
#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
const FIXTURE_SCHEMA: &str = "python-slm-p2-burn-cubecl-cuda-fixture-v1";

#[derive(Clone, Debug)]
pub(crate) struct SelectionOptions {
    pub provider: String,
    pub backend: String,
    pub cuda_root: Option<PathBuf>,
    pub vs_instance_id: Option<String>,
    pub device_uuid: Option<String>,
}

pub(crate) fn select_backend(options: SelectionOptions) -> Result<Value> {
    let request = parse_request(&options)?;
    if request.provider != RequestedProvider::Cuda {
        return Err(deferred_provider(&options.provider));
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = options;
        return Err(XtaskError::gate(
            "DEFERRED_POST_P16",
            "backend selection is implemented only for native Windows x86_64",
            "Use the Windows RTX 5090 prototype; portable providers remain deferred.",
        ));
    }
    #[cfg(not(feature = "p2-cuda"))]
    {
        let SelectionOptions {
            cuda_root,
            vs_instance_id,
            device_uuid,
            ..
        } = options;
        let _ = (cuda_root, vs_instance_id, device_uuid);
        Err(XtaskError::gate(
            "P2_BACKEND_NOT_AVAILABLE",
            "burn-cubecl-cuda was not compiled into this xtask binary",
            "Run xtask select-backend with --features p2-cuda.",
        ))
    }
    #[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
    {
        select_windows(request, options)
    }
}

fn parse_request(options: &SelectionOptions) -> Result<ParsedRequest> {
    let provider = match options.provider.as_str() {
        "cuda" => RequestedProvider::Cuda,
        "rocm" => RequestedProvider::Rocm,
        "metal" => RequestedProvider::Metal,
        value => {
            return Err(XtaskError::new(
                "P2_PROVIDER_INVALID",
                Category::Usage,
                format!("unsupported provider identifier {value}"),
                "Use cuda, rocm, or metal.",
            ));
        }
    };
    let backend = match options.backend.as_str() {
        "auto" => RequestedBackend::Auto,
        BURN_CUBECL_CUDA => RequestedBackend::BurnCubeclCuda,
        value => {
            return Err(XtaskError::new(
                "P2_BACKEND_INVALID",
                Category::Usage,
                format!("unsupported backend identifier {value}"),
                "Use auto or burn-cubecl-cuda.",
            ));
        }
    };
    if provider != RequestedProvider::Cuda {
        return Err(deferred_provider(&options.provider));
    }
    Ok(ParsedRequest {
        provider,
        backend,
        device_uuid: options.device_uuid.clone(),
    })
}

fn deferred_provider(provider: &str) -> XtaskError {
    XtaskError::gate(
        "DEFERRED_POST_P16",
        format!("provider {provider} is designed but not implemented before P16A"),
        "Use the CUDA prototype or wait for the post-P16 portability phases.",
    )
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
fn select_windows(request: ParsedRequest, options: SelectionOptions) -> Result<Value> {
    let p1b = p1b::probe_report(ProbeOptions {
        cuda_root: options.cuda_root,
        vs_instance_id: options.vs_instance_id,
        device_uuid: options.device_uuid,
    })?;
    let fixture = run_candidate(&p1b)?;
    validate_fixture(&fixture)?;

    let product_request = BackendRequest {
        profile: PROFILE.to_owned(),
        provider: ProviderIdentity::Cuda,
        backend: match request.backend {
            RequestedBackend::Auto => BackendRequestKind::Auto,
            RequestedBackend::BurnCubeclCuda => BackendRequestKind::BurnCubeclCuda,
        },
        device_uuid: request.device_uuid.clone(),
    };
    let candidate = CandidateResult {
        capability: burn_cubecl_cuda_capability(),
        compiled: true,
        implemented: true,
        passed: true,
        failure_code: None,
    };
    let selected = select_candidate(&product_request, std::slice::from_ref(&candidate))
        .map_err(selection_error)?;
    let p1b_sha256 = p1b.canonical_sha256()?;
    let device_uuid = p1b.field_str("/device/uuid")?;
    let device_model = p1b.field_str("/device/model")?;
    let compute_capability = p1b.field_str("/device/compute_capability")?;
    let total_vram_bytes = p1b.field_u64("/device/total_vram_bytes")?;

    let result = json!({
        "schema": RESULT_SCHEMA,
        "status": "SELECTED",
        "qualification_status": "SKIPPED",
        "support_level": "implemented",
        "profile": PROFILE,
        "request": product_request,
        "selected": {
            "backend": selected.capability.backend,
            "provider": selected.capability.provider,
            "framework": selected.capability.framework,
            "burn_version": "0.21.0",
            "cubecl_version": "0.10.0"
        },
        "device": {
            "uuid": device_uuid,
            "model": device_model,
            "compute_capability": compute_capability,
            "total_vram_bytes": total_vram_bytes
        },
        "p1b": {
            "diagnostic_result_hash_algorithm": "sha256-canonical-json-v1",
            "diagnostic_result_sha256": p1b_sha256,
            "toolkit_version": p1b.field_str("/toolkit/version")?,
            "visual_studio_instance_id": p1b.field_str("/compiler/visual_studio_instance_id")?,
            "msvc_tools_version": p1b.field_str("/compiler/msvc_tools_version")?,
            "windows_sdk_version": p1b.field_str("/compiler/windows_sdk_version")?,
            "driver_version": p1b.field_u64("/driver_version")?,
            "cuda_runtime_version": p1b.field_u64("/runtime_libraries/cuda_runtime")?,
            "cublas_version": p1b.field_u64("/runtime_libraries/cublas")?,
            "cublaslt_version": p1b.field_u64("/runtime_libraries/cublaslt")?
        },
        "correctness": {
            "dtype": "bf16-storage-fp32-loss",
            "forward_values_f32": fixture["forward_values_f32"],
            "exact": fixture["forward_exact"],
            "finite": fixture["finite"]
        },
        "gradient": {
            "acceptance": "exact-f32-little-endian-bytes",
            "a_f32_le_hex": fixture["gradient_a_f32_le_hex"],
            "b_f32_le_hex": fixture["gradient_b_f32_le_hex"],
            "exact": fixture["gradient_bytes_exact"]
        },
        "memory": {
            "allocation_bytes": fixture["allocation_bytes"],
            "allocation_touched": fixture["allocation_touched"],
            "telemetry_source": "p1b-native-diagnostic-preflight",
            "free_memory_before_bytes": p1b.value()["mixed_artifact"]["execution"]["free_memory_before_bytes"],
            "free_memory_during_bytes": p1b.value()["mixed_artifact"]["execution"]["free_memory_during_bytes"],
            "free_memory_after_bytes": p1b.value()["mixed_artifact"]["execution"]["free_memory_after_bytes"],
            "leak_decision_basis": "owned-resource-destruction-and-contained-process-exit"
        },
        "launch": {
            "sentinel_first": fixture["sentinel_first"],
            "sentinel_last": fixture["sentinel_last"],
            "synchronized": fixture["synchronized"]
        },
        "cleanup": {
            "owned_resources_released": fixture["owned_resources_released"],
            "contained_process_terminated": true,
            "process_job_empty": true,
            "temporary_directory_removed": true,
            "persistent_artifacts_written": false,
            "receipts_written": false
        },
        "limitations": {
            "qualification": false,
            "performance": false,
            "model_parity": false,
            "checkpoint_parity": false,
            "full_run": false
        }
    });
    validate_result_surface(&result)?;
    Ok(result)
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
fn selection_error(error: rust_llm_pretrain::backend::BackendSelectionError) -> XtaskError {
    XtaskError::gate(
        error.code,
        error.message,
        "Correct the explicit candidate or retry after the implemented candidate passes every check.",
    )
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
fn run_candidate(p1b: &ProbeReport) -> Result<Value> {
    let temp = OwnedTemp::create()?;
    let captures = temp.path().join("captures");
    fs::create_dir(&captures).io_context(
        "P2_CAPTURE_CREATE_FAILED",
        "could not create the private candidate capture directory",
    )?;
    let program = std::env::current_exe().map_err(|error| {
        XtaskError::environment(
            "P2_CURRENT_EXECUTABLE_FAILED",
            format!("could not resolve the current xtask executable: {error}"),
        )
    })?;
    let uuid = p1b.field_str("/device/uuid")?.to_owned();
    let private_profile = temp.path().join("profile");
    let private_local = private_profile.join("AppData/Local");
    let private_roaming = private_profile.join("AppData/Roaming");
    let private_temp = temp.path().join("tmp");
    let private_cuda_cache = temp.path().join("cuda-cache");
    for directory in [
        &private_profile,
        &private_local,
        &private_roaming,
        &private_temp,
        &private_cuda_cache,
    ] {
        fs::create_dir_all(directory).io_context(
            "P2_PRIVATE_ENVIRONMENT_CREATE_FAILED",
            "could not create a private candidate environment directory",
        )?;
    }
    let mut child_path = p1b.cuda_root().join("bin").into_os_string();
    if let Some(existing) = std::env::var_os("PATH") {
        child_path.push(";");
        child_path.push(existing);
    }
    let args = vec![
        OsString::from("p2-cuda-candidate"),
        OsString::from("--device-uuid"),
        OsString::from(&uuid),
    ];
    let output = crate::p1a_process::run(&DirectCommand {
        policy: ProcessPolicy::CudaProbe,
        program,
        args,
        display_argv: vec![
            "${XTASK}".to_owned(),
            "p2-cuda-candidate".to_owned(),
            "--device-uuid".to_owned(),
            "${DEVICE_UUID}".to_owned(),
        ],
        cwd: temp.path().to_path_buf(),
        environment: BTreeMap::from([
            (
                "CUDA_VISIBLE_DEVICES".to_owned(),
                Some(OsString::from(&uuid)),
            ),
            (
                "CUDA_PATH".to_owned(),
                Some(p1b.cuda_root().as_os_str().to_owned()),
            ),
            (
                "CUDA_HOME".to_owned(),
                Some(p1b.cuda_root().as_os_str().to_owned()),
            ),
            (
                "CUDA_CACHE_PATH".to_owned(),
                Some(private_cuda_cache.as_os_str().to_owned()),
            ),
            ("CUDA_CACHE_DISABLE".to_owned(), Some(OsString::from("0"))),
            ("PATH".to_owned(), Some(child_path)),
            ("TEMP".to_owned(), Some(private_temp.as_os_str().to_owned())),
            ("TMP".to_owned(), Some(private_temp.as_os_str().to_owned())),
            (
                "USERPROFILE".to_owned(),
                Some(private_profile.as_os_str().to_owned()),
            ),
            (
                "HOME".to_owned(),
                Some(private_profile.as_os_str().to_owned()),
            ),
            (
                "LOCALAPPDATA".to_owned(),
                Some(private_local.as_os_str().to_owned()),
            ),
            (
                "APPDATA".to_owned(),
                Some(private_roaming.as_os_str().to_owned()),
            ),
        ]),
        timeout: Duration::from_secs(300),
        capture_directory: captures,
        capture_stem: "burn-cubecl-cuda".to_owned(),
        qualified_persistent_roots: vec![p1b.cuda_root().to_path_buf()],
        qualified_persistent_files: Vec::new(),
    })?;
    require_candidate_audit(&output)?;
    let value: Value = serde_json::from_slice(trim_ascii(&output.stdout)).map_err(|error| {
        XtaskError::integrity(
            "P2_CANDIDATE_OUTPUT_INVALID",
            format!("candidate output is not one closed JSON object: {error}"),
        )
    })?;
    temp.close()?;
    Ok(value)
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
fn require_candidate_audit(output: &crate::p1a_process::AuditedOutput) -> Result<()> {
    let audit = &output.audit;
    if output.exit_code != 0
        || audit.timed_out
        || !audit.process_tree_terminated
        || audit.unexpected_descendants
        || audit.exit_races != 0
        || audit.audited_process_count == 0
        || audit.audited_process_count != audit.covered_process_count
        || !audit.forbidden_processes.is_empty()
        || !audit.forbidden_modules.is_empty()
    {
        return Err(XtaskError::gate(
            "P2_CANDIDATE_EXECUTION_FAILED",
            "burn-cubecl-cuda failed correctness, containment, timeout, or cleanup checks",
            "Inspect local stderr and correct the CUDA runtime without selecting a fallback.",
        ));
    }
    Ok(())
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
fn validate_fixture(value: &Value) -> Result<()> {
    let object = value.as_object().ok_or_else(|| {
        XtaskError::integrity(
            "P2_CANDIDATE_OUTPUT_INVALID",
            "candidate result is not an object",
        )
    })?;
    let expected = [
        "allocation_bytes",
        "allocation_touched",
        "backend",
        "device_ordinal",
        "finite",
        "forward_exact",
        "forward_values_f32",
        "gradient_a_f32_le_hex",
        "gradient_b_f32_le_hex",
        "gradient_bytes_exact",
        "owned_resources_released",
        "provider",
        "schema",
        "sentinel_first",
        "sentinel_last",
        "status",
        "synchronized",
    ];
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(XtaskError::integrity(
            "P2_CANDIDATE_OUTPUT_INVALID",
            "candidate result does not have the exact closed field set",
        ));
    }
    if value["schema"] != FIXTURE_SCHEMA
        || value["status"] != "PASS"
        || value["backend"] != BURN_CUBECL_CUDA
        || value["provider"] != "cuda"
        || value["device_ordinal"] != 0
        || value["forward_exact"] != true
        || value["gradient_bytes_exact"] != true
        || value["finite"] != true
        || value["allocation_bytes"] != p1b::ALLOCATION_BYTES
        || value["allocation_touched"] != true
        || value["sentinel_first"] != 7.0
        || value["sentinel_last"] != 7.0
        || value["synchronized"] != true
        || value["owned_resources_released"] != true
    {
        return Err(XtaskError::integrity(
            "P2_CANDIDATE_CHECK_FAILED",
            "candidate result failed an exact correctness, gradient, memory, launch, or cleanup invariant",
        ));
    }
    Ok(())
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
pub(crate) fn candidate_child(device_uuid: &str) -> Result<Value> {
    let visible = std::env::var("CUDA_VISIBLE_DEVICES").map_err(|_| {
        XtaskError::integrity(
            "P2_DEVICE_BINDING_MISSING",
            "contained candidate is missing the exact CUDA UUID binding",
        )
    })?;
    if visible != device_uuid || !valid_gpu_uuid(device_uuid) {
        return Err(XtaskError::integrity(
            "P2_DEVICE_BINDING_MISMATCH",
            "contained candidate UUID does not match CUDA_VISIBLE_DEVICES",
        ));
    }
    serde_json::to_value(
        rust_llm_pretrain::backend::run_burn_cubecl_cuda_fixture().map_err(|error| {
            XtaskError::gate(
                "P2_CANDIDATE_FIXTURE_FAILED",
                error.to_string(),
                "Correct the selected CUDA runtime or backend implementation.",
            )
        })?,
    )
    .map_err(|error| {
        XtaskError::integrity(
            "P2_CANDIDATE_OUTPUT_INVALID",
            format!("could not serialize candidate diagnostics: {error}"),
        )
    })
}

#[cfg(not(all(windows, target_arch = "x86_64", feature = "p2-cuda")))]
pub(crate) fn candidate_child(_device_uuid: &str) -> Result<Value> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "the CUDA candidate child is not installed in this build",
        "Use native Windows x86_64 and build xtask with --features p2-cuda.",
    ))
}

#[cfg(any(feature = "p2-cuda", test))]
fn require_exact_keys(value: &Value, keys: &[&str]) -> Result<()> {
    let object = value.as_object().ok_or_else(|| {
        XtaskError::integrity("P2_RESULT_INVALID", "P2 result component is not an object")
    })?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(XtaskError::integrity(
            "P2_RESULT_INVALID",
            "P2 result component does not have its exact closed field set",
        ));
    }
    Ok(())
}

#[cfg(any(feature = "p2-cuda", test))]
fn contains_local_path(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            let bytes = text.as_bytes();
            bytes.windows(3).any(|window| {
                window[0].is_ascii_alphabetic()
                    && window[1] == b':'
                    && matches!(window[2], b'\\' | b'/')
            }) || text.starts_with("\\\\")
                || text.starts_with("//")
                || text.contains("/Users/")
                || text.contains("/home/")
        }
        Value::Array(values) => values.iter().any(contains_local_path),
        Value::Object(values) => values.values().any(contains_local_path),
        _ => false,
    }
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
fn validate_result_surface(value: &Value) -> Result<()> {
    require_exact_keys(
        value,
        &[
            "schema",
            "status",
            "qualification_status",
            "support_level",
            "profile",
            "request",
            "selected",
            "device",
            "p1b",
            "correctness",
            "gradient",
            "memory",
            "launch",
            "cleanup",
            "limitations",
        ],
    )?;
    if value["schema"] != RESULT_SCHEMA
        || value["status"] != "SELECTED"
        || value["qualification_status"] != "SKIPPED"
        || value["support_level"] != "implemented"
        || contains_local_path(value)
    {
        return Err(XtaskError::integrity(
            "P2_RESULT_INVALID",
            "P2 result has an invalid identity, claim level, or raw local path",
        ));
    }
    Ok(())
}

#[cfg(any(feature = "p2-cuda", test))]
fn valid_gpu_uuid(value: &str) -> bool {
    let Some(value) = value.strip_prefix("GPU-") else {
        return false;
    };
    let compact = value.replace('-', "");
    compact.len() == 32 && compact.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
fn trim_ascii(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    &value[start..end]
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
struct OwnedTemp {
    path: Option<PathBuf>,
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
impl OwnedTemp {
    fn create() -> Result<Self> {
        let parent = fs::canonicalize(std::env::temp_dir()).io_context(
            "P2_TEMP_CREATE_FAILED",
            "could not canonicalize the system temporary directory",
        )?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| {
                XtaskError::environment(
                    "P2_TEMP_CREATE_FAILED",
                    "system clock is before the Unix epoch",
                )
            })?
            .as_nanos();
        for attempt in 0_u32..128 {
            let path = parent.join(format!(
                "python-slm-p2-{:08x}-{now:032x}-{attempt:02x}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path: Some(path) }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(XtaskError::environment(
                        "P2_TEMP_CREATE_FAILED",
                        format!("could not create private candidate directory: {error}"),
                    ));
                }
            }
        }
        Err(XtaskError::environment(
            "P2_TEMP_CREATE_FAILED",
            "could not allocate a unique private candidate directory",
        ))
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("owned temp remains live")
    }

    fn close(mut self) -> Result<()> {
        let path = self.path.take().expect("owned temp remains live");
        fs::remove_dir_all(path).io_context(
            "P2_TEMP_REMOVE_FAILED",
            "could not remove the private candidate directory",
        )
    }
}

#[cfg(all(windows, target_arch = "x86_64", feature = "p2-cuda"))]
impl Drop for OwnedTemp {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(provider: &str, backend: &str) -> SelectionOptions {
        SelectionOptions {
            provider: provider.to_owned(),
            backend: backend.to_owned(),
            cuda_root: None,
            vs_instance_id: None,
            device_uuid: None,
        }
    }

    #[test]
    fn request_parser_is_closed_and_defers_portable_providers() {
        assert_eq!(
            parse_request(&options("cuda", "auto")).unwrap().provider,
            RequestedProvider::Cuda
        );
        assert_eq!(
            parse_request(&options("cuda", "other")).unwrap_err().code,
            "P2_BACKEND_INVALID"
        );
        assert_eq!(
            parse_request(&options("other", "auto")).unwrap_err().code,
            "P2_PROVIDER_INVALID"
        );
        assert_eq!(
            parse_request(&options("rocm", "auto")).unwrap_err().code,
            "DEFERRED_POST_P16"
        );
    }

    #[test]
    fn result_helpers_reject_unknown_keys_and_raw_local_paths() {
        require_exact_keys(&serde_json::json!({"a": 1}), &["a"]).unwrap();
        assert_eq!(
            require_exact_keys(&serde_json::json!({"a": 1, "b": 2}), &["a"])
                .unwrap_err()
                .code,
            "P2_RESULT_INVALID"
        );
        assert!(contains_local_path(
            &serde_json::json!({"path": "C:/private/tool"})
        ));
        assert!(!contains_local_path(
            &serde_json::json!({"path": "${CUDA_ROOT}/bin/nvcc.exe"})
        ));
    }

    #[test]
    fn gpu_uuid_parser_rejects_ordinals_and_malformed_values() {
        assert!(valid_gpu_uuid("GPU-00000000-0000-0000-0000-000000000001"));
        assert!(!valid_gpu_uuid("0"));
        assert!(!valid_gpu_uuid("GPU-not-a-uuid"));
    }

    #[test]
    fn default_build_rejects_before_any_cuda_discovery() {
        #[cfg(not(feature = "p2-cuda"))]
        assert_eq!(
            select_backend(options("cuda", "auto")).unwrap_err().code,
            "P2_BACKEND_NOT_AVAILABLE"
        );
    }
}
