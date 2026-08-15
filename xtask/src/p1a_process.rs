use crate::error::{Result, XtaskError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_CAPTURE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessPolicy {
    HostOnly,
    CudaProbe,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QualifiedPersistentFile {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct DirectCommand {
    pub policy: ProcessPolicy,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub display_argv: Vec<String>,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, Option<OsString>>,
    pub timeout: Duration,
    pub capture_directory: PathBuf,
    pub capture_stem: String,
    pub qualified_persistent_roots: Vec<PathBuf>,
    pub qualified_persistent_files: Vec<QualifiedPersistentFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessAudit {
    pub audit_method: String,
    pub atomic_job_assignment: bool,
    pub root_process_id: u32,
    pub root_creation_time_100ns: u64,
    pub audited_process_count: usize,
    pub covered_process_count: usize,
    pub successful_snapshots: usize,
    pub exit_races: usize,
    pub executable_names: Vec<String>,
    pub process_identities: Vec<AuditedProcessIdentity>,
    pub loaded_modules: Vec<LoadedModuleIdentity>,
    pub forbidden_processes: Vec<String>,
    pub forbidden_modules: Vec<String>,
    pub process_tree_terminated: bool,
    pub unexpected_descendants: bool,
    pub qualified_tool_descendants_cleaned: bool,
    pub timed_out: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuditedProcessIdentity {
    pub process_id: u32,
    pub creation_time_100ns: u64,
    pub executable_name: String,
    pub canonical_path_sha256: String,
    pub executable_sha256: String,
    pub executable_bytes: u64,
    pub path_class: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoadedModuleIdentity {
    pub process_id: u32,
    pub creation_time_100ns: u64,
    pub module_name: String,
    pub canonical_path_sha256: String,
    pub module_sha256: String,
    pub module_bytes: u64,
    pub path_class: String,
}

#[derive(Clone, Debug)]
pub(crate) struct AuditedOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub audit: ProcessAudit,
}

pub(crate) fn forbidden_python_process_name(leaf: &str) -> bool {
    let lower = leaf.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    matches!(
        stem,
        "py" | "pyw" | "python" | "pythonw" | "pypy" | "pypyw" | "pip" | "pipx" | "uv"
    ) || versioned_python_process_name(stem, "python")
        || versioned_python_process_name(stem, "pythonw")
        || versioned_python_process_name(stem, "pypy")
        || versioned_python_process_name(stem, "pypyw")
        || versioned_python_process_name(stem, "pip")
}

fn versioned_python_process_name(stem: &str, prefix: &str) -> bool {
    stem.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix
                .chars()
                .all(|character| character.is_ascii_digit() || matches!(character, '.' | '-'))
    })
}

#[cfg(not(windows))]
pub(crate) fn run(_command: &DirectCommand) -> Result<AuditedOutput> {
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        "the native host process auditor is implemented only for the Windows prototype",
        "Run the selected prototype on Windows or wait for P17 host portability.",
    ))
}

#[cfg(windows)]
mod windows {
    use super::*;
    use crate::error::IoContext;
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::ptr::{null, null_mut};
    use std::thread;
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{
        CloseHandle, DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, DUPLICATE_SAME_ACCESS,
        DuplicateHandle, ERROR_BAD_LENGTH, ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA,
        ERROR_NO_MORE_FILES, ERROR_PARTIAL_COPY, ERROR_SEM_TIMEOUT, EXCEPTION_BREAKPOINT, FILETIME,
        GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_READ,
        GetFileInformationByHandle, GetFinalPathNameByHandleW,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, TH32CS_SNAPMODULE,
        TH32CS_SNAPMODULE32,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList,
        JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
        TerminateJobObject,
    };
    use windows_sys::Win32::System::SystemInformation::{
        GetSystemDirectoryW, GetSystemWow64DirectoryW, IMAGE_FILE_MACHINE_AMD64,
        IMAGE_FILE_MACHINE_I386, IMAGE_FILE_MACHINE_UNKNOWN,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
        DEBUG_PROCESS, DETACHED_PROCESS, DeleteProcThreadAttributeList,
        EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetProcessTimes,
        InitializeProcThreadAttributeList, IsWow64Process2, OpenProcess,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
        TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
    };

    const EXCEPTION_DEBUG_EVENT: u32 = 1;
    const CREATE_PROCESS_DEBUG_EVENT: u32 = 3;
    const EXIT_PROCESS_DEBUG_EVENT: u32 = 5;
    const LOAD_DLL_DEBUG_EVENT: u32 = 6;
    const STATUS_WX86_BREAKPOINT: i32 = 0x4000_001f;
    const NATIVE_DIRECTORY_BUFFER_UNITS: usize = 32_767;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawCreateProcessDebugInfo {
        file: HANDLE,
        process: HANDLE,
        thread: HANDLE,
        base_of_image: *mut core::ffi::c_void,
        debug_info_file_offset: u32,
        debug_info_size: u32,
        thread_local_base: *mut core::ffi::c_void,
        start_address: *mut core::ffi::c_void,
        image_name: *mut core::ffi::c_void,
        unicode: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawLoadDllDebugInfo {
        file: HANDLE,
        base_of_dll: *mut core::ffi::c_void,
        debug_info_file_offset: u32,
        debug_info_size: u32,
        image_name: *mut core::ffi::c_void,
        unicode: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawExceptionRecord {
        code: i32,
        flags: u32,
        record: *mut RawExceptionRecord,
        address: *mut core::ffi::c_void,
        parameter_count: u32,
        information: [usize; 15],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawExceptionDebugInfo {
        record: RawExceptionRecord,
        first_chance: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    union RawDebugEventPayload {
        create_process: RawCreateProcessDebugInfo,
        load_dll: RawLoadDllDebugInfo,
        exception: RawExceptionDebugInfo,
        // EXCEPTION_DEBUG_INFO is the largest documented DEBUG_EVENT arm. The
        // additional pointer-sized element keeps this backing storage large
        // enough on both 32-bit and 64-bit Windows without depending on the
        // optional windows-sys Debug feature.
        storage: [usize; 21],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawDebugEvent {
        code: u32,
        process_id: u32,
        thread_id: u32,
        payload: RawDebugEventPayload,
    }

    impl Default for RawDebugEvent {
        fn default() -> Self {
            // SAFETY: an all-zero DEBUG_EVENT is valid output storage for Win32.
            unsafe { zeroed() }
        }
    }

    const _: () = {
        assert!(size_of::<RawDebugEvent>() >= 12 + size_of::<RawExceptionDebugInfo>());
    };

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn WaitForDebugEvent(event: *mut RawDebugEvent, milliseconds: u32) -> i32;
        fn ContinueDebugEvent(process_id: u32, thread_id: u32, status: i32) -> i32;
    }

    struct OwnedHandle(HANDLE);

    impl OwnedHandle {
        fn new(handle: HANDLE, code: &'static str, context: &str) -> Result<Self> {
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                return Err(last_error(code, context));
            }
            Ok(Self(handle))
        }

        fn raw(&self) -> HANDLE {
            self.0
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                // SAFETY: the wrapper owns one live Win32 handle and closes it once.
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    struct CreatedProcess {
        process: OwnedHandle,
        thread: OwnedHandle,
        process_id: u32,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct ExecutableIdentity {
        canonical_path: PathBuf,
        canonical_path_sha256: String,
        sha256: String,
        bytes: u64,
        last_write_time: u64,
        volume_serial_number: u32,
        file_index: u64,
        forbidden_content_markers: BTreeSet<String>,
    }

    struct ForbiddenImageIdentity {
        label: String,
        sha256: String,
        bytes: u64,
        _lock: File,
    }

    struct BoundFile {
        file: File,
        canonical_path: PathBuf,
        canonical_path_sha256: String,
        bytes: u64,
        last_write_time: u64,
        volume_serial_number: u32,
        file_index: u64,
    }

    struct ProcessPathPolicy {
        system32: PathBuf,
        syswow64: PathBuf,
        qualified_files: Vec<BoundFile>,
    }

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct ProcessIdentity {
        process_id: u32,
        creation_time_100ns: u64,
    }

    struct SuspendedChildGuard {
        process: HANDLE,
        job: HANDLE,
        contained: bool,
        armed: bool,
    }

    impl SuspendedChildGuard {
        fn new(process: HANDLE, job: HANDLE) -> Self {
            Self {
                process,
                job,
                contained: false,
                armed: true,
            }
        }

        fn contained(&mut self) {
            self.contained = true;
        }

        fn disarm(&mut self) {
            self.armed = false;
        }
    }

    impl Drop for SuspendedChildGuard {
        fn drop(&mut self) {
            if !self.armed {
                return;
            }
            // SAFETY: this guard is armed only while the created child remains suspended.
            unsafe {
                if self.contained {
                    TerminateJobObject(self.job, 126);
                } else {
                    TerminateProcess(self.process, 126);
                }
                WaitForSingleObject(self.process, 10_000);
            }
        }
    }

    struct JobTerminationGuard {
        job: HANDLE,
        armed: bool,
    }

    impl JobTerminationGuard {
        fn new(job: HANDLE) -> Self {
            Self { job, armed: true }
        }

        fn disarm(&mut self) {
            self.armed = false;
        }
    }

    impl Drop for JobTerminationGuard {
        fn drop(&mut self) {
            if self.armed {
                // SAFETY: the handle is a private Job owned by the surrounding runner.
                unsafe { TerminateJobObject(self.job, 126) };
            }
        }
    }

    struct AttributeList {
        storage: Vec<usize>,
    }

    impl AttributeList {
        fn for_handles(handles: &mut [HANDLE; 3]) -> Result<Self> {
            let mut bytes = 0_usize;
            // SAFETY: the documented sizing call uses a null list and writes `bytes`.
            let sized = unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes) };
            // SAFETY: last-error is consumed immediately after the sizing call.
            if sized != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || bytes == 0 {
                return Err(last_error(
                    "P1A_HANDLE_ALLOWLIST_SIZE_FAILED",
                    "could not size the child handle allowlist",
                ));
            }
            let words = bytes.div_ceil(size_of::<usize>());
            let mut list = Self {
                storage: vec![0_usize; words],
            };
            // SAFETY: storage is suitably aligned and large enough for the opaque list.
            if unsafe { InitializeProcThreadAttributeList(list.raw(), 1, 0, &mut bytes) } == 0 {
                list.storage.clear();
                return Err(last_error(
                    "P1A_HANDLE_ALLOWLIST_INIT_FAILED",
                    "could not initialize the child handle allowlist",
                ));
            }
            // SAFETY: the attribute value is exactly the three live inheritable handles.
            if unsafe {
                UpdateProcThreadAttribute(
                    list.raw(),
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_mut_ptr().cast(),
                    size_of_val(handles),
                    null_mut(),
                    null(),
                )
            } == 0
            {
                return Err(last_error(
                    "P1A_HANDLE_ALLOWLIST_UPDATE_FAILED",
                    "could not install the child handle allowlist",
                ));
            }
            Ok(list)
        }

        fn raw(&mut self) -> *mut core::ffi::c_void {
            self.storage.as_mut_ptr().cast()
        }
    }

    impl Drop for AttributeList {
        fn drop(&mut self) {
            if !self.storage.is_empty() {
                // SAFETY: a nonempty list reached successful initialization.
                unsafe { DeleteProcThreadAttributeList(self.raw()) };
            }
        }
    }

    #[derive(Default)]
    struct AuditState {
        identities: BTreeSet<ProcessIdentity>,
        successful_snapshots: usize,
        exit_races: usize,
        executable_names: BTreeSet<String>,
        process_identities: BTreeSet<AuditedProcessIdentity>,
        loaded_modules: BTreeSet<LoadedModuleIdentity>,
        forbidden_processes: BTreeSet<String>,
        forbidden_modules: BTreeSet<String>,
        executable_covered_identities: BTreeSet<ProcessIdentity>,
        snapshot_covered_identities: BTreeSet<ProcessIdentity>,
        forbidden_image_identities: Vec<ForbiddenImageIdentity>,
    }

    struct DebugProcessState {
        handle: OwnedHandle,
        identity: ProcessIdentity,
        initial_boundary: InitialDebugBoundary,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum InitialDebugBoundary {
        NativePending,
        Wow64NativePending,
        Wow64Pending,
        Complete,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum InitialBoundaryAction {
        NotHandled,
        Continue,
        Sample,
    }

    fn initial_boundary_action(
        boundary: &mut InitialDebugBoundary,
        first_chance: u32,
        code: i32,
    ) -> InitialBoundaryAction {
        if first_chance == 0 {
            return InitialBoundaryAction::NotHandled;
        }
        match (*boundary, code) {
            (InitialDebugBoundary::NativePending, EXCEPTION_BREAKPOINT) => {
                *boundary = InitialDebugBoundary::Complete;
                InitialBoundaryAction::Sample
            }
            (InitialDebugBoundary::Wow64NativePending, EXCEPTION_BREAKPOINT) => {
                *boundary = InitialDebugBoundary::Wow64Pending;
                InitialBoundaryAction::Continue
            }
            (InitialDebugBoundary::Wow64Pending, STATUS_WX86_BREAKPOINT) => {
                *boundary = InitialDebugBoundary::Complete;
                InitialBoundaryAction::Sample
            }
            _ => InitialBoundaryAction::NotHandled,
        }
    }

    #[derive(Default)]
    struct DebugState {
        processes: BTreeMap<u32, DebugProcessState>,
        root_exit_code: Option<u32>,
    }

    #[derive(Clone, Copy)]
    enum DebugWait {
        Event { code: u32, process_id: u32 },
        Timeout,
    }

    pub(crate) fn run(command: &DirectCommand) -> Result<AuditedOutput> {
        validate_spec(command)?;
        let (program_before, program_lock) = bind_executable(&command.program)?;
        let path_policy = bind_process_path_policy(command)?;
        fs::create_dir_all(&command.capture_directory).io_context(
            "P1A_CAPTURE_DIRECTORY_FAILED",
            "could not create the private command capture directory",
        )?;
        let stdout_path = command
            .capture_directory
            .join(format!("{}.stdout", command.capture_stem));
        let stderr_path = command
            .capture_directory
            .join(format!("{}.stderr", command.capture_stem));
        if stdout_path.exists() || stderr_path.exists() {
            return Err(XtaskError::integrity(
                "P1A_CAPTURE_COLLISION",
                "private command capture path already exists",
            ));
        }
        let stdout_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stdout_path)
            .io_context(
                "P1A_CAPTURE_CREATE_FAILED",
                "could not create stdout capture",
            )?;
        let stderr_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stderr_path)
            .io_context(
                "P1A_CAPTURE_CREATE_FAILED",
                "could not create stderr capture",
            )?;
        let stdin_file = File::open("NUL").io_context(
            "P1A_STDIN_OPEN_FAILED",
            "could not open the null input device",
        )?;
        let stdin_child = duplicate_inheritable(stdin_file.as_raw_handle() as HANDLE)?;
        let stdout_child = duplicate_inheritable(stdout_file.as_raw_handle() as HANDLE)?;
        let stderr_child = duplicate_inheritable(stderr_file.as_raw_handle() as HANDLE)?;

        let job = create_job()?;
        let mut state = AuditState {
            forbidden_image_identities: bind_forbidden_system_images()?,
            ..AuditState::default()
        };
        let mut created = create_suspended_process(
            command,
            &program_before.canonical_path,
            stdin_child.raw(),
            stdout_child.raw(),
            stderr_child.raw(),
        )?;
        drop(stdin_child);
        drop(stdout_child);
        drop(stderr_child);
        let mut child_guard = SuspendedChildGuard::new(created.process.raw(), job.raw());
        let root_image = process_image(created.process_id)?;
        if !windows_path_eq(
            &fs::canonicalize(&root_image).io_context(
                "P1A_ROOT_IMAGE_CANONICALIZE_FAILED",
                "could not canonicalize the suspended root process image",
            )?,
            &program_before.canonical_path,
        ) {
            return Err(XtaskError::integrity(
                "P1A_ROOT_IMAGE_IDENTITY_MISMATCH",
                "the suspended root process image differs from the bound executable",
            ));
        }
        // SAFETY: process is suspended, handles are live, and the unnamed Job is private.
        if unsafe { AssignProcessToJobObject(job.raw(), created.process.raw()) } == 0 {
            return Err(last_error(
                "P1A_JOB_ASSIGNMENT_FAILED",
                "could not atomically contain the suspended child in the Job Object",
            ));
        }
        child_guard.contained();
        // Resume past CREATE_SUSPENDED so Windows can deliver the root creation
        // event. The debug port stops it before user entry, and every descendant
        // creation event likewise stops the new process before user code executes.
        // SAFETY: this is the primary thread returned suspended by CreateProcessW.
        if unsafe { ResumeThread(created.thread.raw()) } == u32::MAX {
            return Err(last_error(
                "P1A_PROCESS_RESUME_FAILED",
                "could not resume the contained child process",
            ));
        }
        let mut debug = DebugState::default();
        match wait_debug_event(
            created.process_id,
            &mut debug,
            &mut state,
            command,
            &path_policy,
            10_000,
        )? {
            DebugWait::Event {
                code: CREATE_PROCESS_DEBUG_EVENT,
                process_id,
            } if process_id == created.process_id => {}
            DebugWait::Timeout => {
                return Err(XtaskError::integrity(
                    "P1A_ROOT_DEBUG_EVENT_MISSING",
                    "the contained root process emitted no creation debug event",
                ));
            }
            DebugWait::Event { code, process_id } => {
                return Err(XtaskError::integrity(
                    "P1A_ROOT_DEBUG_EVENT_INVALID",
                    format!(
                        "the first debug event was not the suspended root process creation: code={code}, pid={process_id}"
                    ),
                ));
            }
        }
        let root_identity = debug
            .processes
            .get(&created.process_id)
            .map(|process| process.identity)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_ROOT_DEBUG_IDENTITY_MISSING",
                    "the root creation event did not retain its creation-bound identity",
                )
            })?;
        child_guard.disarm();
        let mut job_guard = JobTerminationGuard::new(job.raw());
        // The primary thread handle is no longer needed after a successful resume.
        created.thread = OwnedHandle(null_mut());

        let started = Instant::now();
        let mut timed_out = false;
        while debug.root_exit_code.is_none() {
            let elapsed = started.elapsed();
            if elapsed >= command.timeout {
                timed_out = true;
                // SAFETY: the private Job handle is live and owns only this command tree.
                unsafe { TerminateJobObject(job.raw(), 124) };
            }
            let remaining = if timed_out {
                Duration::from_secs(10)
            } else {
                command.timeout.saturating_sub(elapsed)
            };
            let wait_ms = remaining.as_millis().clamp(1, 50) as u32;
            match wait_debug_event(
                created.process_id,
                &mut debug,
                &mut state,
                command,
                &path_policy,
                wait_ms,
            )? {
                DebugWait::Event { .. } => {}
                DebugWait::Timeout if timed_out => break,
                DebugWait::Timeout => continue,
            }
        }

        let exit_code = if timed_out {
            124
        } else {
            debug.root_exit_code.ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_ROOT_EXIT_EVENT_MISSING",
                    "the root process ended without a matching exit debug event",
                )
            })?
        };

        let drain_started = Instant::now();
        let mut remaining = Vec::new();
        while drain_started.elapsed() < Duration::from_secs(10) {
            remaining = job_process_ids(job.raw())?;
            if remaining.is_empty() {
                break;
            }
            match wait_debug_event(
                created.process_id,
                &mut debug,
                &mut state,
                command,
                &path_policy,
                5,
            )? {
                DebugWait::Event { .. } | DebugWait::Timeout => {}
            }
        }
        let mut unexpected_descendants = false;
        let mut qualified_tool_descendants_cleaned = false;
        if !remaining.is_empty() {
            if !timed_out && qualified_survivors(&remaining, &state, command)? {
                qualified_tool_descendants_cleaned = true;
            } else {
                unexpected_descendants = true;
            }
            // SAFETY: the private Job owns only this command tree.
            unsafe { TerminateJobObject(job.raw(), 125) };
        }
        let termination_started = Instant::now();
        let mut process_tree_terminated = false;
        while termination_started.elapsed() < Duration::from_secs(10) {
            if job_process_ids(job.raw())?.is_empty() && debug.processes.is_empty() {
                process_tree_terminated = true;
                break;
            }
            match wait_debug_event(
                created.process_id,
                &mut debug,
                &mut state,
                command,
                &path_policy,
                25,
            )? {
                DebugWait::Event { .. } | DebugWait::Timeout => {}
            }
        }

        drop(stdout_file);
        drop(stderr_file);
        drop(stdin_file);
        let stdout = read_bounded(&stdout_path)?;
        let stderr = read_bounded(&stderr_path)?;
        let _ = fs::remove_file(&stdout_path);
        let _ = fs::remove_file(&stderr_path);

        if !process_tree_terminated {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_TREE_NOT_TERMINATED",
                "the contained command Job did not drain after bounded termination",
            ));
        }
        job_guard.disarm();
        let module_covered_identities = state
            .loaded_modules
            .iter()
            .map(|module| ProcessIdentity {
                process_id: module.process_id,
                creation_time_100ns: module.creation_time_100ns,
            })
            .collect::<BTreeSet<_>>();
        let total_processes = job_total_processes(job.raw())?;
        if state.identities.is_empty()
            || state.exit_races != 0
            || state.executable_covered_identities != state.identities
            || state.snapshot_covered_identities != state.identities
            || module_covered_identities != state.identities
            || state.process_identities.len() != state.identities.len()
            || total_processes != state.identities.len()
        {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_AUDIT_INCOMPLETE",
                format!(
                    "every process must have exact executable/module coverage: total={total_processes}, observed={}, executable={}, snapshots={}, modules={}, records={}, exit_races={}",
                    state.identities.len(),
                    state.executable_covered_identities.len(),
                    state.snapshot_covered_identities.len(),
                    module_covered_identities.len(),
                    state.process_identities.len(),
                    state.exit_races,
                ),
            ));
        }
        require_qualified_files_observed(&path_policy.qualified_files, &state.loaded_modules)?;
        if command.policy == ProcessPolicy::HostOnly {
            require_vswhere_syswow64_observed(command, &state.loaded_modules)?;
        }
        let program_after = identity_from_locked_file(&program_lock)?;
        if program_after != program_before {
            return Err(XtaskError::integrity(
                "P1A_EXECUTED_PROGRAM_CHANGED",
                "the executed program identity changed across the audited command",
            ));
        }
        let audit = ProcessAudit {
            audit_method: "windows_job_object_toolhelp32_v1".to_owned(),
            atomic_job_assignment: true,
            root_process_id: root_identity.process_id,
            root_creation_time_100ns: root_identity.creation_time_100ns,
            audited_process_count: state.identities.len(),
            covered_process_count: state
                .executable_covered_identities
                .intersection(&state.snapshot_covered_identities)
                .count(),
            successful_snapshots: state.successful_snapshots,
            exit_races: state.exit_races,
            executable_names: state.executable_names.into_iter().collect(),
            process_identities: state.process_identities.into_iter().collect(),
            loaded_modules: state.loaded_modules.into_iter().collect(),
            forbidden_processes: state.forbidden_processes.into_iter().collect(),
            forbidden_modules: state.forbidden_modules.into_iter().collect(),
            process_tree_terminated,
            unexpected_descendants,
            qualified_tool_descendants_cleaned,
            timed_out,
        };
        Ok(AuditedOutput {
            exit_code: exit_code as i32,
            stdout,
            stderr,
            audit,
        })
    }

    fn validate_spec(command: &DirectCommand) -> Result<()> {
        if !command.program.is_absolute() || !command.program.is_file() {
            return Err(XtaskError::integrity(
                "P1A_COMMAND_PROGRAM_INVALID",
                "a P1A child program must be an existing absolute regular file",
            ));
        }
        if !command.cwd.is_absolute() || !command.cwd.is_dir() {
            return Err(XtaskError::integrity(
                "P1A_COMMAND_CWD_INVALID",
                "a P1A child working directory must be an existing absolute directory",
            ));
        }
        if command.display_argv.len() != command.args.len() + 1
            || command
                .display_argv
                .first()
                .is_none_or(|value| !value.starts_with("${"))
            || command.timeout.is_zero()
        {
            return Err(XtaskError::integrity(
                "P1A_COMMAND_SPEC_INVALID",
                "a P1A child command has a malformed public argv or timeout",
            ));
        }
        if command.capture_stem.is_empty()
            || command.capture_stem.len() > 64
            || !command
                .capture_stem
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(XtaskError::integrity(
                "P1A_CAPTURE_STEM_INVALID",
                "a P1A capture stem must be a bounded portable identifier",
            ));
        }
        if command
            .capture_directory
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(XtaskError::integrity(
                "P1A_CAPTURE_DIRECTORY_INVALID",
                "a P1A capture directory cannot contain parent traversal",
            ));
        }
        for root in &command.qualified_persistent_roots {
            if !root.is_absolute() || !root.is_dir() {
                return Err(XtaskError::integrity(
                    "P1A_QUALIFIED_ROOT_INVALID",
                    "every qualified process path root must be an existing absolute directory",
                ));
            }
        }
        validate_policy_environment(command.policy, &command.environment)?;
        validate_qualified_persistent_file_count(command)?;
        for file in &command.qualified_persistent_files {
            if !file.path.is_absolute()
                || !file.path.is_file()
                || !crate::hash::is_lower_sha256(&file.sha256)
                || file.bytes == 0
            {
                return Err(XtaskError::integrity(
                    "P1A_QUALIFIED_FILE_INVALID",
                    "every exact qualified process file must bind an absolute regular path, SHA-256, and positive length",
                ));
            }
        }
        Ok(())
    }

    fn validate_policy_environment(
        policy: ProcessPolicy,
        environment: &BTreeMap<String, Option<OsString>>,
    ) -> Result<()> {
        let cuda_only = [
            "CUDA_VISIBLE_DEVICES",
            "CUDA_CACHE_PATH",
            "CUDA_CACHE_DISABLE",
        ];
        if policy != ProcessPolicy::CudaProbe
            && cuda_only.iter().any(|key| environment.contains_key(*key))
        {
            return Err(XtaskError::integrity(
                "P1A_COMMAND_ENVIRONMENT_OVERRIDE_FORBIDDEN",
                "CUDA selection and cache overrides are permitted only by the CUDA process policy",
            ));
        }
        Ok(())
    }
    fn validate_qualified_persistent_file_count(command: &DirectCommand) -> Result<()> {
        let expected = match command.policy {
            ProcessPolicy::HostOnly => usize::from(is_vswhere_command(command)),
            ProcessPolicy::CudaProbe => 0,
        };
        if command.qualified_persistent_files.len() != expected {
            return Err(XtaskError::integrity(
                "P1A_QUALIFIED_FILE_COUNT_INVALID",
                "the fixed vswhere command requires exactly one exact runtime file and every other command requires none",
            ));
        }
        Ok(())
    }

    fn bind_executable(path: &Path) -> Result<(ExecutableIdentity, File)> {
        let bound = bind_file(path, "P1A_PROGRAM")?;
        let identity = complete_file_identity(&bound)?;
        Ok((identity, bound.file))
    }

    fn bind_process_path_policy(command: &DirectCommand) -> Result<ProcessPathPolicy> {
        let system32 = fs::canonicalize(system_directory()?).io_context(
            "P1A_SYSTEM32_INVALID",
            "could not canonicalize System32 for process classification",
        )?;
        let syswow64 = fs::canonicalize(system_wow64_directory()?).io_context(
            "P1A_SYSWOW64_INVALID",
            "could not canonicalize SysWOW64 for process classification",
        )?;
        let valid_system32 = system32
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|leaf| leaf.eq_ignore_ascii_case("System32"));
        let valid_syswow64 = syswow64
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|leaf| leaf.eq_ignore_ascii_case("SysWOW64"));
        if !valid_system32
            || !valid_syswow64
            || windows_path_eq(&system32, &syswow64)
            || system32.parent().is_none()
            || system32.parent() != syswow64.parent()
        {
            return Err(XtaskError::integrity(
                "P1A_SYSTEM_RUNTIME_ROOTS_INVALID",
                "native System32 and SysWOW64 are not distinct canonical sibling directories",
            ));
        }
        Ok(ProcessPathPolicy {
            system32,
            syswow64,
            qualified_files: bind_qualified_persistent_files(command)?,
        })
    }

    fn bind_qualified_persistent_files(command: &DirectCommand) -> Result<Vec<BoundFile>> {
        validate_policy_environment(command.policy, &command.environment)?;
        validate_qualified_persistent_file_count(command)?;
        let program_parent = command.program.parent().ok_or_else(|| {
            XtaskError::integrity("P1A_COMMAND_PROGRAM_INVALID", "program has no parent")
        })?;
        let program_parent = fs::canonicalize(program_parent).io_context(
            "P1A_COMMAND_PROGRAM_INVALID",
            "could not canonicalize the audited program directory",
        )?;
        let mut qualified_roots = Vec::with_capacity(command.qualified_persistent_roots.len());
        for root in &command.qualified_persistent_roots {
            qualified_roots.push(fs::canonicalize(root).io_context(
                "P1A_QUALIFIED_ROOT_INVALID",
                "could not canonicalize a qualified process path root",
            )?);
        }
        let mut bound_files: Vec<BoundFile> =
            Vec::with_capacity(command.qualified_persistent_files.len());
        for expected in &command.qualified_persistent_files {
            if !expected
                .path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|leaf| {
                    leaf.eq_ignore_ascii_case(
                        "Microsoft.VisualStudio.Setup.Configuration.Native.dll",
                    )
                })
            {
                return Err(XtaskError::integrity(
                    "P1A_QUALIFIED_FILE_LEAF_FORBIDDEN",
                    "the exact vswhere runtime file does not have the fixed Setup Configuration leaf",
                ));
            }
            let bound = bind_file(&expected.path, "P1A_QUALIFIED_PERSISTENT_FILE")?;
            let observed = complete_file_identity(&bound)?;
            if observed.sha256 != expected.sha256 || observed.bytes != expected.bytes {
                return Err(XtaskError::integrity(
                    "P1A_QUALIFIED_FILE_IDENTITY_MISMATCH",
                    "the exact qualified file differs from its predeclared locked identity",
                ));
            }
            if path_within(&bound.canonical_path, &program_parent)
                || qualified_roots
                    .iter()
                    .any(|root| path_within(&bound.canonical_path, root))
            {
                return Err(XtaskError::integrity(
                    "P1A_QUALIFIED_FILE_SCOPE_AMBIGUOUS",
                    "an exact qualified file is already covered by a broader qualified directory",
                ));
            }
            if bound_files
                .iter()
                .any(|existing| windows_path_eq(&bound.canonical_path, &existing.canonical_path))
            {
                return Err(XtaskError::integrity(
                    "P1A_QUALIFIED_FILE_DUPLICATE",
                    "the exact qualified file set contains a duplicate canonical path",
                ));
            }
            bound_files.push(bound);
        }
        Ok(bound_files)
    }

    fn require_qualified_files_observed(
        qualified_files: &[BoundFile],
        loaded_modules: &BTreeSet<LoadedModuleIdentity>,
    ) -> Result<()> {
        for file in qualified_files {
            let expected = complete_file_identity(file)?;
            let mut matches = loaded_modules.iter().filter(|module| {
                module.path_class == "qualified_tool_file"
                    && module.canonical_path_sha256 == expected.canonical_path_sha256
            });
            let Some(observed) = matches.next() else {
                return Err(XtaskError::integrity(
                    "P1A_QUALIFIED_FILE_NOT_OBSERVED",
                    "an exact qualified file was not observed in the completed module audit",
                ));
            };
            if matches.next().is_some()
                || observed.module_sha256 != expected.sha256
                || observed.module_bytes != expected.bytes
            {
                return Err(XtaskError::integrity(
                    "P1A_QUALIFIED_FILE_OBSERVATION_INVALID",
                    "an exact qualified file was not observed exactly once with its bound identity",
                ));
            }
        }
        Ok(())
    }

    fn require_vswhere_syswow64_observed(
        command: &DirectCommand,
        loaded_modules: &BTreeSet<LoadedModuleIdentity>,
    ) -> Result<()> {
        let observed = loaded_modules
            .iter()
            .any(|module| module.path_class == "windows_syswow64");
        if observed != is_vswhere_command(command) {
            return Err(XtaskError::integrity(
                "P1A_VSWHERE_SYSWOW64_OBSERVATION_INVALID",
                "the fixed vswhere command must observe the WOW64 system runtime and no other command may do so",
            ));
        }
        Ok(())
    }

    fn identity_from_locked_file(file: &File) -> Result<ExecutableIdentity> {
        let bound = bind_open_file(file.try_clone().io_context(
            "P1A_PROGRAM_LOCK_DUPLICATE_FAILED",
            "could not duplicate the bound executable handle",
        )?)?;
        complete_file_identity(&bound)
    }

    fn bind_file(path: &Path, code_prefix: &str) -> Result<BoundFile> {
        if !path.is_absolute() {
            return Err(XtaskError::integrity(
                "P1A_AUDITED_FILE_PATH_INVALID",
                format!("{code_prefix} path is not absolute"),
            ));
        }
        let link_metadata = fs::symlink_metadata(path).io_context(
            "P1A_AUDITED_FILE_LSTAT_FAILED",
            format!("could not inspect the {code_prefix} path without following links"),
        )?;
        if link_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(XtaskError::integrity(
                "P1A_AUDITED_FILE_REPARSE_REJECTED",
                format!("{code_prefix} path is a reparse point"),
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            // The loaded/executed bytes remain immutable while their identity is bound.
            .share_mode(FILE_SHARE_READ)
            .open(path)
            .io_context(
                "P1A_AUDITED_FILE_LOCK_FAILED",
                format!("could not hold a deny-write/delete handle on {code_prefix}"),
            )?;
        let bound = bind_open_file(file)?;
        let requested = fs::canonicalize(path).io_context(
            "P1A_AUDITED_FILE_CANONICALIZE_FAILED",
            format!("could not canonicalize {code_prefix}"),
        )?;
        if !windows_path_eq(&requested, &bound.canonical_path) {
            return Err(XtaskError::integrity(
                "P1A_AUDITED_FILE_HANDLE_PATH_MISMATCH",
                format!("{code_prefix} path does not resolve to the opened file handle"),
            ));
        }
        Ok(bound)
    }

    fn bind_open_file(file: File) -> Result<BoundFile> {
        let metadata = file.metadata().io_context(
            "P1A_AUDITED_FILE_IDENTITY_FAILED",
            "could not inspect an opened audited file",
        )?;
        if !metadata.is_file()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || metadata.file_size() == 0
        {
            return Err(XtaskError::integrity(
                "P1A_AUDITED_FILE_IDENTITY_INVALID",
                "an opened audited file is empty, non-regular, or a reparse point",
            ));
        }
        let (volume_serial_number, file_index) = file_id(&file)?;
        let canonical_path = final_path(&file)?;
        require_path_ancestors_not_reparse(&canonical_path)?;
        Ok(BoundFile {
            file,
            canonical_path_sha256: canonical_path_hash(&canonical_path),
            canonical_path,
            bytes: metadata.file_size(),
            last_write_time: metadata.last_write_time(),
            volume_serial_number,
            file_index,
        })
    }

    fn require_path_ancestors_not_reparse(path: &Path) -> Result<()> {
        let mut current = path.parent();
        while let Some(ancestor) = current {
            let metadata = fs::symlink_metadata(ancestor).io_context(
                "P1A_AUDITED_PATH_ANCESTOR_FAILED",
                "could not inspect an audited path ancestor",
            )?;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(XtaskError::integrity(
                    "P1A_AUDITED_PATH_ANCESTOR_REPARSE_REJECTED",
                    "an audited executable or module path traverses a reparse-point ancestor",
                ));
            }
            if ancestor.parent().is_none() {
                break;
            }
            current = ancestor.parent();
        }
        Ok(())
    }

    fn complete_file_identity(bound: &BoundFile) -> Result<ExecutableIdentity> {
        const MAX_AUDITED_PE_BYTES: u64 = 1024 * 1024 * 1024;
        if bound.bytes > MAX_AUDITED_PE_BYTES {
            return Err(XtaskError::integrity(
                "P1A_RUNTIME_PE_TOO_LARGE",
                "an audited process image exceeds the closed one-GiB PE parser bound",
            ));
        }
        let mut input = bound.file.try_clone().io_context(
            "P1A_AUDITED_FILE_HASH_FAILED",
            "could not duplicate an audited file handle for hashing",
        )?;
        input.seek(SeekFrom::Start(0)).io_context(
            "P1A_AUDITED_FILE_HASH_FAILED",
            "could not rewind an audited file handle for hashing",
        )?;
        let mut hasher = Sha256::new();
        let capacity = usize::try_from(bound.bytes).map_err(|_| {
            XtaskError::integrity(
                "P1A_RUNTIME_PE_TOO_LARGE",
                "an audited process image exceeds the addressable PE parser bound",
            )
        })?;
        let mut image = Vec::with_capacity(capacity);
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = input.read(&mut buffer).io_context(
                "P1A_AUDITED_FILE_HASH_FAILED",
                "could not hash an audited file through its bound handle",
            )?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            image.extend_from_slice(&buffer[..read]);
        }
        let after = bound.file.metadata().io_context(
            "P1A_AUDITED_FILE_REVALIDATION_FAILED",
            "could not revalidate an audited file after hashing",
        )?;
        let (after_volume, after_index) = file_id(&bound.file)?;
        if after.file_size() != bound.bytes
            || after.last_write_time() != bound.last_write_time
            || after_volume != bound.volume_serial_number
            || after_index != bound.file_index
            || !windows_path_eq(&final_path(&bound.file)?, &bound.canonical_path)
        {
            return Err(XtaskError::integrity(
                "P1A_AUDITED_FILE_CHANGED",
                "an audited file identity changed while it was being hashed",
            ));
        }
        if image.len() != capacity {
            return Err(XtaskError::integrity(
                "P1A_AUDITED_FILE_CHANGED",
                "an audited process image length changed while it was being hashed",
            ));
        }
        let forbidden_content_markers = inspect_runtime_pe(&image)?;
        Ok(ExecutableIdentity {
            canonical_path: bound.canonical_path.clone(),
            canonical_path_sha256: bound.canonical_path_sha256.clone(),
            sha256: hex::encode(hasher.finalize()),
            bytes: bound.bytes,
            last_write_time: bound.last_write_time,
            volume_serial_number: bound.volume_serial_number,
            file_index: bound.file_index,
            forbidden_content_markers,
        })
    }

    #[derive(Clone, Copy)]
    struct RuntimePeLayout {
        pe64: bool,
        image_base: u64,
        size_of_headers: u32,
        data_directory: usize,
        directory_count: usize,
        section_table: usize,
        section_count: usize,
    }

    fn inspect_runtime_pe(bytes: &[u8]) -> Result<BTreeSet<String>> {
        let layout = runtime_pe_layout(bytes)?;
        let mut markers = BTreeSet::new();
        for index in 0..layout.section_count {
            let section = layout.section_table + index * 40;
            let name = runtime_fixed_name(bytes, section, 8, "section")?;
            let compact = name
                .bytes()
                .filter(|byte| byte.is_ascii_alphanumeric())
                .map(|byte| byte.to_ascii_lowercase())
                .collect::<Vec<_>>();
            if compact
                .windows(b"nvfat".len())
                .any(|value| value == b"nvfat")
            {
                markers.insert("pe-section:nvidia-fatbin".to_owned());
            }
            if compact
                .windows(b"hipfat".len())
                .any(|value| value == b"hipfat")
            {
                markers.insert("pe-section:hip-fatbin".to_owned());
            }
        }

        let mut libraries = BTreeSet::new();
        let mut symbols = BTreeSet::new();
        if let Some((rva, size)) = runtime_data_directory(bytes, layout, 1)? {
            parse_runtime_imports(bytes, layout, rva, size, &mut libraries, &mut symbols)?;
        }
        if let Some((rva, size)) = runtime_data_directory(bytes, layout, 13)? {
            parse_runtime_delay_imports(bytes, layout, rva, size, &mut libraries, &mut symbols)?;
        }
        if let Some((rva, size)) = runtime_data_directory(bytes, layout, 0)? {
            parse_runtime_exports(bytes, layout, rva, size, &mut symbols)?;
        }

        for library in libraries {
            if forbidden_module(&library, ProcessPolicy::HostOnly) {
                markers.insert(format!("pe-import-library:{library}"));
            }
        }
        for symbol in symbols {
            if let Some(classification) = forbidden_runtime_symbol(&symbol) {
                markers.insert(format!("pe-symbol:{classification}"));
            }
        }
        Ok(markers)
    }

    fn runtime_pe_layout(bytes: &[u8]) -> Result<RuntimePeLayout> {
        if bytes.get(..2) != Some(b"MZ") {
            return Err(runtime_pe_error(
                "P1A_RUNTIME_PE_HEADER_INVALID",
                "an audited process image lacks an MZ header",
            ));
        }
        let pe = runtime_read_u32(bytes, 0x3c)? as usize;
        if bytes.get(pe..pe.saturating_add(4)) != Some(b"PE\0\0") {
            return Err(runtime_pe_error(
                "P1A_RUNTIME_PE_HEADER_INVALID",
                "an audited process image lacks a PE signature",
            ));
        }
        let coff = pe.checked_add(4).ok_or_else(runtime_pe_bounds_error)?;
        let section_count = runtime_read_u16(bytes, coff + 2)? as usize;
        if section_count == 0 || section_count > 96 {
            return Err(runtime_pe_error(
                "P1A_RUNTIME_PE_HEADER_INVALID",
                "an audited process image has an invalid PE section count",
            ));
        }
        let optional_size = runtime_read_u16(bytes, coff + 16)? as usize;
        let optional = coff.checked_add(20).ok_or_else(runtime_pe_bounds_error)?;
        let magic = runtime_read_u16(bytes, optional)?;
        let (pe64, image_base, data_directory, directory_count_offset, fixed_size) = match magic {
            0x10b => (
                false,
                runtime_read_u32(bytes, optional + 28)? as u64,
                optional + 96,
                optional + 92,
                96usize,
            ),
            0x20b => (
                true,
                runtime_read_u64(bytes, optional + 24)?,
                optional + 112,
                optional + 108,
                112usize,
            ),
            _ => {
                return Err(runtime_pe_error(
                    "P1A_RUNTIME_PE_OPTIONAL_HEADER_INVALID",
                    "an audited process image has an unsupported PE optional-header magic",
                ));
            }
        };
        if optional_size < fixed_size {
            return Err(runtime_pe_error(
                "P1A_RUNTIME_PE_OPTIONAL_HEADER_INVALID",
                "an audited process image has a truncated PE optional header",
            ));
        }
        let directory_count = runtime_read_u32(bytes, directory_count_offset)? as usize;
        let available_directories = (optional_size - fixed_size) / 8;
        if directory_count > available_directories || directory_count > 16 {
            return Err(runtime_pe_error(
                "P1A_RUNTIME_PE_OPTIONAL_HEADER_INVALID",
                "an audited process image declares unavailable PE data directories",
            ));
        }
        let section_table = optional
            .checked_add(optional_size)
            .ok_or_else(runtime_pe_bounds_error)?;
        runtime_slice(
            bytes,
            section_table,
            section_count
                .checked_mul(40)
                .ok_or_else(runtime_pe_bounds_error)?,
        )?;
        let size_of_headers = runtime_read_u32(bytes, optional + 60)?;
        if size_of_headers == 0 || size_of_headers as usize > bytes.len() {
            return Err(runtime_pe_error(
                "P1A_RUNTIME_PE_OPTIONAL_HEADER_INVALID",
                "an audited process image has an invalid PE header span",
            ));
        }
        for index in 0..section_count {
            let section = section_table + index * 40;
            runtime_fixed_name(bytes, section, 8, "section")?;
            let raw_size = runtime_read_u32(bytes, section + 16)? as usize;
            let raw_offset = runtime_read_u32(bytes, section + 20)? as usize;
            if raw_size != 0 {
                runtime_slice(bytes, raw_offset, raw_size)?;
            }
        }
        Ok(RuntimePeLayout {
            pe64,
            image_base,
            size_of_headers,
            data_directory,
            directory_count,
            section_table,
            section_count,
        })
    }

    fn runtime_data_directory(
        bytes: &[u8],
        layout: RuntimePeLayout,
        index: usize,
    ) -> Result<Option<(u32, u32)>> {
        if index >= layout.directory_count {
            return Ok(None);
        }
        let entry = layout
            .data_directory
            .checked_add(index.checked_mul(8).ok_or_else(runtime_pe_bounds_error)?)
            .ok_or_else(runtime_pe_bounds_error)?;
        let rva = runtime_read_u32(bytes, entry)?;
        let size = runtime_read_u32(bytes, entry + 4)?;
        match (rva, size) {
            (0, 0) => Ok(None),
            (0, _) | (_, 0) => Err(runtime_pe_error(
                "P1A_RUNTIME_PE_DIRECTORY_INVALID",
                "an audited process image has an incomplete PE data-directory reference",
            )),
            _ => {
                runtime_rva_slice(bytes, layout, rva, 1)?;
                Ok(Some((rva, size)))
            }
        }
    }

    fn parse_runtime_imports(
        bytes: &[u8],
        layout: RuntimePeLayout,
        directory_rva: u32,
        directory_size: u32,
        libraries: &mut BTreeSet<String>,
        symbols: &mut BTreeSet<String>,
    ) -> Result<()> {
        let maximum = (directory_size as usize / 20).min(4096);
        if maximum == 0 {
            return Err(runtime_pe_bounds_error());
        }
        for index in 0..maximum {
            let offset_rva = directory_rva
                .checked_add((index * 20) as u32)
                .ok_or_else(runtime_pe_bounds_error)?;
            let descriptor = runtime_rva_slice(bytes, layout, offset_rva, 20)?;
            if descriptor.iter().all(|byte| *byte == 0) {
                return Ok(());
            }
            let original_thunk = u32::from_le_bytes(descriptor[0..4].try_into().unwrap());
            let name_rva = u32::from_le_bytes(descriptor[12..16].try_into().unwrap());
            let first_thunk = u32::from_le_bytes(descriptor[16..20].try_into().unwrap());
            libraries.insert(runtime_rva_name(bytes, layout, name_rva, "import library")?);
            let thunk = if original_thunk != 0 {
                original_thunk
            } else {
                first_thunk
            };
            if thunk != 0 {
                parse_runtime_thunks(bytes, layout, thunk, symbols)?;
            }
        }
        Err(runtime_pe_error(
            "P1A_RUNTIME_PE_IMPORT_LIMIT_EXCEEDED",
            "an audited process image import table lacks a bounded terminator",
        ))
    }

    fn parse_runtime_delay_imports(
        bytes: &[u8],
        layout: RuntimePeLayout,
        directory_rva: u32,
        directory_size: u32,
        libraries: &mut BTreeSet<String>,
        symbols: &mut BTreeSet<String>,
    ) -> Result<()> {
        let maximum = (directory_size as usize / 32).min(4096);
        if maximum == 0 {
            return Err(runtime_pe_bounds_error());
        }
        for index in 0..maximum {
            let offset_rva = directory_rva
                .checked_add((index * 32) as u32)
                .ok_or_else(runtime_pe_bounds_error)?;
            let descriptor = runtime_rva_slice(bytes, layout, offset_rva, 32)?;
            if descriptor.iter().all(|byte| *byte == 0) {
                return Ok(());
            }
            let attributes = u32::from_le_bytes(descriptor[0..4].try_into().unwrap());
            let name = u32::from_le_bytes(descriptor[4..8].try_into().unwrap());
            let import_names = u32::from_le_bytes(descriptor[16..20].try_into().unwrap());
            let name_rva = runtime_delay_rva(name, attributes, layout.image_base)?;
            libraries.insert(runtime_rva_name(
                bytes,
                layout,
                name_rva,
                "delay-import library",
            )?);
            if import_names != 0 {
                parse_runtime_thunks(
                    bytes,
                    layout,
                    runtime_delay_rva(import_names, attributes, layout.image_base)?,
                    symbols,
                )?;
            }
        }
        Err(runtime_pe_error(
            "P1A_RUNTIME_PE_DELAY_IMPORT_LIMIT_EXCEEDED",
            "an audited process image delay-import table lacks a bounded terminator",
        ))
    }

    fn runtime_delay_rva(value: u32, attributes: u32, image_base: u64) -> Result<u32> {
        if attributes & 1 != 0 {
            return Ok(value);
        }
        let value = value as u64;
        let rva = value.checked_sub(image_base).ok_or_else(|| {
            runtime_pe_error(
                "P1A_RUNTIME_PE_DELAY_IMPORT_INVALID",
                "an audited process image delay-import VA precedes its image base",
            )
        })?;
        u32::try_from(rva).map_err(|_| runtime_pe_bounds_error())
    }

    fn parse_runtime_thunks(
        bytes: &[u8],
        layout: RuntimePeLayout,
        thunk_rva: u32,
        symbols: &mut BTreeSet<String>,
    ) -> Result<()> {
        let width = if layout.pe64 { 8usize } else { 4usize };
        let ordinal_mask = if layout.pe64 {
            1_u64 << 63
        } else {
            1_u64 << 31
        };
        for index in 0..65_536usize {
            let offset = index
                .checked_mul(width)
                .and_then(|value| thunk_rva.checked_add(value as u32))
                .ok_or_else(runtime_pe_bounds_error)?;
            let value = if layout.pe64 {
                let entry = runtime_rva_slice(bytes, layout, offset, 8)?;
                u64::from_le_bytes(entry.try_into().unwrap())
            } else {
                let entry = runtime_rva_slice(bytes, layout, offset, 4)?;
                u32::from_le_bytes(entry.try_into().unwrap()) as u64
            };
            if value == 0 {
                return Ok(());
            }
            if value & ordinal_mask == 0 {
                let name_rva = u32::try_from(value).map_err(|_| runtime_pe_bounds_error())?;
                let name_offset = runtime_rva_to_offset(bytes, layout, name_rva)?;
                runtime_slice(bytes, name_offset, 2)?;
                symbols.insert(runtime_c_string(
                    bytes,
                    name_offset + 2,
                    1024,
                    "import symbol",
                )?);
            }
        }
        Err(runtime_pe_error(
            "P1A_RUNTIME_PE_IMPORT_SYMBOL_LIMIT_EXCEEDED",
            "an audited process image import-symbol table lacks a bounded terminator",
        ))
    }

    fn parse_runtime_exports(
        bytes: &[u8],
        layout: RuntimePeLayout,
        directory_rva: u32,
        directory_size: u32,
        symbols: &mut BTreeSet<String>,
    ) -> Result<()> {
        if directory_size < 40 {
            return Err(runtime_pe_bounds_error());
        }
        let directory = runtime_rva_slice(bytes, layout, directory_rva, 40)?;
        let name_count = u32::from_le_bytes(directory[24..28].try_into().unwrap()) as usize;
        let names_rva = u32::from_le_bytes(directory[32..36].try_into().unwrap());
        if name_count > 1_000_000 || (name_count != 0 && names_rva == 0) {
            return Err(runtime_pe_error(
                "P1A_RUNTIME_PE_EXPORT_LIMIT_EXCEEDED",
                "an audited process image has an invalid PE export-name table",
            ));
        }
        for index in 0..name_count {
            let entry_rva = names_rva
                .checked_add((index * 4) as u32)
                .ok_or_else(runtime_pe_bounds_error)?;
            let entry = runtime_rva_slice(bytes, layout, entry_rva, 4)?;
            let name_rva = u32::from_le_bytes(entry.try_into().unwrap());
            symbols.insert(runtime_rva_name(bytes, layout, name_rva, "export symbol")?);
        }
        Ok(())
    }

    fn runtime_rva_name(
        bytes: &[u8],
        layout: RuntimePeLayout,
        rva: u32,
        label: &'static str,
    ) -> Result<String> {
        let offset = runtime_rva_to_offset(bytes, layout, rva)?;
        runtime_c_string(bytes, offset, 1024, label)
    }

    fn runtime_c_string(
        bytes: &[u8],
        offset: usize,
        limit: usize,
        label: &'static str,
    ) -> Result<String> {
        let tail = bytes.get(offset..).ok_or_else(runtime_pe_bounds_error)?;
        let end = tail
            .iter()
            .take(limit)
            .position(|byte| *byte == 0)
            .ok_or_else(|| {
                runtime_pe_error(
                    "P1A_RUNTIME_PE_NAME_INVALID",
                    format!("an audited process image has an unterminated {label}"),
                )
            })?;
        let name = std::str::from_utf8(&tail[..end]).map_err(|_| {
            runtime_pe_error(
                "P1A_RUNTIME_PE_NAME_INVALID",
                format!("an audited process image has a non-UTF-8 {label}"),
            )
        })?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'/' && byte != b'\\')
        {
            return Err(runtime_pe_error(
                "P1A_RUNTIME_PE_NAME_INVALID",
                format!("an audited process image has an unsafe {label}"),
            ));
        }
        Ok(name.to_ascii_lowercase())
    }

    fn runtime_fixed_name(
        bytes: &[u8],
        offset: usize,
        width: usize,
        label: &'static str,
    ) -> Result<String> {
        let field = runtime_slice(bytes, offset, width)?;
        let end = field.iter().position(|byte| *byte == 0).unwrap_or(width);
        let name = std::str::from_utf8(&field[..end]).map_err(|_| {
            runtime_pe_error(
                "P1A_RUNTIME_PE_NAME_INVALID",
                format!("an audited process image has a non-UTF-8 {label} name"),
            )
        })?;
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(runtime_pe_error(
                "P1A_RUNTIME_PE_NAME_INVALID",
                format!("an audited process image has an invalid {label} name"),
            ));
        }
        Ok(name.to_ascii_lowercase())
    }

    fn runtime_rva_to_offset(bytes: &[u8], layout: RuntimePeLayout, rva: u32) -> Result<usize> {
        if rva < layout.size_of_headers {
            let offset = rva as usize;
            if offset < bytes.len() {
                return Ok(offset);
            }
        }
        for index in 0..layout.section_count {
            let section = layout.section_table + index * 40;
            let virtual_address = runtime_read_u32(bytes, section + 12)?;
            let raw_size = runtime_read_u32(bytes, section + 16)?;
            let raw_offset = runtime_read_u32(bytes, section + 20)?;
            if rva >= virtual_address && rva < virtual_address.saturating_add(raw_size) {
                let delta = rva - virtual_address;
                let offset = raw_offset
                    .checked_add(delta)
                    .ok_or_else(runtime_pe_bounds_error)? as usize;
                if offset < bytes.len() {
                    return Ok(offset);
                }
            }
        }
        Err(runtime_pe_error(
            "P1A_RUNTIME_PE_RVA_INVALID",
            "an audited process image RVA is outside its file-backed PE sections",
        ))
    }

    fn runtime_rva_slice(
        bytes: &[u8],
        layout: RuntimePeLayout,
        rva: u32,
        length: usize,
    ) -> Result<&[u8]> {
        runtime_slice(bytes, runtime_rva_to_offset(bytes, layout, rva)?, length)
    }

    fn runtime_slice(bytes: &[u8], offset: usize, length: usize) -> Result<&[u8]> {
        bytes
            .get(offset..offset.saturating_add(length))
            .ok_or_else(runtime_pe_bounds_error)
    }

    fn runtime_read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(
            runtime_slice(bytes, offset, 2)?.try_into().unwrap(),
        ))
    }

    fn runtime_read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(
            runtime_slice(bytes, offset, 4)?.try_into().unwrap(),
        ))
    }

    fn runtime_read_u64(bytes: &[u8], offset: usize) -> Result<u64> {
        Ok(u64::from_le_bytes(
            runtime_slice(bytes, offset, 8)?.try_into().unwrap(),
        ))
    }

    fn forbidden_runtime_symbol(symbol: &str) -> Option<&'static str> {
        let symbol = symbol.to_ascii_lowercase();
        let symbol = symbol.strip_prefix("__imp_").unwrap_or(&symbol);
        let symbol = symbol.strip_prefix('_').unwrap_or(symbol);
        let exact = [
            ("py_initialize", "python-api"),
            ("py_initializeex", "python-api"),
            ("py_finalize", "python-api"),
            ("py_finalizeex", "python-api"),
            ("pyimport_import", "python-api"),
            ("pyimport_importmodule", "python-api"),
            ("pyeval_evalcode", "python-api"),
            ("pyrun_simplestring", "python-api"),
            ("pygilstate_ensure", "python-api"),
            ("pygilstate_release", "python-api"),
            ("pyobject_call", "python-api"),
            ("pymain_main", "python-api"),
            ("cudamalloc", "cuda-runtime-api"),
            ("cudafree", "cuda-runtime-api"),
            ("cudalaunchkernel", "cuda-runtime-api"),
            ("cudagetdevicecount", "cuda-runtime-api"),
            ("cuinit", "cuda-driver-api"),
            ("cudevicegetcount", "cuda-driver-api"),
            ("culaunchkernel", "cuda-driver-api"),
            ("cumemalloc", "cuda-driver-api"),
            ("cublascreate_v2", "cublas-api"),
            ("cublasgemmex", "cublas-api"),
            ("cudnncreate", "cudnn-api"),
            ("cudnngetversion", "cudnn-api"),
            ("nvrtccreateprogram", "nvrtc-api"),
            ("nvrtccompileprogram", "nvrtc-api"),
            ("nvmlinit_v2", "nvml-api"),
            ("nvmldevicegetcount_v2", "nvml-api"),
            ("cuptiactivityenable", "cupti-api"),
            ("hipmalloc", "hip-runtime-api"),
            ("hipfree", "hip-runtime-api"),
            ("hiplaunchkernel", "hip-runtime-api"),
            ("hipgetdevicecount", "hip-runtime-api"),
            ("rocblas_create_handle", "rocblas-api"),
            ("miopencreate", "miopen-api"),
            ("ortgetapibase", "onnxruntime-api"),
            ("tf_version", "tensorflow-api"),
            ("tf_newstatus", "tensorflow-api"),
        ];
        if let Some((_, classification)) = exact.iter().find(|(name, _)| symbol == *name) {
            return Some(classification);
        }
        if symbol.contains("@torch@@") || symbol.contains("_zn5torch") {
            return Some("libtorch-export");
        }
        if symbol.contains("@c10@@") || symbol.contains("_zn3c10") {
            return Some("libtorch-c10-export");
        }
        None
    }

    fn runtime_pe_error(code: &'static str, message: impl Into<String>) -> XtaskError {
        XtaskError::integrity(code, message)
    }

    fn runtime_pe_bounds_error() -> XtaskError {
        runtime_pe_error(
            "P1A_RUNTIME_PE_BOUNDS_INVALID",
            "an audited process image PE structure exceeds file bounds",
        )
    }

    fn bind_forbidden_system_images() -> Result<Vec<ForbiddenImageIdentity>> {
        let system = system_directory()?;
        let windows = system.parent().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_SYSTEM_DIRECTORY_INVALID",
                "the native Windows system directory has no parent",
            )
        })?;
        let candidates = [
            ("cmd", system.join("cmd.exe")),
            ("wsl", system.join("wsl.exe")),
            ("wslhost", system.join("wslhost.exe")),
            ("bash", system.join("bash.exe")),
            (
                "windows-powershell",
                system.join("WindowsPowerShell/v1.0/powershell.exe"),
            ),
            ("python-launcher", windows.join("py.exe")),
            ("python-windowed-launcher", windows.join("pyw.exe")),
        ];
        let mut identities = Vec::new();
        for (label, path) in candidates {
            if !path.is_file() {
                continue;
            }
            let bound = bind_file(&path, "P1A_FORBIDDEN_SYSTEM_IMAGE")?;
            let identity = complete_file_identity(&bound)?;
            identities.push(ForbiddenImageIdentity {
                label: label.to_owned(),
                sha256: identity.sha256,
                bytes: identity.bytes,
                _lock: bound.file,
            });
        }
        identities.sort_by(|left, right| left.label.cmp(&right.label));
        if !identities.iter().any(|identity| identity.label == "cmd")
            || !identities
                .iter()
                .any(|identity| identity.label == "windows-powershell")
        {
            return Err(XtaskError::integrity(
                "P1A_FORBIDDEN_SYSTEM_IMAGE_MISSING",
                "the native Windows shell identity set is incomplete",
            ));
        }
        Ok(identities)
    }

    fn system_directory() -> Result<PathBuf> {
        let mut buffer = vec![0_u16; NATIVE_DIRECTORY_BUFFER_UNITS];
        // SAFETY: the buffer is writable for the exact length supplied to Win32.
        let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if length == 0 || length as usize >= buffer.len() {
            return Err(last_error(
                "P1A_SYSTEM_DIRECTORY_FAILED",
                "could not resolve the native Windows system directory",
            ));
        }
        let mut end = length as usize;
        while end > 0 && buffer[end - 1] == 0 {
            end -= 1;
        }
        if end == 0 || buffer[..end].contains(&0) {
            return Err(XtaskError::integrity(
                "P1A_SYSTEM_DIRECTORY_INVALID",
                "the native Windows system directory has an invalid UTF-16 terminator",
            ));
        }
        buffer.truncate(end);
        Ok(PathBuf::from(OsString::from_wide(&buffer)))
    }

    fn system_wow64_directory() -> Result<PathBuf> {
        let mut buffer = vec![0_u16; NATIVE_DIRECTORY_BUFFER_UNITS];
        // SAFETY: the buffer is writable for the exact length supplied to Win32.
        let length = unsafe { GetSystemWow64DirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if length == 0 || length as usize >= buffer.len() {
            return Err(last_error(
                "P1A_SYSWOW64_DIRECTORY_FAILED",
                "could not resolve the native Windows SysWOW64 directory",
            ));
        }
        let mut end = length as usize;
        while end > 0 && buffer[end - 1] == 0 {
            end -= 1;
        }
        if end == 0 || buffer[..end].contains(&0) {
            return Err(XtaskError::integrity(
                "P1A_SYSWOW64_DIRECTORY_INVALID",
                "the native Windows SysWOW64 directory has an invalid UTF-16 terminator",
            ));
        }
        buffer.truncate(end);
        Ok(PathBuf::from(OsString::from_wide(&buffer)))
    }

    fn forbidden_identity_marker(file: &ExecutableIdentity, state: &AuditState) -> Option<String> {
        state
            .forbidden_image_identities
            .iter()
            .find(|identity| identity.bytes == file.bytes && identity.sha256 == file.sha256)
            .map(|identity| format!("pe-image-sha256:{}", identity.label))
    }

    fn final_path(file: &File) -> Result<PathBuf> {
        let mut buffer = vec![0_u16; 32_768];
        // SAFETY: the handle is live and the buffer is writable for its declared length.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle() as HANDLE,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                0,
            )
        };
        if length == 0 {
            return Err(last_error(
                "P1A_AUDITED_FILE_FINAL_PATH_FAILED",
                "could not resolve the final path of an audited file handle",
            ));
        }
        if length as usize >= buffer.len() {
            return Err(XtaskError::integrity(
                "P1A_AUDITED_FILE_FINAL_PATH_TRUNCATED",
                "an audited file final path reached the fixed Windows path boundary",
            ));
        }
        buffer.truncate(length as usize);
        let path = PathBuf::from(OsString::from_wide(&buffer));
        if !path.is_absolute() {
            return Err(XtaskError::integrity(
                "P1A_AUDITED_FILE_FINAL_PATH_INVALID",
                "an audited file handle resolved to a non-absolute final path",
            ));
        }
        Ok(path)
    }

    fn file_id(file: &File) -> Result<(u32, u64)> {
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the file handle is live and the output structure is fully sized.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut information) }
            == 0
        {
            return Err(last_error(
                "P1A_AUDITED_FILE_IDENTITY_QUERY_FAILED",
                "could not bind an audited file to its volume and file-index identity",
            ));
        }
        let file_index =
            (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
        if information.dwVolumeSerialNumber == 0 || file_index == 0 {
            return Err(XtaskError::integrity(
                "P1A_AUDITED_FILE_IDENTITY_INVALID",
                "an audited file has a zero volume serial or file index",
            ));
        }
        Ok((information.dwVolumeSerialNumber, file_index))
    }

    fn canonical_path_hash(path: &Path) -> String {
        let mut bytes = Vec::new();
        for value in path.as_os_str().encode_wide() {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        crate::hash::bytes(&bytes)
    }

    fn duplicate_inheritable(source: HANDLE) -> Result<OwnedHandle> {
        let mut duplicate = null_mut();
        // SAFETY: source is a live file handle; the duplicate is created in this process
        // with identical access and inheritance enabled solely for the attribute allowlist.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                source,
                GetCurrentProcess(),
                &mut duplicate,
                0,
                1,
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(last_error(
                "P1A_CAPTURE_DUPLICATE_FAILED",
                "could not create an inheritable private capture handle",
            ));
        }
        OwnedHandle::new(
            duplicate,
            "P1A_CAPTURE_DUPLICATE_FAILED",
            "DuplicateHandle returned an invalid capture handle",
        )
    }

    fn create_job() -> Result<OwnedHandle> {
        // SAFETY: null security/name requests one private unnamed Job Object.
        let job = OwnedHandle::new(
            unsafe { CreateJobObjectW(null(), null()) },
            "P1A_JOB_CREATE_FAILED",
            "could not create the private command Job Object",
        )?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: buffer and size describe a fully initialized limit structure.
        if unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(last_error(
                "P1A_JOB_LIMIT_FAILED",
                "could not set kill-on-close containment on the command Job",
            ));
        }
        Ok(job)
    }

    fn create_suspended_process(
        command: &DirectCommand,
        program: &Path,
        stdin: HANDLE,
        stdout: HANDLE,
        stderr: HANDLE,
    ) -> Result<CreatedProcess> {
        let application = wide_null(program.as_os_str());
        let mut command_line = wide_null(&build_command_line(&command.args));
        let cwd = wide_null(command.cwd.as_os_str());
        let environment = environment_block(&command.environment)?;
        let mut allowed_handles = [stdin, stdout, stderr];
        let mut attributes = AttributeList::for_handles(&mut allowed_handles)?;
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = stdin;
        startup.StartupInfo.hStdOutput = stdout;
        startup.StartupInfo.hStdError = stderr;
        startup.lpAttributeList = attributes.raw();
        let mut information: PROCESS_INFORMATION = unsafe { zeroed() };
        // A detached 32-bit vswhere avoids an otherwise Job-contained conhost.exe
        // which Windows does not report through this debugger's creation stream.
        // Preserve the established no-window mode for every other audited command.
        let console_creation_mode = if is_vswhere_command(command) {
            DETACHED_PROCESS
        } else {
            CREATE_NO_WINDOW
        };
        // SAFETY: every pointer refers to a live, correctly terminated buffer; inherited
        // handles are constrained by PROC_THREAD_ATTRIBUTE_HANDLE_LIST to the three
        // private duplicated standard streams above.
        let created = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                null::<SECURITY_ATTRIBUTES>(),
                null::<SECURITY_ATTRIBUTES>(),
                1,
                CREATE_SUSPENDED
                    | DEBUG_PROCESS
                    | CREATE_UNICODE_ENVIRONMENT
                    | console_creation_mode
                    | EXTENDED_STARTUPINFO_PRESENT,
                environment.as_ptr().cast(),
                cwd.as_ptr(),
                (&raw const startup.StartupInfo),
                &mut information,
            )
        };
        if created == 0 {
            return Err(last_error(
                "P1A_PROCESS_CREATE_FAILED",
                "could not create the audited child process suspended",
            ));
        }
        if information.hProcess.is_null()
            || information.hProcess == INVALID_HANDLE_VALUE
            || information.hThread.is_null()
            || information.hThread == INVALID_HANDLE_VALUE
        {
            // SAFETY: any valid handles are fresh outputs owned by this failed wrapper.
            unsafe {
                if !information.hProcess.is_null() && information.hProcess != INVALID_HANDLE_VALUE {
                    TerminateProcess(information.hProcess, 126);
                    WaitForSingleObject(information.hProcess, 10_000);
                    CloseHandle(information.hProcess);
                }
                if !information.hThread.is_null() && information.hThread != INVALID_HANDLE_VALUE {
                    CloseHandle(information.hThread);
                }
            }
            return Err(XtaskError::integrity(
                "P1A_PROCESS_CREATE_HANDLES_INVALID",
                "CreateProcessW succeeded without returning both owned child handles",
            ));
        }
        Ok(CreatedProcess {
            process: OwnedHandle::new(
                information.hProcess,
                "P1A_PROCESS_CREATE_FAILED",
                "CreateProcessW returned an invalid process handle",
            )?,
            thread: OwnedHandle::new(
                information.hThread,
                "P1A_PROCESS_CREATE_FAILED",
                "CreateProcessW returned an invalid thread handle",
            )?,
            process_id: information.dwProcessId,
        })
    }

    fn wait_debug_event(
        root_pid: u32,
        debug: &mut DebugState,
        state: &mut AuditState,
        command: &DirectCommand,
        path_policy: &ProcessPathPolicy,
        milliseconds: u32,
    ) -> Result<DebugWait> {
        let mut event = RawDebugEvent::default();
        // SAFETY: this is the process-creating thread, and `event` is correctly
        // aligned writable storage for one native DEBUG_EVENT.
        if unsafe { WaitForDebugEvent(&mut event, milliseconds) } == 0 {
            // SAFETY: last-error is consumed immediately after WaitForDebugEvent.
            if unsafe { GetLastError() } == ERROR_SEM_TIMEOUT {
                return Ok(DebugWait::Timeout);
            }
            return Err(last_error(
                "P1A_DEBUG_EVENT_WAIT_FAILED",
                "could not receive the next contained process debug event",
            ));
        }

        let continue_status = match event.code {
            CREATE_PROCESS_DEBUG_EVENT => {
                // SAFETY: the active union arm is selected by the event code.
                let info = unsafe { event.payload.create_process };
                let result = handle_create_process_debug_event(
                    &event,
                    info,
                    debug,
                    state,
                    command,
                    path_policy,
                );
                close_debug_file(info.file);
                if let Err(error) = result {
                    return continue_before_error(&event, DBG_CONTINUE, error);
                }
                DBG_CONTINUE
            }
            LOAD_DLL_DEBUG_EVENT => {
                // SAFETY: the active union arm is selected by the event code.
                let info = unsafe { event.payload.load_dll };
                let result = record_debug_module(
                    event.process_id,
                    info.file,
                    debug,
                    state,
                    command,
                    path_policy,
                );
                close_debug_file(info.file);
                if let Err(error) = result {
                    return continue_before_error(&event, DBG_CONTINUE, error);
                }
                DBG_CONTINUE
            }
            EXCEPTION_DEBUG_EVENT => {
                // SAFETY: the active union arm is selected by the event code.
                let exception = unsafe { event.payload.exception };
                let Some(process) = debug.processes.get_mut(&event.process_id) else {
                    return continue_before_error(
                        &event,
                        DBG_EXCEPTION_NOT_HANDLED,
                        XtaskError::integrity(
                            "P1A_DEBUG_PROCESS_UNKNOWN",
                            "an exception debug event referenced an unregistered process",
                        ),
                    );
                };
                match initial_boundary_action(
                    &mut process.initial_boundary,
                    exception.first_chance,
                    exception.record.code,
                ) {
                    InitialBoundaryAction::NotHandled => DBG_EXCEPTION_NOT_HANDLED,
                    InitialBoundaryAction::Continue => DBG_CONTINUE,
                    InitialBoundaryAction::Sample => {
                        let result = sample_pid_handle(
                            process.handle.raw(),
                            process.identity,
                            state,
                            command,
                            path_policy,
                        );
                        if let Err(error) = result {
                            return continue_before_error(&event, DBG_CONTINUE, error);
                        }
                        DBG_CONTINUE
                    }
                }
            }
            EXIT_PROCESS_DEBUG_EVENT => {
                let Some(process) = debug.processes.get(&event.process_id) else {
                    return continue_before_error(
                        &event,
                        DBG_CONTINUE,
                        XtaskError::integrity(
                            "P1A_DEBUG_PROCESS_UNKNOWN",
                            "an exit debug event referenced an unregistered process",
                        ),
                    );
                };
                if process.initial_boundary != InitialDebugBoundary::Complete {
                    return continue_before_error(
                        &event,
                        DBG_CONTINUE,
                        XtaskError::integrity(
                            "P1A_DEBUG_INITIAL_BREAKPOINT_MISSING",
                            "a process exited before its pre-entry module snapshot boundary",
                        ),
                    );
                }
                let result = verify_debug_process_coverage(process, state);
                if let Err(error) = result {
                    return continue_before_error(&event, DBG_CONTINUE, error);
                }
                if event.process_id == root_pid {
                    // SAFETY: the active union arm begins with the documented exit code.
                    debug.root_exit_code = Some(unsafe { event.payload.storage[0] as u32 });
                }
                debug.processes.remove(&event.process_id);
                DBG_CONTINUE
            }
            _ => DBG_CONTINUE,
        };

        continue_debug_event(&event, continue_status)?;
        Ok(DebugWait::Event {
            code: event.code,
            process_id: event.process_id,
        })
    }

    fn handle_create_process_debug_event(
        event: &RawDebugEvent,
        info: RawCreateProcessDebugInfo,
        debug: &mut DebugState,
        state: &mut AuditState,
        command: &DirectCommand,
        path_policy: &ProcessPathPolicy,
    ) -> Result<()> {
        let process_handle = OwnedHandle::new(
            info.process,
            "P1A_DEBUG_PROCESS_HANDLE_INVALID",
            "a process creation event omitted its kernel-held process handle",
        )?;
        let thread_handle = OwnedHandle::new(
            info.thread,
            "P1A_DEBUG_THREAD_HANDLE_INVALID",
            "a process creation event omitted its kernel-held initial-thread handle",
        )?;
        let identity = process_identity_handle(process_handle.raw(), event.process_id)?;
        if debug.processes.contains_key(&event.process_id) {
            return Err(XtaskError::integrity(
                "P1A_DEBUG_PROCESS_DUPLICATE",
                "a process creation event reused an active process identifier",
            ));
        }
        let initial_boundary = initial_debug_boundary(process_handle.raw())?;
        let debug_image = bound_file_from_debug_handle(info.file, "P1A_PROCESS_IMAGE")?;
        let queried_image = fs::canonicalize(process_image_handle(process_handle.raw())?)
            .io_context(
                "P1A_PROCESS_IMAGE_CANONICALIZE_FAILED",
                "could not canonicalize the process image reported by its kernel handle",
            )?;
        if !windows_path_eq(&queried_image, &debug_image.canonical_path) {
            return Err(XtaskError::integrity(
                "P1A_DEBUG_PROCESS_IMAGE_MISMATCH",
                "the process creation image handle differs from the executing process image",
            ));
        }
        record_executable(
            identity,
            complete_file_identity(&debug_image)?,
            state,
            command,
            path_policy,
        )?;
        debug.processes.insert(
            event.process_id,
            DebugProcessState {
                handle: process_handle,
                identity,
                initial_boundary,
            },
        );
        // The initial-thread event handle is never used after the creation
        // boundary. Close it now; the retained process handle remains owned by
        // `DebugProcessState` until the matching exit event (or auditor abort).
        drop(thread_handle);
        Ok(())
    }

    fn record_debug_module(
        pid: u32,
        debug_file: HANDLE,
        debug: &DebugState,
        state: &mut AuditState,
        command: &DirectCommand,
        path_policy: &ProcessPathPolicy,
    ) -> Result<()> {
        if debug_file.is_null() || debug_file == INVALID_HANDLE_VALUE {
            return Err(XtaskError::integrity(
                "P1A_DEBUG_MODULE_HANDLE_INVALID",
                "a module-load event omitted the loaded image file handle",
            ));
        }
        let process = debug.processes.get(&pid).ok_or_else(|| {
            XtaskError::integrity(
                "P1A_DEBUG_PROCESS_UNKNOWN",
                "a module-load event referenced an unregistered process",
            )
        })?;
        let locked = bound_file_from_debug_handle(debug_file, "P1A_LOADED_MODULE")?;
        record_loaded_module(
            process.identity,
            complete_file_identity(&locked)?,
            state,
            command,
            path_policy,
        )
    }

    fn verify_debug_process_coverage(
        process: &DebugProcessState,
        state: &AuditState,
    ) -> Result<()> {
        // The CREATE_PROCESS debug event already bound this map entry to the
        // kernel-reported PID and creation time before user code ran. While its
        // matching EXIT_PROCESS event is pending, that PID cannot be reused.
        // GetProcessTimes is not reliable for every already-exited process on
        // all supported Windows builds, so do not re-query a redundant identity
        // through an exit-state handle here.
        let has_module = state.loaded_modules.iter().any(|module| {
            module.process_id == process.identity.process_id
                && module.creation_time_100ns == process.identity.creation_time_100ns
        });
        if !state
            .executable_covered_identities
            .contains(&process.identity)
            || !state
                .snapshot_covered_identities
                .contains(&process.identity)
            || !has_module
        {
            return Err(XtaskError::integrity(
                "P1A_DEBUG_PROCESS_COVERAGE_INCOMPLETE",
                "a process reached exit without executable, pre-entry snapshot, and module coverage",
            ));
        }
        Ok(())
    }

    fn bound_file_from_debug_handle(debug_file: HANDLE, code_prefix: &str) -> Result<BoundFile> {
        if debug_file.is_null() || debug_file == INVALID_HANDLE_VALUE {
            return Err(XtaskError::integrity(
                "P1A_DEBUG_FILE_HANDLE_INVALID",
                format!("a {code_prefix} debug event omitted its image file handle"),
            ));
        }
        let mut duplicate = null_mut();
        // SAFETY: the debug-event file handle is live until this event is
        // continued. The non-inheritable duplicate becomes Rust-owned below.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                debug_file,
                GetCurrentProcess(),
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(last_error(
                "P1A_DEBUG_FILE_DUPLICATE_FAILED",
                "could not retain a debug-event image file identity",
            ));
        }
        // SAFETY: DuplicateHandle returned one owned Win32 file handle which is
        // transferred exactly once to File and closed by its Drop implementation.
        let event_bound = bind_open_file(unsafe { File::from_raw_handle(duplicate) })?;
        let locked = bind_file(&event_bound.canonical_path, code_prefix)?;
        if event_bound.volume_serial_number != locked.volume_serial_number
            || event_bound.file_index != locked.file_index
            || !windows_path_eq(&event_bound.canonical_path, &locked.canonical_path)
        {
            return Err(XtaskError::integrity(
                "P1A_DEBUG_FILE_IDENTITY_MISMATCH",
                "a debug-event image handle differs from its deny-write/delete path binding",
            ));
        }
        Ok(locked)
    }

    fn continue_debug_event(event: &RawDebugEvent, status: i32) -> Result<()> {
        // SAFETY: the event was returned by WaitForDebugEvent on this thread and
        // has not previously been continued.
        if unsafe { ContinueDebugEvent(event.process_id, event.thread_id, status) } == 0 {
            return Err(last_error(
                "P1A_DEBUG_EVENT_CONTINUE_FAILED",
                "could not continue a contained process debug event",
            ));
        }
        Ok(())
    }

    fn continue_before_error<T>(
        event: &RawDebugEvent,
        status: i32,
        original: XtaskError,
    ) -> Result<T> {
        match continue_debug_event(event, status) {
            Ok(()) => Err(original),
            Err(continue_error) => Err(XtaskError::integrity(
                "P1A_DEBUG_EVENT_CLEANUP_FAILED",
                format!(
                    "audit failed and the stopped debug event could not be continued: original={original}; cleanup={continue_error}"
                ),
            )),
        }
    }

    fn close_debug_file(handle: HANDLE) {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            // SAFETY: CREATE_PROCESS/LOAD_DLL debug-event file handles belong to
            // the debugger and must be closed exactly once after inspection.
            unsafe { CloseHandle(handle) };
        }
    }

    fn sample_pid_handle(
        process: HANDLE,
        identity: ProcessIdentity,
        state: &mut AuditState,
        command: &DirectCommand,
        path_policy: &ProcessPathPolicy,
    ) -> Result<()> {
        state.identities.insert(identity);
        let image = process_image_handle(process)?;
        let executable = if state.executable_covered_identities.contains(&identity) {
            None
        } else {
            Some(bind_file(&image, "P1A_PROCESS_IMAGE")?)
        };
        let module_paths = module_paths(identity.process_id)?;
        let mut modules = Vec::with_capacity(module_paths.len());
        for module_path in module_paths {
            modules.push(bind_file(&module_path, "P1A_LOADED_MODULE")?);
        }
        if process_identity_handle(process, identity.process_id)? != identity {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_IDENTITY_CHANGED",
                "an observed PID changed creation identity during module enumeration",
            ));
        }
        // Binding every backing file immediately after an in-lifetime Toolhelp snapshot
        // closes the snapshot/hash gap. A signalled process handle is not itself an
        // unresolved race: the kernel preserves this exact creation-time-bound process
        // object and all deny-write/delete file handles after exit.
        if modules.is_empty() {
            return Err(XtaskError::integrity(
                "P1A_MODULE_AUDIT_EMPTY",
                "an observed live process had no auditable executable module",
            ));
        }
        if let Some(executable) = executable {
            record_executable(
                identity,
                complete_file_identity(&executable)?,
                state,
                command,
                path_policy,
            )?;
        }
        let expected_executable = state
            .process_identities
            .iter()
            .find(|record| {
                record.process_id == identity.process_id
                    && record.creation_time_100ns == identity.creation_time_100ns
            })
            .cloned()
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_EXECUTABLE_AUDIT_MISSING",
                    "module evidence has no matching executable identity evidence",
                )
            })?;
        let observed_module_keys = modules
            .iter()
            .map(|module| module.canonical_path_sha256.clone())
            .collect::<BTreeSet<_>>();
        let mut event_module_keys = state
            .loaded_modules
            .iter()
            .filter(|module| {
                module.process_id == identity.process_id
                    && module.creation_time_100ns == identity.creation_time_100ns
            })
            .map(|module| module.canonical_path_sha256.clone())
            .collect::<BTreeSet<_>>();
        event_module_keys.insert(expected_executable.canonical_path_sha256.clone());
        if observed_module_keys != event_module_keys {
            return Err(XtaskError::integrity(
                "P1A_DEBUG_MODULE_SET_MISMATCH",
                "the pre-entry Toolhelp module set differs from creation/load debug-event identities",
            ));
        }
        state.successful_snapshots += 1;
        state.snapshot_covered_identities.insert(identity);
        let mut executable_module_seen = false;
        for module in modules {
            let module = complete_file_identity(&module)?;
            executable_module_seen |= module.canonical_path_sha256
                == expected_executable.canonical_path_sha256
                && module.sha256 == expected_executable.executable_sha256
                && module.bytes == expected_executable.executable_bytes;
            record_loaded_module(identity, module, state, command, path_policy)?;
        }
        if !executable_module_seen {
            return Err(XtaskError::integrity(
                "P1A_EXECUTABLE_MODULE_IDENTITY_MISSING",
                "the in-lifetime module snapshot omitted the bound process executable",
            ));
        }
        Ok(())
    }

    fn record_loaded_module(
        identity: ProcessIdentity,
        module: ExecutableIdentity,
        state: &mut AuditState,
        command: &DirectCommand,
        path_policy: &ProcessPathPolicy,
    ) -> Result<()> {
        let module_name = module
            .canonical_path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_MODULE_NAME_NON_UTF8",
                    "an observed module name is not valid UTF-8",
                )
            })?
            .to_ascii_lowercase();
        let path_class = classify_path(&module.canonical_path, command, path_policy)?;
        if forbidden_module(&module_name, command.policy) {
            state.forbidden_modules.insert(module_name.clone());
        }
        if let Some(marker) = forbidden_identity_marker(&module, state) {
            state
                .forbidden_modules
                .insert(format!("{module_name}:{marker}"));
        }
        for marker in &module.forbidden_content_markers {
            if cuda_content_marker_allowed(command.policy, marker) {
                continue;
            }
            state
                .forbidden_modules
                .insert(format!("{module_name}:{marker}"));
        }
        state.loaded_modules.insert(LoadedModuleIdentity {
            process_id: identity.process_id,
            creation_time_100ns: identity.creation_time_100ns,
            module_name,
            canonical_path_sha256: module.canonical_path_sha256,
            module_sha256: module.sha256,
            module_bytes: module.bytes,
            path_class,
        });
        Ok(())
    }

    fn record_executable(
        identity: ProcessIdentity,
        file: ExecutableIdentity,
        state: &mut AuditState,
        command: &DirectCommand,
        path_policy: &ProcessPathPolicy,
    ) -> Result<()> {
        let leaf = file
            .canonical_path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_PROCESS_IMAGE_NON_UTF8",
                    "an observed process image name is not valid UTF-8",
                )
            })?
            .to_ascii_lowercase();
        state.executable_names.insert(leaf.clone());
        if forbidden_process(&leaf, command.policy) {
            state.forbidden_processes.insert(leaf.clone());
        }
        if let Some(marker) = forbidden_identity_marker(&file, state) {
            state.forbidden_processes.insert(format!("{leaf}:{marker}"));
        }
        for marker in &file.forbidden_content_markers {
            if cuda_content_marker_allowed(command.policy, marker) {
                continue;
            }
            state.forbidden_processes.insert(format!("{leaf}:{marker}"));
        }
        let record = AuditedProcessIdentity {
            process_id: identity.process_id,
            creation_time_100ns: identity.creation_time_100ns,
            executable_name: leaf,
            canonical_path_sha256: file.canonical_path_sha256,
            executable_sha256: file.sha256,
            executable_bytes: file.bytes,
            path_class: classify_path(&file.canonical_path, command, path_policy)?,
        };
        if let Some(existing) = state.process_identities.iter().find(|existing| {
            existing.process_id == identity.process_id
                && existing.creation_time_100ns == identity.creation_time_100ns
        }) {
            if existing != &record {
                return Err(XtaskError::integrity(
                    "P1A_PROCESS_IDENTITY_CONFLICT",
                    "a process identity produced conflicting executable evidence",
                ));
            }
        } else if !state.process_identities.insert(record) {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_IDENTITY_DUPLICATE",
                "a process identity could not be inserted into executable evidence",
            ));
        }
        state.executable_covered_identities.insert(identity);
        Ok(())
    }

    fn open_observed_process(pid: u32) -> Result<OwnedHandle> {
        // SAFETY: PID came from the private Job Object; access is read-only.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        OwnedHandle::new(
            handle,
            "P1A_PROCESS_IDENTITY_OPEN_FAILED",
            "could not open an observed process identity",
        )
    }

    fn process_identity_handle(handle: HANDLE, pid: u32) -> Result<ProcessIdentity> {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: the handle and all four output buffers are valid.
        if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0
        {
            return Err(last_error(
                "P1A_PROCESS_IDENTITY_QUERY_FAILED",
                "could not bind an observed process creation time",
            ));
        }
        let creation_time_100ns =
            (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        if creation_time_100ns == 0 {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_IDENTITY_INVALID",
                "an observed process has a zero creation time",
            ));
        }
        Ok(ProcessIdentity {
            process_id: pid,
            creation_time_100ns,
        })
    }

    fn initial_debug_boundary(handle: HANDLE) -> Result<InitialDebugBoundary> {
        let mut process_machine = IMAGE_FILE_MACHINE_UNKNOWN;
        let mut native_machine = IMAGE_FILE_MACHINE_UNKNOWN;
        // SAFETY: the process handle is live and both output pointers are writable.
        if unsafe { IsWow64Process2(handle, &mut process_machine, &mut native_machine) } == 0 {
            return Err(last_error(
                "P1A_PROCESS_MACHINE_QUERY_FAILED",
                "could not determine an observed process debug architecture",
            ));
        }
        match (process_machine, native_machine) {
            (IMAGE_FILE_MACHINE_UNKNOWN, IMAGE_FILE_MACHINE_AMD64) => {
                Ok(InitialDebugBoundary::NativePending)
            }
            (IMAGE_FILE_MACHINE_I386, IMAGE_FILE_MACHINE_AMD64) => {
                Ok(InitialDebugBoundary::Wow64NativePending)
            }
            _ => Err(XtaskError::gate(
                "P1A_PROCESS_MACHINE_UNSUPPORTED",
                "an observed process is neither native AMD64 nor x86 under AMD64 WOW64",
                "Use only the selected native AMD64 or x86 WOW64 host tools.",
            )),
        }
    }

    fn classify_path(
        path: &Path,
        command: &DirectCommand,
        path_policy: &ProcessPathPolicy,
    ) -> Result<String> {
        let canonical = path;
        if path_within(canonical, &path_policy.system32) {
            return Ok("windows_system32".to_owned());
        }
        if is_vswhere_command(command) && path_within(canonical, &path_policy.syswow64) {
            return Ok("windows_syswow64".to_owned());
        }
        let program_parent = command.program.parent().ok_or_else(|| {
            XtaskError::integrity("P1A_COMMAND_PROGRAM_INVALID", "program has no parent")
        })?;
        let program_parent = fs::canonicalize(program_parent).io_context(
            "P1A_COMMAND_PROGRAM_INVALID",
            "could not canonicalize the audited program directory",
        )?;
        if path_within(canonical, &program_parent) {
            return Ok("root_tool_directory".to_owned());
        }
        if is_vswhere_command(command)
            && path_policy
                .qualified_files
                .iter()
                .any(|file| windows_path_eq(canonical, &file.canonical_path))
        {
            return Ok("qualified_tool_file".to_owned());
        }
        for root in &command.qualified_persistent_roots {
            let root = fs::canonicalize(root).io_context(
                "P1A_QUALIFIED_ROOT_INVALID",
                "could not canonicalize a qualified process path root",
            )?;
            if path_within(canonical, &root) {
                return Ok("qualified_tool_root".to_owned());
            }
        }
        let work_root = command.capture_directory.parent().ok_or_else(|| {
            XtaskError::integrity(
                "P1A_CAPTURE_DIRECTORY_FAILED",
                "the private command capture directory has no owned work root",
            )
        })?;
        let work_root = fs::canonicalize(work_root).io_context(
            "P1A_CAPTURE_DIRECTORY_FAILED",
            "could not canonicalize the private command work root",
        )?;
        if path_within(canonical, &work_root) {
            return Ok("qualified_working_tree".to_owned());
        }
        let cwd = fs::canonicalize(&command.cwd).io_context(
            "P1A_COMMAND_CWD_INVALID",
            "could not canonicalize the audited working directory",
        )?;
        if path_within(canonical, &cwd) {
            return Ok("qualified_working_tree".to_owned());
        }
        Err(XtaskError::gate(
            "P1A_PROCESS_PATH_CLASS_REJECTED",
            "an observed executable or loaded module is outside every closed qualified root",
            "Remove the unqualified process or module and retry without weakening path classification.",
        ))
    }

    fn is_vswhere_command(command: &DirectCommand) -> bool {
        command
            .display_argv
            .first()
            .is_some_and(|program| program == "${VSWHERE}")
            && command
                .program
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|leaf| leaf.eq_ignore_ascii_case("vswhere.exe"))
    }

    fn process_image(pid: u32) -> Result<PathBuf> {
        let handle = open_observed_process(pid)?;
        process_image_handle(handle.raw())
    }

    fn process_image_handle(handle: HANDLE) -> Result<PathBuf> {
        let mut buffer = vec![0_u16; 32_768];
        let mut length = buffer.len() as u32;
        // SAFETY: buffer and mutable character count are valid for this process handle.
        if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
            return Err(last_error(
                "P1A_PROCESS_IMAGE_QUERY_FAILED",
                "could not resolve an observed process image",
            ));
        }
        if length == 0 || length as usize >= buffer.len() {
            return Err(XtaskError::integrity(
                "P1A_PROCESS_IMAGE_TRUNCATED",
                "an observed process image path was empty or reached the buffer boundary",
            ));
        }
        buffer.truncate(length as usize);
        Ok(PathBuf::from(OsString::from_wide(&buffer)))
    }

    fn path_within(path: &Path, root: &Path) -> bool {
        let mut path_components = path.components();
        for root_component in root.components() {
            let Some(path_component) = path_components.next() else {
                return false;
            };
            if !path_component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&root_component.as_os_str().to_string_lossy())
            {
                return false;
            }
        }
        true
    }

    fn windows_path_eq(left: &Path, right: &Path) -> bool {
        path_within(left, right)
            && path_within(right, left)
            && left.components().count() == right.components().count()
    }

    fn module_paths(pid: u32) -> Result<Vec<PathBuf>> {
        for _ in 0..500 {
            let modules = module_paths_once(pid)?;
            if !modules.is_empty() {
                return Ok(modules);
            }
            // A just-resumed suspended image can report the documented empty terminal
            // boundary before loader initialization. Retry only that non-error state.
            thread::sleep(Duration::from_millis(1));
        }
        Err(XtaskError::integrity(
            "P1A_MODULE_ENUMERATION_EMPTY",
            "an observed process exposed no modules after bounded loader initialization retries",
        ))
    }

    fn module_paths_once(pid: u32) -> Result<Vec<PathBuf>> {
        let mut snapshot = INVALID_HANDLE_VALUE;
        for _ in 0..500 {
            // SAFETY: flags and pid are plain values; returned handle is owned on success.
            snapshot =
                unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
            if snapshot != INVALID_HANDLE_VALUE {
                break;
            }
            // SAFETY: reads thread-local last-error state immediately after the failure.
            let error = unsafe { GetLastError() };
            if !matches!(error, ERROR_BAD_LENGTH | ERROR_PARTIAL_COPY) {
                return Err(last_error(
                    "P1A_MODULE_SNAPSHOT_FAILED",
                    "could not snapshot an observed process module list",
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(last_error(
                "P1A_MODULE_SNAPSHOT_RETRY_EXHAUSTED",
                "module snapshot remained unstable after bounded retries",
            ));
        }
        let snapshot = OwnedHandle(snapshot);
        let mut entry = MODULEENTRY32W {
            dwSize: size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        // SAFETY: entry has the required size and remains live across enumeration.
        if unsafe { Module32FirstW(snapshot.raw(), &mut entry) } == 0 {
            // SAFETY: last-error is read immediately after Module32FirstW.
            let error = unsafe { GetLastError() };
            if error == ERROR_NO_MORE_FILES {
                return Ok(Vec::new());
            }
            return Err(XtaskError::environment(
                "P1A_MODULE_ENUMERATION_START_FAILED",
                format!("could not begin module enumeration (Win32 error {error})"),
            ));
        }
        let mut modules = Vec::new();
        loop {
            let path_length = entry
                .szExePath
                .iter()
                .position(|value| *value == 0)
                .ok_or_else(|| {
                    XtaskError::integrity(
                        "P1A_MODULE_PATH_TRUNCATED",
                        "Toolhelp32 returned a non-terminated module path",
                    )
                })?;
            if path_length >= entry.szExePath.len() - 1 {
                return Err(XtaskError::integrity(
                    "P1A_MODULE_PATH_TRUNCATED",
                    "Toolhelp32 module path reached the fixed buffer boundary",
                ));
            }
            let path = PathBuf::from(OsString::from_wide(&entry.szExePath[..path_length]));
            modules.push(path);
            entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
            // SAFETY: entry and snapshot remain valid.
            if unsafe { Module32NextW(snapshot.raw(), &mut entry) } == 0 {
                // SAFETY: last-error is consumed immediately after Module32NextW.
                let error = unsafe { GetLastError() };
                if error == ERROR_NO_MORE_FILES {
                    break;
                }
                return Err(XtaskError::environment(
                    "P1A_MODULE_ENUMERATION_NEXT_FAILED",
                    format!(
                        "module enumeration failed before its terminal boundary (Win32 error {error})"
                    ),
                ));
            }
        }
        modules.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
        modules.dedup();
        Ok(modules)
    }

    fn job_process_ids(job: HANDLE) -> Result<Vec<u32>> {
        let mut capacity = 32_usize;
        loop {
            let bytes = 8 + capacity * size_of::<usize>();
            let mut buffer = vec![0_u8; bytes];
            let mut returned = 0_u32;
            // SAFETY: buffer is writable for its advertised byte length.
            let ok = unsafe {
                QueryInformationJobObject(
                    job,
                    JobObjectBasicProcessIdList,
                    buffer.as_mut_ptr().cast(),
                    bytes as u32,
                    &mut returned,
                )
            };
            if ok != 0 {
                let count = u32::from_ne_bytes(buffer[4..8].try_into().expect("fixed slice"));
                if count as usize > capacity {
                    capacity = count as usize + 8;
                    continue;
                }
                let mut pids = Vec::with_capacity(count as usize);
                for index in 0..count as usize {
                    let offset = 8 + index * size_of::<usize>();
                    let value = usize::from_ne_bytes(
                        buffer[offset..offset + size_of::<usize>()]
                            .try_into()
                            .expect("fixed slice"),
                    );
                    if value != 0 {
                        pids.push(value as u32);
                    }
                }
                pids.sort_unstable();
                pids.dedup();
                return Ok(pids);
            }
            // SAFETY: last-error is read immediately after the failed query.
            if unsafe { GetLastError() } == ERROR_MORE_DATA {
                capacity *= 2;
                if capacity <= 4096 {
                    continue;
                }
            }
            return Err(last_error(
                "P1A_JOB_QUERY_FAILED",
                "could not enumerate the contained command process tree",
            ));
        }
    }

    fn job_total_processes(job: HANDLE) -> Result<usize> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        let mut returned = 0_u32;
        // SAFETY: output points to a fully sized accounting structure.
        if unsafe {
            QueryInformationJobObject(
                job,
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                &mut returned,
            )
        } == 0
        {
            return Err(last_error(
                "P1A_JOB_ACCOUNTING_FAILED",
                "could not read total process accounting for the contained command tree",
            ));
        }
        if returned as usize != size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() {
            return Err(XtaskError::integrity(
                "P1A_JOB_ACCOUNTING_SIZE_INVALID",
                "the Job Object returned an unexpected accounting structure size",
            ));
        }
        Ok(accounting.TotalProcesses as usize)
    }

    fn qualified_survivors(
        remaining: &[u32],
        state: &AuditState,
        command: &DirectCommand,
    ) -> Result<bool> {
        if remaining.is_empty() || command.qualified_persistent_roots.is_empty() {
            return Ok(false);
        }
        for pid in remaining {
            let process = open_observed_process(*pid)?;
            let identity = process_identity_handle(process.raw(), *pid)?;
            let path = process_image_handle(process.raw())?;
            let leaf = path.file_name().and_then(OsStr::to_str).ok_or_else(|| {
                XtaskError::integrity(
                    "P1A_SURVIVOR_IMAGE_NON_UTF8",
                    "a persistent qualified-tool image name is not valid UTF-8",
                )
            })?;
            let canonical_path = fs::canonicalize(&path).io_context(
                "P1A_SURVIVOR_CANONICALIZE_FAILED",
                "could not canonicalize a persistent qualified tool",
            )?;
            let mut within_root = false;
            for root in &command.qualified_persistent_roots {
                let root = fs::canonicalize(root).io_context(
                    "P1A_SURVIVOR_ROOT_CANONICALIZE_FAILED",
                    "could not canonicalize a qualified persistent-tool root",
                )?;
                within_root |= path_within(&canonical_path, &root);
            }
            if !leaf.eq_ignore_ascii_case("vctip.exe")
                || !within_root
                || !state.executable_covered_identities.contains(&identity)
                || !state.snapshot_covered_identities.contains(&identity)
            {
                return Ok(false);
            }
        }
        Ok(state.forbidden_processes.is_empty() && state.forbidden_modules.is_empty())
    }

    fn forbidden_process(leaf: &str, policy: ProcessPolicy) -> bool {
        let lower = leaf.to_ascii_lowercase();
        let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
        super::forbidden_python_process_name(leaf)
            || matches!(
                stem,
                "cmd"
                    | "powershell"
                    | "pwsh"
                    | "wsl"
                    | "wslhost"
                    | "wslservice"
                    | "bash"
                    | "sh"
                    | "dash"
                    | "zsh"
                    | "fish"
                    | "cc"
                    | "c++"
                    | "gcc"
                    | "g++"
                    | "clang"
                    | "clang-cl"
                    | "hipcc"
                    | "hipconfig"
                    | "clang-offload-bundler"
                    | "rocminfo"
            )
            || (policy == ProcessPolicy::HostOnly
                && matches!(stem, "nvcc" | "ptxas" | "nvlink" | "fatbinary" | "cudafe++"))
    }

    fn forbidden_module(leaf: &str, policy: ProcessPolicy) -> bool {
        let lower = leaf.to_ascii_lowercase();
        ((lower.starts_with("python") || lower.starts_with("pypy")) && lower.ends_with(".dll"))
            || (policy == ProcessPolicy::HostOnly
                && ["nvcuda", "cudart", "cublas"]
                    .iter()
                    .any(|token| lower.contains(token)))
            || [
                "cudnn",
                "nvrtc",
                "nvml",
                "cupti",
                "nccl",
                "amdhip",
                "amd_comgr",
                "hsa-runtime",
                "hiprtc",
                "rocblas",
                "miopen",
                "torch",
                "libtorch",
                "c10",
                "tensorflow",
                "onnxruntime",
                "metal",
                "mpsgraph",
            ]
            .iter()
            .any(|token| lower.contains(token))
    }

    fn cuda_content_marker_allowed(policy: ProcessPolicy, marker: &str) -> bool {
        policy == ProcessPolicy::CudaProbe
            && (marker == "pe-section:nvidia-fatbin"
                || marker == "pe-symbol:cuda-driver-api"
                || marker == "pe-symbol:cuda-runtime-api"
                || marker == "pe-symbol:cublas-api"
                || marker
                    .strip_prefix("pe-import-library:")
                    .is_some_and(|library| {
                        let lower = library.to_ascii_lowercase();
                        ["nvcuda", "cudart", "cublas"]
                            .iter()
                            .any(|token| lower.contains(token))
                    }))
    }

    fn environment_block(overrides: &BTreeMap<String, Option<OsString>>) -> Result<Vec<u16>> {
        let mut environment = BTreeMap::<String, (OsString, OsString)>::new();
        for (key, value) in std::env::vars_os() {
            let key_text = key.to_string_lossy().to_ascii_uppercase();
            // Windows exposes per-drive current-directory pseudo-variables whose names
            // begin with '='. CreateProcess rebuilds those itself; they are not ordinary
            // environment keys and cannot be represented as key=value entries here.
            if key_text.starts_with('=') || !inherited_environment_key_allowed(&key_text) {
                continue;
            }
            environment.insert(key_text, (key, value));
        }
        for (key, value) in overrides {
            let normalized = key.to_ascii_uppercase();
            if normalized != *key || !override_environment_key_allowed(&normalized) {
                return Err(XtaskError::integrity(
                    "P1A_COMMAND_ENVIRONMENT_OVERRIDE_FORBIDDEN",
                    format!(
                        "child environment override {key:?} is outside the closed P1A allowlist"
                    ),
                ));
            }
            if let Some(value) = value {
                environment.insert(normalized, (OsString::from(key), value.clone()));
            } else {
                environment.remove(&normalized);
            }
        }
        let mut block = Vec::new();
        for (_, (key, value)) in environment {
            let key_text = key.to_string_lossy();
            if key_text.is_empty()
                || key_text.contains('=')
                || value.to_string_lossy().contains('\0')
            {
                return Err(XtaskError::integrity(
                    "P1A_COMMAND_ENVIRONMENT_INVALID",
                    "child environment contains an invalid key or value",
                ));
            }
            block.extend(
                OsStr::new(&format!("{}={}", key_text, value.to_string_lossy())).encode_wide(),
            );
            block.push(0);
        }
        block.push(0);
        Ok(block)
    }

    fn inherited_environment_key_allowed(key: &str) -> bool {
        matches!(
            key,
            "SYSTEMROOT"
                | "WINDIR"
                | "SYSTEMDRIVE"
                | "PATH"
                | "PATHEXT"
                | "TEMP"
                | "TMP"
                | "USERPROFILE"
                | "HOME"
                | "HOMEDRIVE"
                | "HOMEPATH"
                | "LOCALAPPDATA"
                | "APPDATA"
                | "PROGRAMDATA"
                | "PROGRAMFILES"
                | "PROGRAMFILES(X86)"
                | "PROGRAMW6432"
                | "COMMONPROGRAMFILES"
                | "COMMONPROGRAMFILES(X86)"
                | "COMMONPROGRAMW6432"
                | "NUMBER_OF_PROCESSORS"
                | "PROCESSOR_ARCHITECTURE"
                | "PROCESSOR_IDENTIFIER"
                | "PROCESSOR_LEVEL"
                | "PROCESSOR_REVISION"
                | "OS"
                | "CARGO_HOME"
                | "RUSTUP_HOME"
                | "RUSTUP_TOOLCHAIN"
                | "RUSTC"
                | "RUSTDOC"
                | "CARGO"
                | "CARGO_MANIFEST_DIR"
                | "CARGO_MANIFEST_PATH"
                | "CARGO_PRIMARY_PACKAGE"
                | "CARGO_CRATE_NAME"
                | "CARGO_BIN_NAME"
                | "CARGO_TARGET_TMPDIR"
                | "HOST"
                | "TARGET"
                | "PROFILE"
                | "OPT_LEVEL"
                | "DEBUG"
                | "OUT_DIR"
                | "NUM_JOBS"
                | "RUST_RECURSION_COUNT"
                | "GIT_EXEC_PATH"
                | "GIT_TEMPLATE_DIR"
        )
    }

    fn override_environment_key_allowed(key: &str) -> bool {
        if cfg!(test)
            && matches!(
                key,
                "P1A_AUDITED_CHILD_HELPER"
                    | "P1A_TEST_FORBIDDEN_HANDLE"
                    | "P1A_TEST_FORBIDDEN_FILE_ID"
            )
        {
            return true;
        }
        inherited_environment_key_allowed(key)
            || matches!(
                key,
                "INCLUDE"
                    | "LIB"
                    | "LIBPATH"
                    | "CARGO_NET_OFFLINE"
                    | "CARGO_TARGET_DIR"
                    | "RUSTC_WRAPPER"
                    | "RUSTC_WORKSPACE_WRAPPER"
                    | "RUSTFLAGS"
                    | "CARGO_ENCODED_RUSTFLAGS"
                    | "CC"
                    | "CXX"
                    | "AR"
                    | "CFLAGS"
                    | "CXXFLAGS"
                    | "LDFLAGS"
                    | "CARGO_BUILD_TARGET"
                    | "CARGO_BUILD_RUSTC_WRAPPER"
                    | "CUDA_PATH"
                    | "CUDA_HOME"
                    | "CUDA_VISIBLE_DEVICES"
                    | "CUDA_CACHE_PATH"
                    | "CUDA_CACHE_DISABLE"
                    | "CUDNN_PATH"
                    | "HIP_PATH"
                    | "ROCM_PATH"
                    | "LIBTORCH"
                    | "PYTHONHOME"
                    | "PYTHONPATH"
                    | "GIT_OPTIONAL_LOCKS"
                    | "GIT_CONFIG_NOSYSTEM"
                    | "GIT_CONFIG_GLOBAL"
                    | "GIT_CONFIG_SYSTEM"
                    | "GIT_CONFIG_COUNT"
                    | "GIT_CONFIG_KEY_0"
                    | "GIT_CONFIG_VALUE_0"
                    | "GIT_CONFIG_KEY_1"
                    | "GIT_CONFIG_VALUE_1"
                    | "GIT_CONFIG_KEY_2"
                    | "GIT_CONFIG_VALUE_2"
                    | "GIT_CONFIG_KEY_3"
                    | "GIT_CONFIG_VALUE_3"
                    | "GIT_NO_REPLACE_OBJECTS"
                    | "GIT_NO_LAZY_FETCH"
            )
    }

    fn build_command_line(args: &[OsString]) -> OsString {
        // The executable path is supplied separately through lpApplicationName. Keeping
        // argv[0] a constant non-path label avoids leaking the host installation path to
        // child diagnostics while preserving Windows CRT argument parsing semantics.
        let mut line = OsString::from("p1a-qualified-program");
        for arg in args {
            line.push(" ");
            line.push(quote_windows(arg));
        }
        line
    }

    fn quote_windows(value: &OsStr) -> OsString {
        let text = value.to_string_lossy();
        if !text.is_empty() && !text.chars().any(|c| c.is_whitespace() || c == '"') {
            return OsString::from(text.as_ref());
        }
        let mut output = String::from("\"");
        let mut slashes = 0;
        for character in text.chars() {
            if character == '\\' {
                slashes += 1;
            } else if character == '"' {
                output.push_str(&"\\".repeat(slashes * 2 + 1));
                output.push('"');
                slashes = 0;
            } else {
                output.push_str(&"\\".repeat(slashes));
                slashes = 0;
                output.push(character);
            }
        }
        output.push_str(&"\\".repeat(slashes * 2));
        output.push('"');
        OsString::from(output)
    }

    fn wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn read_bounded(path: &Path) -> Result<Vec<u8>> {
        let metadata = fs::metadata(path).io_context(
            "P1A_CAPTURE_READ_FAILED",
            "could not inspect command capture",
        )?;
        if metadata.len() > MAX_CAPTURE_BYTES {
            return Err(XtaskError::integrity(
                "P1A_COMMAND_OUTPUT_LIMIT_EXCEEDED",
                "audited command output exceeded the fixed 16 MiB capture limit",
            ));
        }
        let mut file = File::open(path)
            .io_context("P1A_CAPTURE_READ_FAILED", "could not open command capture")?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.read_to_end(&mut bytes)
            .io_context("P1A_CAPTURE_READ_FAILED", "could not read command capture")?;
        Ok(bytes)
    }

    fn last_error(code: &'static str, context: &str) -> XtaskError {
        // SAFETY: reads the calling thread's last-error value.
        let value = unsafe { GetLastError() };
        XtaskError::environment(code, format!("{context} (Win32 error {value})"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::Foundation::GetHandleInformation;

        #[test]
        fn windows_quoting_handles_spaces_quotes_and_trailing_slashes() {
            assert_eq!(quote_windows(OsStr::new("plain")), "plain");
            assert_eq!(quote_windows(OsStr::new("two words")), "\"two words\"");
            assert_eq!(quote_windows(OsStr::new("a\"b")), "\"a\\\"b\"");
            assert_eq!(quote_windows(OsStr::new("a b\\")), "\"a b\\\\\"");
        }

        #[test]
        fn sensitive_process_and_module_names_are_closed() {
            for leaf in [
                "python.exe",
                "python3.13.exe",
                "pypy310.exe",
                "pip3.13.exe",
                "powershell.exe",
                "pwsh.exe",
                "cmd.exe",
                "wsl.exe",
                "bash.exe",
            ] {
                assert!(forbidden_process(leaf, ProcessPolicy::HostOnly));
                assert!(forbidden_process(leaf, ProcessPolicy::CudaProbe));
            }
            assert!(forbidden_process("nvcc.exe", ProcessPolicy::HostOnly));
            assert!(!forbidden_process("nvcc.exe", ProcessPolicy::CudaProbe));
            assert!(!forbidden_process("ptxas.exe", ProcessPolicy::CudaProbe));
            assert!(!forbidden_process("nvlink.exe", ProcessPolicy::CudaProbe));
            assert!(!forbidden_process("rustc.exe", ProcessPolicy::HostOnly));
            assert!(!forbidden_process("cl.exe", ProcessPolicy::HostOnly));
            assert!(forbidden_module("python313.dll", ProcessPolicy::CudaProbe));
            assert!(forbidden_module("pypy3.10.dll", ProcessPolicy::CudaProbe));
            assert!(forbidden_module("cublas64_13.dll", ProcessPolicy::HostOnly));
            assert!(!forbidden_module(
                "cublas64_13.dll",
                ProcessPolicy::CudaProbe
            ));
            assert!(forbidden_module(
                "onnxruntime_providers_cuda.dll",
                ProcessPolicy::CudaProbe
            ));
            assert!(!forbidden_module(
                "tree-sitter-python.dll",
                ProcessPolicy::HostOnly
            ));
            assert!(cuda_content_marker_allowed(
                ProcessPolicy::CudaProbe,
                "pe-section:nvidia-fatbin"
            ));
            assert!(!cuda_content_marker_allowed(
                ProcessPolicy::HostOnly,
                "pe-section:nvidia-fatbin"
            ));
        }

        #[test]
        fn current_test_image_has_no_forbidden_runtime_content() {
            let bound = bind_file(&std::env::current_exe().unwrap(), "P1A_TEST_IMAGE").unwrap();
            let identity = complete_file_identity(&bound).unwrap();
            assert!(
                identity.forbidden_content_markers.is_empty(),
                "unexpected markers: {:?}",
                identity.forbidden_content_markers
            );
        }

        #[test]
        fn child_environment_policy_is_closed() {
            assert!(inherited_environment_key_allowed("SYSTEMROOT"));
            assert!(inherited_environment_key_allowed("RUSTUP_HOME"));
            assert!(!inherited_environment_key_allowed("CARGO_PKG_NAME"));
            assert!(!inherited_environment_key_allowed(
                "CARGO_PKG_UNREVIEWED_METADATA"
            ));
            assert!(!inherited_environment_key_allowed("COMSPEC"));
            assert!(!inherited_environment_key_allowed("PYTHONPATH"));
            assert!(override_environment_key_allowed("PYTHONPATH"));
            assert!(override_environment_key_allowed("GIT_NO_REPLACE_OBJECTS"));
            assert!(override_environment_key_allowed("GIT_NO_LAZY_FETCH"));
            assert!(override_environment_key_allowed("RUSTC"));
            assert!(override_environment_key_allowed("RUSTDOC"));
            assert!(override_environment_key_allowed("RUSTUP_TOOLCHAIN"));
            assert!(!override_environment_key_allowed("UNREVIEWED_BUILD_FLAG"));
            let cuda_environment = BTreeMap::from([(
                "CUDA_VISIBLE_DEVICES".to_owned(),
                Some(OsString::from("GPU-00000000-0000-0000-0000-000000000001")),
            )]);
            assert_eq!(
                validate_policy_environment(ProcessPolicy::HostOnly, &cuda_environment)
                    .unwrap_err()
                    .code,
                "P1A_COMMAND_ENVIRONMENT_OVERRIDE_FORBIDDEN"
            );
            validate_policy_environment(ProcessPolicy::CudaProbe, &cuda_environment).unwrap();

            let removals = [
                "GIT_NO_REPLACE_OBJECTS",
                "GIT_NO_LAZY_FETCH",
                "RUSTC",
                "RUSTDOC",
                "RUSTUP_TOOLCHAIN",
            ]
            .into_iter()
            .map(|name| (name.to_owned(), None))
            .collect();
            environment_block(&removals).expect("closed variables must permit explicit removal");
        }

        #[test]
        fn debug_event_handles_are_closed_by_their_raii_owners() {
            let duplicate = || {
                let mut handle = null_mut();
                // SAFETY: the current process pseudo-handle is always valid, and the
                // output slot receives one new non-inheritable handle owned by this test.
                assert_ne!(
                    unsafe {
                        DuplicateHandle(
                            GetCurrentProcess(),
                            GetCurrentProcess(),
                            GetCurrentProcess(),
                            &mut handle,
                            0,
                            0,
                            DUPLICATE_SAME_ACCESS,
                        )
                    },
                    0
                );
                handle
            };
            let process_raw = duplicate();
            let thread_raw = duplicate();
            let process = OwnedHandle::new(process_raw, "TEST", "test process handle").unwrap();
            let thread = OwnedHandle::new(thread_raw, "TEST", "test thread handle").unwrap();

            drop(thread);
            let mut flags = 0;
            // SAFETY: querying a stale numeric handle is the intended closure probe.
            assert_eq!(unsafe { GetHandleInformation(thread_raw, &mut flags) }, 0);
            // SAFETY: the process handle remains live under its distinct owner.
            assert_ne!(
                unsafe { GetHandleInformation(process.raw(), &mut flags) },
                0
            );

            drop(process);
            // SAFETY: querying a stale numeric handle is the intended closure probe.
            assert_eq!(unsafe { GetHandleInformation(process_raw, &mut flags) }, 0);
        }

        #[test]
        fn initial_debug_boundaries_distinguish_native_and_wow64_loaders() {
            let mut native = InitialDebugBoundary::NativePending;
            assert_eq!(
                initial_boundary_action(&mut native, 1, EXCEPTION_BREAKPOINT),
                InitialBoundaryAction::Sample
            );
            assert_eq!(native, InitialDebugBoundary::Complete);

            let mut wow64 = InitialDebugBoundary::Wow64NativePending;
            assert_eq!(
                initial_boundary_action(&mut wow64, 1, EXCEPTION_BREAKPOINT),
                InitialBoundaryAction::Continue
            );
            assert_eq!(wow64, InitialDebugBoundary::Wow64Pending);
            assert_eq!(
                initial_boundary_action(&mut wow64, 1, STATUS_WX86_BREAKPOINT),
                InitialBoundaryAction::Sample
            );
            assert_eq!(wow64, InitialDebugBoundary::Complete);

            let mut second_chance = InitialDebugBoundary::NativePending;
            assert_eq!(
                initial_boundary_action(&mut second_chance, 0, EXCEPTION_BREAKPOINT),
                InitialBoundaryAction::NotHandled
            );
            assert_eq!(second_chance, InitialDebugBoundary::NativePending);
        }

        #[test]
        fn audited_grandchild_helper() {
            if std::env::var_os("P1A_AUDITED_GRANDCHILD_HELPER").is_some() {
                println!("P1A_AUDITED_GRANDCHILD_PASS");
            }
        }

        #[test]
        fn audited_child_helper() {
            if std::env::var_os("P1A_AUDITED_CHILD_HELPER").is_some() {
                if let Some(raw) = std::env::var_os("P1A_TEST_FORBIDDEN_HANDLE") {
                    let handle = raw.to_string_lossy().parse::<usize>().unwrap() as HANDLE;
                    let expected = std::env::var("P1A_TEST_FORBIDDEN_FILE_ID").unwrap();
                    let mut actual = BY_HANDLE_FILE_INFORMATION::default();
                    // SAFETY: this intentionally probes an untrusted inherited handle
                    // value. Success is acceptable only when Windows reused the numeric
                    // slot for a different kernel file object.
                    if unsafe { GetFileInformationByHandle(handle, &mut actual) } != 0 {
                        let actual_index = (u64::from(actual.nFileIndexHigh) << 32)
                            | u64::from(actual.nFileIndexLow);
                        assert_ne!(
                            format!("{}:{actual_index}", actual.dwVolumeSerialNumber),
                            expected,
                            "the non-allowlisted parent file object was inherited"
                        );
                    }
                }
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .arg("--exact")
                    .arg("p1a_process::windows::tests::audited_grandchild_helper")
                    .arg("--nocapture")
                    .env("P1A_AUDITED_GRANDCHILD_HELPER", "1")
                    .spawn()
                    .unwrap();
                assert!(child.wait().unwrap().success());
                println!("P1A_AUDITED_CHILD_PASS");
            }
        }

        #[test]
        fn suspended_job_runner_audits_and_drains_root_process() {
            let temp = tempfile::tempdir().unwrap();
            let mut environment = BTreeMap::new();
            environment.insert(
                "P1A_AUDITED_CHILD_HELPER".to_owned(),
                Some(OsString::from("1")),
            );
            let forbidden_path = temp.path().join("forbidden-handle.bin");
            let forbidden_file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&forbidden_path)
                .unwrap();
            let forbidden_handle = forbidden_file.as_raw_handle() as HANDLE;
            let (forbidden_volume, forbidden_index) = file_id(&forbidden_file).unwrap();
            use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
            // SAFETY: this test-owned file handle remains live through the child run.
            assert_ne!(
                unsafe {
                    SetHandleInformation(forbidden_handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)
                },
                0
            );
            environment.insert(
                "P1A_TEST_FORBIDDEN_HANDLE".to_owned(),
                Some(OsString::from((forbidden_handle as usize).to_string())),
            );
            environment.insert(
                "P1A_TEST_FORBIDDEN_FILE_ID".to_owned(),
                Some(OsString::from(format!(
                    "{forbidden_volume}:{forbidden_index}"
                ))),
            );
            let make_command = |run_index: usize| DirectCommand {
                policy: ProcessPolicy::HostOnly,
                program: std::env::current_exe().unwrap(),
                args: vec![
                    OsString::from("--exact"),
                    OsString::from("p1a_process::windows::tests::audited_child_helper"),
                    OsString::from("--nocapture"),
                ],
                display_argv: vec![
                    "${TEST_BINARY}".to_owned(),
                    "--exact".to_owned(),
                    "p1a_process::windows::tests::audited_child_helper".to_owned(),
                    "--nocapture".to_owned(),
                ],
                cwd: temp.path().to_path_buf(),
                environment: environment.clone(),
                timeout: Duration::from_secs(30),
                capture_directory: temp.path().join("captures"),
                capture_stem: format!("child-{run_index}"),
                qualified_persistent_roots: Vec::new(),
                qualified_persistent_files: Vec::new(),
            };
            let mut outputs = Vec::new();
            for run_index in 0..20 {
                outputs.push(run(&make_command(run_index)).unwrap_or_else(|error| {
                    panic!("short-lived descendant audit run {run_index} failed: {error}")
                }));
            }
            let output = outputs.pop().unwrap();
            for output in &outputs {
                assert!(
                    output.audit.audited_process_count >= 2,
                    "expected root plus short-lived descendant: {:?}",
                    output.audit.executable_names
                );
                assert_eq!(
                    output.audit.covered_process_count,
                    output.audit.audited_process_count
                );
                assert_eq!(output.audit.exit_races, 0);
            }
            assert_eq!(
                output.exit_code,
                0,
                "stderr={}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8(output.stdout)
                    .unwrap()
                    .contains("P1A_AUDITED_CHILD_PASS")
            );
            assert!(output.stderr.is_empty());
            assert!(output.audit.atomic_job_assignment);
            assert!(output.audit.process_tree_terminated);
            assert!(!output.audit.unexpected_descendants);
            assert!(output.audit.audited_process_count >= 2);
            assert_eq!(
                output.audit.covered_process_count,
                output.audit.audited_process_count
            );
            assert_eq!(output.audit.exit_races, 0);
            assert!(output.audit.successful_snapshots >= 1);
            assert!(
                output.audit.forbidden_processes.is_empty(),
                "unexpected forbidden processes: {:?}",
                output.audit.forbidden_processes
            );
            assert!(
                output.audit.forbidden_modules.is_empty(),
                "unexpected forbidden modules: {:?}",
                output.audit.forbidden_modules
            );
            assert_eq!(
                output.audit.process_identities.len(),
                output.audit.audited_process_count
            );
            assert!(!output.audit.loaded_modules.is_empty());
            let process_keys = output
                .audit
                .process_identities
                .iter()
                .map(|record| (record.process_id, record.creation_time_100ns))
                .collect::<BTreeSet<_>>();
            let module_keys = output
                .audit
                .loaded_modules
                .iter()
                .map(|record| (record.process_id, record.creation_time_100ns))
                .collect::<BTreeSet<_>>();
            assert_eq!(process_keys, module_keys);
            assert!(output.audit.process_identities.iter().all(|record| {
                record.creation_time_100ns > 0
                    && record.executable_bytes > 0
                    && crate::hash::is_lower_sha256(&record.canonical_path_sha256)
                    && crate::hash::is_lower_sha256(&record.executable_sha256)
                    && matches!(
                        record.path_class.as_str(),
                        "windows_system32"
                            | "root_tool_directory"
                            | "qualified_tool_root"
                            | "qualified_working_tree"
                    )
            }));
            assert!(output.audit.loaded_modules.iter().all(|record| {
                record.creation_time_100ns > 0
                    && record.module_bytes > 0
                    && crate::hash::is_lower_sha256(&record.canonical_path_sha256)
                    && crate::hash::is_lower_sha256(&record.module_sha256)
            }));
            for process in &output.audit.process_identities {
                assert!(output.audit.loaded_modules.iter().any(|module| {
                    module.process_id == process.process_id
                        && module.creation_time_100ns == process.creation_time_100ns
                        && module.canonical_path_sha256 == process.canonical_path_sha256
                        && module.module_sha256 == process.executable_sha256
                        && module.module_bytes == process.executable_bytes
                }));
            }
        }

        #[test]
        fn audited_real_vswhere_closes_wow64_runtime_without_conhost() {
            let Ok(program) = crate::p1a_windows::discover_vswhere_path() else {
                return;
            };
            let runtime_binding = crate::p1a_windows::bind_vswhere_runtime()
                .expect("installed vswhere must expose its locked native runtime binding");
            let runtime_identity = runtime_binding.setup_configuration_identity();
            let setup_runtime = QualifiedPersistentFile {
                path: runtime_identity.path.clone(),
                sha256: runtime_identity.sha256.clone(),
                bytes: runtime_identity.bytes,
            };
            let temp = tempfile::tempdir().unwrap();
            let output = run(&DirectCommand {
                policy: ProcessPolicy::HostOnly,
                program,
                args: crate::p1a_windows::VSWHERE_ARGS
                    .iter()
                    .map(OsString::from)
                    .collect(),
                display_argv: std::iter::once("${VSWHERE}".to_owned())
                    .chain(
                        crate::p1a_windows::VSWHERE_ARGS
                            .iter()
                            .map(|value| (*value).to_owned()),
                    )
                    .collect(),
                cwd: temp.path().to_path_buf(),
                environment: BTreeMap::new(),
                timeout: Duration::from_secs(10),
                capture_directory: temp.path().join("captures"),
                capture_stem: "vswhere-runtime".to_owned(),
                qualified_persistent_roots: Vec::new(),
                qualified_persistent_files: vec![setup_runtime],
            })
            .unwrap();
            assert_eq!(output.exit_code, 0);
            assert!(output.audit.process_tree_terminated);
            assert_eq!(output.audit.audited_process_count, 1);
            assert_eq!(output.audit.covered_process_count, 1);
            assert_eq!(output.audit.successful_snapshots, 1);
            assert_eq!(output.audit.executable_names, ["vswhere.exe"]);
            assert!(
                !output
                    .audit
                    .executable_names
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case("conhost.exe"))
            );
            assert!(
                output
                    .audit
                    .loaded_modules
                    .iter()
                    .any(|module| { module.path_class == "windows_syswow64" })
            );
            assert_eq!(
                output
                    .audit
                    .loaded_modules
                    .iter()
                    .filter(|module| module.path_class == "qualified_tool_file")
                    .count(),
                1
            );
            assert!(output.audit.loaded_modules.iter().all(|module| {
                matches!(
                    module.path_class.as_str(),
                    "windows_system32"
                        | "windows_syswow64"
                        | "root_tool_directory"
                        | "qualified_tool_file"
                )
            }));
            let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(parsed.is_array());
            assert!(output.stderr.is_empty());
        }

        #[test]
        fn path_classification_is_component_aware_and_case_insensitive() {
            assert!(path_within(
                Path::new(r"C:\Program Files\Git\cmd\git.exe"),
                Path::new(r"c:\program files\git")
            ));
            assert!(!path_within(
                Path::new(r"C:\Program Files\Github\git.exe"),
                Path::new(r"C:\Program Files\Git")
            ));
            assert!(windows_path_eq(
                Path::new(r"C:\Windows\System32\kernel32.dll"),
                Path::new(r"c:\WINDOWS\system32\KERNEL32.DLL")
            ));
        }

        #[test]
        fn syswow64_and_exact_file_classes_are_vswhere_only() {
            let temp = tempfile::tempdir().unwrap();
            let cwd = temp.path().join("repository");
            let captures = temp.path().join("private-work").join("captures");
            let program = temp.path().join("program").join("vswhere.exe");
            let exact = temp
                .path()
                .join("qualified")
                .join("Microsoft.VisualStudio.Setup.Configuration.Native.dll");
            fs::create_dir_all(&cwd).unwrap();
            fs::create_dir_all(&captures).unwrap();
            fs::create_dir_all(program.parent().unwrap()).unwrap();
            fs::create_dir_all(exact.parent().unwrap()).unwrap();
            let current_image = std::env::current_exe().unwrap();
            fs::copy(&current_image, &program).unwrap();
            fs::copy(&current_image, &exact).unwrap();
            let exact_bound = bind_file(&exact, "P1A_TEST_QUALIFIED_FILE").unwrap();
            let exact_identity = complete_file_identity(&exact_bound).unwrap();
            let command = DirectCommand {
                policy: ProcessPolicy::HostOnly,
                program,
                args: Vec::new(),
                display_argv: vec!["${VSWHERE}".to_owned()],
                cwd,
                environment: BTreeMap::new(),
                timeout: Duration::from_secs(1),
                capture_directory: captures,
                capture_stem: "classification".to_owned(),
                qualified_persistent_roots: Vec::new(),
                qualified_persistent_files: vec![QualifiedPersistentFile {
                    path: exact.clone(),
                    sha256: exact_identity.sha256,
                    bytes: exact_identity.bytes,
                }],
            };
            validate_spec(&command).unwrap();
            let mut missing_exact_file = command.clone();
            missing_exact_file.qualified_persistent_files.clear();
            assert_eq!(
                validate_spec(&missing_exact_file).unwrap_err().code,
                "P1A_QUALIFIED_FILE_COUNT_INVALID"
            );
            assert_eq!(
                bind_qualified_persistent_files(&missing_exact_file)
                    .err()
                    .expect("vswhere without its exact runtime file must fail")
                    .code,
                "P1A_QUALIFIED_FILE_COUNT_INVALID"
            );
            let mut duplicate_exact_file = command.clone();
            duplicate_exact_file
                .qualified_persistent_files
                .push(duplicate_exact_file.qualified_persistent_files[0].clone());
            assert_eq!(
                validate_spec(&duplicate_exact_file).unwrap_err().code,
                "P1A_QUALIFIED_FILE_COUNT_INVALID"
            );
            assert_eq!(
                bind_qualified_persistent_files(&duplicate_exact_file)
                    .err()
                    .expect("vswhere with duplicate exact runtime files must fail")
                    .code,
                "P1A_QUALIFIED_FILE_COUNT_INVALID"
            );
            let path_policy = bind_process_path_policy(&command).unwrap();
            let mut mismatched_identity = command.clone();
            mismatched_identity.qualified_persistent_files[0].sha256 = "0".repeat(64);
            assert_eq!(
                bind_qualified_persistent_files(&mismatched_identity)
                    .err()
                    .expect("mismatched exact-file identity must fail")
                    .code,
                "P1A_QUALIFIED_FILE_IDENTITY_MISMATCH"
            );
            assert_eq!(
                classify_path(&fs::canonicalize(&exact).unwrap(), &command, &path_policy,).unwrap(),
                "qualified_tool_file"
            );
            let wow64_ntdll =
                fs::canonicalize(system_wow64_directory().unwrap().join("ntdll.dll")).unwrap();
            assert_eq!(
                classify_path(&wow64_ntdll, &command, &path_policy).unwrap(),
                "windows_syswow64"
            );
            assert_eq!(
                require_qualified_files_observed(&path_policy.qualified_files, &BTreeSet::new())
                    .unwrap_err()
                    .code,
                "P1A_QUALIFIED_FILE_NOT_OBSERVED"
            );
            assert_eq!(
                require_vswhere_syswow64_observed(&command, &BTreeSet::new())
                    .unwrap_err()
                    .code,
                "P1A_VSWHERE_SYSWOW64_OBSERVATION_INVALID"
            );

            let mut non_vswhere = command.clone();
            non_vswhere.display_argv = vec!["${TEST_BINARY}".to_owned()];
            assert_eq!(
                bind_qualified_persistent_files(&non_vswhere)
                    .err()
                    .expect("non-vswhere exact file must fail")
                    .code,
                "P1A_QUALIFIED_FILE_COUNT_INVALID"
            );
            non_vswhere.qualified_persistent_files.clear();
            let non_vswhere_policy = bind_process_path_policy(&non_vswhere).unwrap();
            assert_eq!(
                classify_path(&wow64_ntdll, &non_vswhere, &non_vswhere_policy)
                    .unwrap_err()
                    .code,
                "P1A_PROCESS_PATH_CLASS_REJECTED"
            );
        }

        #[test]
        fn path_classification_includes_the_exact_private_work_root() {
            let temp = tempfile::tempdir().unwrap();
            let cwd = temp.path().join("repository");
            let work_root = temp.path().join("private-work");
            let captures = work_root.join("captures");
            let generated = work_root.join("target/debug/build-script.exe");
            fs::create_dir_all(&cwd).unwrap();
            fs::create_dir_all(&captures).unwrap();
            fs::create_dir_all(generated.parent().unwrap()).unwrap();
            fs::write(&generated, b"test-only").unwrap();
            let command = DirectCommand {
                policy: ProcessPolicy::HostOnly,
                program: std::env::current_exe().unwrap(),
                args: Vec::new(),
                display_argv: vec!["${TEST_BINARY}".to_owned()],
                cwd,
                environment: BTreeMap::new(),
                timeout: Duration::from_secs(1),
                capture_directory: captures,
                capture_stem: "test".to_owned(),
                qualified_persistent_roots: Vec::new(),
                qualified_persistent_files: Vec::new(),
            };
            let path_policy = bind_process_path_policy(&command).unwrap();
            assert_eq!(
                classify_path(
                    &fs::canonicalize(generated).unwrap(),
                    &command,
                    &path_policy,
                )
                .unwrap(),
                "qualified_working_tree"
            );
        }

        #[test]
        fn canonical_path_hash_is_stable_and_case_preserving() {
            let first = canonical_path_hash(Path::new(r"C:\Tools\rustc.exe"));
            let second = canonical_path_hash(Path::new(r"C:\Tools\rustc.exe"));
            let different_case = canonical_path_hash(Path::new(r"c:\tools\rustc.exe"));
            assert_eq!(first, second);
            assert_ne!(first, different_case);
            assert!(crate::hash::is_lower_sha256(&first));
        }
    }
}

#[cfg(windows)]
pub(crate) use windows::run;
