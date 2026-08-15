use crate::error::{Category, IoContext, Result, XtaskError};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::{Child, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CAPTURE_LIMIT_BYTES: u64 = 2 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, Eq, PartialEq)]
struct GateCommand {
    id: &'static str,
    args: &'static [&'static str],
}

const GATE_COMMANDS: &[GateCommand] = &[
    GateCommand {
        id: "Q001",
        args: &["fmt", "--all", "--", "--check"],
    },
    GateCommand {
        id: "Q002",
        args: &[
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--offline",
            "--",
            "-D",
            "warnings",
        ],
    },
    GateCommand {
        id: "Q003",
        args: &[
            "test",
            "--locked",
            "--features",
            "cpu-reference",
            "--offline",
        ],
    },
    GateCommand {
        id: "Q004",
        args: &["test", "--locked", "-p", "xtask", "--offline"],
    },
    GateCommand {
        id: "Q005",
        args: &["check", "--locked", "--no-default-features", "--offline"],
    },
    GateCommand {
        id: "Q006",
        args: &[
            "test",
            "--locked",
            "-p",
            "xtask",
            "--features",
            "p2-cuda",
            "--no-run",
            "--offline",
        ],
    },
    GateCommand {
        id: "Q007",
        args: &[
            "check",
            "--locked",
            "--no-default-features",
            "--features",
            "cuda",
            "--offline",
        ],
    },
];

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct StreamSummary {
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandSummary {
    id: String,
    tool: &'static str,
    argv: Vec<String>,
    exit_code: i32,
    duration_ns: u64,
    stdout: StreamSummary,
    stderr: StreamSummary,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct CleanupSummary {
    temporary_target_removed: bool,
    process_job_empty: bool,
    repository_artifacts_written: bool,
    receipt_artifacts_written: bool,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct QualityGateResult {
    schema: &'static str,
    status: &'static str,
    qualification_status: &'static str,
    profile: &'static str,
    source_state_before_sha256: String,
    source_state_after_sha256: String,
    source_state_stable: bool,
    commands: Vec<CommandSummary>,
    cleanup: CleanupSummary,
}

#[derive(Debug)]
struct CapturedStream {
    data: Vec<u8>,
    sha256: String,
    bytes: u64,
}

#[derive(Debug)]
struct ProcessResult {
    status: ExitStatus,
    duration: Duration,
    stdout: CapturedStream,
    stderr: CapturedStream,
}

pub(crate) fn run() -> Result<Value> {
    #[cfg(not(windows))]
    {
        return Err(XtaskError::gate(
            "DEFERRED_POST_P16",
            "quality-gate is implemented only for the prototype Windows host",
            "Run the gate on native Windows or wait for the post-P16 portability phase.",
        ));
    }
    #[cfg(windows)]
    {
        run_windows()
    }
}

#[cfg(windows)]
fn run_windows() -> Result<Value> {
    let repository = std::env::current_dir()
        .io_context("QUALITY_CWD_FAILED", "could not read the current directory")?
        .canonicalize()
        .io_context(
            "QUALITY_CWD_FAILED",
            "could not canonicalize the repository root",
        )?;
    require_workspace_root(&repository)?;

    let cargo = PathBuf::from(env!("CARGO")).canonicalize().io_context(
        "QUALITY_CARGO_NOT_BOUND",
        "could not bind the Cargo executable",
    )?;
    require_regular_file(&cargo, "QUALITY_CARGO_NOT_BOUND")?;
    let (git, _) = crate::p1a_windows::discover_git_path()?;
    require_regular_file(&git, "QUALITY_GIT_NOT_BOUND")?;

    let target = OwnedTempRoot::create()?;
    let before = run_checked(
        "Q000",
        "${GIT}",
        &git,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        &repository,
        None,
        COMMAND_TIMEOUT,
    )?;

    let mut summaries = Vec::with_capacity(GATE_COMMANDS.len());
    for command in GATE_COMMANDS {
        let result = run_checked(
            command.id,
            "${CARGO}",
            &cargo,
            command.args,
            &repository,
            Some(target.path()),
            COMMAND_TIMEOUT,
        )?;
        summaries.push(summary(command.id, "${CARGO}", command.args, &result));
    }

    let after = run_checked(
        "Q008",
        "${GIT}",
        &git,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        &repository,
        None,
        COMMAND_TIMEOUT,
    )?;
    if before.stdout.data != after.stdout.data {
        return Err(XtaskError::integrity(
            "QUALITY_SOURCE_STATE_CHANGED",
            "the quality gate changed the repository status",
        ));
    }

    let before_hash = before.stdout.sha256;
    let after_hash = after.stdout.sha256;
    target.remove()?;
    let value = QualityGateResult {
        schema: "python-slm-quality-gate-result-v1",
        status: "PASS",
        qualification_status: "SKIPPED",
        profile: "prototype-windows-5090-v1",
        source_state_before_sha256: before_hash,
        source_state_after_sha256: after_hash,
        source_state_stable: true,
        commands: summaries,
        cleanup: CleanupSummary {
            temporary_target_removed: true,
            process_job_empty: true,
            repository_artifacts_written: false,
            receipt_artifacts_written: false,
        },
    };
    serde_json::to_value(value).map_err(|error| {
        XtaskError::new(
            "QUALITY_RESULT_SERIALIZATION_FAILED",
            Category::Internal,
            format!("could not serialize the quality-gate result: {error}"),
            "Inspect the quality-gate implementation.",
        )
    })
}

fn require_workspace_root(repository: &Path) -> Result<()> {
    for relative in ["Cargo.toml", "xtask/Cargo.toml", "TODO.md"] {
        if !repository.join(relative).is_file() {
            return Err(XtaskError::new(
                "QUALITY_REPOSITORY_ROOT_INVALID",
                Category::Usage,
                "quality-gate must run from the python-slm repository root",
                "Change to the repository root and retry.",
            ));
        }
    }
    Ok(())
}

fn require_regular_file(path: &Path, code: &'static str) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).io_context(code, "could not inspect a required executable")?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(XtaskError::environment(
            code,
            "a required executable is not a regular non-symlink file",
        ));
    }
    Ok(())
}

fn run_checked(
    id: &str,
    tool: &'static str,
    executable: &Path,
    args: &[&str],
    cwd: &Path,
    target: Option<&Path>,
    timeout: Duration,
) -> Result<ProcessResult> {
    let result = run_process(executable, args, cwd, target, &[], timeout)?;
    if !result.status.success() {
        return Err(XtaskError::gate(
            "QUALITY_COMMAND_FAILED",
            format!(
                "{id} {tool} exited with code {} (stdout {}, stderr {})",
                result.status.code().unwrap_or(-1),
                result.stdout.sha256,
                result.stderr.sha256
            ),
            "Correct the automated gate failure and rerun the fixed command plan.",
        ));
    }
    Ok(result)
}

fn summary(id: &str, tool: &'static str, args: &[&str], result: &ProcessResult) -> CommandSummary {
    CommandSummary {
        id: id.to_owned(),
        tool,
        argv: std::iter::once(tool.to_owned())
            .chain(args.iter().map(|value| (*value).to_owned()))
            .collect(),
        exit_code: result.status.code().unwrap_or(-1),
        duration_ns: u64::try_from(result.duration.as_nanos()).unwrap_or(u64::MAX),
        stdout: StreamSummary {
            sha256: result.stdout.sha256.clone(),
            bytes: result.stdout.bytes,
        },
        stderr: StreamSummary {
            sha256: result.stderr.sha256.clone(),
            bytes: result.stderr.bytes,
        },
    }
}

fn run_process(
    executable: &Path,
    args: &[&str],
    cwd: &Path,
    target: Option<&Path>,
    environment: &[(&str, OsString)],
    timeout: Duration,
) -> Result<ProcessResult> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TERM_COLOR", "never")
        .env("TERM", "dumb");
    if let Some(target) = target {
        command.env("CARGO_TARGET_DIR", target);
    }
    for (key, value) in environment {
        command.env(key, value);
    }

    let started = Instant::now();
    let mut child = command.spawn().io_context(
        "QUALITY_COMMAND_SPAWN_FAILED",
        "could not start a fixed gate command",
    )?;
    #[cfg(windows)]
    let job = Job::attach(&mut child)?;

    let stdout = child.stdout.take().ok_or_else(|| {
        XtaskError::environment(
            "QUALITY_CAPTURE_MISSING",
            "the fixed command has no stdout capture",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        XtaskError::environment(
            "QUALITY_CAPTURE_MISSING",
            "the fixed command has no stderr capture",
        )
    })?;
    let stdout_reader = thread::spawn(move || capture(stdout));
    let stderr_reader = thread::spawn(move || capture(stderr));

    let status = loop {
        if let Some(status) = child.try_wait().io_context(
            "QUALITY_COMMAND_WAIT_FAILED",
            "could not poll a fixed gate command",
        )? {
            break status;
        }
        if started.elapsed() >= timeout {
            #[cfg(windows)]
            job.terminate(124)?;
            #[cfg(not(windows))]
            child.kill().io_context(
                "QUALITY_COMMAND_TERMINATE_FAILED",
                "could not terminate a timed-out command",
            )?;
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(XtaskError::gate(
                "QUALITY_COMMAND_TIMEOUT",
                "a fixed quality-gate command exceeded its timeout",
                "Correct the hung build or test before retrying.",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    };

    let stdout = stdout_reader.join().map_err(|_| {
        XtaskError::environment(
            "QUALITY_CAPTURE_FAILED",
            "the stdout capture worker panicked",
        )
    })??;
    let stderr = stderr_reader.join().map_err(|_| {
        XtaskError::environment(
            "QUALITY_CAPTURE_FAILED",
            "the stderr capture worker panicked",
        )
    })??;
    #[cfg(windows)]
    job.require_empty(Duration::from_secs(5))?;
    #[cfg(windows)]
    drop(job);
    Ok(ProcessResult {
        status,
        duration: started.elapsed(),
        stdout,
        stderr,
    })
}

fn capture(mut reader: impl Read) -> Result<CapturedStream> {
    let mut digest = Sha256::new();
    let mut data = Vec::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .io_context("QUALITY_CAPTURE_FAILED", "could not read command output")?;
        if read == 0 {
            break;
        }
        total = total.checked_add(read as u64).ok_or_else(|| {
            XtaskError::environment(
                "QUALITY_CAPTURE_OVERFLOW",
                "command output length overflowed",
            )
        })?;
        digest.update(&buffer[..read]);
        if total <= CAPTURE_LIMIT_BYTES {
            data.extend_from_slice(&buffer[..read]);
        }
    }
    if total > CAPTURE_LIMIT_BYTES {
        return Err(XtaskError::gate(
            "QUALITY_CAPTURE_LIMIT_EXCEEDED",
            "a fixed command exceeded the bounded output capture",
            "Reduce unexpected command output before retrying.",
        ));
    }
    Ok(CapturedStream {
        data,
        sha256: hex::encode(digest.finalize()),
        bytes: total,
    })
}

struct OwnedTempRoot {
    path: PathBuf,
    removed: bool,
}

impl OwnedTempRoot {
    fn create() -> Result<Self> {
        let base = std::env::temp_dir().canonicalize().io_context(
            "QUALITY_TEMP_ROOT_INVALID",
            "could not canonicalize the temporary root",
        )?;
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                XtaskError::environment(
                    "QUALITY_TIME_INVALID",
                    "system time predates the Unix epoch",
                )
            })?
            .as_nanos();
        for attempt in 0..32_u32 {
            let path = base.join(format!(
                "python-slm-p3-quality-{}-{epoch}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        removed: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(XtaskError::environment(
                        "QUALITY_TEMP_CREATE_FAILED",
                        format!("could not create the owned temporary root: {error}"),
                    ));
                }
            }
        }
        Err(XtaskError::environment(
            "QUALITY_TEMP_CREATE_FAILED",
            "could not allocate a unique owned temporary root",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn remove(mut self) -> Result<()> {
        fs::remove_dir_all(&self.path).io_context(
            "QUALITY_TEMP_REMOVE_FAILED",
            "could not remove the owned temporary root",
        )?;
        self.removed = true;
        Ok(())
    }
}

impl Drop for OwnedTempRoot {
    fn drop(&mut self) {
        if !self.removed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Job {
    fn attach(child: &mut Child) -> Result<Self> {
        use std::mem::size_of;
        use std::os::windows::io::AsRawHandle;
        use std::ptr::null;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        // SAFETY: null security and name pointers request a private unnamed Job Object.
        let handle = unsafe { CreateJobObjectW(null(), null()) };
        if handle.is_null() {
            return Err(last_windows_error(
                "QUALITY_JOB_CREATE_FAILED",
                "could not create the private quality-gate Job Object",
            ));
        }
        let job = Self(handle);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the buffer is initialized and its exact size is supplied.
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(last_windows_error(
                "QUALITY_JOB_LIMIT_FAILED",
                "could not enable kill-on-close for the quality-gate Job",
            ));
        }
        let process = child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        // SAFETY: both handles are live and owned by this process.
        if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
            let _ = child.kill();
            return Err(last_windows_error(
                "QUALITY_JOB_ASSIGN_FAILED",
                "could not contain the fixed quality-gate command",
            ));
        }
        Ok(job)
    }

    fn require_empty(&self, timeout: Duration) -> Result<()> {
        use std::mem::size_of;
        use std::ptr::null_mut;
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject,
        };

        let started = Instant::now();
        loop {
            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            // SAFETY: the output buffer and size match the requested information class.
            if unsafe {
                QueryInformationJobObject(
                    self.0,
                    JobObjectBasicAccountingInformation,
                    (&raw mut accounting).cast(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    null_mut(),
                )
            } == 0
            {
                return Err(last_windows_error(
                    "QUALITY_JOB_QUERY_FAILED",
                    "could not query the quality-gate Job",
                ));
            }
            if accounting.ActiveProcesses == 0 {
                return Ok(());
            }
            if started.elapsed() >= timeout {
                return Err(XtaskError::environment(
                    "QUALITY_PROCESS_JOB_NOT_EMPTY",
                    "the fixed command exited while contained descendants remained active",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate(&self, code: u32) -> Result<()> {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        // SAFETY: self owns a live Job Object handle.
        if unsafe { TerminateJobObject(self.0, code) } == 0 {
            return Err(last_windows_error(
                "QUALITY_COMMAND_TERMINATE_FAILED",
                "could not terminate the timed-out quality-gate Job",
            ));
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        // SAFETY: this type exclusively owns the non-null handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn last_windows_error(code: &'static str, message: &'static str) -> XtaskError {
    XtaskError::environment(
        code,
        format!(
            "{message}; Win32 error {}",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_plan_is_exact_and_contains_no_publication_command() {
        assert_eq!(
            GATE_COMMANDS
                .iter()
                .map(|value| value.id)
                .collect::<Vec<_>>(),
            ["Q001", "Q002", "Q003", "Q004", "Q005", "Q006", "Q007"]
        );
        let joined = GATE_COMMANDS
            .iter()
            .flat_map(|value| value.args)
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        for forbidden in [
            "receipt",
            "acceptance",
            "pointer",
            "verify-env",
            "probe-cuda",
        ] {
            assert!(!joined.contains(forbidden));
        }
    }

    #[test]
    fn owned_temporary_root_is_removed() {
        let root = OwnedTempRoot::create().unwrap();
        let path = root.path().to_owned();
        fs::write(path.join("sentinel"), b"owned").unwrap();
        root.remove().unwrap();
        assert!(!path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn timed_out_child_is_terminated_as_a_job() {
        if std::env::var_os("P3_TIMEOUT_CHILD").is_some() {
            thread::sleep(Duration::from_secs(10));
            return;
        }
        let executable = std::env::current_exe().unwrap();
        let cwd = std::env::current_dir().unwrap();
        let error = run_process(
            &executable,
            &[
                "--exact",
                "quality_gate::tests::timed_out_child_is_terminated_as_a_job",
                "--nocapture",
            ],
            &cwd,
            None,
            &[("P3_TIMEOUT_CHILD", OsString::from("1"))],
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert_eq!(error.code, "QUALITY_COMMAND_TIMEOUT");
    }

    #[test]
    fn known_empty_stream_hash_is_stable() {
        let captured = capture(std::io::empty()).unwrap();
        assert_eq!(captured.sha256, crate::hash::bytes(b""));
        assert_eq!(captured.bytes, 0);
    }
}
