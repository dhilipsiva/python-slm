use crate::error::{IoContext, Result, XtaskError};
use crate::p1a_process::{AuditedProcessIdentity, LoadedModuleIdentity, ProcessAudit};
use crate::process::FileRef;
use crate::{hash, json_schema, publication};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

pub(crate) struct ReceiptSchemas {
    pub evidence: Value,
    pub artifacts: BTreeMap<String, Value>,
    pub acceptance: Value,
    pub pointer: Value,
}

pub(crate) fn expected_command_metadata(
    argv: &[String],
) -> Result<(&'static str, &'static str, &'static str)> {
    let first = argv.first().map(String::as_str).ok_or_else(|| {
        XtaskError::integrity(
            "P1A_COMMAND_PLAN_MISMATCH",
            "frozen command plan contains an empty argv",
        )
    })?;
    match first {
        "${GIT}" => Ok(("git", "${REPO}", "not_applicable")),
        "cargo" => Ok(("non_git", "${REPO}", "cargo_offline_enforced")),
        "${VSWHERE}" | "${CARGO}" => Ok(("non_git", "${REPO}", "not_applicable")),
        "${RUSTC}" if argv.get(1).map(String::as_str) == Some("-vV") => {
            Ok(("non_git", "${REPO}", "not_applicable"))
        }
        "${CL}"
        | "${LIB}"
        | "${RUSTC}"
        | "${DUMPBIN}"
        | "${P1A_TEMP}/rust-target/p1a_abi_probe.exe" => {
            Ok(("non_git", "${P1A_TEMP}", "not_applicable"))
        }
        _ => Err(XtaskError::integrity(
            "P1A_COMMAND_PLAN_MISMATCH",
            format!("frozen command plan has no metadata policy for {first}"),
        )),
    }
}

pub(crate) fn validate_json(schema: &Value, value: &Value, code: &'static str) -> Result<()> {
    json_schema::validate(schema, value, code)
}

pub(crate) fn validate_terminal_run(
    run_root: &Path,
    schemas: &ReceiptSchemas,
    expected_artifact_paths: &[&str],
    expected_command_plan: &[Vec<String>],
) -> Result<Value> {
    publication::require_no_follow_tree(run_root)?;
    let seal_sha256 = hash::file(&run_root.join("SHA256SUMS"))?;
    publication::verify_seal(run_root, &seal_sha256)?;
    let evidence_path = run_root.join("evidence.json");
    let evidence: Value = read_json(&evidence_path, "P1A_EVIDENCE_JSON_INVALID")?;
    json_schema::validate(&schemas.evidence, &evidence, "P1A_EVIDENCE_SCHEMA_INVALID")?;
    let status = evidence
        .pointer("/status")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_EVIDENCE_STATUS_INVALID",
                "terminal evidence has no status",
            )
        })?;
    let artifacts = evidence
        .pointer("/artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_EVIDENCE_ARTIFACTS_INVALID",
                "terminal evidence has no artifact array",
            )
        })?;
    let mut artifact_refs = BTreeMap::new();
    let mut artifact_values = BTreeMap::new();
    for reference in artifacts {
        let reference: FileRef = serde_json::from_value(reference.clone()).map_err(|error| {
            XtaskError::integrity(
                "P1A_EVIDENCE_ARTIFACTS_INVALID",
                format!("artifact reference is malformed: {error}"),
            )
        })?;
        validate_file_ref(run_root, &reference)?;
        if artifact_refs
            .insert(reference.path.clone(), reference)
            .is_some()
        {
            return Err(XtaskError::integrity(
                "P1A_EVIDENCE_ARTIFACTS_INVALID",
                "terminal evidence contains a duplicate artifact reference",
            ));
        }
    }
    let expected = expected_artifact_paths
        .iter()
        .filter(|path| artifact_refs.contains_key(**path))
        .copied()
        .collect::<BTreeSet<_>>();
    if status == "PASS" && expected.len() != expected_artifact_paths.len() {
        return Err(XtaskError::integrity(
            "P1A_EVIDENCE_ARTIFACTS_INVALID",
            "PASS evidence omits a required artifact",
        ));
    }
    for (relative, reference) in &artifact_refs {
        let schema = schemas.artifacts.get(relative).ok_or_else(|| {
            XtaskError::integrity(
                "P1A_EVIDENCE_ARTIFACTS_INVALID",
                format!("terminal evidence references unknown artifact {relative}"),
            )
        })?;
        let artifact: Value = read_json(&run_root.join(relative), "P1A_ARTIFACT_JSON_INVALID")?;
        json_schema::validate(schema, &artifact, "P1A_ARTIFACT_SCHEMA_INVALID")?;
        if hash::file(&run_root.join(relative))? != reference.sha256 {
            return Err(XtaskError::integrity(
                "P1A_ARTIFACT_HASH_MISMATCH",
                format!("artifact bytes do not match evidence for {relative}"),
            ));
        }
        artifact_values.insert(relative.clone(), artifact);
    }
    require_embedded_reference(
        run_root,
        &evidence,
        "/source/identity_ref",
        artifact_refs.get("artifacts/source-identity.json"),
    )?;
    validate_cross_file_relations(run_root, &evidence, &artifact_values)?;
    require_embedded_reference(
        run_root,
        &evidence,
        "/p0a_dependency/reference",
        artifact_refs.get("artifacts/p0a-dependency.json"),
    )?;

    let commands = evidence
        .pointer("/commands")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_EVIDENCE_COMMANDS_INVALID",
                "terminal evidence has no command array",
            )
        })?;
    if commands.len() > expected_command_plan.len()
        || (status == "PASS" && commands.len() != expected_command_plan.len())
        || evidence
            .pointer("/command_plan/command_count")
            .and_then(Value::as_u64)
            != Some(commands.len() as u64)
    {
        return Err(XtaskError::integrity(
            "P1A_COMMAND_PLAN_MISMATCH",
            "terminal command count differs from the frozen exact plan",
        ));
    }
    for (index, (command, expected_argv)) in commands.iter().zip(expected_command_plan).enumerate()
    {
        let expected_id = format!("C{:03}", index + 1);
        let (expected_kind, expected_cwd, expected_network) =
            expected_command_metadata(expected_argv)?;
        if command.pointer("/id").and_then(Value::as_str) != Some(&expected_id)
            || command.pointer("/argv")
                != Some(&Value::Array(
                    expected_argv.iter().cloned().map(Value::String).collect(),
                ))
            || command.pointer("/command_kind").and_then(Value::as_str) != Some(expected_kind)
            || command.pointer("/cwd").and_then(Value::as_str) != Some(expected_cwd)
            || command.pointer("/network_mode").and_then(Value::as_str) != Some(expected_network)
        {
            return Err(XtaskError::integrity(
                "P1A_COMMAND_PLAN_MISMATCH",
                format!("command {expected_id} differs from the frozen execution plan"),
            ));
        }
        for (stream, suffix) in [("stdout", "stdout.txt"), ("stderr", "stderr.txt")] {
            let reference: FileRef =
                serde_json::from_value(command.get(stream).cloned().ok_or_else(|| {
                    XtaskError::integrity(
                        "P1A_COMMAND_STREAM_REF_INVALID",
                        format!("command {expected_id} has no {stream} reference"),
                    )
                })?)
                .map_err(|error| {
                    XtaskError::integrity(
                        "P1A_COMMAND_STREAM_REF_INVALID",
                        format!("command {expected_id} {stream} reference is malformed: {error}"),
                    )
                })?;
            if reference.path != format!("commands/{expected_id}.{suffix}") {
                return Err(XtaskError::integrity(
                    "P1A_COMMAND_STREAM_REF_INVALID",
                    format!("command {expected_id} {stream} path is not ID-bound"),
                ));
            }
            validate_file_ref(run_root, &reference)?;
        }
        validate_process_audit(command)?;
    }
    validate_command_timings(&evidence, commands)?;
    if let (Some(host), Some(native_probe)) = (
        artifact_values.get("artifacts/host-environment.json"),
        artifact_values.get("artifacts/native-abi-probe.json"),
    ) {
        validate_transcript_relations(run_root, &evidence, host, native_probe)?;
    }

    let declared_entries = evidence
        .pointer("/seal/entries")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_SEAL_ENTRY_COUNT_INVALID",
                "terminal evidence has no seal entry count",
            )
        })?;
    let seal_entries = fs::read_to_string(run_root.join("SHA256SUMS"))
        .io_context(
            "P1A_SEAL_READ_FAILED",
            "could not count terminal seal entries",
        )?
        .lines()
        .count() as u64;
    let expected_entries = 1 + artifact_refs.len() as u64 + (2 * commands.len() as u64);
    if declared_entries != expected_entries || seal_entries != expected_entries {
        return Err(XtaskError::integrity(
            "P1A_SEAL_ENTRY_COUNT_INVALID",
            "terminal evidence, seal, artifact, and transcript counts disagree",
        ));
    }
    Ok(evidence)
}

fn validate_cross_file_relations(
    run_root: &Path,
    evidence: &Value,
    artifacts: &BTreeMap<String, Value>,
) -> Result<()> {
    let run_id = run_root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_RUN_ID_INVALID",
                "terminal run directory has no canonical UTF-8 identity",
            )
        })?;
    if evidence.pointer("/run_id").and_then(Value::as_str) != Some(run_id) {
        return Err(XtaskError::integrity(
            "P1A_EVIDENCE_RUN_RELATION_INVALID",
            "terminal evidence run ID differs from its immutable directory",
        ));
    }
    let source = artifacts
        .get("artifacts/source-identity.json")
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_SOURCE_ARTIFACT_MISSING",
                "terminal evidence omits its source-identity artifact",
            )
        })?;
    for (evidence_pointer, source_pointer) in [
        ("/source/commit", "/commit"),
        ("/source/tree", "/tree"),
        ("/source/branch", "/branch"),
        ("/source/dirty", "/dirty"),
        ("/source/cargo_lock_sha256", "/cargo_lock_sha256"),
    ] {
        if evidence.pointer(evidence_pointer) != source.pointer(source_pointer) {
            return Err(XtaskError::integrity(
                "P1A_SOURCE_EVIDENCE_RELATION_INVALID",
                format!("terminal evidence {evidence_pointer} differs from source identity"),
            ));
        }
    }
    let dependency = artifacts
        .get("artifacts/p0a-dependency.json")
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_P0A_DEPENDENCY_MISSING",
                "terminal evidence omits its P0A dependency artifact",
            )
        })?;
    validate_p0a_source_relation(source, dependency)?;
    for (evidence_pointer, dependency_pointer) in [
        ("/p0a_dependency/status", "/status"),
        ("/p0a_dependency/acceptance_sha256", "/acceptance_sha256"),
        ("/p0a_dependency/run_id", "/run_id"),
        ("/p0a_dependency/seal_sha256", "/seal_sha256"),
    ] {
        if evidence.pointer(evidence_pointer) != dependency.pointer(dependency_pointer) {
            return Err(XtaskError::integrity(
                "P1A_P0A_EVIDENCE_RELATION_INVALID",
                format!("terminal evidence {evidence_pointer} differs from P0A dependency"),
            ));
        }
    }
    let bundle = artifacts
        .get("artifacts/schema-bundle.json")
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_SCHEMA_BUNDLE_MISSING",
                "terminal evidence omits its schema-bundle artifact",
            )
        })?;
    if source.pointer("/schema_bundle_sha256") != bundle.pointer("/bundle_sha256") {
        return Err(XtaskError::integrity(
            "P1A_SCHEMA_SOURCE_RELATION_INVALID",
            "source identity and schema bundle disagree",
        ));
    }
    if let (Some(host), Some(native_probe)) = (
        artifacts.get("artifacts/host-environment.json"),
        artifacts.get("artifacts/native-abi-probe.json"),
    ) {
        validate_host_semantic_relations(host)?;
        validate_native_probe_semantic_relations(native_probe)?;
        validate_host_native_input_relation(host, native_probe)?;
        validate_executed_tool_relations(evidence, host, native_probe)?;
    }
    if let (Some(host), Some(cpu)) = (
        artifacts.get("artifacts/host-environment.json"),
        artifacts.get("artifacts/cpu-isolation.json"),
    ) {
        validate_cpu_semantic_relations(host, cpu)?;
    }
    Ok(())
}

fn validate_p0a_source_relation(source: &Value, dependency: &Value) -> Result<()> {
    require_equal_pointers(
        dependency,
        "/verified_at_source_commit",
        source,
        "/commit",
        "P1A_P0A_SOURCE_RELATION_INVALID",
        "P0A verification source commit differs from the P1A source identity",
    )
}

fn require_equal_pointers(
    left: &Value,
    left_pointer: &str,
    right: &Value,
    right_pointer: &str,
    code: &'static str,
    message: &str,
) -> Result<()> {
    let left_value = left.pointer(left_pointer).ok_or_else(|| {
        XtaskError::integrity(code, format!("missing relation endpoint {left_pointer}"))
    })?;
    let right_value = right.pointer(right_pointer).ok_or_else(|| {
        XtaskError::integrity(code, format!("missing relation endpoint {right_pointer}"))
    })?;
    if left_value != right_value {
        return Err(XtaskError::integrity(code, message));
    }
    Ok(())
}

fn validate_host_semantic_relations(host: &Value) -> Result<()> {
    const CODE: &str = "P1A_HOST_SEMANTIC_RELATION_INVALID";
    require_equal_pointers(
        host,
        "/result/toolchain_identity_stability/before",
        host,
        "/result/toolchain_identity_stability/after",
        CODE,
        "toolchain stability before and after snapshots differ",
    )?;
    if host
        .pointer("/result/toolchain_identity_stability/stable")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(XtaskError::integrity(
            CODE,
            "toolchain stability is not affirmatively true",
        ));
    }
    require_equal_pointers(
        host,
        "/result/input_stability/before_sha256",
        host,
        "/result/input_stability/after_sha256",
        CODE,
        "host input manifest changed during qualification",
    )?;
    if host
        .pointer("/result/input_stability/stable")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(XtaskError::integrity(
            CODE,
            "host input stability is not affirmatively true",
        ));
    }
    for (sdk_pointer, snapshot_pointer) in [
        (
            "/result/windows_sdk/include_tree_sha256",
            "/result/toolchain_identity_stability/before/windows_sdk_include_tree/sha256",
        ),
        (
            "/result/windows_sdk/x64_lib_tree_sha256",
            "/result/toolchain_identity_stability/before/windows_sdk_x64_lib_tree/sha256",
        ),
    ] {
        require_equal_pointers(
            host,
            sdk_pointer,
            host,
            snapshot_pointer,
            CODE,
            "Windows SDK tree digest differs from its stability snapshot",
        )?;
    }
    let packages = host
        .pointer("/result/cargo_build_policy/activated_packages")
        .and_then(Value::as_array)
        .ok_or_else(|| XtaskError::integrity(CODE, "activated package list is absent"))?;
    let packages = packages
        .iter()
        .map(|package| {
            package.as_str().ok_or_else(|| {
                XtaskError::integrity(CODE, "activated package entry is not a string")
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let recomputed = hash::bytes(packages.join("\n").as_bytes());
    if host
        .pointer("/result/cargo_build_policy/activated_packages_sha256")
        .and_then(Value::as_str)
        != Some(&recomputed)
    {
        return Err(XtaskError::integrity(
            CODE,
            "activated package digest does not match the emitter join rule",
        ));
    }
    let candidates = host
        .pointer("/result/native_toolchain/visual_studio_candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| XtaskError::integrity(CODE, "Visual Studio candidate list is absent"))?;
    let selected_id = host
        .pointer("/result/native_toolchain/selected_visual_studio_instance_id")
        .and_then(Value::as_str)
        .ok_or_else(|| XtaskError::integrity(CODE, "selected Visual Studio ID is absent"))?;
    let selected = candidates
        .iter()
        .filter(|candidate| {
            candidate.pointer("/instance_id").and_then(Value::as_str) == Some(selected_id)
        })
        .collect::<Vec<_>>();
    if selected.len() != 1 {
        return Err(XtaskError::integrity(
            CODE,
            "selected Visual Studio ID does not identify exactly one candidate",
        ));
    }
    let selected = selected[0];
    for (candidate_pointer, host_pointer) in [
        (
            "/product_id",
            "/result/native_toolchain/selected_visual_studio_product_id",
        ),
        (
            "/installation_version",
            "/result/native_toolchain/visual_studio_version",
        ),
        (
            "/installation_path",
            "/result/native_toolchain/visual_studio_installation_path",
        ),
        (
            "/complete",
            "/result/native_toolchain/visual_studio_complete",
        ),
        (
            "/launchable",
            "/result/native_toolchain/visual_studio_launchable",
        ),
    ] {
        require_equal_pointers(
            selected,
            candidate_pointer,
            host,
            host_pointer,
            CODE,
            "selected Visual Studio projection differs from its candidate record",
        )?;
    }
    require_equal_pointers(
        host,
        "/result/native_toolchain/selected_visual_studio_product_id",
        host,
        "/result/native_toolchain/visual_studio_edition",
        CODE,
        "Visual Studio edition differs from the selected product identity",
    )?;
    if selected
        .pointer("/reboot_required")
        .and_then(Value::as_bool)
        != Some(false)
    {
        return Err(XtaskError::integrity(
            CODE,
            "selected Visual Studio candidate requires a reboot",
        ));
    }
    Ok(())
}

fn validate_native_probe_semantic_relations(native_probe: &Value) -> Result<()> {
    const CODE: &str = "P1A_NATIVE_SEMANTIC_RELATION_INVALID";
    require_equal_pointers(
        native_probe,
        "/result/input_stability/before_sha256",
        native_probe,
        "/result/input_stability/after_sha256",
        CODE,
        "native-probe input manifest changed during qualification",
    )?;
    if native_probe
        .pointer("/result/input_stability/stable")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(XtaskError::integrity(
            CODE,
            "native-probe input stability is not affirmatively true",
        ));
    }
    let imports = native_probe
        .pointer("/result/binary_audit/imports")
        .and_then(Value::as_array)
        .ok_or_else(|| XtaskError::integrity(CODE, "native import list is absent"))?;
    let classifications = native_probe
        .pointer("/result/binary_audit/import_classifications")
        .and_then(Value::as_array)
        .ok_or_else(|| XtaskError::integrity(CODE, "native import classifications are absent"))?;
    if imports.len() != classifications.len() {
        return Err(XtaskError::integrity(
            CODE,
            "native imports and classifications have different cardinality",
        ));
    }
    let imports = imports
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| XtaskError::integrity(CODE, "native import name is not a string"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let classified = classifications
        .iter()
        .map(|value| {
            let name = value
                .pointer("/name")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    XtaskError::integrity(CODE, "native import classification has no name")
                })?;
            let expected_class = if name.eq_ignore_ascii_case("vcruntime140.dll") {
                "msvc_release_runtime"
            } else if name.to_ascii_lowercase().starts_with("api-ms-win-crt-") {
                "ucrt_release"
            } else {
                "windows_system"
            };
            if value.pointer("/class").and_then(Value::as_str) != Some(expected_class) {
                return Err(XtaskError::integrity(
                    CODE,
                    format!("native import {name} has the wrong emitter classification"),
                ));
            }
            Ok(name)
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if imports.len() != classifications.len() || imports != classified {
        return Err(XtaskError::integrity(
            CODE,
            "native imports are not in exact one-to-one correspondence with classifications",
        ));
    }
    Ok(())
}

fn validate_host_native_input_relation(host: &Value, native_probe: &Value) -> Result<()> {
    const CODE: &str = "P1A_INPUT_STABILITY_RELATION_INVALID";
    for pointer in [
        "/result/input_stability/before_sha256",
        "/result/input_stability/after_sha256",
    ] {
        require_equal_pointers(
            host,
            pointer,
            native_probe,
            pointer,
            CODE,
            "host and native-probe input manifests do not bind the same source bytes",
        )?;
    }
    Ok(())
}

fn validate_cpu_semantic_relations(host: &Value, cpu: &Value) -> Result<()> {
    const CODE: &str = "P1A_CPU_SEMANTIC_RELATION_INVALID";
    require_equal_pointers(
        cpu,
        "/result/before",
        cpu,
        "/result/after",
        CODE,
        "CPU isolation before and after snapshots differ",
    )?;
    for pointer in [
        "/result/topology_stable",
        "/result/affinity_stable",
        "/result/power_policy_stable",
    ] {
        if cpu.pointer(pointer).and_then(Value::as_bool) != Some(true) {
            return Err(XtaskError::integrity(
                CODE,
                format!("CPU isolation relation {pointer} is not affirmatively true"),
            ));
        }
    }
    for (cpu_pointer, host_pointer) in [
        (
            "/result/before/processor_group_count",
            "/result/cpu/processor_groups",
        ),
        (
            "/result/before/active_logical_processors",
            "/result/cpu/logical_processors",
        ),
        (
            "/result/before/logical_processor_union_mask",
            "/result/cpu/logical_processor_union_mask",
        ),
        (
            "/result/before/core_topology_sha256",
            "/result/cpu/core_topology_sha256",
        ),
        (
            "/policy/selected_processor_group",
            "/result/before/process_group",
        ),
        (
            "/policy/selected_affinity_mask",
            "/result/before/process_affinity_mask",
        ),
    ] {
        let (right, right_pointer) = if host_pointer.starts_with("/result/cpu/") {
            (host, host_pointer)
        } else {
            (cpu, host_pointer)
        };
        require_equal_pointers(
            cpu,
            cpu_pointer,
            right,
            right_pointer,
            CODE,
            "CPU topology, affinity, or policy projection is inconsistent",
        )?;
    }
    let mask = |pointer: &str| -> Result<u64> {
        let text = cpu
            .pointer(pointer)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                XtaskError::integrity(CODE, format!("CPU affinity endpoint {pointer} is absent"))
            })?;
        u64::from_str_radix(text.strip_prefix("0x").unwrap_or_default(), 16).map_err(|_| {
            XtaskError::integrity(CODE, format!("CPU affinity endpoint {pointer} is invalid"))
        })
    };
    let process_mask = mask("/result/before/process_affinity_mask")?;
    let thread_mask = mask("/result/before/thread_group_mask")?;
    let system_mask = mask("/result/before/system_affinity_mask")?;
    let union_mask = mask("/result/before/logical_processor_union_mask")?;
    let selected_count = cpu
        .pointer("/result/before/selected_logical_processor_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            XtaskError::integrity(CODE, "CPU selected logical-processor count is absent")
        })?;
    if process_mask == 0
        || process_mask & !thread_mask != 0
        || process_mask & !system_mask != 0
        || process_mask & !union_mask != 0
        || u64::from(process_mask.count_ones()) != selected_count
    {
        return Err(XtaskError::integrity(
            CODE,
            "CPU selected affinity is not a nonempty subset of the stable topology masks",
        ));
    }
    let minimum = cpu
        .pointer("/result/before/clock_policy/minimum_processor_percent_ac")
        .and_then(Value::as_u64)
        .ok_or_else(|| XtaskError::integrity(CODE, "CPU minimum AC clock policy is absent"))?;
    let maximum = cpu
        .pointer("/result/before/clock_policy/maximum_processor_percent_ac")
        .and_then(Value::as_u64)
        .ok_or_else(|| XtaskError::integrity(CODE, "CPU maximum AC clock policy is absent"))?;
    if minimum > maximum {
        return Err(XtaskError::integrity(
            CODE,
            "CPU minimum AC processor policy exceeds its maximum",
        ));
    }
    Ok(())
}

fn validate_executed_tool_relations(
    evidence: &Value,
    host: &Value,
    native_probe: &Value,
) -> Result<()> {
    let commands = evidence
        .pointer("/commands")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P1A_TOOL_EXECUTION_RELATION_INVALID",
                "PASS evidence has no command records",
            )
        })?;
    for command in commands {
        let Some(program) = command.pointer("/argv/0").and_then(Value::as_str) else {
            return Err(XtaskError::integrity(
                "P1A_TOOL_EXECUTION_RELATION_INVALID",
                "command record has no executable identity",
            ));
        };
        let expected = match program {
            "${VSWHERE}" => host.pointer("/result/native_toolchain/vswhere"),
            "${RUSTC}" => host.pointer("/result/rust_toolchain/rustc"),
            "${CARGO}" | "cargo" => host.pointer("/result/rust_toolchain/cargo"),
            "${CL}" => {
                let is_cpp = command
                    .pointer("/argv")
                    .and_then(Value::as_array)
                    .is_some_and(|argv| argv.iter().any(|value| value.as_str() == Some("/TP")));
                host.pointer(if is_cpp {
                    "/result/native_toolchain/cpp_compiler"
                } else {
                    "/result/native_toolchain/c_compiler"
                })
            }
            "${LIB}" => host.pointer("/result/native_toolchain/archiver"),
            "${DUMPBIN}" => host.pointer("/result/native_toolchain/binary_inspector"),
            "${P1A_TEMP}/rust-target/p1a_abi_probe.exe" => {
                native_probe.pointer("/result/probe_executable")
            }
            "${GIT}" => {
                require_git_root_identity(command)?;
                None
            }
            other => {
                return Err(XtaskError::integrity(
                    "P1A_TOOL_EXECUTION_RELATION_INVALID",
                    format!("command plan contains an unbound executable {other}"),
                ));
            }
        };
        if let Some(expected) = expected {
            require_process_file_identity(command, expected, program)?;
        }
        if program == "${CL}" {
            let (frontend, frontend_module) = if command
                .pointer("/argv")
                .and_then(Value::as_array)
                .is_some_and(|argv| argv.iter().any(|value| value.as_str() == Some("/TC")))
            {
                ("/result/native_toolchain/c_frontend", "c1.dll")
            } else {
                ("/result/native_toolchain/cpp_frontend", "c1xx.dll")
            };
            require_module_file_identity(
                command,
                host.pointer(frontend).ok_or_else(|| {
                    XtaskError::integrity(
                        "P1A_TOOL_EXECUTION_RELATION_INVALID",
                        "host artifact omits the selected compiler frontend",
                    )
                })?,
                frontend_module,
            )?;
            require_module_file_identity(
                command,
                host.pointer("/result/native_toolchain/optimizer_codegen")
                    .ok_or_else(|| {
                        XtaskError::integrity(
                            "P1A_TOOL_EXECUTION_RELATION_INVALID",
                            "host artifact omits the optimizer/code generator",
                        )
                    })?,
                "c2.dll",
            )?;
        }
        if program == "${RUSTC}"
            && command
                .pointer("/argv")
                .and_then(Value::as_array)
                .is_some_and(|argv| {
                    argv.iter()
                        .any(|value| value.as_str() == Some("linker=${LINK}"))
                })
        {
            require_descendant_process_file_identity(
                command,
                host.pointer("/result/native_toolchain/linker")
                    .ok_or_else(|| {
                        XtaskError::integrity(
                            "P1A_TOOL_EXECUTION_RELATION_INVALID",
                            "host artifact omits the selected linker",
                        )
                    })?,
                "link.exe",
                "linker",
            )?;
        }
        if program == "${P1A_TEMP}/rust-target/p1a_abi_probe.exe" {
            for (module_name, pointer) in [
                ("ucrtbase.dll", "/result/runtime/ucrtbase"),
                ("vcruntime140.dll", "/result/runtime/vcruntime"),
            ] {
                require_module_file_identity(
                    command,
                    host.pointer(pointer).ok_or_else(|| {
                        XtaskError::integrity(
                            "P1A_RUNTIME_EXECUTION_RELATION_INVALID",
                            format!("host artifact omits {pointer}"),
                        )
                    })?,
                    module_name,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_command_timings(evidence: &Value, commands: &[Value]) -> Result<()> {
    const CODE: &str = "P1A_COMMAND_TIMING_INVALID";
    const MAX_CAPTURE_OVERHEAD_NS: i128 = 1_000_000_000;
    let generated = evidence
        .pointer("/generated_at_utc")
        .and_then(Value::as_str)
        .ok_or_else(|| XtaskError::integrity(CODE, "terminal evidence has no generation time"))?;
    let generated = parse_utc_nanos(generated)?;
    let mut previous_finished = None;
    let mut pre_admission_finished = None;
    let mut post_admission_started = None;
    for (index, command) in commands.iter().enumerate() {
        let id = command
            .pointer("/id")
            .and_then(Value::as_str)
            .unwrap_or("unknown command");
        let started = command
            .pointer("/started_at_utc")
            .and_then(Value::as_str)
            .ok_or_else(|| XtaskError::integrity(CODE, format!("{id} has no start timestamp")))?;
        let finished = command
            .pointer("/finished_at_utc")
            .and_then(Value::as_str)
            .ok_or_else(|| XtaskError::integrity(CODE, format!("{id} has no finish timestamp")))?;
        let started = parse_utc_nanos(started)?;
        let finished = parse_utc_nanos(finished)?;
        let duration = command
            .pointer("/duration_ns")
            .and_then(Value::as_u64)
            .filter(|duration| *duration > 0)
            .ok_or_else(|| {
                XtaskError::integrity(CODE, format!("{id} has no positive monotonic duration"))
            })?;
        let wall_elapsed = finished.checked_sub(started).ok_or_else(|| {
            XtaskError::integrity(CODE, format!("{id} finishes before it starts"))
        })?;
        let duration = i128::from(duration);
        if wall_elapsed < duration || wall_elapsed - duration > MAX_CAPTURE_OVERHEAD_NS {
            return Err(XtaskError::integrity(
                CODE,
                format!("{id} wall-clock interval does not bind its monotonic duration"),
            ));
        }
        if previous_finished.is_some_and(|previous| started < previous) {
            return Err(XtaskError::integrity(
                CODE,
                format!("{id} starts before the preceding command finished"),
            ));
        }
        if index == 11 {
            pre_admission_finished = Some(finished);
        } else if index == 12 {
            post_admission_started = Some(started);
        }
        previous_finished = Some(finished);
    }
    if pre_admission_finished.is_some_and(|finished| generated < finished)
        || post_admission_started.is_some_and(|started| generated > started)
    {
        return Err(XtaskError::integrity(
            CODE,
            "terminal evidence generation time is not between admission command C012 and post-admission command C013",
        ));
    }
    Ok(())
}

fn parse_utc_nanos(value: &str) -> Result<i128> {
    const CODE: &str = "P1A_COMMAND_TIMING_INVALID";
    let bytes = value.as_bytes();
    if bytes.len() != 30
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[29] != b'Z'
        || bytes.iter().enumerate().any(|(index, byte)| {
            !matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 29) && !byte.is_ascii_digit()
        })
    {
        return Err(XtaskError::integrity(
            CODE,
            "command timestamp is not exact UTC with nanosecond precision",
        ));
    }
    let field = |start: usize, end: usize| -> Result<u32> {
        value[start..end]
            .parse::<u32>()
            .map_err(|_| XtaskError::integrity(CODE, "command timestamp has an invalid field"))
    };
    let year = field(0, 4)?;
    let month = field(5, 7)?;
    let day = field(8, 10)?;
    let hour = field(11, 13)?;
    let minute = field(14, 16)?;
    let second = field(17, 19)?;
    let nanos = field(20, 29)?;
    let leap = |candidate: u32| {
        candidate.is_multiple_of(4)
            && (!candidate.is_multiple_of(100) || candidate.is_multiple_of(400))
    };
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(year) => 29,
        2 => 28,
        _ => 0,
    };
    if year < 1970 || day == 0 || day > days_in_month || hour > 23 || minute > 59 || second > 59 {
        return Err(XtaskError::integrity(
            CODE,
            "command timestamp contains an out-of-range UTC field",
        ));
    }
    let years = i128::from(year - 1970);
    let leap_days = i128::from((year - 1) / 4 - 1969 / 4)
        - i128::from((year - 1) / 100 - 1969 / 100)
        + i128::from((year - 1) / 400 - 1969 / 400);
    let month_days = [31_u32, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        [..month.saturating_sub(1) as usize]
        .iter()
        .copied()
        .map(i128::from)
        .sum::<i128>()
        + i128::from(month > 2 && leap(year));
    let days = years * 365 + leap_days + month_days + i128::from(day - 1);
    let seconds =
        ((days * 24 + i128::from(hour)) * 60 + i128::from(minute)) * 60 + i128::from(second);
    Ok(seconds * 1_000_000_000 + i128::from(nanos))
}

fn validate_transcript_relations(
    run_root: &Path,
    evidence: &Value,
    host: &Value,
    native_probe: &Value,
) -> Result<()> {
    validate_vswhere_transcript(&command_stdout(run_root, evidence, "C013")?, host)?;
    validate_rustc_transcript(&command_stdout(run_root, evidence, "C014")?, host)?;
    validate_cargo_transcript(&command_stdout(run_root, evidence, "C015")?, host)?;
    validate_cargo_tree_transcript(&command_stdout(run_root, evidence, "C016")?, host)?;
    validate_abi_transcript(&command_stdout(run_root, evidence, "C022")?, native_probe)?;
    validate_dumpbin_transcript(&command_stdout(run_root, evidence, "C023")?, native_probe)
}

fn command_stdout(run_root: &Path, evidence: &Value, id: &str) -> Result<Vec<u8>> {
    const CODE: &str = "P1A_TRANSCRIPT_RELATION_INVALID";
    let matches = evidence
        .pointer("/commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|command| command.pointer("/id").and_then(Value::as_str) == Some(id))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(XtaskError::integrity(
            CODE,
            format!("transcript binding does not identify exactly one {id}"),
        ));
    }
    let reference: FileRef = serde_json::from_value(
        matches[0]
            .pointer("/stdout")
            .cloned()
            .ok_or_else(|| XtaskError::integrity(CODE, format!("{id} has no stdout reference")))?,
    )
    .map_err(|error| {
        XtaskError::integrity(CODE, format!("{id} stdout reference is malformed: {error}"))
    })?;
    if reference.path != format!("commands/{id}.stdout.txt") {
        return Err(XtaskError::integrity(
            CODE,
            format!("{id} stdout transcript path is not ID-bound"),
        ));
    }
    validate_file_ref(run_root, &reference)?;
    fs::read(run_root.join(&reference.path)).io_context(
        CODE,
        format!("could not read the receipt-bound {id} stdout transcript"),
    )
}

fn transcript_text<'a>(bytes: &'a [u8], label: &str) -> Result<&'a str> {
    std::str::from_utf8(bytes).map_err(|_| {
        XtaskError::integrity(
            "P1A_TRANSCRIPT_RELATION_INVALID",
            format!("{label} transcript is not UTF-8"),
        )
    })
}

fn validate_vswhere_transcript(bytes: &[u8], host: &Value) -> Result<()> {
    const CODE: &str = "P1A_TRANSCRIPT_RELATION_INVALID";
    let observed: Value = serde_json::from_slice(bytes).map_err(|error| {
        XtaskError::integrity(
            CODE,
            format!("C013 vswhere transcript is invalid JSON: {error}"),
        )
    })?;
    let observed = observed.as_array().ok_or_else(|| {
        XtaskError::integrity(CODE, "C013 vswhere transcript is not a candidate array")
    })?;
    let claimed = host
        .pointer("/result/native_toolchain/visual_studio_candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| XtaskError::integrity(CODE, "host artifact omits VS candidates"))?;
    if observed.len() != claimed.len() {
        return Err(XtaskError::integrity(
            CODE,
            "C013 candidate count differs from the host artifact",
        ));
    }
    let mut observed_by_id = BTreeMap::new();
    for candidate in observed {
        let id = candidate
            .pointer("/instanceId")
            .and_then(Value::as_str)
            .ok_or_else(|| XtaskError::integrity(CODE, "C013 candidate omits instanceId"))?;
        if observed_by_id.insert(id, candidate).is_some() {
            return Err(XtaskError::integrity(
                CODE,
                "C013 candidate inventory contains duplicate instance IDs",
            ));
        }
    }
    for claim in claimed {
        let id = claim
            .pointer("/instance_id")
            .and_then(Value::as_str)
            .ok_or_else(|| XtaskError::integrity(CODE, "host VS candidate omits instance ID"))?;
        let observed = observed_by_id.get(id).ok_or_else(|| {
            XtaskError::integrity(CODE, format!("C013 transcript omits VS candidate {id}"))
        })?;
        for (observed_pointer, claim_pointer) in [
            ("/productId", "/product_id"),
            ("/installationVersion", "/installation_version"),
            ("/installationPath", "/installation_path"),
            ("/isComplete", "/complete"),
            ("/isLaunchable", "/launchable"),
            ("/isRebootRequired", "/reboot_required"),
        ] {
            require_equal_pointers(
                observed,
                observed_pointer,
                claim,
                claim_pointer,
                CODE,
                "C013 candidate claim differs from the vswhere transcript",
            )?;
        }
    }
    Ok(())
}

fn unique_version_field<'a>(text: &'a str, label: &str) -> Result<&'a str> {
    const CODE: &str = "P1A_TRANSCRIPT_RELATION_INVALID";
    let prefix = format!("{label}:");
    let matches = text
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix).map(str::trim))
        .collect::<Vec<_>>();
    if matches.len() != 1 || matches[0].is_empty() {
        return Err(XtaskError::integrity(
            CODE,
            format!("C014 transcript does not contain exactly one {label} field"),
        ));
    }
    Ok(matches[0])
}

fn validate_rustc_transcript(bytes: &[u8], host: &Value) -> Result<()> {
    const CODE: &str = "P1A_TRANSCRIPT_RELATION_INVALID";
    let text = transcript_text(bytes, "C014 rustc")?;
    for (label, pointer) in [
        ("release", "/result/rust_toolchain/release"),
        ("host", "/result/rust_toolchain/host"),
        ("LLVM version", "/result/rust_toolchain/llvm_version"),
    ] {
        if host.pointer(pointer).and_then(Value::as_str) != Some(unique_version_field(text, label)?)
        {
            return Err(XtaskError::integrity(
                CODE,
                format!("C014 {label} differs from the host artifact"),
            ));
        }
    }
    let release = unique_version_field(text, "release")?;
    let parts = release
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| XtaskError::integrity(CODE, "C014 release is not three-part semver"))?;
    if parts.len() != 3
        || host
            .pointer("/result/rust_toolchain/release_semver/major")
            .and_then(Value::as_u64)
            != parts.first().copied()
        || host
            .pointer("/result/rust_toolchain/release_semver/minor")
            .and_then(Value::as_u64)
            != parts.get(1).copied()
        || host
            .pointer("/result/rust_toolchain/release_semver/patch")
            .and_then(Value::as_u64)
            != parts.get(2).copied()
    {
        return Err(XtaskError::integrity(
            CODE,
            "C014 release text differs from its structured semver claim",
        ));
    }
    Ok(())
}

fn validate_cargo_transcript(bytes: &[u8], host: &Value) -> Result<()> {
    const CODE: &str = "P1A_TRANSCRIPT_RELATION_INVALID";
    let text = transcript_text(bytes, "C015 cargo")?;
    let first = text.lines().next().unwrap_or_default();
    let fields = first.split_whitespace().collect::<Vec<_>>();
    let release = host
        .pointer("/result/rust_toolchain/cargo/version")
        .and_then(Value::as_str);
    if fields.len() < 2
        || fields[0] != "cargo"
        || Some(fields[1]) != release
        || host
            .pointer("/result/rust_toolchain/release")
            .and_then(Value::as_str)
            != release
    {
        return Err(XtaskError::integrity(
            CODE,
            "C015 Cargo version differs from the coherent Rust toolchain claim",
        ));
    }
    Ok(())
}

fn parse_cargo_tree_transcript(text: &str) -> Result<Vec<String>> {
    const CODE: &str = "P1A_TRANSCRIPT_RELATION_INVALID";
    let mut packages = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let name = fields.next().unwrap_or_default();
        let version = fields
            .next()
            .and_then(|value| value.strip_prefix('v'))
            .unwrap_or_default();
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
                CODE,
                "C016 transcript is not prefix-free Cargo package output",
            ));
        }
        packages.push(format!("{name}@{version}"));
    }
    packages.sort();
    packages.dedup();
    Ok(packages)
}

fn validate_cargo_tree_transcript(bytes: &[u8], host: &Value) -> Result<()> {
    const CODE: &str = "P1A_TRANSCRIPT_RELATION_INVALID";
    let observed = parse_cargo_tree_transcript(transcript_text(bytes, "C016 cargo tree")?)?;
    let claimed = host
        .pointer("/result/cargo_build_policy/activated_packages")
        .and_then(Value::as_array)
        .ok_or_else(|| XtaskError::integrity(CODE, "host artifact omits activated packages"))?
        .iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| XtaskError::integrity(CODE, "host activated package is not a string"))?;
    if observed != claimed {
        return Err(XtaskError::integrity(
            CODE,
            "C016 activated package set differs from the host artifact",
        ));
    }
    Ok(())
}

fn validate_abi_transcript(bytes: &[u8], native_probe: &Value) -> Result<()> {
    const CODE: &str = "P1A_TRANSCRIPT_RELATION_INVALID";
    let observed = transcript_text(bytes, "C022 ABI probe")?;
    let observed_sha256 = hash::bytes(bytes);
    for probe in ["rust_c_probe", "rust_cpp_probe"] {
        let prefix = format!("/result/{probe}");
        if native_probe
            .pointer(&format!("{prefix}/observed_stdout"))
            .and_then(Value::as_str)
            != Some(observed)
            || native_probe
                .pointer(&format!("{prefix}/output_sha256"))
                .and_then(Value::as_str)
                != Some(&observed_sha256)
        {
            return Err(XtaskError::integrity(
                CODE,
                format!("C022 transcript differs from {probe} claims"),
            ));
        }
    }
    Ok(())
}

fn parse_dumpbin_transcript(bytes: &[u8]) -> Result<Vec<String>> {
    const CODE: &str = "P1A_TRANSCRIPT_RELATION_INVALID";
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    if !(lower.contains("machine (x64)") || lower.contains("8664 machine")) {
        return Err(XtaskError::integrity(
            CODE,
            "C023 transcript does not identify an AMD64 PE machine",
        ));
    }
    let mut imports = text
        .lines()
        .filter_map(|line| {
            let value = line.trim();
            (value.len() > 4
                && value.to_ascii_lowercase().ends_with(".dll")
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)))
            .then(|| value.to_ascii_lowercase())
        })
        .collect::<Vec<_>>();
    imports.sort();
    imports.dedup();
    if imports.is_empty() {
        return Err(XtaskError::integrity(
            CODE,
            "C023 transcript contains no imported DLLs",
        ));
    }
    Ok(imports)
}

fn validate_dumpbin_transcript(bytes: &[u8], native_probe: &Value) -> Result<()> {
    const CODE: &str = "P1A_TRANSCRIPT_RELATION_INVALID";
    let imports = parse_dumpbin_transcript(bytes)?;
    let claimed = native_probe
        .pointer("/result/binary_audit/imports")
        .and_then(Value::as_array)
        .ok_or_else(|| XtaskError::integrity(CODE, "native artifact omits imports"))?
        .iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| XtaskError::integrity(CODE, "native import is not a string"))?;
    if native_probe
        .pointer("/result/binary_audit/pe_machine")
        .and_then(Value::as_str)
        != Some("AMD64")
        || imports != claimed
    {
        return Err(XtaskError::integrity(
            CODE,
            "C023 machine or import claims differ from the dumpbin transcript",
        ));
    }
    Ok(())
}

fn expected_sha_and_bytes<'a>(value: &'a Value, label: &str) -> Result<(&'a str, u64)> {
    let sha256 = value.pointer("/sha256").and_then(Value::as_str);
    let bytes = value.pointer("/bytes").and_then(Value::as_u64);
    match (sha256, bytes) {
        (Some(sha256), Some(bytes)) if hash::is_lower_sha256(sha256) && bytes > 0 => {
            Ok((sha256, bytes))
        }
        _ => Err(XtaskError::integrity(
            "P1A_TOOL_EXECUTION_RELATION_INVALID",
            format!("receipt identity for {label} is malformed"),
        )),
    }
}

fn require_process_file_identity(command: &Value, expected: &Value, label: &str) -> Result<()> {
    let (sha256, bytes) = expected_sha_and_bytes(expected, label)?;
    let root = root_process_identity(command)?;
    if root.pointer("/executable_sha256").and_then(Value::as_str) != Some(sha256)
        || root.pointer("/executable_bytes").and_then(Value::as_u64) != Some(bytes)
    {
        return Err(XtaskError::integrity(
            "P1A_TOOL_EXECUTION_RELATION_INVALID",
            format!("process audit root is not the receipt-bound {label}"),
        ));
    }
    Ok(())
}

fn require_descendant_process_file_identity(
    command: &Value,
    expected: &Value,
    executable_name: &str,
    label: &str,
) -> Result<()> {
    let (sha256, bytes) = expected_sha_and_bytes(expected, label)?;
    let audit = command.pointer("/process_audit").ok_or_else(|| {
        XtaskError::integrity(
            "P1A_TOOL_EXECUTION_RELATION_INVALID",
            "command has no process audit",
        )
    })?;
    let root_process_id = audit.pointer("/root_process_id").and_then(Value::as_u64);
    let root_creation_time = audit
        .pointer("/root_creation_time_100ns")
        .and_then(Value::as_u64);
    let mut matches = audit
        .pointer("/process_identities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|identity| {
            let is_root = identity.pointer("/process_id").and_then(Value::as_u64)
                == root_process_id
                && identity
                    .pointer("/creation_time_100ns")
                    .and_then(Value::as_u64)
                    == root_creation_time;
            !is_root
                && identity
                    .pointer("/executable_name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.eq_ignore_ascii_case(executable_name))
                && identity
                    .pointer("/executable_sha256")
                    .and_then(Value::as_str)
                    == Some(sha256)
                && identity
                    .pointer("/executable_bytes")
                    .and_then(Value::as_u64)
                    == Some(bytes)
        });
    if root_process_id.is_none()
        || root_creation_time.is_none()
        || matches.next().is_none()
        || matches.next().is_some()
    {
        return Err(XtaskError::integrity(
            "P1A_TOOL_EXECUTION_RELATION_INVALID",
            format!("process audit does not contain exactly one receipt-bound {label} descendant"),
        ));
    }
    Ok(())
}

fn root_process_identity(command: &Value) -> Result<&Value> {
    let audit = command.pointer("/process_audit").ok_or_else(|| {
        XtaskError::integrity(
            "P1A_TOOL_EXECUTION_RELATION_INVALID",
            "command has no process audit",
        )
    })?;
    let root_process_id = audit.pointer("/root_process_id").and_then(Value::as_u64);
    let root_creation_time = audit
        .pointer("/root_creation_time_100ns")
        .and_then(Value::as_u64);
    let mut root_matches = audit
        .pointer("/process_identities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|identity| {
            identity.pointer("/process_id").and_then(Value::as_u64) == root_process_id
                && identity
                    .pointer("/creation_time_100ns")
                    .and_then(Value::as_u64)
                    == root_creation_time
        });
    let root = root_matches.next();
    if root_process_id.is_none()
        || root_creation_time.is_none()
        || root.is_none()
        || root_matches.next().is_some()
    {
        return Err(XtaskError::integrity(
            "P1A_TOOL_EXECUTION_RELATION_INVALID",
            "process audit does not identify exactly one creation-bound root",
        ));
    }
    Ok(root.expect("root presence checked"))
}

fn require_git_root_identity(command: &Value) -> Result<()> {
    let root = root_process_identity(command)?;
    let valid = root
        .pointer("/executable_name")
        .and_then(Value::as_str)
        .is_some_and(|name| name.eq_ignore_ascii_case("git.exe"))
        && root.pointer("/path_class").and_then(Value::as_str) == Some("root_tool_directory")
        && root
            .pointer("/canonical_path_sha256")
            .and_then(Value::as_str)
            .is_some_and(hash::is_lower_sha256)
        && root
            .pointer("/executable_sha256")
            .and_then(Value::as_str)
            .is_some_and(hash::is_lower_sha256)
        && root
            .pointer("/executable_bytes")
            .and_then(Value::as_u64)
            .is_some_and(|bytes| bytes > 0);
    if !valid {
        return Err(XtaskError::integrity(
            "P1A_TOOL_EXECUTION_RELATION_INVALID",
            "Git command root is not an exact audited git.exe root-tool identity",
        ));
    }
    Ok(())
}

fn require_module_file_identity(
    command: &Value,
    expected: &Value,
    module_name: &str,
) -> Result<()> {
    let (sha256, bytes) = expected_sha_and_bytes(expected, module_name)?;
    let matches = command
        .pointer("/process_audit/loaded_modules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|module| {
            module
                .pointer("/module_name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case(module_name))
                && module.pointer("/module_sha256").and_then(Value::as_str) == Some(sha256)
                && module.pointer("/module_bytes").and_then(Value::as_u64) == Some(bytes)
        })
        .count();
    if matches == 0 {
        return Err(XtaskError::integrity(
            "P1A_MODULE_EXECUTION_RELATION_INVALID",
            format!("process audit did not load receipt-bound {module_name}"),
        ));
    }
    Ok(())
}

fn require_embedded_reference(
    run_root: &Path,
    evidence: &Value,
    pointer: &str,
    expected: Option<&FileRef>,
) -> Result<()> {
    let expected = expected.ok_or_else(|| {
        XtaskError::integrity(
            "P1A_EVIDENCE_REFERENCE_INVALID",
            format!("terminal evidence omits artifact required by {pointer}"),
        )
    })?;
    let actual: FileRef =
        serde_json::from_value(evidence.pointer(pointer).cloned().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_EVIDENCE_REFERENCE_INVALID",
                format!("terminal evidence omits {pointer}"),
            )
        })?)
        .map_err(|error| {
            XtaskError::integrity(
                "P1A_EVIDENCE_REFERENCE_INVALID",
                format!("terminal evidence {pointer} is malformed: {error}"),
            )
        })?;
    if &actual != expected {
        return Err(XtaskError::integrity(
            "P1A_EVIDENCE_REFERENCE_INVALID",
            format!("terminal evidence {pointer} differs from its artifact reference"),
        ));
    }
    validate_file_ref(run_root, &actual)
}

fn validate_file_ref(run_root: &Path, reference: &FileRef) -> Result<()> {
    let relative = Path::new(&reference.path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(XtaskError::integrity(
            "P1A_FILE_REF_PATH_INVALID",
            "receipt file reference is not a contained normal relative path",
        ));
    }
    let path = run_root.join(relative);
    let metadata = fs::symlink_metadata(&path).io_context(
        "P1A_FILE_REF_MISSING",
        format!("could not inspect receipt reference {}", reference.path),
    )?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != reference.bytes
    {
        return Err(XtaskError::integrity(
            "P1A_FILE_REF_IDENTITY_INVALID",
            format!(
                "receipt reference {} has the wrong file identity",
                reference.path
            ),
        ));
    }
    hash::require_file(&path, &reference.sha256, "P1A_FILE_REF_HASH_MISMATCH")
}

fn validate_process_audit(command: &Value) -> Result<()> {
    let status = command
        .pointer("/status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(audit_value) = command
        .get("process_audit")
        .filter(|value| !value.is_null())
    else {
        if status == "PASS" {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_AUDIT_MISSING",
                "passing command has no process audit",
            ));
        }
        return Ok(());
    };
    let audit: ProcessAudit = serde_json::from_value(audit_value.clone()).map_err(|error| {
        XtaskError::integrity(
            "P1A_PROCESS_AUDIT_INVALID",
            format!("process audit is malformed: {error}"),
        )
    })?;
    let processes = audit
        .process_identities
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let root_identity = audit.process_identities.iter().filter(|process| {
        process.process_id == audit.root_process_id
            && process.creation_time_100ns == audit.root_creation_time_100ns
    });
    if root_identity.count() != 1 {
        return Err(XtaskError::integrity(
            "P1A_PROCESS_AUDIT_RELATION_INVALID",
            "process audit root does not identify exactly one creation-bound process",
        ));
    }
    let modules = audit
        .loaded_modules
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    validate_process_path_classes(command, &audit, &processes, &modules)?;
    let expected_executable_names = audit
        .process_identities
        .iter()
        .map(|process| process.executable_name.clone())
        .collect::<BTreeSet<_>>();
    let recorded_executable_names = audit
        .executable_names
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if processes.len() != audit.process_identities.len()
        || modules.len() != audit.loaded_modules.len()
        || recorded_executable_names.len() != audit.executable_names.len()
        || recorded_executable_names != expected_executable_names
        || audit.covered_process_count > audit.audited_process_count
        || audit.process_identities.len() != audit.audited_process_count
        || audit.successful_snapshots != audit.audited_process_count
    {
        return Err(XtaskError::integrity(
            "P1A_PROCESS_AUDIT_RELATION_INVALID",
            "process audit counts or identities are inconsistent",
        ));
    }
    for process in &processes {
        if !has_executable_module(process, &modules) {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_AUDIT_RELATION_INVALID",
                "audited process lacks its exact executable module identity",
            ));
        }
    }
    let passed = command.pointer("/exit_code").and_then(Value::as_i64) == Some(0)
        && audit.atomic_job_assignment
        && audit.audited_process_count > 0
        && audit.covered_process_count == audit.audited_process_count
        && audit.successful_snapshots > 0
        && audit.exit_races == 0
        && audit.process_tree_terminated
        && !audit.unexpected_descendants
        && audit.forbidden_processes.is_empty()
        && audit.forbidden_modules.is_empty()
        && !audit.timed_out;
    if (status == "PASS") != passed {
        return Err(XtaskError::integrity(
            "P1A_PROCESS_AUDIT_STATUS_INVALID",
            "command status does not match its process-audit result",
        ));
    }
    Ok(())
}

fn has_executable_module(
    process: &AuditedProcessIdentity,
    modules: &BTreeSet<LoadedModuleIdentity>,
) -> bool {
    modules.iter().any(|module| {
        module.process_id == process.process_id
            && module.creation_time_100ns == process.creation_time_100ns
            && module.canonical_path_sha256 == process.canonical_path_sha256
            && module.module_sha256 == process.executable_sha256
            && module.module_bytes == process.executable_bytes
            && module.path_class == process.path_class
    })
}

fn validate_process_path_classes(
    command: &Value,
    audit: &ProcessAudit,
    processes: &BTreeSet<AuditedProcessIdentity>,
    modules: &BTreeSet<LoadedModuleIdentity>,
) -> Result<()> {
    const CODE: &str = "P1A_PROCESS_PATH_CLASS_RELATION_INVALID";
    const QUALIFIED_TOOL_FILE: &str = "qualified_tool_file";
    const WINDOWS_SYSWOW64: &str = "windows_syswow64";
    const VSWHERE_SETUP_MODULE: &str = "Microsoft.VisualStudio.Setup.Configuration.Native.dll";

    let valid_class = |class: &str| {
        matches!(
            class,
            "windows_system32"
                | WINDOWS_SYSWOW64
                | "root_tool_directory"
                | QUALIFIED_TOOL_FILE
                | "qualified_tool_root"
                | "qualified_working_tree"
        )
    };
    for process in processes {
        if !valid_class(&process.path_class) {
            return Err(XtaskError::integrity(
                CODE,
                "process audit contains an unknown executable path class",
            ));
        }
        if matches!(
            process.path_class.as_str(),
            WINDOWS_SYSWOW64 | QUALIFIED_TOOL_FILE
        ) {
            return Err(XtaskError::integrity(
                CODE,
                "module-only path class was applied to a process executable",
            ));
        }
    }

    let is_vswhere = command.pointer("/argv/0").and_then(Value::as_str) == Some("${VSWHERE}");
    let is_root_module = |module: &LoadedModuleIdentity| {
        module.process_id == audit.root_process_id
            && module.creation_time_100ns == audit.root_creation_time_100ns
    };
    let mut syswow64_modules = 0usize;
    let mut qualified_tool_files = 0usize;
    for module in modules {
        if !valid_class(&module.path_class) {
            return Err(XtaskError::integrity(
                CODE,
                "process audit contains an unknown module path class",
            ));
        }
        match module.path_class.as_str() {
            WINDOWS_SYSWOW64 => {
                if !is_vswhere
                    || !is_root_module(module)
                    || !module.module_name.to_ascii_lowercase().ends_with(".dll")
                {
                    return Err(XtaskError::integrity(
                        CODE,
                        "SysWOW64 classification is not a root-bound vswhere DLL module",
                    ));
                }
                syswow64_modules += 1;
            }
            QUALIFIED_TOOL_FILE => {
                qualified_tool_files += 1;
                if !is_vswhere
                    || !is_root_module(module)
                    || !module
                        .module_name
                        .eq_ignore_ascii_case(VSWHERE_SETUP_MODULE)
                {
                    return Err(XtaskError::integrity(
                        CODE,
                        "exact qualified file is not the root-bound vswhere setup module",
                    ));
                }
            }
            _ => {}
        }
    }
    let expected_qualified_tool_files = usize::from(is_vswhere);
    if qualified_tool_files != expected_qualified_tool_files || is_vswhere != (syswow64_modules > 0)
    {
        return Err(XtaskError::integrity(
            CODE,
            "process audit does not contain the exact command-bound WOW64 runtime class set",
        ));
    }
    Ok(())
}

fn read_json(path: &Path, code: &'static str) -> Result<Value> {
    let bytes = fs::read(path).io_context(code, format!("could not read {}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        XtaskError::integrity(code, format!("invalid JSON at {}: {error}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn digest(character: char) -> String {
        character.to_string().repeat(64)
    }

    fn tree(character: char) -> Value {
        json!({
            "sha256": digest(character),
            "directory_count": 1,
            "file_count": 1,
            "total_bytes": 1
        })
    }

    fn host_semantic_fixture() -> Value {
        let packages = vec!["alpha 1.0.0", "beta 2.0.0"];
        let package_digest = hash::bytes(packages.join("\n").as_bytes());
        let snapshot = json!({
            "selection_manifest_sha256": digest('1'),
            "qualified_file_manifest_sha256": digest('2'),
            "qualified_file_count": 19,
            "qualified_file_total_bytes": 19,
            "msvc_include_tree": tree('3'),
            "msvc_x64_lib_tree": tree('4'),
            "windows_sdk_include_tree": tree('5'),
            "windows_sdk_x64_lib_tree": tree('6'),
            "bundle_sha256": digest('7')
        });
        json!({
            "result": {
                "cpu": {
                    "processor_groups": 1,
                    "logical_processors": 32,
                    "logical_processor_union_mask": "0x00000000FFFFFFFF",
                    "core_topology_sha256": digest('8')
                },
                "windows_sdk": {
                    "include_tree_sha256": digest('5'),
                    "x64_lib_tree_sha256": digest('6')
                },
                "native_toolchain": {
                    "visual_studio_candidates": [{
                        "instance_id": "vs-instance",
                        "product_id": "Microsoft.VisualStudio.Product.Community",
                        "installation_version": "17.14.1",
                        "installation_path": "${VS_INSTALL}",
                        "complete": true,
                        "launchable": true,
                        "reboot_required": false
                    }],
                    "selected_visual_studio_instance_id": "vs-instance",
                    "selected_visual_studio_product_id": "Microsoft.VisualStudio.Product.Community",
                    "visual_studio_installation_path": "${VS_INSTALL}",
                    "visual_studio_edition": "Microsoft.VisualStudio.Product.Community",
                    "visual_studio_version": "17.14.1",
                    "visual_studio_complete": true,
                    "visual_studio_launchable": true
                },
                "toolchain_identity_stability": {
                    "before": snapshot,
                    "after": snapshot,
                    "stable": true
                },
                "cargo_build_policy": {
                    "activated_packages": packages,
                    "activated_packages_sha256": package_digest
                },
                "input_stability": {
                    "before_sha256": digest('9'),
                    "after_sha256": digest('9'),
                    "stable": true
                }
            }
        })
    }

    fn native_semantic_fixture() -> Value {
        json!({
            "result": {
                "binary_audit": {
                    "imports": ["kernel32.dll", "vcruntime140.dll"],
                    "import_classifications": [
                        {"name": "kernel32.dll", "class": "windows_system"},
                        {"name": "vcruntime140.dll", "class": "msvc_release_runtime"}
                    ]
                },
                "input_stability": {
                    "before_sha256": digest('9'),
                    "after_sha256": digest('9'),
                    "stable": true
                }
            }
        })
    }

    fn cpu_semantic_fixture() -> Value {
        let snapshot = json!({
            "processor_group_count": 1,
            "active_logical_processors": 32,
            "logical_processor_union_mask": "0x00000000FFFFFFFF",
            "core_topology_sha256": digest('8'),
            "process_group": 0,
            "thread_group_mask": "0xFFFFFFFF",
            "process_affinity_mask": "0xFFFFFFFF",
            "system_affinity_mask": "0xFFFFFFFF",
            "selected_logical_processor_count": 32,
            "power_scheme_guid": "381b4222-f694-41f0-9685-ff5bb260df2e",
            "power_scheme_name": "Balanced",
            "ac_line_status": "online",
            "power_value_source": "ac",
            "clock_policy": {
                "minimum_processor_percent_ac": 5,
                "maximum_processor_percent_ac": 100,
                "boost_mode_ac": 2,
                "energy_performance_preference_ac": 50
            }
        });
        json!({
            "policy": {
                "selected_processor_group": 0,
                "selected_affinity_mask": "0xFFFFFFFF"
            },
            "result": {
                "before": snapshot,
                "after": snapshot,
                "topology_stable": true,
                "affinity_stable": true,
                "power_policy_stable": true
            }
        })
    }

    fn audit_command_fixture() -> Value {
        json!({
            "status": "PASS",
            "exit_code": 0,
            "process_audit": {
                "audit_method": "test",
                "atomic_job_assignment": true,
                "root_process_id": 1,
                "root_creation_time_100ns": 2,
                "audited_process_count": 1,
                "covered_process_count": 1,
                "successful_snapshots": 1,
                "exit_races": 0,
                "executable_names": ["tool.exe"],
                "process_identities": [{
                    "process_id": 1,
                    "creation_time_100ns": 2,
                    "executable_name": "tool.exe",
                    "canonical_path_sha256": digest('a'),
                    "executable_sha256": digest('b'),
                    "executable_bytes": 3,
                    "path_class": "qualified_tool_root"
                }],
                "loaded_modules": [{
                    "process_id": 1,
                    "creation_time_100ns": 2,
                    "module_name": "tool.exe",
                    "canonical_path_sha256": digest('a'),
                    "module_sha256": digest('b'),
                    "module_bytes": 3,
                    "path_class": "qualified_tool_root"
                }],
                "forbidden_processes": [],
                "forbidden_modules": [],
                "process_tree_terminated": true,
                "unexpected_descendants": false,
                "qualified_tool_descendants_cleaned": true,
                "timed_out": false
            }
        })
    }

    #[test]
    fn evidence_schema_closes_the_process_path_class_vocabulary() {
        let schema: Value = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docs/schemas/P1A-prototype-v2/python-slm-p1a-phase-evidence-v1.schema.json"
        )))
        .unwrap();
        let path_class = schema.pointer("/definitions/auditedPathClass").unwrap();
        for class in [
            "windows_system32",
            "windows_syswow64",
            "root_tool_directory",
            "qualified_tool_file",
            "qualified_tool_root",
            "qualified_working_tree",
        ] {
            json_schema::validate(path_class, &json!(class), "P1A_TEST_PATH_CLASS_INVALID")
                .unwrap();
        }
        assert_eq!(
            json_schema::validate(
                path_class,
                &json!("unreviewed_future_root"),
                "P1A_TEST_PATH_CLASS_INVALID",
            )
            .unwrap_err()
            .code,
            "P1A_TEST_PATH_CLASS_INVALID"
        );
    }

    #[test]
    fn file_refs_reject_parent_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let reference = FileRef {
            path: "../escape".to_owned(),
            sha256: "0".repeat(64),
            bytes: 0,
        };
        assert_eq!(
            validate_file_ref(temp.path(), &reference).unwrap_err().code,
            "P1A_FILE_REF_PATH_INVALID"
        );
    }

    #[test]
    fn cl_frontends_and_codegen_are_bound_as_loaded_modules() {
        let sha = |character: char| character.to_string().repeat(64);
        let identity = |character: char| {
            json!({
                "sha256": sha(character),
                "bytes": 1024
            })
        };
        let process_identity = |character: char| {
            json!({
                "process_id": 1,
                "creation_time_100ns": 2,
                "executable_sha256": sha(character),
                "executable_bytes": 1024
            })
        };
        let module_identity = |name: &str, character: char| {
            json!({
                "module_name": name,
                "module_sha256": sha(character),
                "module_bytes": 1024
            })
        };
        let command =
            |language_flag: &str, compiler_sha: char, frontend_name: &str, frontend_sha: char| {
                json!({
                    "argv": ["${CL}", language_flag],
                    "process_audit": {
                        "root_process_id": 1,
                        "root_creation_time_100ns": 2,
                        "process_identities": [process_identity(compiler_sha)],
                        "loaded_modules": [
                            module_identity(frontend_name, frontend_sha),
                            module_identity("c2.dll", 'd')
                        ]
                    }
                })
            };
        let host = json!({
            "result": {
                "native_toolchain": {
                    "c_compiler": identity('a'),
                    "cpp_compiler": identity('e'),
                    "c_frontend": identity('b'),
                    "cpp_frontend": identity('c'),
                    "optimizer_codegen": identity('d')
                }
            }
        });
        let mut evidence = json!({
            "commands": [
                command("/TC", 'a', "c1.dll", 'b'),
                command("/TP", 'e', "c1xx.dll", 'c')
            ]
        });

        validate_executed_tool_relations(&evidence, &host, &json!({})).unwrap();

        let mut wrong_cpp = evidence.clone();
        wrong_cpp["commands"][1]["process_audit"]["process_identities"] =
            json!([process_identity('a')]);
        assert_eq!(
            validate_executed_tool_relations(&wrong_cpp, &host, &json!({}))
                .unwrap_err()
                .code,
            "P1A_TOOL_EXECUTION_RELATION_INVALID"
        );

        let first = evidence.pointer_mut("/commands/0/process_audit").unwrap();
        let mut frontend_process = process_identity('b');
        frontend_process["process_id"] = json!(2);
        frontend_process["creation_time_100ns"] = json!(3);
        let mut codegen_process = process_identity('d');
        codegen_process["process_id"] = json!(3);
        codegen_process["creation_time_100ns"] = json!(4);
        first["process_identities"] =
            json!([process_identity('a'), frontend_process, codegen_process]);
        first["loaded_modules"] = json!([]);
        assert_eq!(
            validate_executed_tool_relations(&evidence, &host, &json!({}))
                .unwrap_err()
                .code,
            "P1A_MODULE_EXECUTION_RELATION_INVALID"
        );
    }

    #[test]
    fn rust_linker_is_bound_as_a_creation_bound_descendant() {
        let mut command = json!({
            "argv": ["${RUSTC}", "linker=${LINK}"],
            "process_audit": {
                "root_process_id": 1,
                "root_creation_time_100ns": 2,
                "process_identities": [
                    {
                        "process_id": 1,
                        "creation_time_100ns": 2,
                        "executable_name": "rustc.exe",
                        "executable_sha256": digest('a'),
                        "executable_bytes": 10
                    },
                    {
                        "process_id": 3,
                        "creation_time_100ns": 4,
                        "executable_name": "link.exe",
                        "executable_sha256": digest('b'),
                        "executable_bytes": 20
                    }
                ]
            }
        });
        let host = json!({
            "result": {
                "rust_toolchain": {"rustc": {"sha256": digest('a'), "bytes": 10}},
                "native_toolchain": {"linker": {"sha256": digest('b'), "bytes": 20}}
            }
        });
        validate_executed_tool_relations(
            &json!({"commands": [command.clone()]}),
            &host,
            &json!({}),
        )
        .unwrap();

        command["process_audit"]["process_identities"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert_eq!(
            validate_executed_tool_relations(&json!({"commands": [command]}), &host, &json!({}))
                .unwrap_err()
                .code,
            "P1A_TOOL_EXECUTION_RELATION_INVALID"
        );
    }

    #[test]
    fn git_command_is_bound_to_its_creation_bound_root_identity() {
        let mut command = audit_command_fixture();
        command["process_audit"]["process_identities"][0]["executable_name"] = json!("git.exe");
        command["process_audit"]["process_identities"][0]["path_class"] =
            json!("root_tool_directory");
        require_git_root_identity(&command).unwrap();

        command["process_audit"]["root_creation_time_100ns"] = json!(3);
        assert_eq!(
            require_git_root_identity(&command).unwrap_err().code,
            "P1A_TOOL_EXECUTION_RELATION_INVALID"
        );
    }

    #[test]
    fn host_relations_reject_snapshot_sdk_input_and_package_mutations() {
        let host = host_semantic_fixture();
        validate_host_semantic_relations(&host).unwrap();

        let mut mutated = host.clone();
        mutated["result"]["toolchain_identity_stability"]["after"]["bundle_sha256"] =
            json!(digest('e'));
        assert_eq!(
            validate_host_semantic_relations(&mutated).unwrap_err().code,
            "P1A_HOST_SEMANTIC_RELATION_INVALID"
        );

        let mut mutated = host.clone();
        mutated["result"]["windows_sdk"]["include_tree_sha256"] = json!(digest('e'));
        assert_eq!(
            validate_host_semantic_relations(&mutated).unwrap_err().code,
            "P1A_HOST_SEMANTIC_RELATION_INVALID"
        );

        let mut mutated = host.clone();
        mutated["result"]["input_stability"]["after_sha256"] = json!(digest('e'));
        assert_eq!(
            validate_host_semantic_relations(&mutated).unwrap_err().code,
            "P1A_HOST_SEMANTIC_RELATION_INVALID"
        );

        let mut mutated = host;
        mutated["result"]["cargo_build_policy"]["activated_packages_sha256"] = json!(digest('e'));
        assert_eq!(
            validate_host_semantic_relations(&mutated).unwrap_err().code,
            "P1A_HOST_SEMANTIC_RELATION_INVALID"
        );
    }

    #[test]
    fn native_and_cross_artifact_relations_reject_mutations() {
        let host = host_semantic_fixture();
        let native = native_semantic_fixture();
        validate_native_probe_semantic_relations(&native).unwrap();
        validate_host_native_input_relation(&host, &native).unwrap();

        let mut mutated = native.clone();
        mutated["result"]["binary_audit"]["import_classifications"][0]["name"] =
            json!("advapi32.dll");
        assert_eq!(
            validate_native_probe_semantic_relations(&mutated)
                .unwrap_err()
                .code,
            "P1A_NATIVE_SEMANTIC_RELATION_INVALID"
        );

        let mut mutated = native.clone();
        mutated["result"]["binary_audit"]["import_classifications"][0]["class"] =
            json!("ucrt_release");
        assert_eq!(
            validate_native_probe_semantic_relations(&mutated)
                .unwrap_err()
                .code,
            "P1A_NATIVE_SEMANTIC_RELATION_INVALID"
        );

        let mut mutated = native;
        mutated["result"]["input_stability"]["before_sha256"] = json!(digest('e'));
        mutated["result"]["input_stability"]["after_sha256"] = json!(digest('e'));
        assert_eq!(
            validate_host_native_input_relation(&host, &mutated)
                .unwrap_err()
                .code,
            "P1A_INPUT_STABILITY_RELATION_INVALID"
        );
    }

    #[test]
    fn p0a_verification_commit_is_bound_to_source_identity() {
        let source = json!({"commit": digest('a')});
        let dependency = json!({"verified_at_source_commit": digest('a')});
        validate_p0a_source_relation(&source, &dependency).unwrap();

        let mutated = json!({"verified_at_source_commit": digest('b')});
        assert_eq!(
            validate_p0a_source_relation(&source, &mutated)
                .unwrap_err()
                .code,
            "P1A_P0A_SOURCE_RELATION_INVALID"
        );
    }

    #[test]
    fn command_timings_reject_order_duration_and_timestamp_mutations() {
        let evidence = json!({"generated_at_utc": "2026-08-13T10:00:00.000000000Z"});
        let commands = vec![
            json!({
                "id": "C001",
                "started_at_utc": "2026-08-13T10:00:00.000000000Z",
                "finished_at_utc": "2026-08-13T10:00:00.100000000Z",
                "duration_ns": 100_000_000
            }),
            json!({
                "id": "C002",
                "started_at_utc": "2026-08-13T10:00:00.100000000Z",
                "finished_at_utc": "2026-08-13T10:00:00.200000000Z",
                "duration_ns": 100_000_000
            }),
        ];
        validate_command_timings(&evidence, &commands).unwrap();

        let mut mutated = commands.clone();
        mutated[0]["finished_at_utc"] = json!("2026-08-13T09:59:59.999999999Z");
        assert_eq!(
            validate_command_timings(&evidence, &mutated)
                .unwrap_err()
                .code,
            "P1A_COMMAND_TIMING_INVALID"
        );

        let mut mutated = commands.clone();
        mutated[1]["started_at_utc"] = json!("2026-08-13T10:00:00.050000000Z");
        mutated[1]["duration_ns"] = json!(150_000_000);
        assert_eq!(
            validate_command_timings(&evidence, &mutated)
                .unwrap_err()
                .code,
            "P1A_COMMAND_TIMING_INVALID"
        );

        let mut mutated = commands.clone();
        mutated[0]["duration_ns"] = json!(200_000_000);
        assert_eq!(
            validate_command_timings(&evidence, &mutated)
                .unwrap_err()
                .code,
            "P1A_COMMAND_TIMING_INVALID"
        );

        let mut mutated = commands.clone();
        mutated[0]["finished_at_utc"] = json!("2026-08-13T10:00:02.000000000Z");
        assert_eq!(
            validate_command_timings(&evidence, &mutated)
                .unwrap_err()
                .code,
            "P1A_COMMAND_TIMING_INVALID"
        );

        let mut mutated = commands;
        mutated[0]["started_at_utc"] = json!("2026-02-30T10:00:00.000000000Z");
        assert_eq!(
            validate_command_timings(&evidence, &mutated)
                .unwrap_err()
                .code,
            "P1A_COMMAND_TIMING_INVALID"
        );
    }

    #[test]
    fn command_timings_bind_attempt_creation_between_c012_and_c013() {
        let evidence = json!({"generated_at_utc": "2026-08-13T10:00:11.500000000Z"});
        let commands = (1..=13)
            .map(|sequence| {
                json!({
                    "id": format!("C{sequence:03}"),
                    "started_at_utc": format!("2026-08-13T10:00:{:02}.000000000Z", sequence - 1),
                    "finished_at_utc": format!("2026-08-13T10:00:{:02}.100000000Z", sequence - 1),
                    "duration_ns": 100_000_000
                })
            })
            .collect::<Vec<_>>();
        validate_command_timings(&evidence, &commands).unwrap();

        for generated_at_utc in [
            "2026-08-13T10:00:10.999999999Z",
            "2026-08-13T10:00:12.000000001Z",
        ] {
            let mutated = json!({"generated_at_utc": generated_at_utc});
            assert_eq!(
                validate_command_timings(&mutated, &commands)
                    .unwrap_err()
                    .code,
                "P1A_COMMAND_TIMING_INVALID"
            );
        }
    }

    #[test]
    fn transcripts_are_semantically_bound_to_artifact_claims() {
        let host = json!({
            "result": {
                "rust_toolchain": {
                    "release": "1.96.1",
                    "release_semver": {"major": 1, "minor": 96, "patch": 1},
                    "host": "x86_64-pc-windows-msvc",
                    "llvm_version": "21.1.0",
                    "cargo": {"version": "1.96.1"}
                },
                "native_toolchain": {
                    "visual_studio_candidates": [{
                        "instance_id": "vs-instance",
                        "product_id": "Microsoft.VisualStudio.Product.Community",
                        "installation_version": "17.14.1",
                        "installation_path": "${VS_INSTALL}",
                        "complete": true,
                        "launchable": true,
                        "reboot_required": false
                    }]
                },
                "cargo_build_policy": {
                    "activated_packages": ["alpha@1.0.0", "beta@2.0.0"]
                }
            }
        });
        let native = json!({
            "result": {
                "rust_c_probe": {
                    "observed_stdout": "P1A_ABI_PASS c=3137 cpp=150\n",
                    "output_sha256": hash::bytes(b"P1A_ABI_PASS c=3137 cpp=150\n")
                },
                "rust_cpp_probe": {
                    "observed_stdout": "P1A_ABI_PASS c=3137 cpp=150\n",
                    "output_sha256": hash::bytes(b"P1A_ABI_PASS c=3137 cpp=150\n")
                },
                "binary_audit": {
                    "pe_machine": "AMD64",
                    "imports": ["kernel32.dll", "vcruntime140.dll"]
                }
            }
        });
        let vswhere = br#"[{
            "instanceId":"vs-instance",
            "productId":"Microsoft.VisualStudio.Product.Community",
            "installationVersion":"17.14.1",
            "installationPath":"${VS_INSTALL}",
            "isComplete":true,
            "isLaunchable":true,
            "isRebootRequired":false
        }]"#;
        let rustc = b"rustc 1.96.1\nbinary: rustc\ncommit-hash: abc\nrelease: 1.96.1\nhost: x86_64-pc-windows-msvc\nLLVM version: 21.1.0\n";
        let cargo = b"cargo 1.96.1 (abc 2026-01-01)\nrelease: 1.96.1\n";
        let tree = b"beta v2.0.0\nalpha v1.0.0\nalpha v1.0.0\n";
        let abi = b"P1A_ABI_PASS c=3137 cpp=150\n";
        let dumpbin = b"FILE HEADER VALUES\n            8664 machine (x64)\n  KERNEL32.dll\n  vcruntime140.dll\n";

        validate_vswhere_transcript(vswhere, &host).unwrap();
        validate_rustc_transcript(rustc, &host).unwrap();
        validate_cargo_transcript(cargo, &host).unwrap();
        validate_cargo_tree_transcript(tree, &host).unwrap();
        validate_abi_transcript(abi, &native).unwrap();
        validate_dumpbin_transcript(dumpbin, &native).unwrap();

        let mut mutated_host = host.clone();
        mutated_host["result"]["native_toolchain"]["visual_studio_candidates"][0]["product_id"] =
            json!("Microsoft.VisualStudio.Product.Professional");
        assert_eq!(
            validate_vswhere_transcript(vswhere, &mutated_host)
                .unwrap_err()
                .code,
            "P1A_TRANSCRIPT_RELATION_INVALID"
        );

        let mut mutated_host = host.clone();
        mutated_host["result"]["native_toolchain"]["visual_studio_candidates"][0]["installation_path"] =
            json!("${PROGRAM_FILES}/different");
        assert_eq!(
            validate_vswhere_transcript(vswhere, &mutated_host)
                .unwrap_err()
                .code,
            "P1A_TRANSCRIPT_RELATION_INVALID"
        );

        let mut mutated_host = host.clone();
        mutated_host["result"]["rust_toolchain"]["release_semver"]["patch"] = json!(2);
        assert_eq!(
            validate_rustc_transcript(rustc, &mutated_host)
                .unwrap_err()
                .code,
            "P1A_TRANSCRIPT_RELATION_INVALID"
        );

        let mut mutated_host = host.clone();
        mutated_host["result"]["rust_toolchain"]["cargo"]["version"] = json!("1.96.2");
        assert_eq!(
            validate_cargo_transcript(cargo, &mutated_host)
                .unwrap_err()
                .code,
            "P1A_TRANSCRIPT_RELATION_INVALID"
        );

        assert_eq!(
            validate_cargo_tree_transcript(b"alpha v1.0.0\n", &host)
                .unwrap_err()
                .code,
            "P1A_TRANSCRIPT_RELATION_INVALID"
        );
        assert_eq!(
            validate_abi_transcript(b"P1A_ABI_PASS c=3137 cpp=151\n", &native)
                .unwrap_err()
                .code,
            "P1A_TRANSCRIPT_RELATION_INVALID"
        );
        assert_eq!(
            validate_dumpbin_transcript(
                b"FILE HEADER VALUES\n8664 machine (x64)\nKERNEL32.dll\n",
                &native,
            )
            .unwrap_err()
            .code,
            "P1A_TRANSCRIPT_RELATION_INVALID"
        );
    }

    #[test]
    fn cpu_relations_reject_snapshot_and_host_topology_mutations() {
        let host = host_semantic_fixture();
        let cpu = cpu_semantic_fixture();
        validate_cpu_semantic_relations(&host, &cpu).unwrap();

        let mut mutated = cpu.clone();
        mutated["result"]["after"]["power_scheme_name"] = json!("Changed");
        assert_eq!(
            validate_cpu_semantic_relations(&host, &mutated)
                .unwrap_err()
                .code,
            "P1A_CPU_SEMANTIC_RELATION_INVALID"
        );

        let mut mutated_host = host;
        mutated_host["result"]["cpu"]["core_topology_sha256"] = json!(digest('e'));
        assert_eq!(
            validate_cpu_semantic_relations(&mutated_host, &cpu)
                .unwrap_err()
                .code,
            "P1A_CPU_SEMANTIC_RELATION_INVALID"
        );

        let mut mutated = cpu;
        mutated["result"]["before"]["selected_logical_processor_count"] = json!(31);
        mutated["result"]["after"]["selected_logical_processor_count"] = json!(31);
        assert_eq!(
            validate_cpu_semantic_relations(&host_semantic_fixture(), &mutated)
                .unwrap_err()
                .code,
            "P1A_CPU_SEMANTIC_RELATION_INVALID"
        );
    }

    #[test]
    fn process_audit_relations_reject_snapshot_count_and_name_mutations() {
        let command = audit_command_fixture();
        validate_process_audit(&command).unwrap();

        let mut mutated = command.clone();
        mutated["process_audit"]["successful_snapshots"] = json!(2);
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_AUDIT_RELATION_INVALID"
        );

        let mut mutated = command;
        mutated["process_audit"]["executable_names"] = json!(["other.exe"]);
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_AUDIT_RELATION_INVALID"
        );

        let mut mutated = audit_command_fixture();
        mutated["process_audit"]["root_creation_time_100ns"] = json!(3);
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_AUDIT_RELATION_INVALID"
        );

        let mut mutated = audit_command_fixture();
        mutated["process_audit"]["loaded_modules"][0]["path_class"] = json!("windows_system32");
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_AUDIT_RELATION_INVALID"
        );
    }

    #[test]
    fn process_path_classes_keep_exact_vswhere_files_closed_and_root_bound() {
        let runtime_module = |name: &str, path_class: &str, character: char| {
            json!({
                "process_id": 1,
                "creation_time_100ns": 2,
                "module_name": name,
                "canonical_path_sha256": digest(character),
                "module_sha256": digest(character),
                "module_bytes": 4,
                "path_class": path_class
            })
        };
        let mut command = audit_command_fixture();
        command["argv"] = json!(["${VSWHERE}"]);
        command["process_audit"]["loaded_modules"]
            .as_array_mut()
            .unwrap()
            .extend([
                runtime_module("ntdll.dll", "windows_syswow64", 'c'),
                runtime_module(
                    "Microsoft.VisualStudio.Setup.Configuration.Native.dll",
                    "qualified_tool_file",
                    'd',
                ),
            ]);
        validate_process_audit(&command).unwrap();

        let mut mutated = command.clone();
        mutated["process_audit"]["loaded_modules"]
            .as_array_mut()
            .unwrap()
            .remove(1);
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_PATH_CLASS_RELATION_INVALID"
        );

        let mut mutated = command.clone();
        mutated["process_audit"]["loaded_modules"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_PATH_CLASS_RELATION_INVALID"
        );

        let mut mutated = command.clone();
        mutated["process_audit"]["loaded_modules"]
            .as_array_mut()
            .unwrap()
            .push(runtime_module(
                "Microsoft.VisualStudio.Setup.Configuration.Native.dll",
                "qualified_tool_file",
                'e',
            ));
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_PATH_CLASS_RELATION_INVALID"
        );

        let mut mutated = command.clone();
        mutated["process_audit"]["loaded_modules"][2]["module_name"] = json!("unrelated.dll");
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_PATH_CLASS_RELATION_INVALID"
        );

        let mut mutated = command.clone();
        mutated["argv"] = json!(["${RUSTC}"]);
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_PATH_CLASS_RELATION_INVALID"
        );

        let mut mutated = command.clone();
        mutated["process_audit"]["loaded_modules"][2]["process_id"] = json!(3);
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_PATH_CLASS_RELATION_INVALID"
        );

        let mut mutated = command.clone();
        mutated["process_audit"]["process_identities"][0]["path_class"] =
            json!("qualified_tool_file");
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_PATH_CLASS_RELATION_INVALID"
        );

        let mut mutated = command;
        mutated["process_audit"]["loaded_modules"][0]["path_class"] =
            json!("unreviewed_future_root");
        assert_eq!(
            validate_process_audit(&mutated).unwrap_err().code,
            "P1A_PROCESS_PATH_CLASS_RELATION_INVALID"
        );
    }
}
