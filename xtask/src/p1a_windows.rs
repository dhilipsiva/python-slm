//! Read-only native-Windows discovery for the prototype P1A host gate.
//!
//! This module deliberately does not use a shell, PowerShell, batch files, WMI, or
//! environment setup scripts.  It discovers the host with Win32 APIs and invokes only
//! the fixed `vswhere.exe` query used to locate Visual Studio.  The caller owns receipt
//! tokenisation: paths in this report are native absolute paths and must not be emitted
//! verbatim into public evidence.

use crate::error::{Result, XtaskError};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub(crate) const ISOLATION_WINDOW_MILLISECONDS: u64 = 2_000;
pub(crate) const EXPECTED_CPU_VENDOR: &str = "AuthenticAMD";
pub(crate) const EXPECTED_CPU_BRAND: &str = "AMD Ryzen 9 9950X3D 16-Core Processor";
pub(crate) const MINIMUM_SYSTEM_IDLE_BASIS_POINTS: u32 = 5_000;
pub(crate) const MAXIMUM_FOREIGN_SINGLE_CORE_BASIS_POINTS: u32 = 5_000;
pub(crate) const VSWHERE_ARGS: &[&str] = &[
    "-version",
    "[17.0,18.0)",
    "-products",
    "*",
    "-requires",
    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
    "-format",
    "json",
    "-utf8",
];
pub(crate) const COMPETING_COMPUTE_NAMES: &[&str] = &[
    "cargo.exe",
    "cl.exe",
    "clang.exe",
    "clang-cl.exe",
    "cmake.exe",
    "hipcc.exe",
    "link.exe",
    "msbuild.exe",
    "ninja.exe",
    "nvcc.exe",
    "py.exe",
    "python.exe",
    "python3.exe",
    "rustc.exe",
];

/// Thresholds are supplied by the receipt policy before sampling.  Keeping them out of
/// the probe prevents a measurement from selecting its own passing limit after the fact.
#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CpuIsolationPolicy {
    pub selected_group: u16,
    pub selected_affinity_mask: u64,
    /// The P1A runner must populate this only from its process ancestry and children
    /// contained by its Windows Job Object.  Arbitrary user-selected exceptions are not
    /// part of the policy.
    pub verifier_ancestry_and_contained_process_identities: BTreeSet<ProcessIdentity>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessIdentity {
    pub process_id: u32,
    pub creation_time_100ns: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowsHostPolicy {
    pub isolation: CpuIsolationPolicy,
    /// Required when more than one complete, launchable VS 2022 instance is present.
    pub visual_studio_instance_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateFinding {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowsVersion {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub service_pack_major: u16,
    pub service_pack_minor: u16,
    pub product_type: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchitectureIdentity {
    pub process_machine: String,
    pub native_machine: String,
    pub process_is_native: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CpuIdentity {
    pub vendor: String,
    pub brand: String,
    pub family: u32,
    pub model: u32,
    pub stepping: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IsaFeatures {
    pub sse2: bool,
    pub sse3: bool,
    pub ssse3: bool,
    pub sse41: bool,
    pub sse42: bool,
    pub popcnt: bool,
    pub aes: bool,
    pub pclmulqdq: bool,
    pub fma: bool,
    pub bmi1: bool,
    pub bmi2: bool,
    pub avx_hardware: bool,
    pub avx_os_enabled: bool,
    pub avx2_hardware: bool,
    pub avx2_os_enabled: bool,
    pub avx512f_hardware: bool,
    pub avx512_os_enabled: bool,
    pub avx512dq: bool,
    pub avx512cd: bool,
    pub avx512bw: bool,
    pub avx512vl: bool,
    pub sha: bool,
    pub vaes: bool,
    pub vpclmulqdq: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoreTopology {
    pub efficiency_class: u8,
    pub smt: bool,
    pub group_masks: Vec<GroupMask>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupMask {
    pub group: u16,
    pub mask: u64,
    pub logical_processors: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessorTopology {
    pub active_group_count: u16,
    pub active_logical_processors: u32,
    pub physical_core_count: u32,
    pub package_count: u32,
    pub cores: Vec<CoreTopology>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AffinitySnapshot {
    pub thread_group: u16,
    pub thread_group_mask: u64,
    pub process_mask: u64,
    pub system_mask: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PowerPolicySnapshot {
    pub active_scheme_guid: String,
    pub active_scheme_name: String,
    pub ac_line_status: String,
    pub processor_minimum_percent: u32,
    pub processor_maximum_percent: u32,
    pub processor_boost_mode: u32,
    pub energy_performance_preference: u32,
    pub value_source: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForeignProcessLoad {
    pub process_id: u32,
    pub creation_time_100ns: Option<u64>,
    pub image_name: String,
    pub cpu_time_100ns: u64,
    pub single_core_basis_points: u32,
    pub approved: bool,
    pub known_compute_name: bool,
}

pub(crate) fn is_competing_foreign_load(load: &ForeignProcessLoad) -> bool {
    !load.approved && load.single_core_basis_points > MAXIMUM_FOREIGN_SINGLE_CORE_BASIS_POINTS
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CpuIsolationMeasurement {
    pub requested_window_milliseconds: u64,
    pub actual_elapsed_nanoseconds: u64,
    pub logical_processor_capacity: u32,
    pub system_idle_delta_100ns: u64,
    pub system_kernel_delta_100ns: u64,
    pub system_user_delta_100ns: u64,
    pub system_busy_basis_points: u32,
    pub ordinary_os_activity_total_cpu_100ns: u64,
    pub ordinary_os_activity_process_count: u32,
    pub largest_unapproved_process_single_core_basis_points: u32,
    pub approved_verifier_cpu_100ns: u64,
    pub inaccessible_processes_at_start: u32,
    pub inaccessible_processes_at_end: u32,
    pub new_processes: u32,
    pub ended_processes: u32,
    pub foreign_process_loads: Vec<ForeignProcessLoad>,
    pub affinity_before: AffinitySnapshot,
    pub affinity_after: AffinitySnapshot,
    pub topology_stable: bool,
    pub affinity_stable: bool,
    pub power_policy_stable: bool,
    pub passed: bool,
    pub violations: Vec<GateFinding>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolFileIdentity {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub file_version: String,
}

/// Keeps the exact native Setup Configuration implementation immutable while the
/// audited `vswhere.exe` query is in flight. The file handle deliberately permits
/// only other readers, so neither the file bytes nor its directory entry can change
/// between discovery and process auditing.
pub(crate) struct VswhereRuntimeBinding {
    setup_configuration: ToolFileIdentity,
    _setup_configuration_lock: std::fs::File,
}

impl VswhereRuntimeBinding {
    pub(crate) fn setup_configuration_identity(&self) -> &ToolFileIdentity {
        &self.setup_configuration
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileIdentity {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VisualStudioCandidate {
    pub instance_id: String,
    pub product_id: String,
    pub installation_version: String,
    pub installation_path: PathBuf,
    pub complete: bool,
    pub launchable: bool,
    pub reboot_required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VisualStudioToolchain {
    pub discovery_method: String,
    pub vswhere: ToolFileIdentity,
    pub query: Vec<String>,
    pub candidates: Vec<VisualStudioCandidate>,
    pub selected_instance_id: String,
    pub product_id: String,
    pub installation_version: String,
    pub installation_path: PathBuf,
    pub msvc_tools_version: String,
    pub msvc_version_file: FileIdentity,
    pub msvc_tools_root: PathBuf,
    pub cl: ToolFileIdentity,
    pub c_frontend: ToolFileIdentity,
    pub cpp_frontend: ToolFileIdentity,
    pub code_generator: ToolFileIdentity,
    pub link: ToolFileIdentity,
    pub lib: ToolFileIdentity,
    pub dumpbin: ToolFileIdentity,
    pub msvc_include: PathBuf,
    pub msvc_x64_lib: PathBuf,
    pub vcruntime_lib: FileIdentity,
    pub vcruntime_redist_version: String,
    pub vcruntime_dll: ToolFileIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowsSdkToolchain {
    pub discovery_method: String,
    pub kits_root: PathBuf,
    pub version: String,
    pub ucrt_version: String,
    pub windows_header: FileIdentity,
    pub ucrt_header: FileIdentity,
    pub kernel32_lib: FileIdentity,
    pub ucrt_lib: FileIdentity,
    pub rc: ToolFileIdentity,
    pub mt: ToolFileIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowsRuntimeIdentities {
    pub resolution_policy: String,
    pub system_directory: PathBuf,
    pub ucrtbase: ToolFileIdentity,
    pub vcruntime: ToolFileIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TreeIdentity {
    pub sha256: String,
    pub directory_count: u64,
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostToolchainStabilitySnapshot {
    pub selection_manifest_sha256: String,
    pub qualified_file_manifest_sha256: String,
    pub qualified_file_count: u32,
    pub qualified_file_total_bytes: u64,
    pub msvc_include_tree: TreeIdentity,
    pub msvc_x64_lib_tree: TreeIdentity,
    pub windows_sdk_include_tree: TreeIdentity,
    pub windows_sdk_x64_lib_tree: TreeIdentity,
    pub bundle_sha256: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CanonicalGroupMask {
    pub group: u16,
    pub mask: String,
    pub logical_processors: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CanonicalCoreTopology {
    pub core_index: u32,
    pub efficiency_class: u8,
    pub smt: bool,
    pub group_masks: Vec<CanonicalGroupMask>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrototypeWindowsHostReport {
    pub schema: String,
    pub profile_id: String,
    pub os: WindowsVersion,
    pub architecture: ArchitectureIdentity,
    pub cpu: CpuIdentity,
    pub isa: IsaFeatures,
    pub topology: ProcessorTopology,
    pub affinity: AffinitySnapshot,
    pub power_policy: PowerPolicySnapshot,
    pub visual_studio: VisualStudioToolchain,
    pub windows_sdk: WindowsSdkToolchain,
    pub system_runtime: WindowsRuntimeIdentities,
    pub isolation: CpuIsolationMeasurement,
    pub qualified: bool,
    pub findings: Vec<GateFinding>,
}

pub(crate) fn canonical_core_topology(
    topology: &ProcessorTopology,
) -> Result<Vec<CanonicalCoreTopology>> {
    if topology.active_group_count != 1
        || !(1..=32).contains(&topology.active_logical_processors)
        || !(1..=16).contains(&topology.physical_core_count)
        || topology.package_count != 1
        || topology.cores.len() != topology.physical_core_count as usize
    {
        return Err(XtaskError::integrity(
            "P1A_CPU_TOPOLOGY_MISMATCH",
            "the Windows-visible 9950X3D topology is empty, exceeds the hardware, or has inconsistent aggregate counts",
        ));
    }

    let mut observed = Vec::with_capacity(topology.cores.len());
    let mut unions = BTreeMap::<u16, u64>::new();
    let mut observed_logical_processors = 0_u32;
    for core in &topology.cores {
        if core.group_masks.len() != 1 {
            return Err(XtaskError::integrity(
                "P1A_CPU_TOPOLOGY_LAYOUT_MISMATCH",
                "a Windows processor core does not have exactly one group-0 mask",
            ));
        }
        let mut canonical_masks = Vec::with_capacity(core.group_masks.len());
        let mut core_logical_processors = 0_u32;
        for group_mask in &core.group_masks {
            let bit_count = group_mask.mask.count_ones();
            let union = unions.entry(group_mask.group).or_default();
            if group_mask.group >= topology.active_group_count
                || group_mask.mask == 0
                || group_mask.logical_processors != bit_count
                || *union & group_mask.mask != 0
            {
                return Err(XtaskError::integrity(
                    "P1A_CPU_TOPOLOGY_LAYOUT_MISMATCH",
                    "Windows processor-group masks are empty, overlapping, out of range, or internally inconsistent",
                ));
            }
            *union |= group_mask.mask;
            core_logical_processors =
                core_logical_processors
                    .checked_add(bit_count)
                    .ok_or_else(|| {
                        XtaskError::integrity(
                            "P1A_CPU_TOPOLOGY_LAYOUT_MISMATCH",
                            "Windows processor topology logical-processor counts overflowed",
                        )
                    })?;
            canonical_masks.push(CanonicalGroupMask {
                group: group_mask.group,
                mask: format!("0x{:016X}", group_mask.mask),
                logical_processors: bit_count,
            });
        }
        if core.smt != (core_logical_processors > 1) {
            return Err(XtaskError::integrity(
                "P1A_CPU_TOPOLOGY_LAYOUT_MISMATCH",
                "a Windows processor core's SMT flag disagrees with its logical-processor masks",
            ));
        }
        observed_logical_processors = observed_logical_processors
            .checked_add(core_logical_processors)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_CPU_TOPOLOGY_LAYOUT_MISMATCH",
                    "Windows processor topology logical-processor counts overflowed",
                )
            })?;
        canonical_masks.sort();
        observed.push((canonical_masks, core.efficiency_class, core.smt));
    }
    if unions.len() != topology.active_group_count as usize
        || observed_logical_processors != topology.active_logical_processors
    {
        return Err(XtaskError::integrity(
            "P1A_CPU_TOPOLOGY_LAYOUT_MISMATCH",
            "Windows processor-group masks do not cover the reported groups and logical processors exactly",
        ));
    }

    observed.sort();
    Ok(observed
        .into_iter()
        .enumerate()
        .map(
            |(core_index, (group_masks, efficiency_class, smt))| CanonicalCoreTopology {
                core_index: core_index as u32,
                efficiency_class,
                smt,
                group_masks,
            },
        )
        .collect())
}

pub(crate) fn processor_group_union_mask(topology: &ProcessorTopology, group: u16) -> Result<u64> {
    if group >= topology.active_group_count {
        return Err(XtaskError::integrity(
            "P1A_CPU_TOPOLOGY_GROUP_INVALID",
            "the selected processor group is outside the Windows topology",
        ));
    }
    let canonical = canonical_core_topology(topology)?;
    let union = canonical.iter().try_fold(0_u64, |union, core| {
        core.group_masks
            .iter()
            .filter(|mask| mask.group == group)
            .try_fold(union, |union, group_mask| {
                let mask = group_mask
                    .mask
                    .strip_prefix("0x")
                    .and_then(|value| u64::from_str_radix(value, 16).ok())
                    .ok_or_else(|| {
                        XtaskError::integrity(
                            "P1A_CPU_TOPOLOGY_CANONICALIZATION_FAILED",
                            "an internal canonical processor mask was malformed",
                        )
                    })?;
                Ok(union | mask)
            })
    })?;
    if union == 0 {
        return Err(XtaskError::integrity(
            "P1A_CPU_TOPOLOGY_GROUP_INVALID",
            "the selected processor group has no active logical processors",
        ));
    }
    Ok(union)
}

pub(crate) fn processor_topology_sha256(topology: &ProcessorTopology) -> Result<String> {
    let canonical = canonical_core_topology(topology)?;
    let bytes = serde_json::to_vec(&canonical).map_err(|error| {
        XtaskError::integrity(
            "P1A_CPU_TOPOLOGY_SERIALIZATION_FAILED",
            format!("could not serialize canonical processor topology: {error}"),
        )
    })?;
    Ok(crate::hash::bytes(&bytes))
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn probe_prototype_windows_host(
    policy: &WindowsHostPolicy,
    audited_vswhere_stdout: &[u8],
) -> Result<PrototypeWindowsHostReport> {
    imp::probe(policy, audited_vswhere_stdout)
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(crate) fn probe_prototype_windows_host(
    _policy: &WindowsHostPolicy,
    _audited_vswhere_stdout: &[u8],
) -> Result<PrototypeWindowsHostReport> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "the prototype host probe is implemented only for native Windows x86_64",
        "Run P1A on prototype-windows-5090-v1; portable hosts are deferred until P17.",
    ))
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn discover_vswhere_path() -> Result<PathBuf> {
    imp::discover_vswhere_path()
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn select_visual_studio(
    requested_instance: Option<&str>,
    audited_vswhere_stdout: &[u8],
) -> Result<VisualStudioToolchain> {
    imp::visual_studio(requested_instance, audited_vswhere_stdout)
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn bind_vswhere_runtime() -> Result<VswhereRuntimeBinding> {
    imp::bind_vswhere_runtime()
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn discover_windows_sdk() -> Result<WindowsSdkToolchain> {
    imp::windows_sdk()
}

/// Resolve the one admitted Git for P1A without consulting `PATH`, a shell, or mutable
/// per-user configuration. The second return value is the canonical installation root
/// that the audited process runner may use to classify Git's native executables/modules.
#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn discover_git_path() -> Result<(PathBuf, PathBuf)> {
    imp::discover_git_path()
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn native_program_files_roots() -> Result<(PathBuf, PathBuf)> {
    imp::native_program_files_roots()
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn native_file_identity(path: &std::path::Path) -> Result<ToolFileIdentity> {
    imp::tool_identity(path)
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn loader_resolved_system_runtime() -> Result<WindowsRuntimeIdentities> {
    imp::loader_resolved_system_runtime()
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn snapshot_host_toolchain(
    report: &PrototypeWindowsHostReport,
) -> Result<HostToolchainStabilitySnapshot> {
    imp::snapshot_host_toolchain(report)
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn revalidate_host_toolchain(
    report: &PrototypeWindowsHostReport,
    expected: &HostToolchainStabilitySnapshot,
) -> Result<HostToolchainStabilitySnapshot> {
    let observed = imp::snapshot_host_toolchain(report)?;
    if &observed != expected {
        return Err(XtaskError::integrity(
            "P1A_TOOLCHAIN_IDENTITY_DRIFT",
            format!(
                "the selected host toolchain changed between stability snapshots (before {}, after {})",
                expected.bundle_sha256, observed.bundle_sha256
            ),
        ));
    }
    Ok(observed)
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn current_affinity_policy() -> Result<(u16, u64)> {
    let snapshot = imp::current_affinity()?;
    Ok((snapshot.thread_group, snapshot.process_mask))
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn current_verifier_ancestry() -> Result<BTreeSet<ProcessIdentity>> {
    imp::current_verifier_ancestry()
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(crate) fn discover_vswhere_path() -> Result<PathBuf> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "vswhere discovery is implemented only for native Windows x86_64",
        "Run P1A on prototype-windows-5090-v1; portable hosts are deferred until P17.",
    ))
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(crate) fn bind_vswhere_runtime() -> Result<VswhereRuntimeBinding> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "vswhere runtime binding is implemented only for native Windows x86_64",
        "Run P1A on prototype-windows-5090-v1; portable hosts are deferred until P17.",
    ))
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(crate) fn discover_git_path() -> Result<(PathBuf, PathBuf)> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "Git discovery for the prototype is implemented only for native Windows x86_64",
        "Run P1A on prototype-windows-5090-v1; portable hosts are deferred until P17.",
    ))
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(crate) fn native_program_files_roots() -> Result<(PathBuf, PathBuf)> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "Program Files known-folder discovery is implemented only for native Windows x86_64",
        "Run P1A on prototype-windows-5090-v1; portable hosts are deferred until P17.",
    ))
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(crate) fn current_affinity_policy() -> Result<(u16, u64)> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "processor-affinity discovery is implemented only for native Windows x86_64",
        "Run P1A on prototype-windows-5090-v1; portable hosts are deferred until P17.",
    ))
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(crate) fn current_verifier_ancestry() -> Result<BTreeSet<ProcessIdentity>> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "process-ancestry discovery is implemented only for native Windows x86_64",
        "Run P1A on prototype-windows-5090-v1; portable hosts are deferred until P17.",
    ))
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(crate) fn loader_resolved_system_runtime() -> Result<WindowsRuntimeIdentities> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "Windows runtime discovery is implemented only for native Windows x86_64",
        "Run P1A on prototype-windows-5090-v1; portable hosts are deferred until P17.",
    ))
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(crate) fn snapshot_host_toolchain(
    _report: &PrototypeWindowsHostReport,
) -> Result<HostToolchainStabilitySnapshot> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "host-toolchain stability snapshots are implemented only for native Windows x86_64",
        "Run P1A on prototype-windows-5090-v1; portable hosts are deferred until P17.",
    ))
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(crate) fn revalidate_host_toolchain(
    _report: &PrototypeWindowsHostReport,
    _expected: &HostToolchainStabilitySnapshot,
) -> Result<HostToolchainStabilitySnapshot> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "host-toolchain stability revalidation is implemented only for native Windows x86_64",
        "Run P1A on prototype-windows-5090-v1; portable hosts are deferred until P17.",
    ))
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod imp {
    use super::*;
    use crate::error::{Category, IoContext};
    use serde::Deserialize;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, c_void};
    use std::fs::{self, File, OpenOptions};
    use std::io::Read;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Path;
    use std::ptr::null_mut;
    use std::thread;
    use std::time::Instant;

    const IMAGE_FILE_MACHINE_UNKNOWN: u16 = 0;
    const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
    const RELATION_PROCESSOR_CORE: u32 = 0;
    const RELATION_PROCESSOR_PACKAGE: u32 = 3;
    const ALL_PROCESSOR_GROUPS: u16 = 0xffff;
    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const ERROR_NO_MORE_FILES: u32 = 18;
    const RRF_RT_REG_SZ: u32 = 0x0000_0002;
    const RRF_SUBKEY_WOW6464KEY: u32 = 0x0001_0000;
    const RRF_SUBKEY_WOW6432KEY: u32 = 0x0002_0000;
    const VSWHERE_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
    const PROCESS_SNAPSHOT_LIMIT: usize = 4096;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;
    const IDENTITY_FILE_LIMIT: u64 = 1024 * 1024 * 1024;
    const TREE_ENTRY_LIMIT: u64 = 1_000_000;
    const QUALIFIED_FILE_COUNT: u32 = 19;
    // GetSystemWow64DirectoryW treats a 32_768-unit capacity as a sizing request on
    // the qualified host and returns a required length without populating the buffer.
    // The largest signed 16-bit unit count remains far above the Windows path bound.
    const SYSWOW64_DIRECTORY_BUFFER_UNITS: usize = i16::MAX as usize;

    type Handle = *mut c_void;
    type Hkey = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const FOLDERID_PROGRAM_FILES_X86: Guid = Guid {
        data1: 0x7c5a40ef,
        data2: 0xa0fb,
        data3: 0x4bfc,
        data4: [0x87, 0x4a, 0xc0, 0xf2, 0xe0, 0xb9, 0xfa, 0x8e],
    };
    const FOLDERID_PROGRAM_FILES: Guid = Guid {
        data1: 0x905e63b6,
        data2: 0xc1bf,
        data3: 0x494e,
        data4: [0xb2, 0x9c, 0x65, 0xb7, 0x32, 0xd3, 0xd2, 0x1a],
    };
    const FOLDERID_PROGRAM_DATA: Guid = Guid {
        data1: 0x62ab5d82,
        data2: 0xfdc1,
        data3: 0x4dc3,
        data4: [0xa9, 0xdd, 0x07, 0x0d, 0x1d, 0x49, 0x5d, 0x97],
    };
    const PROCESSOR_SETTINGS_SUBGROUP: Guid = Guid {
        data1: 0x54533251,
        data2: 0x82be,
        data3: 0x4824,
        data4: [0x96, 0xc1, 0x47, 0xb6, 0x0b, 0x74, 0x0d, 0x00],
    };
    const PROCESSOR_THROTTLE_MINIMUM: Guid = Guid {
        data1: 0x893dee8e,
        data2: 0x2bef,
        data3: 0x41e0,
        data4: [0x89, 0xc6, 0xb5, 0x5d, 0x09, 0x29, 0x96, 0x4c],
    };
    const PROCESSOR_THROTTLE_MAXIMUM: Guid = Guid {
        data1: 0xbc5038f7,
        data2: 0x23e0,
        data3: 0x4960,
        data4: [0x96, 0xda, 0x33, 0xab, 0xaf, 0x59, 0x35, 0xec],
    };
    const PROCESSOR_PERF_BOOST_MODE: Guid = Guid {
        data1: 0xbe337238,
        data2: 0x0d82,
        data3: 0x4146,
        data4: [0xa9, 0x60, 0x4f, 0x37, 0x49, 0xd4, 0x70, 0xc7],
    };
    const PROCESSOR_ENERGY_PERFORMANCE_PREFERENCE: Guid = Guid {
        data1: 0x36687f9e,
        data2: 0xe3a5,
        data3: 0x4dbf,
        data4: [0xb1, 0xdc, 0x15, 0xeb, 0x38, 0x1c, 0x68, 0x63],
    };

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct OsVersionInfoExW {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform_id: u32,
        csd_version: [u16; 128],
        service_pack_major: u16,
        service_pack_minor: u16,
        suite_mask: u16,
        product_type: u8,
        reserved: u8,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct GroupAffinity {
        mask: usize,
        group: u16,
        reserved: [u16; 3],
    }

    #[repr(C)]
    struct ProcessEntry32W {
        size: u32,
        usage: u32,
        process_id: u32,
        default_heap_id: usize,
        module_id: u32,
        threads: u32,
        parent_process_id: u32,
        priority_base: i32,
        flags: u32,
        exe_file: [u16; 260],
    }

    #[repr(C)]
    struct SystemPowerStatus {
        ac_line_status: u8,
        battery_flag: u8,
        battery_life_percent: u8,
        system_status_flag: u8,
        battery_life_time: u32,
        battery_full_life_time: u32,
    }

    #[repr(C)]
    struct VsFixedFileInfo {
        signature: u32,
        struct_version: u32,
        file_version_ms: u32,
        file_version_ls: u32,
        product_version_ms: u32,
        product_version_ls: u32,
        file_flags_mask: u32,
        file_flags: u32,
        file_os: u32,
        file_type: u32,
        file_subtype: u32,
        file_date_ms: u32,
        file_date_ls: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> Handle;
        fn GetCurrentThread() -> Handle;
        fn IsWow64Process2(
            process: Handle,
            process_machine: *mut u16,
            native_machine: *mut u16,
        ) -> i32;
        fn GetActiveProcessorGroupCount() -> u16;
        fn GetActiveProcessorCount(group_number: u16) -> u32;
        fn GetProcessAffinityMask(
            process: Handle,
            process_affinity_mask: *mut usize,
            system_affinity_mask: *mut usize,
        ) -> i32;
        fn GetThreadGroupAffinity(thread: Handle, group_affinity: *mut GroupAffinity) -> i32;
        fn GetLogicalProcessorInformationEx(
            relationship_type: u32,
            buffer: *mut u8,
            returned_length: *mut u32,
        ) -> i32;
        fn GetSystemTimes(
            idle_time: *mut FileTime,
            kernel_time: *mut FileTime,
            user_time: *mut FileTime,
        ) -> i32;
        fn GetSystemTimeAsFileTime(system_time: *mut FileTime);
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
        fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        fn GetProcessTimes(
            process: Handle,
            creation_time: *mut FileTime,
            exit_time: *mut FileTime,
            kernel_time: *mut FileTime,
            user_time: *mut FileTime,
        ) -> i32;
        fn CloseHandle(object: Handle) -> i32;
        fn LocalFree(memory: Handle) -> Handle;
        fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
        fn GetSystemWow64DirectoryW(buffer: *mut u16, size: u32) -> u32;
        fn LoadLibraryExW(file_name: *const u16, file: Handle, flags: u32) -> Handle;
        fn GetModuleFileNameW(module: Handle, file_name: *mut u16, size: u32) -> u32;
        fn FreeLibrary(module: Handle) -> i32;
        fn GetLastError() -> u32;
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version_information: *mut OsVersionInfoExW) -> i32;
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            folder_id: *const Guid,
            flags: u32,
            token: Handle,
            path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(memory: *mut c_void);
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: Hkey,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            value_type: *mut u32,
            data: *mut c_void,
            data_size: *mut u32,
        ) -> i32;
    }

    #[link(name = "powrprof")]
    unsafe extern "system" {
        fn PowerGetActiveScheme(
            user_root_power_key: Hkey,
            active_policy_guid: *mut *mut Guid,
        ) -> u32;
        fn PowerReadACValueIndex(
            root_power_key: Hkey,
            scheme_guid: *const Guid,
            subgroup_of_power_settings_guid: *const Guid,
            power_setting_guid: *const Guid,
            ac_value_index: *mut u32,
        ) -> u32;
        fn PowerReadFriendlyName(
            root_power_key: Hkey,
            scheme_guid: *const Guid,
            subgroup_of_power_settings_guid: *const Guid,
            power_setting_guid: *const Guid,
            buffer: *mut u8,
            buffer_size: *mut u32,
        ) -> u32;
    }

    #[link(name = "version")]
    unsafe extern "system" {
        fn GetFileVersionInfoSizeW(file_name: *const u16, handle: *mut u32) -> u32;
        fn GetFileVersionInfoW(
            file_name: *const u16,
            handle: u32,
            length: u32,
            data: *mut c_void,
        ) -> i32;
        fn VerQueryValueW(
            block: *const c_void,
            sub_block: *const u16,
            buffer: *mut *mut c_void,
            length: *mut u32,
        ) -> i32;
    }

    pub(super) fn probe(
        policy: &WindowsHostPolicy,
        audited_vswhere_stdout: &[u8],
    ) -> Result<PrototypeWindowsHostReport> {
        validate_policy(policy)?;

        let os = os_version()?;
        let architecture = architecture()?;
        let cpu = cpu_identity();
        let isa = isa_features();
        let topology = topology()?;
        let affinity = current_affinity()?;
        let power_policy = power_policy()?;
        let visual_studio = visual_studio(
            policy.visual_studio_instance_id.as_deref(),
            audited_vswhere_stdout,
        )?;
        let windows_sdk = windows_sdk()?;
        let system_runtime = loader_resolved_system_runtime()?;
        let isolation = measure_isolation(policy, &topology, &power_policy)?;

        let mut findings = Vec::new();
        if architecture.native_machine != "AMD64" || !architecture.process_is_native {
            finding(
                &mut findings,
                "P1A_NATIVE_AMD64_REQUIRED",
                "the process must be native AMD64 on AMD64 Windows",
            );
        }
        if cpu.vendor != EXPECTED_CPU_VENDOR {
            finding(
                &mut findings,
                "P1A_CPU_VENDOR_MISMATCH",
                format!("expected {EXPECTED_CPU_VENDOR}, observed {}", cpu.vendor),
            );
        }
        if cpu.brand != EXPECTED_CPU_BRAND {
            finding(
                &mut findings,
                "P1A_CPU_MODEL_MISMATCH",
                format!("expected {EXPECTED_CPU_BRAND}, observed {}", cpu.brand),
            );
        }
        if (cpu.family, cpu.model, cpu.stepping) != (26, 68, 0) {
            finding(
                &mut findings,
                "P1A_CPU_SIGNATURE_MISMATCH",
                format!(
                    "expected CPUID family/model/stepping 26/68/0, observed {}/{}/{}",
                    cpu.family, cpu.model, cpu.stepping
                ),
            );
        }
        if !(isa.sse2
            && isa.sse3
            && isa.ssse3
            && isa.sse41
            && isa.sse42
            && isa.popcnt
            && isa.aes
            && isa.pclmulqdq
            && isa.avx_hardware
            && isa.avx_os_enabled
            && isa.avx2_hardware
            && isa.avx2_os_enabled
            && isa.fma
            && isa.bmi1
            && isa.bmi2
            && isa.avx512f_hardware
            && isa.avx512_os_enabled
            && isa.avx512dq
            && isa.avx512cd
            && isa.avx512bw
            && isa.avx512vl
            && isa.sha
            && isa.vaes
            && isa.vpclmulqdq)
        {
            finding(
                &mut findings,
                "P1A_CPU_ISA_MISMATCH",
                "the prototype CPU lacks a required, OS-enabled Zen 5 instruction-set feature",
            );
        }
        if canonical_core_topology(&topology).is_err() {
            finding(
                &mut findings,
                "P1A_CPU_TOPOLOGY_LAYOUT_MISMATCH",
                "Windows reported an empty, overlapping, or internally inconsistent processor topology",
            );
        }
        if power_policy.ac_line_status != "online"
            || power_policy.value_source != "ac"
            || power_policy.processor_minimum_percent > 100
            || power_policy.processor_maximum_percent > 100
            || power_policy.processor_minimum_percent > power_policy.processor_maximum_percent
            || power_policy.processor_boost_mode > 6
            || power_policy.energy_performance_preference > 100
        {
            finding(
                &mut findings,
                "P1A_POWER_POLICY_INVALID",
                "the host is not on AC power or returned an invalid processor power policy",
            );
        }
        findings.extend(isolation.violations.iter().cloned());

        Ok(PrototypeWindowsHostReport {
            schema: "python-slm-p1a-windows-host-probe-v1".to_owned(),
            profile_id: "prototype-windows-5090-v1".to_owned(),
            os,
            architecture,
            cpu,
            isa,
            topology,
            affinity,
            power_policy,
            visual_studio,
            windows_sdk,
            system_runtime,
            qualified: findings.is_empty(),
            findings,
            isolation,
        })
    }

    fn validate_policy(policy: &WindowsHostPolicy) -> Result<()> {
        if policy.isolation.selected_affinity_mask == 0 || policy.isolation.selected_group != 0 {
            return Err(XtaskError::new(
                "P1A_ISOLATION_POLICY_INVALID",
                Category::Usage,
                "the predeclared isolation policy contains an invalid group or mask",
                "Freeze group 0 and a nonzero affinity mask before probing.",
            ));
        }
        let current_identity = process_identity(std::process::id())?;
        if policy
            .isolation
            .verifier_ancestry_and_contained_process_identities
            .is_empty()
            || policy
                .isolation
                .verifier_ancestry_and_contained_process_identities
                .len()
                > 128
            || !policy
                .isolation
                .verifier_ancestry_and_contained_process_identities
                .contains(&current_identity)
            || policy
                .isolation
                .verifier_ancestry_and_contained_process_identities
                .iter()
                .any(|identity| identity.process_id == 0 || identity.creation_time_100ns == 0)
        {
            return Err(XtaskError::new(
                "P1A_ISOLATION_POLICY_INVALID",
                Category::Usage,
                "the approved process identity set is empty, unbounded, malformed, or omits this verifier creation identity",
                "Build the policy only from the native verifier ancestry and Job-contained process identities.",
            ));
        }
        if let Some(instance_id) = &policy.visual_studio_instance_id
            && (instance_id.trim() != instance_id
                || instance_id.is_empty()
                || instance_id.contains('\r')
                || instance_id.contains('\n'))
        {
            return Err(XtaskError::new(
                "P1A_VS_INSTANCE_POLICY_INVALID",
                Category::Usage,
                "the predeclared Visual Studio instance ID is not a nonempty single line",
                "Freeze the exact vswhere instanceId or omit it when exactly one instance is installed.",
            ));
        }
        Ok(())
    }

    fn finding(findings: &mut Vec<GateFinding>, code: &str, message: impl Into<String>) {
        findings.push(GateFinding {
            code: code.to_owned(),
            message: message.into(),
        });
    }

    fn os_version() -> Result<WindowsVersion> {
        let mut info: OsVersionInfoExW = unsafe { zeroed() };
        info.size = size_of::<OsVersionInfoExW>() as u32;
        // SAFETY: `info` is a correctly sized, writable OSVERSIONINFOEXW buffer.
        let status = unsafe { RtlGetVersion(&mut info) };
        if status != 0 {
            return Err(win_environment(
                "P1A_WINDOWS_VERSION_FAILED",
                format!("RtlGetVersion failed with NTSTATUS 0x{status:08x}"),
            ));
        }
        Ok(WindowsVersion {
            major: info.major,
            minor: info.minor,
            build: info.build,
            service_pack_major: info.service_pack_major,
            service_pack_minor: info.service_pack_minor,
            product_type: info.product_type,
        })
    }

    fn architecture() -> Result<ArchitectureIdentity> {
        let mut process_machine = 0_u16;
        let mut native_machine = 0_u16;
        // SAFETY: the pseudo-handle is valid and both output pointers are writable.
        let ok = unsafe {
            IsWow64Process2(
                GetCurrentProcess(),
                &mut process_machine,
                &mut native_machine,
            )
        };
        if ok == 0 {
            return Err(last_error(
                "P1A_ARCHITECTURE_QUERY_FAILED",
                "IsWow64Process2 failed",
            ));
        }
        Ok(ArchitectureIdentity {
            process_machine: machine_name(if process_machine == IMAGE_FILE_MACHINE_UNKNOWN {
                native_machine
            } else {
                process_machine
            })
            .to_owned(),
            native_machine: machine_name(native_machine).to_owned(),
            process_is_native: process_machine == IMAGE_FILE_MACHINE_UNKNOWN
                && native_machine == IMAGE_FILE_MACHINE_AMD64,
        })
    }

    fn machine_name(machine: u16) -> &'static str {
        match machine {
            IMAGE_FILE_MACHINE_UNKNOWN => "NATIVE",
            IMAGE_FILE_MACHINE_AMD64 => "AMD64",
            0x014c => "I386",
            0xaa64 => "ARM64",
            0x01c4 => "ARMNT",
            _ => "UNKNOWN",
        }
    }

    fn cpu_identity() -> CpuIdentity {
        use std::arch::x86_64::__cpuid;

        let leaf0 = __cpuid(0);
        let mut vendor_bytes = Vec::with_capacity(12);
        vendor_bytes.extend_from_slice(&leaf0.ebx.to_le_bytes());
        vendor_bytes.extend_from_slice(&leaf0.edx.to_le_bytes());
        vendor_bytes.extend_from_slice(&leaf0.ecx.to_le_bytes());
        let vendor = String::from_utf8_lossy(&vendor_bytes).into_owned();

        let leaf1 = __cpuid(1);
        let base_family = (leaf1.eax >> 8) & 0x0f;
        let base_model = (leaf1.eax >> 4) & 0x0f;
        let extended_family = (leaf1.eax >> 20) & 0xff;
        let extended_model = (leaf1.eax >> 16) & 0x0f;
        let family = if base_family == 0x0f {
            base_family + extended_family
        } else {
            base_family
        };
        let model = if base_family == 0x06 || base_family == 0x0f {
            base_model | (extended_model << 4)
        } else {
            base_model
        };

        let extended_max = __cpuid(0x8000_0000).eax;
        let mut brand_bytes = Vec::with_capacity(48);
        if extended_max >= 0x8000_0004 {
            for leaf in 0x8000_0002..=0x8000_0004 {
                let value = __cpuid(leaf);
                brand_bytes.extend_from_slice(&value.eax.to_le_bytes());
                brand_bytes.extend_from_slice(&value.ebx.to_le_bytes());
                brand_bytes.extend_from_slice(&value.ecx.to_le_bytes());
                brand_bytes.extend_from_slice(&value.edx.to_le_bytes());
            }
        }
        let brand_raw = String::from_utf8_lossy(&brand_bytes);
        let brand = brand_raw
            .trim_matches(char::from(0))
            .split_ascii_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        CpuIdentity {
            vendor,
            brand,
            family,
            model,
            stepping: leaf1.eax & 0x0f,
        }
    }

    fn isa_features() -> IsaFeatures {
        use std::arch::x86_64::{__cpuid, __cpuid_count, _xgetbv};

        let maximum = __cpuid(0).eax;
        let leaf1 = __cpuid(1);
        let osxsave = leaf1.ecx & (1 << 27) != 0;
        let avx_hardware = leaf1.ecx & (1 << 28) != 0;
        // SAFETY: XGETBV is executed only when CPUID reports OSXSAVE.
        let xcr0 = if osxsave { unsafe { _xgetbv(0) } } else { 0 };
        let avx_os_enabled = avx_hardware && xcr0 & 0b110 == 0b110;

        let leaf7 = if maximum >= 7 {
            __cpuid_count(7, 0)
        } else {
            std::arch::x86_64::CpuidResult {
                eax: 0,
                ebx: 0,
                ecx: 0,
                edx: 0,
            }
        };
        let avx512f = leaf7.ebx & (1 << 16) != 0;
        let avx512_os_enabled = avx512f && xcr0 & 0xe6 == 0xe6;

        IsaFeatures {
            sse2: leaf1.edx & (1 << 26) != 0,
            sse3: leaf1.ecx & 1 != 0,
            ssse3: leaf1.ecx & (1 << 9) != 0,
            sse41: leaf1.ecx & (1 << 19) != 0,
            sse42: leaf1.ecx & (1 << 20) != 0,
            popcnt: leaf1.ecx & (1 << 23) != 0,
            aes: leaf1.ecx & (1 << 25) != 0,
            pclmulqdq: leaf1.ecx & (1 << 1) != 0,
            fma: leaf1.ecx & (1 << 12) != 0 && avx_os_enabled,
            bmi1: leaf7.ebx & (1 << 3) != 0,
            bmi2: leaf7.ebx & (1 << 8) != 0,
            avx_hardware,
            avx_os_enabled,
            avx2_hardware: leaf7.ebx & (1 << 5) != 0,
            avx2_os_enabled: leaf7.ebx & (1 << 5) != 0 && avx_os_enabled,
            avx512f_hardware: avx512f,
            avx512_os_enabled,
            avx512dq: leaf7.ebx & (1 << 17) != 0 && avx512_os_enabled,
            avx512cd: leaf7.ebx & (1 << 28) != 0 && avx512_os_enabled,
            avx512bw: leaf7.ebx & (1 << 30) != 0 && avx512_os_enabled,
            avx512vl: leaf7.ebx & (1 << 31) != 0 && avx512_os_enabled,
            sha: leaf7.ebx & (1 << 29) != 0,
            vaes: leaf7.ecx & (1 << 9) != 0 && avx_os_enabled,
            vpclmulqdq: leaf7.ecx & (1 << 10) != 0 && avx_os_enabled,
        }
    }

    fn topology() -> Result<ProcessorTopology> {
        // SAFETY: these functions have no pointer arguments and are read-only.
        let active_group_count = unsafe { GetActiveProcessorGroupCount() };
        let active_logical_processors = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) };
        if active_group_count == 0 || active_logical_processors == 0 {
            return Err(last_error(
                "P1A_PROCESSOR_GROUP_QUERY_FAILED",
                "Windows reported no active processor groups or processors",
            ));
        }
        let records = logical_processor_records(RELATION_PROCESSOR_CORE)?;
        let mut cores = Vec::new();
        let mut cursor = 0_usize;
        while cursor < records.len() {
            if records.len() - cursor < 32 {
                return Err(invalid_topology("truncated processor-core record"));
            }
            let relationship = read_u32(&records, cursor)?;
            let record_size = read_u32(&records, cursor + 4)? as usize;
            if relationship != RELATION_PROCESSOR_CORE
                || record_size < 48
                || cursor.checked_add(record_size).is_none()
                || cursor + record_size > records.len()
            {
                return Err(invalid_topology("invalid processor-core record header"));
            }
            let flags = records[cursor + 8];
            let efficiency_class = records[cursor + 9];
            let group_count = read_u16(&records, cursor + 30)? as usize;
            if group_count == 0 || 32 + group_count * 16 > record_size {
                return Err(invalid_topology("invalid processor-core group count"));
            }
            let mut group_masks = Vec::with_capacity(group_count);
            for index in 0..group_count {
                let offset = cursor + 32 + index * 16;
                let mask = read_usize(&records, offset)? as u64;
                let group = read_u16(&records, offset + size_of::<usize>())?;
                if mask == 0 || group >= active_group_count {
                    return Err(invalid_topology("invalid processor-core group mask"));
                }
                group_masks.push(GroupMask {
                    group,
                    mask,
                    logical_processors: mask.count_ones(),
                });
            }
            cores.push(CoreTopology {
                efficiency_class,
                smt: flags & 1 != 0,
                group_masks,
            });
            cursor += record_size;
        }
        let package_count = count_logical_records(RELATION_PROCESSOR_PACKAGE)?;
        Ok(ProcessorTopology {
            active_group_count,
            active_logical_processors,
            physical_core_count: cores.len() as u32,
            package_count,
            cores,
        })
    }

    fn logical_processor_records(relationship: u32) -> Result<Vec<u8>> {
        let mut bytes = 0_u32;
        // The first call deliberately supplies no buffer and obtains its required size.
        // SAFETY: `bytes` is writable and a null buffer is the documented sizing query.
        unsafe { GetLogicalProcessorInformationEx(relationship, null_mut(), &mut bytes) };
        if !(8..=16 * 1024 * 1024).contains(&bytes) {
            return Err(last_error(
                "P1A_TOPOLOGY_QUERY_FAILED",
                format!("invalid topology buffer length {bytes}"),
            ));
        }
        let mut buffer = vec![0_u8; bytes as usize];
        let mut returned = bytes;
        // SAFETY: the buffer is writable for exactly `returned` bytes.
        let ok = unsafe {
            GetLogicalProcessorInformationEx(relationship, buffer.as_mut_ptr(), &mut returned)
        };
        if ok == 0 || returned == 0 || returned > bytes {
            return Err(last_error(
                "P1A_TOPOLOGY_QUERY_FAILED",
                "GetLogicalProcessorInformationEx failed",
            ));
        }
        buffer.truncate(returned as usize);
        Ok(buffer)
    }

    fn count_logical_records(relationship: u32) -> Result<u32> {
        let records = logical_processor_records(relationship)?;
        let mut cursor = 0_usize;
        let mut count = 0_u32;
        while cursor < records.len() {
            if records.len() - cursor < 8 {
                return Err(invalid_topology("truncated topology record"));
            }
            let observed = read_u32(&records, cursor)?;
            let record_size = read_u32(&records, cursor + 4)? as usize;
            if observed != relationship
                || record_size < 8
                || cursor.checked_add(record_size).is_none()
                || cursor + record_size > records.len()
            {
                return Err(invalid_topology("invalid topology record"));
            }
            count += 1;
            cursor += record_size;
        }
        Ok(count)
    }

    fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
        let value = bytes
            .get(offset..offset + 2)
            .ok_or_else(|| invalid_topology("truncated u16 topology field"))?;
        Ok(u16::from_ne_bytes([value[0], value[1]]))
    }

    fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
        let value = bytes
            .get(offset..offset + 4)
            .ok_or_else(|| invalid_topology("truncated u32 topology field"))?;
        Ok(u32::from_ne_bytes([value[0], value[1], value[2], value[3]]))
    }

    fn read_usize(bytes: &[u8], offset: usize) -> Result<usize> {
        let value = bytes
            .get(offset..offset + size_of::<usize>())
            .ok_or_else(|| invalid_topology("truncated affinity-mask topology field"))?;
        let mut array = [0_u8; size_of::<usize>()];
        array.copy_from_slice(value);
        Ok(usize::from_ne_bytes(array))
    }

    fn invalid_topology(message: impl Into<String>) -> XtaskError {
        XtaskError::integrity("P1A_TOPOLOGY_INVALID", message)
    }

    pub(super) fn current_affinity() -> Result<AffinitySnapshot> {
        let mut process_mask = 0_usize;
        let mut system_mask = 0_usize;
        // SAFETY: the pseudo-handle is valid and both masks are writable.
        let process_ok = unsafe {
            GetProcessAffinityMask(GetCurrentProcess(), &mut process_mask, &mut system_mask)
        };
        if process_ok == 0 {
            return Err(last_error(
                "P1A_PROCESS_AFFINITY_QUERY_FAILED",
                "GetProcessAffinityMask failed",
            ));
        }
        let mut thread_affinity = GroupAffinity {
            mask: 0,
            group: 0,
            reserved: [0; 3],
        };
        // SAFETY: the pseudo-handle is valid and the affinity structure is writable.
        let thread_ok = unsafe { GetThreadGroupAffinity(GetCurrentThread(), &mut thread_affinity) };
        if thread_ok == 0 {
            return Err(last_error(
                "P1A_THREAD_AFFINITY_QUERY_FAILED",
                "GetThreadGroupAffinity failed",
            ));
        }
        Ok(AffinitySnapshot {
            thread_group: thread_affinity.group,
            thread_group_mask: thread_affinity.mask as u64,
            process_mask: process_mask as u64,
            system_mask: system_mask as u64,
        })
    }

    pub(super) fn current_verifier_ancestry() -> Result<BTreeSet<ProcessIdentity>> {
        // Capture one kernel-owned process table and follow only the current process's
        // parent chain. This admits the fixed `cargo run ... xtask` launcher while
        // refusing a Python-launched verifier and arbitrary workload exceptions.
        let raw_snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        let snapshot = OwnedHandle::new(raw_snapshot).ok_or_else(|| {
            last_error(
                "P1A_PROCESS_ANCESTRY_FAILED",
                "CreateToolhelp32Snapshot failed for verifier ancestry",
            )
        })?;
        let mut entry: ProcessEntry32W = unsafe { zeroed() };
        entry.size = size_of::<ProcessEntry32W>() as u32;
        let mut has_entry = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        if !has_entry {
            let error = unsafe { GetLastError() };
            if error != ERROR_NO_MORE_FILES {
                return Err(win_environment(
                    "P1A_PROCESS_ANCESTRY_FAILED",
                    format!("Process32FirstW failed with Win32 error {error}"),
                ));
            }
        }
        let mut parents = BTreeMap::new();
        let mut names = BTreeMap::new();
        let mut observed = 0_usize;
        while has_entry {
            observed += 1;
            if observed > PROCESS_SNAPSHOT_LIMIT {
                return Err(win_environment(
                    "P1A_PROCESS_ANCESTRY_FAILED",
                    format!("process snapshot exceeded {PROCESS_SNAPSHOT_LIMIT} entries"),
                ));
            }
            parents.insert(entry.process_id, entry.parent_process_id);
            let image_name = utf16_fixed(&entry.exe_file);
            validate_process_image_name(&image_name)?;
            names.insert(entry.process_id, image_name);
            entry = unsafe { zeroed() };
            entry.size = size_of::<ProcessEntry32W>() as u32;
            has_entry = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        let final_error = unsafe { GetLastError() };
        if final_error != ERROR_NO_MORE_FILES {
            return Err(win_environment(
                "P1A_PROCESS_ANCESTRY_FAILED",
                format!("Process32NextW failed with Win32 error {final_error}"),
            ));
        }
        let ancestry_process_ids =
            verifier_ancestry_process_ids(&parents, &names, std::process::id())?;
        let mut ancestry = BTreeSet::new();
        for process_id in ancestry_process_ids {
            ancestry.insert(process_identity(process_id)?);
        }
        Ok(ancestry)
    }

    fn verifier_ancestry_process_ids(
        parents: &BTreeMap<u32, u32>,
        names: &BTreeMap<u32, String>,
        verifier_process_id: u32,
    ) -> Result<Vec<u32>> {
        let mut ancestry = Vec::new();
        let mut visited_pids = BTreeSet::new();
        let mut current = verifier_process_id;
        let mut reached_interactive_boundary = false;
        for _ in 0..64 {
            if current == 0 || !visited_pids.insert(current) {
                break;
            }
            let image_name = names.get(&current).ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_PROCESS_ANCESTRY_INVALID",
                    "the verifier ancestry contains a process absent from the kernel snapshot",
                )
            })?;
            if crate::p1a_process::forbidden_python_process_name(image_name) {
                return Err(XtaskError::gate(
                    "P1A_PYTHON_LAUNCHER_REJECTED",
                    "the verifier process ancestry contains a Python executable",
                    "Invoke Cargo directly from an ordinary native Windows shell.",
                ));
            }
            if is_native_interactive_boundary(image_name) {
                // Windows Terminal is activated by a service, so its kernel parent is
                // normally svchost.exe rather than the user process that requested the
                // terminal.  That protected service ancestry neither launched the
                // verifier nor belongs in the CPU-load exception set.  The desktop or
                // terminal process is a boundary marker only and is not itself approved.
                reached_interactive_boundary = true;
                current = 0;
                break;
            }
            ancestry.push(current);
            current = parents.get(&current).copied().unwrap_or(0);
        }
        if !visited_pids.contains(&verifier_process_id)
            || current != 0
            || !reached_interactive_boundary
            || ancestry.is_empty()
        {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_ANCESTRY_INVALID",
                "the verifier launcher ancestry did not reach one closed native interactive boundary",
            ));
        }
        Ok(ancestry)
    }

    fn is_native_interactive_boundary(image_name: &str) -> bool {
        image_name.eq_ignore_ascii_case("WindowsTerminal.exe")
            || image_name.eq_ignore_ascii_case("explorer.exe")
    }

    fn process_identity(process_id: u32) -> Result<ProcessIdentity> {
        // SAFETY: the PID comes from the kernel process snapshot and the access is read-only.
        let process = OwnedHandle::new(unsafe {
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id)
        })
        .ok_or_else(|| {
            last_error(
                "P1A_PROCESS_IDENTITY_OPEN_FAILED",
                "could not open a verifier-ancestry process",
            )
        })?;
        let mut creation = FileTime::default();
        let mut exit = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        // SAFETY: the process handle is valid and every output buffer is writable.
        if unsafe { GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user) }
            == 0
        {
            return Err(last_error(
                "P1A_PROCESS_IDENTITY_QUERY_FAILED",
                "could not bind a verifier-ancestry process creation time",
            ));
        }
        let creation_time_100ns = file_time(creation);
        if creation_time_100ns == 0 {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_IDENTITY_INVALID",
                "a verifier-ancestry process has a zero creation time",
            ));
        }
        Ok(ProcessIdentity {
            process_id,
            creation_time_100ns,
        })
    }

    fn power_policy() -> Result<PowerPolicySnapshot> {
        let mut scheme_pointer: *mut Guid = null_mut();
        // SAFETY: the output pointer is writable; the API allocates a GUID with LocalAlloc.
        let status = unsafe { PowerGetActiveScheme(null_mut(), &mut scheme_pointer) };
        if status != 0 || scheme_pointer.is_null() {
            return Err(win_environment(
                "P1A_POWER_SCHEME_QUERY_FAILED",
                format!("PowerGetActiveScheme failed with Win32 status {status}"),
            ));
        }
        // Copy before freeing the API-owned allocation.
        // SAFETY: a successful PowerGetActiveScheme returned one initialized GUID.
        let scheme = unsafe { *scheme_pointer };
        // SAFETY: `scheme_pointer` is the LocalAlloc allocation returned above.
        unsafe { LocalFree(scheme_pointer.cast()) };

        let mut status_info: SystemPowerStatus = unsafe { zeroed() };
        // SAFETY: `status_info` is a writable SYSTEM_POWER_STATUS buffer.
        if unsafe { GetSystemPowerStatus(&mut status_info) } == 0 {
            return Err(last_error(
                "P1A_POWER_SOURCE_QUERY_FAILED",
                "GetSystemPowerStatus failed",
            ));
        }
        let read = |setting: &Guid, label: &'static str| -> Result<u32> {
            let mut value = 0_u32;
            // SAFETY: all GUID pointers are valid for the call and `value` is writable.
            let code = unsafe {
                PowerReadACValueIndex(
                    null_mut(),
                    &scheme,
                    &PROCESSOR_SETTINGS_SUBGROUP,
                    setting,
                    &mut value,
                )
            };
            if code != 0 {
                return Err(win_environment(
                    "P1A_POWER_POLICY_QUERY_FAILED",
                    format!("could not read {label}; Win32 status {code}"),
                ));
            }
            Ok(value)
        };
        Ok(PowerPolicySnapshot {
            active_scheme_guid: format_guid(&scheme),
            active_scheme_name: power_scheme_name(&scheme)?,
            ac_line_status: match status_info.ac_line_status {
                0 => "offline",
                1 => "online",
                _ => "unknown",
            }
            .to_owned(),
            processor_minimum_percent: read(
                &PROCESSOR_THROTTLE_MINIMUM,
                "minimum processor state",
            )?,
            processor_maximum_percent: read(
                &PROCESSOR_THROTTLE_MAXIMUM,
                "maximum processor state",
            )?,
            processor_boost_mode: read(&PROCESSOR_PERF_BOOST_MODE, "processor boost mode")?,
            energy_performance_preference: read(
                &PROCESSOR_ENERGY_PERFORMANCE_PREFERENCE,
                "energy performance preference",
            )?,
            value_source: "ac".to_owned(),
        })
    }

    fn power_scheme_name(scheme: &Guid) -> Result<String> {
        let mut bytes = 0_u32;
        // SAFETY: this is the documented sizing query for the scheme's friendly name.
        let first = unsafe {
            PowerReadFriendlyName(
                null_mut(),
                scheme,
                null_mut(),
                null_mut(),
                null_mut(),
                &mut bytes,
            )
        };
        if (first != 0 && first != 234)
            || !(2..=64 * 1024).contains(&bytes)
            || !bytes.is_multiple_of(2)
        {
            return Err(win_environment(
                "P1A_POWER_SCHEME_NAME_QUERY_FAILED",
                format!("friendly-name sizing query failed with status {first} and size {bytes}"),
            ));
        }
        let mut units = vec![0_u16; bytes as usize / 2];
        // SAFETY: the UTF-16 buffer is writable for the size returned above.
        let second = unsafe {
            PowerReadFriendlyName(
                null_mut(),
                scheme,
                null_mut(),
                null_mut(),
                units.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if second != 0 {
            return Err(win_environment(
                "P1A_POWER_SCHEME_NAME_QUERY_FAILED",
                format!("friendly-name query failed with status {second}"),
            ));
        }
        while units.last() == Some(&0) {
            units.pop();
        }
        let name = String::from_utf16(&units).map_err(|error| {
            XtaskError::integrity(
                "P1A_POWER_SCHEME_NAME_INVALID",
                format!("power scheme name is not valid UTF-16: {error}"),
            )
        })?;
        if name.is_empty() || name.contains('\r') || name.contains('\n') {
            return Err(XtaskError::integrity(
                "P1A_POWER_SCHEME_NAME_INVALID",
                "power scheme name is not a nonempty single line",
            ));
        }
        Ok(name)
    }

    fn format_guid(guid: &Guid) -> String {
        format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            guid.data1,
            guid.data2,
            guid.data3,
            guid.data4[0],
            guid.data4[1],
            guid.data4[2],
            guid.data4[3],
            guid.data4[4],
            guid.data4[5],
            guid.data4[6],
            guid.data4[7]
        )
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct VswhereInstance {
        instance_id: String,
        product_id: String,
        installation_version: String,
        installation_path: PathBuf,
        is_complete: bool,
        is_launchable: bool,
        is_reboot_required: bool,
    }

    pub(super) fn discover_vswhere_path() -> Result<PathBuf> {
        let program_files_x86 = known_folder(&FOLDERID_PROGRAM_FILES_X86)?;
        require_regular_file(
            &program_files_x86
                .join("Microsoft Visual Studio")
                .join("Installer")
                .join("vswhere.exe"),
            "P1A_VSWHERE_NOT_FOUND",
        )
    }

    pub(super) fn bind_vswhere_runtime() -> Result<VswhereRuntimeBinding> {
        // Validate the native 32-bit system directory up front. The process auditor
        // resolves it independently when classifying loaded images; this discovery
        // makes C013 fail before launch if the host does not expose the canonical
        // System32/SysWOW64 sibling layout.
        system_wow64_directory()?;

        let program_data = known_folder(&FOLDERID_PROGRAM_DATA)?;
        let setup_x86_root = require_directory(
            &program_data
                .join("Microsoft")
                .join("VisualStudio")
                .join("Setup")
                .join("x86"),
            "P1A_SETUP_CONFIGURATION_ROOT_INVALID",
        )?;
        require_contained(
            &setup_x86_root,
            &program_data,
            "P1A_SETUP_CONFIGURATION_ROOT_ESCAPE",
        )?;
        let setup_configuration_path =
            setup_x86_root.join("Microsoft.VisualStudio.Setup.Configuration.Native.dll");
        let (setup_configuration, setup_configuration_lock) =
            bind_locked_tool_identity(&setup_configuration_path)?;
        if setup_configuration.path.parent() != Some(setup_x86_root.as_path()) {
            return Err(XtaskError::integrity(
                "P1A_SETUP_CONFIGURATION_PATH_ESCAPE",
                "the canonical Setup Configuration implementation escaped its exact x86 root",
            ));
        }
        Ok(VswhereRuntimeBinding {
            setup_configuration,
            _setup_configuration_lock: setup_configuration_lock,
        })
    }

    pub(super) fn discover_git_path() -> Result<(PathBuf, PathBuf)> {
        let (program_files, _) = native_program_files_roots()?;
        let git_root = require_directory(&program_files.join("Git"), "P1A_GIT_ROOT_NOT_FOUND")?;
        require_contained(&git_root, &program_files, "P1A_GIT_ROOT_ESCAPE")?;
        let command_git =
            require_regular_file(&git_root.join("cmd").join("git.exe"), "P1A_GIT_NOT_FOUND")?;
        let command_identity = tool_identity(&command_git)?;
        let alternate = git_root.join("bin").join("git.exe");
        match fs::symlink_metadata(&alternate) {
            Ok(_) => {
                let alternate = require_regular_file(&alternate, "P1A_GIT_ALTERNATE_INVALID")?;
                let alternate_identity = tool_identity(&alternate)?;
                if command_identity.sha256 != alternate_identity.sha256
                    || command_identity.bytes != alternate_identity.bytes
                    || command_identity.file_version != alternate_identity.file_version
                {
                    return Err(XtaskError::integrity(
                        "P1A_GIT_IDENTITY_AMBIGUOUS",
                        "Program Files Git exposes nonidentical cmd and bin git.exe launchers",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(XtaskError::environment(
                    "P1A_GIT_ALTERNATE_INVALID",
                    format!("could not inspect the alternate Program Files Git launcher: {error}"),
                ));
            }
        }
        Ok((command_git, git_root))
    }

    pub(super) fn native_program_files_roots() -> Result<(PathBuf, PathBuf)> {
        let program_files = known_folder(&FOLDERID_PROGRAM_FILES)?;
        let program_files_x86 = known_folder(&FOLDERID_PROGRAM_FILES_X86)?;
        if program_files == program_files_x86
            || program_files.file_name().and_then(OsStr::to_str) != Some("Program Files")
            || program_files_x86.file_name().and_then(OsStr::to_str) != Some("Program Files (x86)")
            || program_files.parent() != program_files_x86.parent()
        {
            return Err(XtaskError::integrity(
                "P1A_PROGRAM_FILES_ROOT_INVALID",
                "native known-folder APIs did not return distinct sibling Program Files roots",
            ));
        }
        Ok((program_files, program_files_x86))
    }

    pub(super) fn loader_resolved_system_runtime() -> Result<WindowsRuntimeIdentities> {
        let system_directory = system_directory()?;
        let ucrtbase = loader_resolved_system_module("ucrtbase.dll", &system_directory)?;
        let vcruntime = loader_resolved_system_module("vcruntime140.dll", &system_directory)?;
        Ok(WindowsRuntimeIdentities {
            resolution_policy: "windows-system32-safe-search-v1".to_owned(),
            system_directory,
            ucrtbase,
            vcruntime,
        })
    }

    fn system_directory() -> Result<PathBuf> {
        let mut buffer = vec![0_u16; 32_768];
        // SAFETY: the buffer is writable for the supplied number of UTF-16 code units.
        let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if length == 0 || length as usize >= buffer.len() {
            return Err(last_error(
                "P1A_SYSTEM_DIRECTORY_QUERY_FAILED",
                "GetSystemDirectoryW did not return one bounded system directory",
            ));
        }
        buffer.truncate(length as usize);
        let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer));
        require_directory(&path, "P1A_SYSTEM_DIRECTORY_INVALID")
    }

    fn system_wow64_directory() -> Result<PathBuf> {
        let mut buffer = vec![0_u16; SYSWOW64_DIRECTORY_BUFFER_UNITS];
        // SAFETY: the native x86_64 process supplies one writable UTF-16 buffer with
        // its exact capacity to the documented SysWOW64 directory query.
        let length = unsafe { GetSystemWow64DirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if length == 0 || length as usize >= buffer.len() {
            return Err(last_error(
                "P1A_SYSWOW64_DIRECTORY_QUERY_FAILED",
                "GetSystemWow64DirectoryW did not return one bounded system directory",
            ));
        }
        buffer.truncate(length as usize);
        trim_native_path_terminators(&mut buffer, "P1A_SYSWOW64_DIRECTORY_INVALID")?;
        let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer));
        let syswow64 = require_directory(&path, "P1A_SYSWOW64_DIRECTORY_INVALID")?;
        let system32 = system_directory()?;
        let has_expected_leaf = syswow64
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|leaf| leaf.eq_ignore_ascii_case("SysWOW64"));
        if !has_expected_leaf
            || syswow64 == system32
            || syswow64.parent().is_none()
            || syswow64.parent() != system32.parent()
        {
            return Err(XtaskError::integrity(
                "P1A_SYSWOW64_DIRECTORY_INVALID",
                "the native SysWOW64 directory is not the distinct canonical sibling of System32",
            ));
        }
        Ok(syswow64)
    }

    fn trim_native_path_terminators(units: &mut Vec<u16>, code: &'static str) -> Result<()> {
        while units.last() == Some(&0) {
            units.pop();
        }
        if units.is_empty() || units.contains(&0) {
            return Err(XtaskError::integrity(
                code,
                "native directory discovery returned an empty path or embedded NUL",
            ));
        }
        Ok(())
    }

    fn loader_resolved_system_module(
        module_name: &str,
        system_directory: &Path,
    ) -> Result<ToolFileIdentity> {
        let module_name_wide = wide_null(module_name);
        // SAFETY: the module name is NUL-terminated, no file handle is supplied, and the
        // documented flag restricts resolution to the Windows system directory.
        let raw_module = unsafe {
            LoadLibraryExW(
                module_name_wide.as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        let module = OwnedModule::new(raw_module).ok_or_else(|| {
            last_error(
                "P1A_SYSTEM_RUNTIME_LOAD_FAILED",
                format!("LoadLibraryExW could not resolve {module_name} from System32"),
            )
        })?;
        let mut buffer = vec![0_u16; 32_768];
        // SAFETY: the module handle is owned and the output buffer is writable.
        let length =
            unsafe { GetModuleFileNameW(module.0, buffer.as_mut_ptr(), buffer.len() as u32) };
        if length == 0 || length as usize >= buffer.len() {
            return Err(last_error(
                "P1A_SYSTEM_RUNTIME_PATH_FAILED",
                format!("GetModuleFileNameW returned no bounded path for {module_name}"),
            ));
        }
        buffer.truncate(length as usize);
        let loader_path = PathBuf::from(std::ffi::OsString::from_wide(&buffer));
        let loader_path = require_regular_file(&loader_path, "P1A_SYSTEM_RUNTIME_PATH_INVALID")?;
        let expected_path = require_regular_file(
            &system_directory.join(module_name),
            "P1A_SYSTEM_RUNTIME_PATH_INVALID",
        )?;
        if loader_path != expected_path {
            return Err(XtaskError::integrity(
                "P1A_SYSTEM_RUNTIME_REDIRECTED",
                format!(
                    "the Windows loader resolved {module_name} outside the canonical system directory"
                ),
            ));
        }
        tool_identity(&loader_path)
    }

    #[derive(Serialize)]
    struct QualifiedFileRecord {
        logical_name: String,
        path_utf16: Vec<u16>,
        sha256: String,
        bytes: u64,
        file_version: Option<String>,
    }

    #[derive(Serialize)]
    struct SelectionCandidateRecord {
        instance_id: String,
        product_id: String,
        installation_version: String,
        installation_path_utf16: Vec<u16>,
        complete: bool,
        launchable: bool,
        reboot_required: bool,
    }

    #[derive(Serialize)]
    struct SelectionManifest {
        schema: String,
        profile_id: String,
        visual_studio_discovery_method: String,
        vswhere_query: Vec<String>,
        visual_studio_candidates: Vec<SelectionCandidateRecord>,
        selected_visual_studio_instance_id: String,
        selected_visual_studio_product_id: String,
        visual_studio_installation_version: String,
        visual_studio_installation_path_utf16: Vec<u16>,
        msvc_tools_version: String,
        msvc_tools_root_utf16: Vec<u16>,
        msvc_runtime_redist_version: String,
        windows_sdk_discovery_method: String,
        windows_kits_root_utf16: Vec<u16>,
        windows_sdk_version: String,
        ucrt_version: String,
        system_runtime_resolution_policy: String,
        system_directory_utf16: Vec<u16>,
    }

    #[derive(Serialize)]
    struct TreeEntryRecord {
        root_label: String,
        relative_components_utf16: Vec<Vec<u16>>,
        kind: String,
        sha256: Option<String>,
        bytes: u64,
    }

    #[derive(Serialize)]
    struct SnapshotBundleManifest {
        algorithm: String,
        selection_manifest_sha256: String,
        qualified_file_manifest_sha256: String,
        qualified_file_count: u32,
        qualified_file_total_bytes: u64,
        msvc_include_tree: TreeIdentity,
        msvc_x64_lib_tree: TreeIdentity,
        windows_sdk_include_tree: TreeIdentity,
        windows_sdk_x64_lib_tree: TreeIdentity,
    }

    pub(super) fn snapshot_host_toolchain(
        report: &PrototypeWindowsHostReport,
    ) -> Result<HostToolchainStabilitySnapshot> {
        if report.schema != "python-slm-p1a-windows-host-probe-v1"
            || report.profile_id != "prototype-windows-5090-v1"
        {
            return Err(XtaskError::integrity(
                "P1A_TOOLCHAIN_SNAPSHOT_REPORT_INVALID",
                "the host report has an unexpected schema or profile identity",
            ));
        }

        let version_text = read_locked_text(&report.visual_studio.msvc_version_file.path, 4096)?;
        if version_text.trim() != report.visual_studio.msvc_tools_version {
            return Err(XtaskError::integrity(
                "P1A_MSVC_SELECTION_DRIFT",
                "the selected MSVC version file no longer names the qualified tools version",
            ));
        }
        let msvc_version_parts = numeric_version(&report.visual_studio.msvc_tools_version)
            .filter(|parts| parts.len() == 3 && parts[0] == 14)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_MSVC_SELECTION_INVALID",
                    "the host report does not contain an exact three-part MSVC 14 version",
                )
            })?;
        let (redist_version, redist_identity) =
            select_vcruntime(&report.visual_studio.installation_path, &msvc_version_parts)?;
        if redist_version != report.visual_studio.vcruntime_redist_version
            || redist_identity != report.visual_studio.vcruntime_dll
        {
            return Err(XtaskError::integrity(
                "P1A_MSVC_RUNTIME_SELECTION_DRIFT",
                "the deterministic MSVC redistributable selection changed after host discovery",
            ));
        }
        let selected_sdk = windows_sdk()?;
        if selected_sdk != report.windows_sdk {
            return Err(XtaskError::integrity(
                "P1A_WINDOWS_SDK_SELECTION_DRIFT",
                "the deterministic Windows SDK selection changed after host discovery",
            ));
        }
        let selected_runtime = loader_resolved_system_runtime()?;
        if selected_runtime != report.system_runtime {
            return Err(XtaskError::integrity(
                "P1A_SYSTEM_RUNTIME_SELECTION_DRIFT",
                "the loader-resolved Windows runtime identity changed after host discovery",
            ));
        }

        let mut files = Vec::with_capacity(QUALIFIED_FILE_COUNT as usize);
        record_tool_identity("vswhere", &report.visual_studio.vswhere, &mut files)?;
        record_file_identity(
            "msvc_version_file",
            &report.visual_studio.msvc_version_file,
            &mut files,
        )?;
        record_tool_identity("msvc_cl", &report.visual_studio.cl, &mut files)?;
        record_tool_identity(
            "msvc_c_frontend",
            &report.visual_studio.c_frontend,
            &mut files,
        )?;
        record_tool_identity(
            "msvc_cpp_frontend",
            &report.visual_studio.cpp_frontend,
            &mut files,
        )?;
        record_tool_identity(
            "msvc_code_generator",
            &report.visual_studio.code_generator,
            &mut files,
        )?;
        record_tool_identity("msvc_link", &report.visual_studio.link, &mut files)?;
        record_tool_identity("msvc_lib", &report.visual_studio.lib, &mut files)?;
        record_tool_identity("msvc_dumpbin", &report.visual_studio.dumpbin, &mut files)?;
        record_file_identity(
            "msvc_vcruntime_lib",
            &report.visual_studio.vcruntime_lib,
            &mut files,
        )?;
        record_tool_identity(
            "compiler_redist_vcruntime",
            &report.visual_studio.vcruntime_dll,
            &mut files,
        )?;
        record_file_identity(
            "windows_header",
            &report.windows_sdk.windows_header,
            &mut files,
        )?;
        record_file_identity("ucrt_header", &report.windows_sdk.ucrt_header, &mut files)?;
        record_file_identity("kernel32_lib", &report.windows_sdk.kernel32_lib, &mut files)?;
        record_file_identity("ucrt_lib", &report.windows_sdk.ucrt_lib, &mut files)?;
        record_tool_identity("windows_rc", &report.windows_sdk.rc, &mut files)?;
        record_tool_identity("windows_mt", &report.windows_sdk.mt, &mut files)?;
        record_tool_identity(
            "loader_resolved_ucrtbase",
            &report.system_runtime.ucrtbase,
            &mut files,
        )?;
        record_tool_identity(
            "loader_resolved_vcruntime",
            &report.system_runtime.vcruntime,
            &mut files,
        )?;
        files.sort_by(|left, right| left.logical_name.cmp(&right.logical_name));
        if files.len() != QUALIFIED_FILE_COUNT as usize {
            return Err(XtaskError::integrity(
                "P1A_TOOLCHAIN_FILE_SET_INVALID",
                "the closed host toolchain file inventory does not contain exactly nineteen entries",
            ));
        }
        let qualified_file_total_bytes = files.iter().try_fold(0_u64, |total, file| {
            total.checked_add(file.bytes).ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_TOOLCHAIN_SIZE_OVERFLOW",
                    "the qualified host toolchain file-size total overflowed u64",
                )
            })
        })?;
        let qualified_file_manifest_sha256 =
            hash_serialized("P1A_TOOLCHAIN_MANIFEST_SERIALIZATION_FAILED", &files)?;

        let candidates = report
            .visual_studio
            .candidates
            .iter()
            .map(|candidate| SelectionCandidateRecord {
                instance_id: candidate.instance_id.clone(),
                product_id: candidate.product_id.clone(),
                installation_version: candidate.installation_version.clone(),
                installation_path_utf16: path_utf16(&candidate.installation_path),
                complete: candidate.complete,
                launchable: candidate.launchable,
                reboot_required: candidate.reboot_required,
            })
            .collect();
        let selection = SelectionManifest {
            schema: report.schema.clone(),
            profile_id: report.profile_id.clone(),
            visual_studio_discovery_method: report.visual_studio.discovery_method.clone(),
            vswhere_query: report.visual_studio.query.clone(),
            visual_studio_candidates: candidates,
            selected_visual_studio_instance_id: report.visual_studio.selected_instance_id.clone(),
            selected_visual_studio_product_id: report.visual_studio.product_id.clone(),
            visual_studio_installation_version: report.visual_studio.installation_version.clone(),
            visual_studio_installation_path_utf16: path_utf16(
                &report.visual_studio.installation_path,
            ),
            msvc_tools_version: report.visual_studio.msvc_tools_version.clone(),
            msvc_tools_root_utf16: path_utf16(&report.visual_studio.msvc_tools_root),
            msvc_runtime_redist_version: report.visual_studio.vcruntime_redist_version.clone(),
            windows_sdk_discovery_method: report.windows_sdk.discovery_method.clone(),
            windows_kits_root_utf16: path_utf16(&report.windows_sdk.kits_root),
            windows_sdk_version: report.windows_sdk.version.clone(),
            ucrt_version: report.windows_sdk.ucrt_version.clone(),
            system_runtime_resolution_policy: report.system_runtime.resolution_policy.clone(),
            system_directory_utf16: path_utf16(&report.system_runtime.system_directory),
        };
        let selection_manifest_sha256 =
            hash_serialized("P1A_TOOLCHAIN_SELECTION_SERIALIZATION_FAILED", &selection)?;

        let msvc_include_tree =
            tree_identity(&[("include", report.visual_studio.msvc_include.as_path())])?;
        let msvc_x64_lib_tree =
            tree_identity(&[("x64", report.visual_studio.msvc_x64_lib.as_path())])?;
        let windows_sdk_include_root = report
            .windows_sdk
            .kits_root
            .join("Include")
            .join(&report.windows_sdk.version);
        let windows_sdk_include_tree =
            tree_identity(&[("include", windows_sdk_include_root.as_path())])?;
        let windows_sdk_ucrt_lib_root = report
            .windows_sdk
            .kits_root
            .join("Lib")
            .join(&report.windows_sdk.version)
            .join("ucrt")
            .join("x64");
        let windows_sdk_um_lib_root = report
            .windows_sdk
            .kits_root
            .join("Lib")
            .join(&report.windows_sdk.version)
            .join("um")
            .join("x64");
        let windows_sdk_x64_lib_tree = tree_identity(&[
            ("ucrt", windows_sdk_ucrt_lib_root.as_path()),
            ("um", windows_sdk_um_lib_root.as_path()),
        ])?;
        let bundle = SnapshotBundleManifest {
            algorithm: "sha256-qualified-files-and-selected-trees-v1".to_owned(),
            selection_manifest_sha256: selection_manifest_sha256.clone(),
            qualified_file_manifest_sha256: qualified_file_manifest_sha256.clone(),
            qualified_file_count: QUALIFIED_FILE_COUNT,
            qualified_file_total_bytes,
            msvc_include_tree: msvc_include_tree.clone(),
            msvc_x64_lib_tree: msvc_x64_lib_tree.clone(),
            windows_sdk_include_tree: windows_sdk_include_tree.clone(),
            windows_sdk_x64_lib_tree: windows_sdk_x64_lib_tree.clone(),
        };
        let bundle_sha256 = hash_serialized("P1A_TOOLCHAIN_BUNDLE_SERIALIZATION_FAILED", &bundle)?;
        Ok(HostToolchainStabilitySnapshot {
            selection_manifest_sha256,
            qualified_file_manifest_sha256,
            qualified_file_count: QUALIFIED_FILE_COUNT,
            qualified_file_total_bytes,
            msvc_include_tree,
            msvc_x64_lib_tree,
            windows_sdk_include_tree,
            windows_sdk_x64_lib_tree,
            bundle_sha256,
        })
    }

    fn record_tool_identity(
        logical_name: &str,
        expected: &ToolFileIdentity,
        records: &mut Vec<QualifiedFileRecord>,
    ) -> Result<()> {
        let observed = tool_identity(&expected.path)?;
        if observed != *expected {
            return Err(XtaskError::integrity(
                "P1A_TOOLCHAIN_FILE_IDENTITY_DRIFT",
                format!("qualified tool identity changed for {logical_name}"),
            ));
        }
        records.push(QualifiedFileRecord {
            logical_name: logical_name.to_owned(),
            path_utf16: path_utf16(&observed.path),
            sha256: observed.sha256,
            bytes: observed.bytes,
            file_version: Some(observed.file_version),
        });
        Ok(())
    }

    fn record_file_identity(
        logical_name: &str,
        expected: &FileIdentity,
        records: &mut Vec<QualifiedFileRecord>,
    ) -> Result<()> {
        let observed = file_identity(&expected.path)?;
        if observed != *expected {
            return Err(XtaskError::integrity(
                "P1A_TOOLCHAIN_FILE_IDENTITY_DRIFT",
                format!("qualified file identity changed for {logical_name}"),
            ));
        }
        records.push(QualifiedFileRecord {
            logical_name: logical_name.to_owned(),
            path_utf16: path_utf16(&observed.path),
            sha256: observed.sha256,
            bytes: observed.bytes,
            file_version: None,
        });
        Ok(())
    }

    fn tree_identity(roots: &[(&str, &Path)]) -> Result<TreeIdentity> {
        if roots.is_empty() {
            return Err(XtaskError::integrity(
                "P1A_TOOLCHAIN_TREE_INVALID",
                "a selected toolchain tree has no roots",
            ));
        }
        let mut labels = BTreeSet::new();
        let mut entries = Vec::new();
        for (label, root) in roots {
            if label.is_empty()
                || label.chars().any(char::is_control)
                || !labels.insert((*label).to_owned())
            {
                return Err(XtaskError::integrity(
                    "P1A_TOOLCHAIN_TREE_INVALID",
                    "a selected toolchain tree has an empty, duplicate, or malformed root label",
                ));
            }
            let root = require_directory(root, "P1A_TOOLCHAIN_TREE_INVALID")?;
            walk_tree(label, &root, &root, &mut entries)?;
        }
        entries.sort_by(|left, right| {
            left.root_label
                .cmp(&right.root_label)
                .then_with(|| {
                    left.relative_components_utf16
                        .cmp(&right.relative_components_utf16)
                })
                .then_with(|| left.kind.cmp(&right.kind))
        });
        if entries.len() as u64 > TREE_ENTRY_LIMIT {
            return Err(XtaskError::integrity(
                "P1A_TOOLCHAIN_TREE_LIMIT_EXCEEDED",
                "a selected toolchain tree exceeded one million entries",
            ));
        }
        let directory_count = entries
            .iter()
            .filter(|entry| entry.kind == "directory")
            .count() as u64;
        let file_count = entries.iter().filter(|entry| entry.kind == "file").count() as u64;
        if directory_count == 0 || file_count == 0 {
            return Err(XtaskError::integrity(
                "P1A_TOOLCHAIN_TREE_EMPTY",
                "a selected toolchain tree contains no directory or no regular file",
            ));
        }
        let total_bytes = entries.iter().try_fold(0_u64, |total, entry| {
            total.checked_add(entry.bytes).ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_TOOLCHAIN_TREE_SIZE_OVERFLOW",
                    "a selected toolchain tree byte total overflowed u64",
                )
            })
        })?;
        Ok(TreeIdentity {
            sha256: hash_serialized("P1A_TOOLCHAIN_TREE_SERIALIZATION_FAILED", &entries)?,
            directory_count,
            file_count,
            total_bytes,
        })
    }

    fn walk_tree(
        label: &str,
        root: &Path,
        directory: &Path,
        entries: &mut Vec<TreeEntryRecord>,
    ) -> Result<()> {
        if entries.len() as u64 >= TREE_ENTRY_LIMIT {
            return Err(XtaskError::integrity(
                "P1A_TOOLCHAIN_TREE_LIMIT_EXCEEDED",
                "a selected toolchain tree exceeded one million entries",
            ));
        }
        let relative = directory.strip_prefix(root).map_err(|_| {
            XtaskError::integrity(
                "P1A_TOOLCHAIN_TREE_ESCAPE",
                "a selected toolchain directory escaped its canonical root",
            )
        })?;
        entries.push(TreeEntryRecord {
            root_label: label.to_owned(),
            relative_components_utf16: path_components_utf16(relative),
            kind: "directory".to_owned(),
            sha256: None,
            bytes: 0,
        });
        let mut children = fs::read_dir(directory)
            .io_context(
                "P1A_TOOLCHAIN_TREE_ENUMERATION_FAILED",
                "could not enumerate a selected toolchain tree",
            )?
            .collect::<std::io::Result<Vec<_>>>()
            .io_context(
                "P1A_TOOLCHAIN_TREE_ENUMERATION_FAILED",
                "could not read a selected toolchain tree entry",
            )?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            if entries.len() as u64 >= TREE_ENTRY_LIMIT {
                return Err(XtaskError::integrity(
                    "P1A_TOOLCHAIN_TREE_LIMIT_EXCEEDED",
                    "a selected toolchain tree exceeded one million entries",
                ));
            }
            let path = child.path();
            let metadata = fs::symlink_metadata(&path).io_context(
                "P1A_TOOLCHAIN_TREE_ENTRY_INVALID",
                "could not inspect a selected toolchain tree entry",
            )?;
            let file_type = child.file_type().io_context(
                "P1A_TOOLCHAIN_TREE_ENTRY_INVALID",
                "could not classify a selected toolchain tree entry",
            )?;
            if file_type.is_symlink()
                || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(XtaskError::integrity(
                    "P1A_TOOLCHAIN_TREE_REPARSE_REJECTED",
                    "a selected toolchain tree crosses a symlink, junction, or reparse point",
                ));
            }
            if file_type.is_dir() {
                walk_tree(label, root, &path, entries)?;
            } else if file_type.is_file() {
                let identity = file_identity(&path)?;
                let relative = identity.path.strip_prefix(root).map_err(|_| {
                    XtaskError::integrity(
                        "P1A_TOOLCHAIN_TREE_ESCAPE",
                        "a selected toolchain file escaped its canonical root",
                    )
                })?;
                entries.push(TreeEntryRecord {
                    root_label: label.to_owned(),
                    relative_components_utf16: path_components_utf16(relative),
                    kind: "file".to_owned(),
                    sha256: Some(identity.sha256),
                    bytes: identity.bytes,
                });
            } else {
                return Err(XtaskError::integrity(
                    "P1A_TOOLCHAIN_TREE_ENTRY_INVALID",
                    "a selected toolchain tree contains a non-file, non-directory entry",
                ));
            }
        }
        Ok(())
    }

    fn hash_serialized<T: Serialize>(code: &'static str, value: &T) -> Result<String> {
        let bytes = serde_json::to_vec(value).map_err(|error| {
            XtaskError::integrity(code, format!("could not serialize hash manifest: {error}"))
        })?;
        Ok(crate::hash::bytes(&bytes))
    }

    fn path_utf16(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().collect()
    }

    fn path_components_utf16(path: &Path) -> Vec<Vec<u16>> {
        path.components()
            .map(|component| component.as_os_str().encode_wide().collect())
            .collect()
    }

    pub(super) fn visual_studio(
        requested_instance: Option<&str>,
        audited_vswhere_stdout: &[u8],
    ) -> Result<VisualStudioToolchain> {
        let vswhere_path = discover_vswhere_path()?;
        if audited_vswhere_stdout.len() > VSWHERE_OUTPUT_LIMIT {
            return Err(win_environment(
                "P1A_VSWHERE_OUTPUT_LIMIT_EXCEEDED",
                "audited vswhere stdout exceeded the 4 MiB capture limit",
            ));
        }
        let mut instances: Vec<VswhereInstance> = serde_json::from_slice(audited_vswhere_stdout)
            .map_err(|error| {
                XtaskError::integrity(
                    "P1A_VSWHERE_JSON_INVALID",
                    format!("vswhere returned invalid JSON: {error}"),
                )
            })?;
        if instances.is_empty() || instances.len() > 128 {
            return Err(XtaskError::integrity(
                "P1A_VSWHERE_INVENTORY_INVALID",
                "vswhere returned an empty or unbounded candidate inventory",
            ));
        }
        let mut instance_ids = BTreeSet::new();
        let mut installation_paths = BTreeSet::new();
        for candidate in &mut instances {
            let version = numeric_version(&candidate.installation_version);
            let canonical_path = require_directory(
                &candidate.installation_path,
                "P1A_VSWHERE_CANDIDATE_PATH_INVALID",
            )?;
            let normalized_id = candidate.instance_id.to_ascii_lowercase();
            let normalized_path = canonical_path
                .to_string_lossy()
                .replace('/', "\\")
                .to_ascii_lowercase();
            if candidate.instance_id.trim() != candidate.instance_id
                || candidate.instance_id.is_empty()
                || candidate.instance_id.len() > 256
                || candidate.instance_id.chars().any(char::is_control)
                || !instance_ids.insert(normalized_id)
                || !valid_product_id(&candidate.product_id)
                || !valid_vs_installation_version(version.as_deref())
                || !candidate.installation_path.is_absolute()
                || !installation_paths.insert(normalized_path)
            {
                return Err(XtaskError::integrity(
                    "P1A_VSWHERE_INVENTORY_INVALID",
                    "vswhere returned a malformed, duplicate, non-VS2022, relative, or aliased candidate",
                ));
            }
            candidate.installation_path = canonical_path;
        }
        instances.sort_by(|left, right| {
            numeric_version(&left.installation_version)
                .cmp(&numeric_version(&right.installation_version))
                .then_with(|| left.instance_id.cmp(&right.instance_id))
        });
        let qualified = instances
            .iter()
            .filter(|candidate| {
                candidate.is_complete && candidate.is_launchable && !candidate.is_reboot_required
            })
            .collect::<Vec<_>>();
        if qualified.is_empty() {
            return Err(win_environment(
                "P1A_VS2022_NOT_FOUND",
                "vswhere found no complete, launchable, non-reboot-pending VS 2022 x64 C++ instance",
            ));
        }
        let selected_index = if let Some(instance_id) = requested_instance {
            qualified
                .iter()
                .position(|candidate| candidate.instance_id == instance_id)
                .ok_or_else(|| {
                    win_environment(
                        "P1A_VS_INSTANCE_NOT_FOUND",
                        format!("the frozen VS instance {instance_id:?} was not qualified"),
                    )
                })?
        } else if qualified.len() == 1 {
            0
        } else {
            return Err(XtaskError::gate(
                "P1A_VS_INSTANCE_AMBIGUOUS",
                format!(
                    "{} complete VS 2022 x64 C++ instances are installed",
                    qualified.len()
                ),
                "Freeze one exact vswhere instanceId in the P1A input policy and rerun.",
            ));
        };

        let candidates = instances
            .iter()
            .map(|candidate| VisualStudioCandidate {
                instance_id: candidate.instance_id.clone(),
                product_id: candidate.product_id.clone(),
                installation_version: candidate.installation_version.clone(),
                installation_path: candidate.installation_path.clone(),
                complete: candidate.is_complete,
                launchable: candidate.is_launchable,
                reboot_required: candidate.is_reboot_required,
            })
            .collect();
        let selected = qualified[selected_index].clone();
        let installation_path = selected.installation_path.clone();
        let version_file = require_regular_file(
            &installation_path
                .join("VC")
                .join("Auxiliary")
                .join("Build")
                .join("Microsoft.VCToolsVersion.default.txt"),
            "P1A_MSVC_VERSION_FILE_INVALID",
        )?;
        require_contained(
            &version_file,
            &installation_path,
            "P1A_MSVC_VERSION_FILE_ESCAPE",
        )?;
        let version_text = read_locked_text(&version_file, 4096)?;
        let msvc_version_file = file_identity(&version_file)?;
        let msvc_tools_version = version_text.trim().to_owned();
        let Some(msvc_version_parts) = numeric_version(&msvc_tools_version) else {
            return Err(XtaskError::integrity(
                "P1A_MSVC_VERSION_INVALID",
                "Microsoft.VCToolsVersion.default.txt is not a dotted numeric version",
            ));
        };
        if msvc_version_parts.len() != 3 || msvc_version_parts[0] != 14 {
            return Err(XtaskError::integrity(
                "P1A_MSVC_VERSION_INVALID",
                "Microsoft.VCToolsVersion.default.txt is not an exact three-part MSVC 14 version",
            ));
        }
        let msvc_tools_root = require_directory(
            &installation_path
                .join("VC")
                .join("Tools")
                .join("MSVC")
                .join(&msvc_tools_version),
            "P1A_MSVC_ROOT_INVALID",
        )?;
        require_contained(&msvc_tools_root, &installation_path, "P1A_MSVC_ROOT_ESCAPE")?;
        let bin_root = msvc_tools_root.join("bin").join("Hostx64").join("x64");
        let msvc_include = require_directory(
            &msvc_tools_root.join("include"),
            "P1A_MSVC_INCLUDE_NOT_FOUND",
        )?;
        let msvc_x64_lib = require_directory(
            &msvc_tools_root.join("lib").join("x64"),
            "P1A_MSVC_LIB_NOT_FOUND",
        )?;
        let vcruntime_lib = file_identity(&require_regular_file(
            &msvc_x64_lib.join("vcruntime.lib"),
            "P1A_MSVC_RUNTIME_LIB_NOT_FOUND",
        )?)?;
        let cl = tool_identity(&bin_root.join("cl.exe"))?;
        let c_frontend = tool_identity(&bin_root.join("c1.dll"))?;
        let cpp_frontend = tool_identity(&bin_root.join("c1xx.dll"))?;
        let code_generator = tool_identity(&bin_root.join("c2.dll"))?;
        let link = tool_identity(&bin_root.join("link.exe"))?;
        let lib = tool_identity(&bin_root.join("lib.exe"))?;
        let dumpbin = tool_identity(&bin_root.join("dumpbin.exe"))?;
        let (vcruntime_redist_version, vcruntime_dll) =
            select_vcruntime(&installation_path, &msvc_version_parts)?;
        for path in [
            &msvc_include,
            &msvc_x64_lib,
            &vcruntime_lib.path,
            &cl.path,
            &c_frontend.path,
            &cpp_frontend.path,
            &code_generator.path,
            &link.path,
            &lib.path,
            &dumpbin.path,
        ] {
            require_contained(path, &msvc_tools_root, "P1A_MSVC_PATH_ESCAPE")?;
        }
        require_contained(
            &vcruntime_dll.path,
            &installation_path,
            "P1A_MSVC_PATH_ESCAPE",
        )?;
        Ok(VisualStudioToolchain {
            discovery_method: "vswhere-fixed-audited-runner".to_owned(),
            vswhere: tool_identity(&vswhere_path)?,
            query: VSWHERE_ARGS
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            candidates,
            selected_instance_id: selected.instance_id,
            product_id: selected.product_id,
            installation_version: selected.installation_version,
            installation_path,
            msvc_tools_version,
            msvc_version_file,
            msvc_tools_root,
            cl,
            c_frontend,
            cpp_frontend,
            code_generator,
            link,
            lib,
            dumpbin,
            msvc_include,
            msvc_x64_lib,
            vcruntime_lib,
            vcruntime_redist_version,
            vcruntime_dll,
        })
    }

    pub(super) fn windows_sdk() -> Result<WindowsSdkToolchain> {
        let kits_root = windows_kits_root()?;
        let include_root = kits_root.join("Include");
        let mut complete_versions = Vec::new();
        for entry in fs::read_dir(&include_root).io_context(
            "P1A_WINDOWS_SDK_ENUMERATION_FAILED",
            "could not enumerate Windows Kits Include",
        )? {
            let entry = entry.io_context(
                "P1A_WINDOWS_SDK_ENUMERATION_FAILED",
                "could not read a Windows Kits Include entry",
            )?;
            if !entry
                .file_type()
                .io_context(
                    "P1A_WINDOWS_SDK_ENUMERATION_FAILED",
                    "could not inspect a Windows Kits Include entry",
                )?
                .is_dir()
            {
                continue;
            }
            let version = entry.file_name().to_string_lossy().into_owned();
            let Some(parts) = four_part_version(&version) else {
                continue;
            };
            let required = [
                kits_root
                    .join("Include")
                    .join(&version)
                    .join("um")
                    .join("Windows.h"),
                kits_root
                    .join("Include")
                    .join(&version)
                    .join("ucrt")
                    .join("stdlib.h"),
                kits_root
                    .join("Lib")
                    .join(&version)
                    .join("um")
                    .join("x64")
                    .join("kernel32.lib"),
                kits_root
                    .join("Lib")
                    .join(&version)
                    .join("ucrt")
                    .join("x64")
                    .join("ucrt.lib"),
                kits_root
                    .join("bin")
                    .join(&version)
                    .join("x64")
                    .join("rc.exe"),
                kits_root
                    .join("bin")
                    .join(&version)
                    .join("x64")
                    .join("mt.exe"),
            ];
            if required.iter().all(|path| path.is_file()) {
                complete_versions.push((parts, version));
            }
        }
        complete_versions.sort();
        let (_, version) = complete_versions.pop().ok_or_else(|| {
            win_environment(
                "P1A_WINDOWS_SDK_NOT_FOUND",
                "no complete four-part x64 Windows SDK/UCRT installation was found",
            )
        })?;
        let windows_header = require_regular_file(
            &kits_root
                .join("Include")
                .join(&version)
                .join("um")
                .join("Windows.h"),
            "P1A_WINDOWS_HEADER_NOT_FOUND",
        )?;
        let ucrt_header = require_regular_file(
            &kits_root
                .join("Include")
                .join(&version)
                .join("ucrt")
                .join("stdlib.h"),
            "P1A_UCRT_HEADER_NOT_FOUND",
        )?;
        let kernel32_lib = require_regular_file(
            &kits_root
                .join("Lib")
                .join(&version)
                .join("um")
                .join("x64")
                .join("kernel32.lib"),
            "P1A_KERNEL32_LIB_NOT_FOUND",
        )?;
        let ucrt_lib = require_regular_file(
            &kits_root
                .join("Lib")
                .join(&version)
                .join("ucrt")
                .join("x64")
                .join("ucrt.lib"),
            "P1A_UCRT_LIB_NOT_FOUND",
        )?;
        let rc_path = kits_root
            .join("bin")
            .join(&version)
            .join("x64")
            .join("rc.exe");
        let mt_path = kits_root
            .join("bin")
            .join(&version)
            .join("x64")
            .join("mt.exe");
        let windows_header = file_identity(&windows_header)?;
        let ucrt_header = file_identity(&ucrt_header)?;
        let kernel32_lib = file_identity(&kernel32_lib)?;
        let ucrt_lib = file_identity(&ucrt_lib)?;
        let rc = tool_identity(&rc_path)?;
        let mt = tool_identity(&mt_path)?;
        for path in [
            &windows_header.path,
            &ucrt_header.path,
            &kernel32_lib.path,
            &ucrt_lib.path,
            &rc.path,
            &mt.path,
        ] {
            require_contained(path, &kits_root, "P1A_WINDOWS_SDK_PATH_ESCAPE")?;
        }
        Ok(WindowsSdkToolchain {
            discovery_method: "HKLM Installed Roots plus complete-file-set selection".to_owned(),
            kits_root,
            version: version.clone(),
            ucrt_version: version.clone(),
            windows_header,
            ucrt_header,
            kernel32_lib,
            ucrt_lib,
            rc,
            mt,
        })
    }

    fn valid_product_id(value: &str) -> bool {
        const PREFIX: &str = "Microsoft.VisualStudio.Product.";
        value.len() > PREFIX.len()
            && value.len() <= 256
            && value.starts_with(PREFIX)
            && value[PREFIX.len()..]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }

    fn valid_vs_installation_version(parts: Option<&[u32]>) -> bool {
        parts.is_some_and(|parts| (3..=4).contains(&parts.len()) && parts[0] == 17)
    }

    fn select_vcruntime(
        installation_path: &Path,
        msvc_version_parts: &[u32],
    ) -> Result<(String, ToolFileIdentity)> {
        let redist_root = require_directory(
            &installation_path.join("VC").join("Redist").join("MSVC"),
            "P1A_VCRUNTIME_ROOT_NOT_FOUND",
        )?;
        require_contained(&redist_root, installation_path, "P1A_VCRUNTIME_ROOT_ESCAPE")?;
        let mut candidates = Vec::new();
        let mut numeric_identities = BTreeSet::new();
        for entry in fs::read_dir(&redist_root).io_context(
            "P1A_VCRUNTIME_ENUMERATION_FAILED",
            "could not enumerate selected Visual Studio redistributables",
        )? {
            let entry = entry.io_context(
                "P1A_VCRUNTIME_ENUMERATION_FAILED",
                "could not read a Visual C++ redistributable entry",
            )?;
            let file_type = entry.file_type().io_context(
                "P1A_VCRUNTIME_ENUMERATION_FAILED",
                "could not inspect a Visual C++ redistributable entry",
            )?;
            if !file_type.is_dir() {
                continue;
            }
            let version = entry.file_name().to_string_lossy().into_owned();
            let Some(parts) = numeric_version(&version) else {
                continue;
            };
            if parts.len() != 3 || parts[0] != 14 {
                return Err(XtaskError::integrity(
                    "P1A_VCRUNTIME_VERSION_INVALID",
                    "a numeric Visual C++ redistributable directory is not an exact three-part MSVC 14 version",
                ));
            }
            if !numeric_identities.insert(parts.clone()) {
                return Err(XtaskError::integrity(
                    "P1A_VCRUNTIME_VERSION_AMBIGUOUS",
                    "multiple Visual C++ redistributable directories normalize to one numeric version",
                ));
            }
            if parts[..2] != msvc_version_parts[..2] {
                continue;
            }
            let path = require_regular_file(
                &entry
                    .path()
                    .join("x64")
                    .join("Microsoft.VC143.CRT")
                    .join("vcruntime140.dll"),
                "P1A_VCRUNTIME_NOT_FOUND",
            )?;
            require_contained(&path, &redist_root, "P1A_VCRUNTIME_PATH_ESCAPE")?;
            candidates.push((parts, version, tool_identity(&path)?));
        }
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        let (_, redist_version, identity) = candidates.pop().ok_or_else(|| {
            win_environment(
                "P1A_VCRUNTIME_NOT_FOUND",
                "no x64 Visual C++ runtime matches the selected MSVC major.minor version",
            )
        })?;
        let runtime_version = four_part_version(&identity.file_version).ok_or_else(|| {
            XtaskError::integrity(
                "P1A_VCRUNTIME_VERSION_INVALID",
                "selected vcruntime140.dll has no exact four-part file version",
            )
        })?;
        if runtime_version[..2] != msvc_version_parts[..2] {
            return Err(XtaskError::integrity(
                "P1A_VCRUNTIME_VERSION_MISMATCH",
                "selected vcruntime140.dll does not match the selected MSVC major.minor version",
            ));
        }
        Ok((redist_version, identity))
    }

    fn known_folder(folder_id: &Guid) -> Result<PathBuf> {
        let mut pointer: *mut u16 = null_mut();
        // SAFETY: the output pointer is writable; no impersonation token is supplied.
        let result = unsafe { SHGetKnownFolderPath(folder_id, 0, null_mut(), &mut pointer) };
        if result < 0 || pointer.is_null() {
            return Err(win_environment(
                "P1A_KNOWN_FOLDER_QUERY_FAILED",
                format!("SHGetKnownFolderPath failed with HRESULT 0x{result:08x}"),
            ));
        }
        let length = unsafe {
            let mut length = 0_usize;
            while *pointer.add(length) != 0 {
                length += 1;
                if length > 32_768 {
                    break;
                }
            }
            length
        };
        if length == 0 || length > 32_768 {
            // SAFETY: the pointer came from SHGetKnownFolderPath.
            unsafe { CoTaskMemFree(pointer.cast()) };
            return Err(XtaskError::integrity(
                "P1A_KNOWN_FOLDER_PATH_INVALID",
                "known-folder API returned an invalid UTF-16 path",
            ));
        }
        // SAFETY: the preceding bounded scan found a terminator after `length` code units.
        let units = unsafe { std::slice::from_raw_parts(pointer, length) };
        let path = PathBuf::from(std::ffi::OsString::from_wide(units));
        // SAFETY: the pointer came from SHGetKnownFolderPath.
        unsafe { CoTaskMemFree(pointer.cast()) };
        require_directory(&path, "P1A_KNOWN_FOLDER_PATH_INVALID")
    }

    fn windows_kits_root() -> Result<PathBuf> {
        const SUBKEY: &str = "SOFTWARE\\Microsoft\\Windows Kits\\Installed Roots";
        const VALUE: &str = "KitsRoot10";
        let mut errors = Vec::new();
        for registry_view in [RRF_SUBKEY_WOW6432KEY, RRF_SUBKEY_WOW6464KEY] {
            match registry_string(SUBKEY, VALUE, registry_view) {
                Ok(path) => {
                    return require_directory(
                        &PathBuf::from(path),
                        "P1A_WINDOWS_KITS_ROOT_INVALID",
                    );
                }
                Err(error) => errors.push(error.message),
            }
        }
        Err(win_environment(
            "P1A_WINDOWS_KITS_ROOT_NOT_FOUND",
            format!(
                "could not read HKLM Installed Roots/KitsRoot10 in either registry view: {}",
                errors.join("; ")
            ),
        ))
    }

    fn registry_string(subkey: &str, value: &str, registry_view: u32) -> Result<String> {
        let subkey = wide_null(subkey);
        let value = wide_null(value);
        let hkey_local_machine = 0x8000_0002_usize as Hkey;
        let mut value_type = 0_u32;
        let mut bytes = 0_u32;
        // SAFETY: the strings are NUL-terminated and the first call is the documented size query.
        let first = unsafe {
            RegGetValueW(
                hkey_local_machine,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ | registry_view,
                &mut value_type,
                null_mut(),
                &mut bytes,
            )
        };
        if first != 0 || !(2..=64 * 1024).contains(&bytes) || !bytes.is_multiple_of(2) {
            return Err(win_environment(
                "P1A_REGISTRY_VALUE_QUERY_FAILED",
                format!("RegGetValueW sizing query failed with status {first}"),
            ));
        }
        let mut units = vec![0_u16; bytes as usize / 2];
        // SAFETY: the buffer is writable for `bytes` bytes and both strings remain valid.
        let second = unsafe {
            RegGetValueW(
                hkey_local_machine,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ | registry_view,
                &mut value_type,
                units.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if second != 0 {
            return Err(win_environment(
                "P1A_REGISTRY_VALUE_QUERY_FAILED",
                format!("RegGetValueW data query failed with status {second}"),
            ));
        }
        while units.last() == Some(&0) {
            units.pop();
        }
        String::from_utf16(&units).map_err(|error| {
            XtaskError::integrity(
                "P1A_REGISTRY_VALUE_INVALID",
                format!("registry path is not valid UTF-16: {error}"),
            )
        })
    }

    fn require_regular_file(path: &Path, code: &'static str) -> Result<PathBuf> {
        require_no_reparse_ancestors(path, code)?;
        let metadata =
            fs::symlink_metadata(path).io_context(code, "could not inspect required file")?;
        if metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || !metadata.is_file()
        {
            return Err(XtaskError::integrity(
                code,
                format!(
                    "required path is not a regular non-symlink file: {}",
                    path.display()
                ),
            ));
        }
        fs::canonicalize(path).io_context(code, "could not canonicalize required file")
    }

    fn require_directory(path: &Path, code: &'static str) -> Result<PathBuf> {
        require_no_reparse_ancestors(path, code)?;
        let metadata =
            fs::symlink_metadata(path).io_context(code, "could not inspect required directory")?;
        if metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || !metadata.is_dir()
        {
            return Err(XtaskError::integrity(
                code,
                format!(
                    "required path is not a non-symlink directory: {}",
                    path.display()
                ),
            ));
        }
        fs::canonicalize(path).io_context(code, "could not canonicalize required directory")
    }

    fn require_no_reparse_ancestors(path: &Path, code: &'static str) -> Result<()> {
        for ancestor in path.ancestors() {
            let metadata = fs::symlink_metadata(ancestor)
                .io_context(code, "could not inspect a required path ancestor")?;
            if metadata.file_type().is_symlink()
                || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(XtaskError::integrity(
                    code,
                    format!(
                        "required path crosses a symlink, junction, or reparse point: {}",
                        ancestor.display()
                    ),
                ));
            }
        }
        Ok(())
    }

    fn require_contained(path: &Path, root: &Path, code: &'static str) -> Result<()> {
        if path == root || path.starts_with(root) {
            Ok(())
        } else {
            Err(XtaskError::integrity(
                code,
                "a canonical toolchain path escaped its canonical installation root",
            ))
        }
    }

    pub(super) fn tool_identity(path: &Path) -> Result<ToolFileIdentity> {
        let (path, sha256, bytes, file_version) = locked_identity(path, true)?;
        let file_version = file_version.ok_or_else(|| {
            XtaskError::integrity(
                "P1A_TOOL_VERSION_QUERY_FAILED",
                "internal version request returned no tool version",
            )
        })?;
        Ok(ToolFileIdentity {
            path,
            sha256,
            bytes,
            file_version,
        })
    }

    fn bind_locked_tool_identity(path: &Path) -> Result<(ToolFileIdentity, File)> {
        let locked = bind_locked_identity(path, true)?;
        let file_version = locked.file_version.ok_or_else(|| {
            XtaskError::integrity(
                "P1A_TOOL_VERSION_QUERY_FAILED",
                "internal version request returned no tool version",
            )
        })?;
        Ok((
            ToolFileIdentity {
                path: locked.path,
                sha256: locked.sha256,
                bytes: locked.bytes,
                file_version,
            },
            locked.file,
        ))
    }

    fn file_identity(path: &Path) -> Result<FileIdentity> {
        let (path, sha256, bytes, _) = locked_identity(path, false)?;
        Ok(FileIdentity {
            path,
            sha256,
            bytes,
        })
    }

    fn locked_identity(
        path: &Path,
        include_version: bool,
    ) -> Result<(PathBuf, String, u64, Option<String>)> {
        let locked = bind_locked_identity(path, include_version)?;
        Ok((
            locked.path,
            locked.sha256,
            locked.bytes,
            locked.file_version,
        ))
    }

    struct LockedIdentity {
        path: PathBuf,
        sha256: String,
        bytes: u64,
        file_version: Option<String>,
        file: File,
    }

    fn bind_locked_identity(path: &Path, include_version: bool) -> Result<LockedIdentity> {
        let path = require_regular_file(path, "P1A_FILE_IDENTITY_INVALID")?;
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .io_context(
                "P1A_FILE_IDENTITY_OPEN_FAILED",
                "could not lock an identity file against write/delete",
            )?;
        let before = file.metadata().io_context(
            "P1A_FILE_IDENTITY_INVALID",
            "could not inspect a locked identity file",
        )?;
        if !before.is_file()
            || before.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || before.file_size() > IDENTITY_FILE_LIMIT
        {
            return Err(XtaskError::integrity(
                "P1A_FILE_IDENTITY_INVALID",
                "identity file is nonregular, reparsed, or exceeds the one-GiB closed bound",
            ));
        }
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).io_context(
                "P1A_FILE_IDENTITY_READ_FAILED",
                "could not hash a locked identity file",
            )?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        let version = include_version.then(|| file_version(&path)).transpose()?;
        let after = file.metadata().io_context(
            "P1A_FILE_IDENTITY_INVALID",
            "could not re-inspect a locked identity file",
        )?;
        if before.file_attributes() != after.file_attributes()
            || before.creation_time() != after.creation_time()
            || before.last_write_time() != after.last_write_time()
            || before.file_size() != after.file_size()
            || fs::canonicalize(&path).io_context(
                "P1A_FILE_IDENTITY_INVALID",
                "could not re-canonicalize a locked identity file",
            )? != path
        {
            return Err(XtaskError::integrity(
                "P1A_FILE_IDENTITY_DRIFT",
                "an identity file changed while it was hashed and versioned",
            ));
        }
        require_no_reparse_ancestors(&path, "P1A_FILE_IDENTITY_INVALID")?;
        Ok(LockedIdentity {
            path,
            sha256: hex::encode(hasher.finalize()),
            bytes: before.file_size(),
            file_version: version,
            file,
        })
    }

    fn read_locked_text(path: &Path, maximum_bytes: u64) -> Result<String> {
        let path = require_regular_file(path, "P1A_LOCKED_TEXT_FILE_INVALID")?;
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .io_context(
                "P1A_LOCKED_TEXT_OPEN_FAILED",
                "could not lock a toolchain metadata file against write/delete",
            )?;
        let before = file.metadata().io_context(
            "P1A_LOCKED_TEXT_FILE_INVALID",
            "could not inspect a locked toolchain metadata file",
        )?;
        if before.file_size() == 0 || before.file_size() > maximum_bytes {
            return Err(XtaskError::integrity(
                "P1A_LOCKED_TEXT_FILE_INVALID",
                "toolchain metadata file is empty or exceeds its closed size bound",
            ));
        }
        let mut bytes = Vec::with_capacity(before.file_size() as usize);
        file.read_to_end(&mut bytes).io_context(
            "P1A_LOCKED_TEXT_READ_FAILED",
            "could not read a locked toolchain metadata file",
        )?;
        let after = file.metadata().io_context(
            "P1A_LOCKED_TEXT_FILE_INVALID",
            "could not re-inspect a locked toolchain metadata file",
        )?;
        if before.file_attributes() != after.file_attributes()
            || before.creation_time() != after.creation_time()
            || before.last_write_time() != after.last_write_time()
            || before.file_size() != after.file_size()
            || bytes.len() as u64 != before.file_size()
        {
            return Err(XtaskError::integrity(
                "P1A_LOCKED_TEXT_DRIFT",
                "toolchain metadata changed while it was read",
            ));
        }
        String::from_utf8(bytes).map_err(|error| {
            XtaskError::integrity(
                "P1A_LOCKED_TEXT_ENCODING_INVALID",
                format!("toolchain metadata is not UTF-8: {error}"),
            )
        })
    }

    fn file_version(path: &Path) -> Result<String> {
        let wide = wide_path(path);
        let mut ignored = 0_u32;
        // SAFETY: `wide` is a NUL-terminated path and `ignored` is writable.
        let bytes = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), &mut ignored) };
        if bytes == 0 || bytes > 16 * 1024 * 1024 {
            return Err(last_error(
                "P1A_TOOL_VERSION_QUERY_FAILED",
                format!("no bounded version resource for {}", path.display()),
            ));
        }
        let mut buffer = vec![0_u8; bytes as usize];
        // SAFETY: the buffer is writable for the exact size returned above.
        if unsafe { GetFileVersionInfoW(wide.as_ptr(), 0, bytes, buffer.as_mut_ptr().cast()) } == 0
        {
            return Err(last_error(
                "P1A_TOOL_VERSION_QUERY_FAILED",
                format!("GetFileVersionInfoW failed for {}", path.display()),
            ));
        }
        let root = wide_null("\\");
        let mut pointer: *mut c_void = null_mut();
        let mut length = 0_u32;
        // SAFETY: the version block and root query are valid and outputs are writable.
        if unsafe {
            VerQueryValueW(
                buffer.as_ptr().cast(),
                root.as_ptr(),
                &mut pointer,
                &mut length,
            )
        } == 0
            || pointer.is_null()
            || length < size_of::<VsFixedFileInfo>() as u32
        {
            return Err(last_error(
                "P1A_TOOL_VERSION_QUERY_FAILED",
                format!("VerQueryValueW failed for {}", path.display()),
            ));
        }
        // SAFETY: VerQueryValueW returned at least one complete VS_FIXEDFILEINFO.
        let info = unsafe { &*(pointer.cast::<VsFixedFileInfo>()) };
        if info.signature != 0xfeef_04bd {
            return Err(XtaskError::integrity(
                "P1A_TOOL_VERSION_INVALID",
                format!("invalid version resource signature for {}", path.display()),
            ));
        }
        Ok(format!(
            "{}.{}.{}.{}",
            info.file_version_ms >> 16,
            info.file_version_ms & 0xffff,
            info.file_version_ls >> 16,
            info.file_version_ls & 0xffff
        ))
    }

    fn numeric_version(value: &str) -> Option<Vec<u32>> {
        let parts: Option<Vec<u32>> = value.split('.').map(|part| part.parse().ok()).collect();
        parts.filter(|parts| !parts.is_empty() && parts.len() <= 4)
    }

    fn four_part_version(value: &str) -> Option<[u32; 4]> {
        let parts = numeric_version(value)?;
        if parts.len() != 4 {
            return None;
        }
        Some([parts[0], parts[1], parts[2], parts[3]])
    }

    fn wide_null(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    fn wide_path(value: &Path) -> Vec<u16> {
        value.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    #[derive(Clone, Debug)]
    struct SystemTimes {
        idle: u64,
        kernel: u64,
        user: u64,
    }

    #[derive(Clone, Debug)]
    struct ProcessCpu {
        process_id: u32,
        creation_time: u64,
        cpu_time: u64,
        image_name: String,
    }

    #[derive(Clone, Debug)]
    struct ProcessSnapshot {
        processes: BTreeMap<(u32, u64), ProcessCpu>,
        inaccessible: u32,
        inaccessible_known_compute: BTreeSet<(u32, String)>,
    }

    struct OwnedHandle(Handle);

    struct OwnedModule(Handle);

    impl OwnedModule {
        fn new(module: Handle) -> Option<Self> {
            (!module.is_null()).then_some(Self(module))
        }
    }

    impl Drop for OwnedModule {
        fn drop(&mut self) {
            // SAFETY: this wrapper is constructed only for a successful LoadLibraryExW call.
            unsafe { FreeLibrary(self.0) };
        }
    }

    impl OwnedHandle {
        fn new(handle: Handle) -> Option<Self> {
            if handle.is_null() || handle as isize == -1 {
                None
            } else {
                Some(Self(handle))
            }
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: this wrapper is constructed only for a valid owned Win32 handle.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn measure_isolation(
        policy: &WindowsHostPolicy,
        topology_before: &ProcessorTopology,
        power_before: &PowerPolicySnapshot,
    ) -> Result<CpuIsolationMeasurement> {
        let affinity_before = current_affinity()?;
        let system_before = system_times()?;
        let wall_before = system_wall_time();
        let started = Instant::now();
        let initial_processes = process_snapshot()?;
        let initial_keys = initial_processes
            .processes
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut process_first = initial_processes.clone();
        let mut process_last = initial_processes.clone();
        let mut inaccessible_known_compute = initial_processes.inaccessible_known_compute.clone();
        // Endpoint-only sampling can miss a short-lived compiler or Python process. Poll
        // throughout the fixed interval and retain every observed creation identity.
        // Aggregate GetSystemTimes remains the backstop for sub-interval activity.
        let process_after = loop {
            thread::sleep(std::time::Duration::from_millis(10));
            let observed = process_snapshot()?;
            for (identity, process) in &observed.processes {
                process_first
                    .processes
                    .entry(*identity)
                    .or_insert_with(|| process.clone());
                process_last.processes.insert(*identity, process.clone());
            }
            inaccessible_known_compute.extend(observed.inaccessible_known_compute.iter().cloned());
            if started.elapsed() >= std::time::Duration::from_millis(ISOLATION_WINDOW_MILLISECONDS)
            {
                break observed;
            }
        };
        let actual_elapsed = started.elapsed();
        let wall_after = system_wall_time();
        let system_after = system_times()?;
        let affinity_after = current_affinity()?;
        let topology_after = topology()?;
        let power_after = power_policy()?;

        let idle_delta = checked_delta(system_after.idle, system_before.idle, "idle")?;
        let kernel_delta = checked_delta(system_after.kernel, system_before.kernel, "kernel")?;
        let user_delta = checked_delta(system_after.user, system_before.user, "user")?;
        if idle_delta > kernel_delta {
            return Err(XtaskError::integrity(
                "P1A_SYSTEM_TIMES_INVALID",
                "idle CPU time exceeded kernel CPU time",
            ));
        }
        let system_capacity = kernel_delta.checked_add(user_delta).ok_or_else(|| {
            XtaskError::integrity("P1A_SYSTEM_TIMES_INVALID", "system CPU time overflowed")
        })?;
        if system_capacity == 0 {
            return Err(XtaskError::integrity(
                "P1A_SYSTEM_TIMES_INVALID",
                "system CPU capacity did not advance during the isolation window",
            ));
        }
        let system_busy = (kernel_delta - idle_delta)
            .checked_add(user_delta)
            .ok_or_else(|| {
                XtaskError::integrity("P1A_SYSTEM_TIMES_INVALID", "busy CPU time overflowed")
            })?;

        let current_process_id = std::process::id();
        let mut loads = Vec::new();
        let new_processes = process_first
            .processes
            .keys()
            .filter(|key| !initial_keys.contains(key))
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        let mut ordinary_os_cpu = 0_u64;
        let mut ordinary_os_process_count = 0_u32;
        let mut approved_cpu = 0_u64;
        let single_core_capacity = (actual_elapsed.as_nanos() / 100)
            .min(u128::from(u64::MAX))
            .max(1) as u64;
        for (key, after) in &process_last.processes {
            let before_cpu = if let Some(before) = initial_processes.processes.get(key) {
                before.cpu_time
            } else if after.creation_time >= wall_before && after.creation_time <= wall_after {
                0
            } else {
                // A process that predates the window but raced the starting snapshot is
                // charged only from its first observed CPU counter.
                process_first
                    .processes
                    .get(key)
                    .map_or(after.cpu_time, |before| before.cpu_time)
            };
            let Some(delta) = after.cpu_time.checked_sub(before_cpu) else {
                continue;
            };
            if after.process_id == current_process_id {
                continue;
            }
            let identity = ProcessIdentity {
                process_id: after.process_id,
                creation_time_100ns: after.creation_time,
            };
            let approved = policy
                .isolation
                .verifier_ancestry_and_contained_process_identities
                .contains(&identity);
            let known_compute_name = is_known_compute_name(&after.image_name);
            let single_core_basis_points = basis_points(delta, single_core_capacity);
            if delta == 0 && !known_compute_name {
                continue;
            }
            if approved {
                approved_cpu = approved_cpu.saturating_add(delta);
            } else if single_core_basis_points <= MAXIMUM_FOREIGN_SINGLE_CORE_BASIS_POINTS {
                ordinary_os_cpu = ordinary_os_cpu.saturating_add(delta);
                ordinary_os_process_count = ordinary_os_process_count.saturating_add(1);
            }
            loads.push(ForeignProcessLoad {
                process_id: after.process_id,
                creation_time_100ns: Some(after.creation_time),
                image_name: after.image_name.clone(),
                cpu_time_100ns: delta,
                single_core_basis_points,
                approved,
                known_compute_name,
            });
        }
        let mut represented_compute_pids: BTreeSet<u32> = loads
            .iter()
            .filter(|load| load.known_compute_name)
            .map(|load| load.process_id)
            .collect();
        for (process_id, image_name) in &inaccessible_known_compute {
            if !represented_compute_pids.insert(*process_id) {
                continue;
            }
            loads.push(ForeignProcessLoad {
                process_id: *process_id,
                creation_time_100ns: None,
                image_name: image_name.clone(),
                cpu_time_100ns: 0,
                single_core_basis_points: 0,
                approved: false,
                known_compute_name: true,
            });
        }
        let ended_processes = process_first
            .processes
            .keys()
            .filter(|key| !process_after.processes.contains_key(key))
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        loads.sort_by(|left, right| {
            right
                .cpu_time_100ns
                .cmp(&left.cpu_time_100ns)
                .then_with(|| left.process_id.cmp(&right.process_id))
        });
        let largest_unapproved = loads
            .iter()
            .filter(|load| !load.approved)
            .map(|load| load.single_core_basis_points)
            .max()
            .unwrap_or(0);
        let system_busy_basis_points = basis_points(system_busy, system_capacity);
        let topology_stable = topology_before == &topology_after;
        let affinity_stable = affinity_before == affinity_after;
        let power_policy_stable = power_before == &power_after;

        let mut violations = Vec::new();
        if affinity_before.thread_group != policy.isolation.selected_group
            || affinity_before.process_mask != policy.isolation.selected_affinity_mask
            || affinity_before.thread_group_mask & policy.isolation.selected_affinity_mask
                != policy.isolation.selected_affinity_mask
            || affinity_before.system_mask & policy.isolation.selected_affinity_mask
                != policy.isolation.selected_affinity_mask
        {
            finding(
                &mut violations,
                "P1A_AFFINITY_POLICY_MISMATCH",
                format!(
                    "expected group {} mask 0x{:016x}, observed thread group {} mask 0x{:016x} and process mask 0x{:016x}",
                    policy.isolation.selected_group,
                    policy.isolation.selected_affinity_mask,
                    affinity_before.thread_group,
                    affinity_before.thread_group_mask,
                    affinity_before.process_mask
                ),
            );
        }
        if !topology_stable {
            finding(
                &mut violations,
                "P1A_TOPOLOGY_DRIFT",
                "processor topology changed during the two-second isolation window",
            );
        }
        if !affinity_stable {
            finding(
                &mut violations,
                "P1A_AFFINITY_DRIFT",
                "process or thread-group affinity changed during the two-second isolation window",
            );
        }
        if !power_policy_stable {
            finding(
                &mut violations,
                "P1A_POWER_POLICY_DRIFT",
                "active power or processor policy changed during the two-second isolation window",
            );
        }
        if system_busy_basis_points > 10_000 - MINIMUM_SYSTEM_IDLE_BASIS_POINTS {
            finding(
                &mut violations,
                "P1A_SYSTEM_BUSY_LIMIT_EXCEEDED",
                format!(
                    "system busy load {system_busy_basis_points} bp exceeded frozen 5000 bp limit"
                ),
            );
        }
        let competing_loads: Vec<&ForeignProcessLoad> = loads
            .iter()
            .filter(|load| is_competing_foreign_load(load))
            .collect();
        if !competing_loads.is_empty() {
            finding(
                &mut violations,
                "P1A_FOREIGN_PROCESS_LIMIT_EXCEEDED",
                format!(
                    "{} unapproved competing process(es) observed; largest load {largest_unapproved} bp of one core",
                    competing_loads.len()
                ),
            );
        }

        Ok(CpuIsolationMeasurement {
            requested_window_milliseconds: ISOLATION_WINDOW_MILLISECONDS,
            actual_elapsed_nanoseconds: actual_elapsed.as_nanos().min(u64::MAX as u128) as u64,
            logical_processor_capacity: topology_before.active_logical_processors,
            system_idle_delta_100ns: idle_delta,
            system_kernel_delta_100ns: kernel_delta,
            system_user_delta_100ns: user_delta,
            system_busy_basis_points,
            ordinary_os_activity_total_cpu_100ns: ordinary_os_cpu,
            ordinary_os_activity_process_count: ordinary_os_process_count,
            largest_unapproved_process_single_core_basis_points: largest_unapproved,
            approved_verifier_cpu_100ns: approved_cpu,
            inaccessible_processes_at_start: initial_processes.inaccessible,
            inaccessible_processes_at_end: process_after.inaccessible,
            new_processes,
            ended_processes,
            foreign_process_loads: loads,
            affinity_before,
            affinity_after,
            topology_stable,
            affinity_stable,
            power_policy_stable,
            passed: violations.is_empty(),
            violations,
        })
    }

    fn system_times() -> Result<SystemTimes> {
        let mut idle = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        // SAFETY: all three FILETIME output buffers are writable.
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return Err(last_error(
                "P1A_SYSTEM_TIMES_QUERY_FAILED",
                "GetSystemTimes failed",
            ));
        }
        Ok(SystemTimes {
            idle: file_time(idle),
            kernel: file_time(kernel),
            user: file_time(user),
        })
    }

    fn system_wall_time() -> u64 {
        let mut value = FileTime::default();
        // SAFETY: `value` is a writable FILETIME buffer.
        unsafe { GetSystemTimeAsFileTime(&mut value) };
        file_time(value)
    }

    fn process_snapshot() -> Result<ProcessSnapshot> {
        // SAFETY: the process snapshot API has no borrowed pointer arguments.
        let raw_snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        let snapshot = OwnedHandle::new(raw_snapshot).ok_or_else(|| {
            last_error(
                "P1A_PROCESS_SNAPSHOT_FAILED",
                "CreateToolhelp32Snapshot failed",
            )
        })?;
        let mut entry: ProcessEntry32W = unsafe { zeroed() };
        entry.size = size_of::<ProcessEntry32W>() as u32;
        // SAFETY: the snapshot is valid and `entry` is correctly sized and writable.
        let mut has_entry = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        if !has_entry {
            // SAFETY: GetLastError has no preconditions.
            let error = unsafe { GetLastError() };
            if error != ERROR_NO_MORE_FILES {
                return Err(win_environment(
                    "P1A_PROCESS_SNAPSHOT_FAILED",
                    format!("Process32FirstW failed with Win32 error {error}"),
                ));
            }
        }
        let mut processes = BTreeMap::new();
        let mut inaccessible = 0_u32;
        let mut inaccessible_known_compute = BTreeSet::new();
        let mut observed = 0_usize;
        while has_entry {
            observed += 1;
            if observed > PROCESS_SNAPSHOT_LIMIT {
                return Err(win_environment(
                    "P1A_PROCESS_SNAPSHOT_LIMIT_EXCEEDED",
                    format!("process snapshot exceeded {PROCESS_SNAPSHOT_LIMIT} entries"),
                ));
            }
            if entry.process_id != 0 {
                let image_name = utf16_fixed(&entry.exe_file);
                validate_process_image_name(&image_name)?;
                // SAFETY: OpenProcess receives a numeric PID from the kernel snapshot.
                let raw_process =
                    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.process_id) };
                if let Some(process) = OwnedHandle::new(raw_process) {
                    let mut creation = FileTime::default();
                    let mut exit = FileTime::default();
                    let mut kernel = FileTime::default();
                    let mut user = FileTime::default();
                    // SAFETY: the process handle is valid and all outputs are writable.
                    if unsafe {
                        GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user)
                    } != 0
                    {
                        let creation_time = file_time(creation);
                        let cpu_time = file_time(kernel).saturating_add(file_time(user));
                        let process = ProcessCpu {
                            process_id: entry.process_id,
                            creation_time,
                            cpu_time,
                            image_name,
                        };
                        processes.insert((process.process_id, process.creation_time), process);
                    } else {
                        inaccessible = inaccessible.saturating_add(1);
                        if is_known_compute_name(&image_name) {
                            inaccessible_known_compute.insert((entry.process_id, image_name));
                        }
                    }
                } else {
                    inaccessible = inaccessible.saturating_add(1);
                    if is_known_compute_name(&image_name) {
                        inaccessible_known_compute.insert((entry.process_id, image_name));
                    }
                }
            }
            entry = unsafe { zeroed() };
            entry.size = size_of::<ProcessEntry32W>() as u32;
            // SAFETY: the snapshot remains valid and the fresh entry is writable.
            has_entry = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        // SAFETY: GetLastError has no preconditions. ERROR_NO_MORE_FILES is the normal terminator.
        let final_error = unsafe { GetLastError() };
        if final_error != ERROR_NO_MORE_FILES {
            return Err(win_environment(
                "P1A_PROCESS_SNAPSHOT_FAILED",
                format!("Process32NextW failed with Win32 error {final_error}"),
            ));
        }
        Ok(ProcessSnapshot {
            processes,
            inaccessible,
            inaccessible_known_compute,
        })
    }

    fn is_known_compute_name(image_name: &str) -> bool {
        COMPETING_COMPUTE_NAMES
            .iter()
            .any(|name| image_name.eq_ignore_ascii_case(name))
            || crate::p1a_process::forbidden_python_process_name(image_name)
    }

    fn validate_process_image_name(image_name: &str) -> Result<()> {
        if image_name.is_empty()
            || image_name.len() > 260
            || image_name.chars().any(char::is_control)
            || image_name
                .chars()
                .any(|character| matches!(character, '\\' | '/' | ':'))
        {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_IMAGE_NAME_INVALID",
                "Windows returned an empty, path-bearing, or malformed process image name",
            ));
        }
        Ok(())
    }

    fn utf16_fixed(units: &[u16]) -> String {
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        std::ffi::OsString::from_wide(&units[..end])
            .to_string_lossy()
            .into_owned()
    }

    fn file_time(value: FileTime) -> u64 {
        (u64::from(value.high) << 32) | u64::from(value.low)
    }

    fn checked_delta(after: u64, before: u64, field: &str) -> Result<u64> {
        after.checked_sub(before).ok_or_else(|| {
            XtaskError::integrity(
                "P1A_SYSTEM_TIMES_INVALID",
                format!("{field} CPU time moved backwards"),
            )
        })
    }

    fn basis_points(numerator: u64, denominator: u64) -> u32 {
        if denominator == 0 {
            return 0;
        }
        ((u128::from(numerator) * 10_000 / u128::from(denominator)).min(u128::from(u32::MAX)))
            as u32
    }

    fn last_error(code: &'static str, message: impl Into<String>) -> XtaskError {
        // SAFETY: GetLastError has no preconditions and does not mutate external state.
        let error = unsafe { GetLastError() };
        win_environment(code, format!("{}; Win32 error {error}", message.into()))
    }

    fn win_environment(code: &'static str, message: impl Into<String>) -> XtaskError {
        XtaskError::environment(code, message)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn single_core_fraction_uses_wall_capacity() {
            assert_eq!(basis_points(10_000_000, 20_000_000), 5_000);
            assert_eq!(basis_points(20_000_000, 20_000_000), 10_000);
        }

        #[test]
        fn known_background_compute_is_competing_only_when_load_exceeds_limit() {
            let mut load = ForeignProcessLoad {
                process_id: 42,
                creation_time_100ns: Some(1),
                image_name: "pythonw.exe".to_owned(),
                cpu_time_100ns: 1,
                single_core_basis_points: 73,
                approved: false,
                known_compute_name: true,
            };
            assert!(!is_competing_foreign_load(&load));
            load.single_core_basis_points = MAXIMUM_FOREIGN_SINGLE_CORE_BASIS_POINTS + 1;
            assert!(is_competing_foreign_load(&load));
            load.approved = true;
            assert!(!is_competing_foreign_load(&load));
        }

        #[test]
        fn sdk_version_requires_exactly_four_numeric_parts() {
            assert_eq!(four_part_version("10.0.26100.0"), Some([10, 0, 26100, 0]));
            assert_eq!(four_part_version("10.0.26100"), None);
            assert_eq!(four_part_version("10.0.latest.0"), None);
        }

        #[test]
        fn compute_inventory_is_case_insensitive_and_closed() {
            assert!(is_known_compute_name("RUSTC.EXE"));
            assert!(is_known_compute_name("python.exe"));
            assert!(is_known_compute_name("pythonw.exe"));
            assert!(is_known_compute_name("python3.13.exe"));
            assert!(is_known_compute_name("pypy310.exe"));
            assert!(!is_known_compute_name("firefox.exe"));
        }

        #[test]
        fn verifier_ancestry_python_classifier_is_version_closed() {
            for executable in [
                "py.exe",
                "pyw.exe",
                "python.exe",
                "pythonw.exe",
                "python313.exe",
                "python3.13.exe",
                "pypy310.exe",
                "pypyw3.10.exe",
            ] {
                assert!(
                    crate::p1a_process::forbidden_python_process_name(executable),
                    "ancestry classifier admitted {executable}"
                );
            }
            assert!(!crate::p1a_process::forbidden_python_process_name(
                "cargo.exe"
            ));
        }

        #[test]
        fn vswhere_identity_fields_are_closed() {
            assert!(valid_product_id("Microsoft.VisualStudio.Product.Community"));
            assert!(!valid_product_id("Microsoft.VisualStudio.Product."));
            assert!(!valid_product_id(
                "Microsoft.VisualStudio.Product.Community\nInjected"
            ));
            assert!(valid_vs_installation_version(Some(&[17, 14, 36331, 7])));
            assert!(!valid_vs_installation_version(Some(&[17])));
            assert!(!valid_vs_installation_version(Some(&[18, 0, 10000])));
        }

        #[test]
        fn process_image_names_are_basename_only() {
            validate_process_image_name("System").expect("native basename");
            validate_process_image_name("rustc.exe").expect("executable basename");
            assert!(validate_process_image_name("").is_err());
            assert!(validate_process_image_name("C:\\Tools\\rustc.exe").is_err());
            assert!(validate_process_image_name("rustc.exe\nspoof").is_err());
        }

        #[test]
        fn git_discovery_is_native_program_files_identity() {
            let (git, root) = discover_git_path().expect("qualified Program Files Git");
            assert_eq!(git.file_name().and_then(OsStr::to_str), Some("git.exe"));
            assert!(git.starts_with(&root));
            assert_eq!(root.file_name().and_then(OsStr::to_str), Some("Git"));
        }

        #[test]
        fn program_files_roots_are_native_known_folders() {
            let (program_files, program_files_x86) =
                native_program_files_roots().expect("native Program Files roots");
            assert_eq!(
                program_files.file_name().and_then(OsStr::to_str),
                Some("Program Files")
            );
            assert_eq!(
                program_files_x86.file_name().and_then(OsStr::to_str),
                Some("Program Files (x86)")
            );
            assert_eq!(program_files.parent(), program_files_x86.parent());
        }

        #[test]
        fn syswow64_is_native_canonical_system32_sibling() {
            assert_eq!(SYSWOW64_DIRECTORY_BUFFER_UNITS, 32_767);
            let system32 = system_directory().expect("native System32 directory");
            let syswow64 = system_wow64_directory().expect("native SysWOW64 directory");
            assert_ne!(syswow64, system32);
            assert_eq!(syswow64.parent(), system32.parent());
            assert!(
                syswow64
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|leaf| leaf.eq_ignore_ascii_case("SysWOW64"))
            );
            assert_eq!(
                fs::canonicalize(&syswow64).expect("canonical SysWOW64 directory"),
                syswow64
            );
        }

        #[test]
        fn native_directory_units_strip_only_trailing_nuls() {
            let mut terminated = OsStr::new("C:\\Windows\\SysWOW64")
                .encode_wide()
                .chain([0, 0])
                .collect::<Vec<_>>();
            trim_native_path_terminators(&mut terminated, "P1A_TEST_NATIVE_PATH_INVALID")
                .expect("strip native terminators");
            assert!(!terminated.is_empty());
            assert!(!terminated.contains(&0));

            let mut embedded = OsStr::new("C:\\Windows")
                .encode_wide()
                .chain([0])
                .chain(OsStr::new("SysWOW64").encode_wide())
                .collect::<Vec<_>>();
            assert!(
                trim_native_path_terminators(&mut embedded, "P1A_TEST_NATIVE_PATH_INVALID")
                    .is_err()
            );
        }

        #[test]
        fn vswhere_runtime_binds_exact_program_data_setup_x86_dll() {
            let binding = bind_vswhere_runtime().expect("locked vswhere runtime binding");
            let program_data = known_folder(&FOLDERID_PROGRAM_DATA).expect("native ProgramData");
            let expected = require_regular_file(
                &program_data
                    .join("Microsoft")
                    .join("VisualStudio")
                    .join("Setup")
                    .join("x86")
                    .join("Microsoft.VisualStudio.Setup.Configuration.Native.dll"),
                "P1A_TEST_SETUP_CONFIGURATION_INVALID",
            )
            .expect("canonical Setup Configuration implementation");
            let identity = binding.setup_configuration_identity();
            assert_eq!(identity.path, expected);
            assert!(identity.path.starts_with(program_data));
            assert!(identity.bytes > 0);
            assert!(crate::hash::is_lower_sha256(&identity.sha256));
            assert!(!identity.file_version.is_empty());
        }

        #[test]
        fn retained_identity_handle_denies_write_and_delete() {
            let workspace = fs::canonicalize(".").expect("canonical workspace");
            let owned = tempfile::Builder::new()
                .prefix(".p1a-locked-identity-test-")
                .tempdir_in(workspace.join("target"))
                .expect("create owned identity directory");
            let path = owned.path().join("identity.dll");
            let renamed = owned.path().join("renamed.dll");
            fs::write(&path, b"locked identity\n").expect("write identity file");
            let locked = bind_locked_identity(&path, false).expect("bind identity file");
            let write_open = OpenOptions::new()
                .write(true)
                .share_mode(FILE_SHARE_READ)
                .open(&path);
            assert!(
                write_open.is_err(),
                "retained identity handle must deny writers"
            );
            assert!(
                fs::rename(&path, &renamed).is_err(),
                "retained identity handle must deny directory-entry replacement"
            );
            drop(locked);
            OpenOptions::new()
                .write(true)
                .share_mode(FILE_SHARE_READ)
                .open(&path)
                .expect("writer admitted after identity handle closes");
            fs::rename(&path, &renamed).expect("rename admitted after identity handle closes");
        }

        #[test]
        fn verifier_ancestry_stops_before_windows_terminal_service_activation() {
            let parents =
                BTreeMap::from([(10, 20), (20, 30), (30, 40), (40, 50), (50, 60), (60, 0)]);
            let names = BTreeMap::from([
                (10, "xtask.exe".to_owned()),
                (20, "cargo.exe".to_owned()),
                (30, "powershell.exe".to_owned()),
                (40, "WindowsTerminal.exe".to_owned()),
                (50, "svchost.exe".to_owned()),
                (60, "services.exe".to_owned()),
            ]);
            assert_eq!(
                verifier_ancestry_process_ids(&parents, &names, 10)
                    .expect("closed native launcher ancestry"),
                vec![10, 20, 30]
            );
        }

        #[test]
        fn verifier_ancestry_rejects_python_before_the_interactive_boundary() {
            let parents = BTreeMap::from([(10, 20), (20, 30), (30, 40), (40, 0)]);
            let names = BTreeMap::from([
                (10, "xtask.exe".to_owned()),
                (20, "cargo.exe".to_owned()),
                (30, "python313.exe".to_owned()),
                (40, "explorer.exe".to_owned()),
            ]);
            assert_eq!(
                verifier_ancestry_process_ids(&parents, &names, 10)
                    .expect_err("Python-launched verifier must fail")
                    .code,
                "P1A_PYTHON_LAUNCHER_REJECTED"
            );
        }

        #[test]
        fn verifier_ancestry_requires_a_closed_interactive_boundary() {
            let parents = BTreeMap::from([(10, 20), (20, 30), (30, 0)]);
            let names = BTreeMap::from([
                (10, "xtask.exe".to_owned()),
                (20, "cargo.exe".to_owned()),
                (30, "service-launcher.exe".to_owned()),
            ]);
            assert_eq!(
                verifier_ancestry_process_ids(&parents, &names, 10)
                    .expect_err("service-launched verifier must not truncate ancestry")
                    .code,
                "P1A_PROCESS_ANCESTRY_INVALID"
            );
        }

        #[test]
        fn verifier_ancestry_contains_current_process() {
            let ancestry = current_verifier_ancestry().expect("native ancestry snapshot");
            assert!(
                ancestry
                    .iter()
                    .any(|identity| identity.process_id == std::process::id()
                        && identity.creation_time_100ns > 0)
            );
        }

        fn prototype_topology(reverse: bool) -> ProcessorTopology {
            let indices: Vec<u32> = if reverse {
                (0..16).rev().collect()
            } else {
                (0..16).collect()
            };
            ProcessorTopology {
                active_group_count: 1,
                active_logical_processors: 32,
                physical_core_count: 16,
                package_count: 1,
                cores: indices
                    .into_iter()
                    .map(|index| CoreTopology {
                        efficiency_class: (index % 2) as u8,
                        smt: true,
                        group_masks: vec![GroupMask {
                            group: 0,
                            mask: 3_u64 << (index * 2),
                            logical_processors: 2,
                        }],
                    })
                    .collect(),
            }
        }

        #[test]
        fn windows_exposed_eight_core_topology_is_accepted_as_observed() {
            let topology = ProcessorTopology {
                active_group_count: 1,
                active_logical_processors: 8,
                physical_core_count: 8,
                package_count: 1,
                cores: (0..8)
                    .map(|index| CoreTopology {
                        efficiency_class: 0,
                        smt: false,
                        group_masks: vec![GroupMask {
                            group: 0,
                            mask: 1_u64 << index,
                            logical_processors: 1,
                        }],
                    })
                    .collect(),
            };
            let canonical = canonical_core_topology(&topology).expect("Windows-visible topology");
            assert_eq!(canonical.len(), 8);
            assert!(canonical.iter().all(|core| !core.smt));
            assert_eq!(processor_group_union_mask(&topology, 0).unwrap(), 0xFF);
        }

        #[test]
        fn prototype_topology_is_canonical_and_order_independent() {
            let ordered = prototype_topology(false);
            let reversed = prototype_topology(true);
            let canonical = canonical_core_topology(&reversed).expect("canonical topology");
            assert_eq!(canonical.len(), 16);
            assert_eq!(canonical[0].core_index, 0);
            assert_eq!(canonical[0].group_masks[0].mask, "0x0000000000000003");
            assert_eq!(canonical[15].core_index, 15);
            assert_eq!(canonical[15].group_masks[0].mask, "0x00000000C0000000");
            assert_eq!(
                processor_group_union_mask(&ordered, 0).expect("union mask"),
                u64::from(u32::MAX)
            );
            assert_eq!(
                processor_topology_sha256(&ordered).expect("ordered hash"),
                processor_topology_sha256(&reversed).expect("reversed hash")
            );
        }

        #[test]
        fn prototype_topology_rejects_overlapping_core_masks() {
            let mut topology = prototype_topology(false);
            topology.cores[1].group_masks[0].mask = topology.cores[0].group_masks[0].mask;
            assert!(canonical_core_topology(&topology).is_err());
        }

        #[test]
        fn loader_runtime_is_resolved_from_canonical_system_directory() {
            let runtime = loader_resolved_system_runtime().expect("loader-resolved runtime");
            assert_eq!(runtime.resolution_policy, "windows-system32-safe-search-v1");
            assert!(runtime.ucrtbase.path.starts_with(&runtime.system_directory));
            assert!(
                runtime
                    .vcruntime
                    .path
                    .starts_with(&runtime.system_directory)
            );
            assert_eq!(
                runtime.ucrtbase.path.file_name().and_then(OsStr::to_str),
                Some("ucrtbase.dll")
            );
            assert_eq!(
                runtime.vcruntime.path.file_name().and_then(OsStr::to_str),
                Some("vcruntime140.dll")
            );
        }

        #[test]
        fn selected_tree_identity_detects_content_mutation() {
            let workspace = fs::canonicalize(".").expect("canonical workspace");
            let owned = tempfile::Builder::new()
                .prefix(".p1a-tree-identity-test-")
                .tempdir_in(workspace.join("target"))
                .expect("create owned test tree");
            let root = owned.path();
            let nested = root.join("nested");
            fs::create_dir_all(&nested).expect("create fresh test tree");
            fs::write(root.join("one.h"), b"one\n").expect("write first file");
            fs::write(nested.join("two.lib"), b"two\n").expect("write second file");
            let before = tree_identity(&[("tree", root)]).expect("initial identity");
            assert_eq!(before.directory_count, 2);
            assert_eq!(before.file_count, 2);
            fs::write(nested.join("two.lib"), b"changed\n").expect("mutate second file");
            let after = tree_identity(&[("tree", root)]).expect("changed identity");
            assert_ne!(before.sha256, after.sha256);
        }

        #[test]
        fn owned_host_and_isolation_schemas_are_closed_documents() {
            for source in [
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../docs/schemas/P1A-prototype-v2/python-slm-p1a-host-environment-v1.schema.json"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../docs/schemas/P1A-prototype-v2/python-slm-p1a-cpu-isolation-v1.schema.json"
                )),
            ] {
                let schema: serde_json::Value =
                    serde_json::from_str(source).expect("schema JSON parses");
                crate::json_schema::validate_schema_document(&schema, "TEST_SCHEMA_INVALID")
                    .expect("schema shape is supported and closed");
            }
        }

        #[test]
        fn host_schema_accepts_future_rust_majors_and_records_windows_builds() {
            let schema: serde_json::Value = serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../docs/schemas/P1A-prototype-v2/python-slm-p1a-host-environment-v1.schema.json"
            )))
            .expect("host schema JSON parses");
            let release = schema
                .pointer("/definitions/rustToolchain/properties/release_semver")
                .expect("release-semver schema");
            crate::json_schema::validate(
                release,
                &serde_json::json!({"major": 1, "minor": 96, "patch": 0}),
                "TEST_SCHEMA_INVALID",
            )
            .expect("Rust 1.96 accepted");
            crate::json_schema::validate(
                release,
                &serde_json::json!({"major": 2, "minor": 0, "patch": 0}),
                "TEST_SCHEMA_INVALID",
            )
            .expect("future Rust major accepted");
            assert!(
                crate::json_schema::validate(
                    release,
                    &serde_json::json!({"major": 1, "minor": 95, "patch": 99}),
                    "TEST_SCHEMA_INVALID",
                )
                .is_err()
            );

            let operating_system = schema
                .pointer("/definitions/operatingSystem")
                .expect("operating-system schema");
            crate::json_schema::validate(
                operating_system,
                &serde_json::json!({
                    "family": "windows",
                    "architecture": "x86_64",
                    "native_architecture": "AMD64",
                    "version": "10.0.19045",
                    "build": 19045,
                    "service_pack_major": 0,
                    "service_pack_minor": 0,
                    "product_type": 1,
                    "native_windows_process": true
                }),
                "TEST_SCHEMA_INVALID",
            )
            .expect("exact Windows build recorded without an unapproved Win11 gate");
        }
    }
}
