use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const HOST_DATA_ADAPTER_SCHEMA: &str = "python-slm-host-data-adapter-v1";
pub const PORTABLE_ARTIFACT_SEMANTICS: &str = "portable-byte-identical-v1";
pub const NATIVE_FILESYSTEM_SEMANTICS: &str = "contained-create-new-native-v1";
pub const SUSPEND_INCLUSIVE_CLOCK: &str = "suspend-inclusive-monotonic-v1";

/// A monotonic nanosecond count that keeps running across system suspend.
///
/// The completion SLA is a wall-clock deadline, so time the host spends asleep is
/// time the run spent not finishing and has to count against it. The obvious
/// clocks all get this wrong in the same direction: `std::time::Instant` is
/// `QueryPerformanceCounter` on Windows and `CLOCK_MONOTONIC` on Linux, and both
/// stop while the machine is suspended, so an overnight run that slept for six
/// hours would report an elapsed time six hours short. `SystemTime` counts the
/// sleep but is not monotonic and moves under clock adjustment.
///
/// Windows `QueryInterruptTime` is the *biased* interrupt-time count, meaning it
/// includes sleep, and Linux `CLOCK_BOOTTIME` is `CLOCK_MONOTONIC` plus suspend.
/// Both are monotonic. Hosts without an implementation fail closed rather than
/// silently substituting a clock that under-reports.
pub fn suspend_inclusive_now_ns() -> Result<u64> {
    #[cfg(target_os = "windows")]
    {
        let mut interrupt_time = 0_u64;
        // SAFETY: the out-pointer refers to initialized writable stack storage and
        // the call has no other preconditions.
        unsafe {
            windows_sys::Win32::System::WindowsProgramming::QueryInterruptTime(&mut interrupt_time);
        }
        // The interrupt-time count is in 100-nanosecond units.
        interrupt_time.checked_mul(100).ok_or_else(|| {
            ProductError::internal(
                "SLA_CLOCK_OVERFLOW",
                "the suspend-inclusive interrupt time overflowed nanoseconds",
            )
        })
    }
    #[cfg(target_os = "linux")]
    {
        let mut instant = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: the out-pointer refers to initialized writable stack storage.
        if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut instant) } != 0 {
            return Err(ProductError::internal(
                "SLA_CLOCK_UNAVAILABLE",
                "CLOCK_BOOTTIME could not be read",
            ));
        }
        let seconds = u64::try_from(instant.tv_sec).map_err(|_| {
            ProductError::internal(
                "SLA_CLOCK_INVALID",
                "CLOCK_BOOTTIME returned a negative time",
            )
        })?;
        seconds
            .checked_mul(1_000_000_000)
            .and_then(|nanoseconds| nanoseconds.checked_add(instant.tv_nsec as u64))
            .ok_or_else(|| {
                ProductError::internal(
                    "SLA_CLOCK_OVERFLOW",
                    "CLOCK_BOOTTIME overflowed nanoseconds",
                )
            })
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "no suspend-inclusive monotonic clock is implemented for this host",
        ))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostPlatform {
    WindowsX86_64,
    LinuxX86_64,
    MacosAppleSilicon,
}

impl HostPlatform {
    pub const fn target_triple(self) -> &'static str {
        match self {
            Self::WindowsX86_64 => "x86_64-pc-windows-msvc",
            Self::LinuxX86_64 => "x86_64-unknown-linux-gnu",
            Self::MacosAppleSilicon => "aarch64-apple-darwin",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostDataAdapter {
    pub schema: String,
    pub host: HostPlatform,
    pub target_triple: String,
    pub artifact_semantics: String,
    pub filesystem_semantics: String,
    pub qualification_status: String,
}

impl HostDataAdapter {
    fn new(host: HostPlatform) -> Self {
        Self {
            schema: HOST_DATA_ADAPTER_SCHEMA.to_owned(),
            host,
            target_triple: host.target_triple().to_owned(),
            artifact_semantics: PORTABLE_ARTIFACT_SEMANTICS.to_owned(),
            filesystem_semantics: NATIVE_FILESYSTEM_SEMANTICS.to_owned(),
            qualification_status: "SKIPPED".to_owned(),
        }
    }
}

pub fn current_host_data_adapter() -> Result<HostDataAdapter> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Ok(HostDataAdapter::new(HostPlatform::WindowsX86_64))
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Ok(HostDataAdapter::new(HostPlatform::LinuxX86_64))
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Ok(HostDataAdapter::new(HostPlatform::MacosAppleSilicon))
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )))]
    {
        Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "the current host has no implemented portable data adapter",
        ))
    }
}

pub fn require_portable_data_host() -> Result<()> {
    current_host_data_adapter().map(|_| ())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AcceleratorProvider {
    Cuda,
    Rocm,
    Metal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    Implemented,
    DeferredPostP16,
}

pub fn require_prototype_tuple(host: HostPlatform, provider: AcceleratorProvider) -> Result<()> {
    if host == HostPlatform::WindowsX86_64 && provider == AcceleratorProvider::Cuda {
        return Ok(());
    }
    Err(ProductError::gate(
        "DEFERRED_POST_P16",
        format!("{host:?}/{provider:?} is designed but deferred"),
    ))
}

pub(crate) fn publish_create_new(from: &Path, to: &Path) -> Result<()> {
    let from_parent = canonical_parent(from)?;
    let to_parent = canonical_parent(to)?;
    if from_parent != to_parent {
        return Err(ProductError::integrity(
            "OUTPUT_CROSS_VOLUME_REJECTED",
            "a generation temporary and its destination must share one parent",
        ));
    }
    if to.exists() {
        return Err(ProductError::integrity(
            "OUTPUT_ALREADY_EXISTS",
            "the create-new destination already exists",
        ));
    }

    #[cfg(windows)]
    publish_create_new_windows(from, to)?;
    #[cfg(target_os = "linux")]
    publish_create_new_linux(from, to)?;
    #[cfg(target_os = "macos")]
    publish_create_new_macos(from, to)?;
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return Err(ProductError::gate(
        "DEFERRED_POST_P16",
        "create-new publication is unavailable on this host",
    ));

    #[cfg(unix)]
    sync_parent_directory(to)?;
    Ok(())
}

fn canonical_parent(path: &Path) -> Result<std::path::PathBuf> {
    path.parent()
        .ok_or_else(|| ProductError::usage("OUTPUT_ROOT_INVALID", "an output has no parent"))?
        .canonicalize()
        .map_err(|_| {
            ProductError::environment(
                "OUTPUT_PARENT_INVALID",
                "could not canonicalize an output parent",
            )
        })
}

#[cfg(windows)]
fn publish_create_new_windows(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let from = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let to = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both buffers are NUL-terminated, remain live, and MOVEFILE_REPLACE_EXISTING is
    // deliberately absent so an existing destination cannot be overwritten.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(ProductError::environment(
            "OUTPUT_PUBLISH_FAILED",
            "could not install the create-new artifact",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn publish_create_new_linux(from: &Path, to: &Path) -> Result<()> {
    let from = unix_path(from)?;
    let to = unix_path(to)?;
    // SAFETY: both C strings are NUL-terminated and renameat2 receives fixed flags and AT_FDCWD.
    if unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } != 0
    {
        return Err(ProductError::environment(
            "OUTPUT_PUBLISH_FAILED",
            "could not install the create-new artifact",
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn publish_create_new_macos(from: &Path, to: &Path) -> Result<()> {
    let from = unix_path(from)?;
    let to = unix_path(to)?;
    // SAFETY: both C strings are NUL-terminated and renamex_np receives the exclusive flag only.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } != 0 {
        return Err(ProductError::environment(
            "OUTPUT_PUBLISH_FAILED",
            "could not install the create-new artifact",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn unix_path(path: &Path) -> Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        ProductError::integrity("OUTPUT_PATH_INVALID", "an output path contains a NUL byte")
    })
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        ProductError::usage("OUTPUT_ROOT_INVALID", "an output has no parent directory")
    })?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| {
            ProductError::environment(
                "OUTPUT_PARENT_SYNC_FAILED",
                "could not sync the published output directory entry",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_prototype_accelerator_tuple_is_implemented() {
        assert!(
            require_prototype_tuple(HostPlatform::WindowsX86_64, AcceleratorProvider::Cuda).is_ok()
        );
        for tuple in [
            (HostPlatform::LinuxX86_64, AcceleratorProvider::Cuda),
            (HostPlatform::WindowsX86_64, AcceleratorProvider::Rocm),
            (HostPlatform::MacosAppleSilicon, AcceleratorProvider::Metal),
        ] {
            assert_eq!(
                require_prototype_tuple(tuple.0, tuple.1).unwrap_err().code,
                "DEFERRED_POST_P16"
            );
        }
    }

    #[test]
    fn current_data_adapter_has_closed_portable_semantics() {
        let adapter = current_host_data_adapter().unwrap();
        assert_eq!(adapter.schema, HOST_DATA_ADAPTER_SCHEMA);
        assert_eq!(adapter.target_triple, adapter.host.target_triple());
        assert_eq!(adapter.artifact_semantics, PORTABLE_ARTIFACT_SEMANTICS);
        assert_eq!(adapter.filesystem_semantics, NATIVE_FILESYSTEM_SEMANTICS);
        assert_eq!(adapter.qualification_status, "SKIPPED");
        let encoded = serde_json::to_value(&adapter).unwrap();
        assert_eq!(encoded.as_object().unwrap().len(), 6);
    }

    #[test]
    fn publication_is_create_new_for_files_and_directories() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first.partial");
        let output = root.path().join("artifact");
        std::fs::write(&first, b"first").unwrap();
        publish_create_new(&first, &output).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"first");

        let second = root.path().join("second.partial");
        std::fs::write(&second, b"second").unwrap();
        assert_eq!(
            publish_create_new(&second, &output).unwrap_err().code,
            "OUTPUT_ALREADY_EXISTS"
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"first");

        let directory = root.path().join("directory.partial");
        let installed = root.path().join("directory");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("manifest.json"), b"{}\n").unwrap();
        publish_create_new(&directory, &installed).unwrap();
        assert_eq!(
            std::fs::read(installed.join("manifest.json")).unwrap(),
            b"{}\n"
        );
    }
}
