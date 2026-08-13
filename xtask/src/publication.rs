use crate::error::{IoContext, Result, XtaskError};
use crate::hash;
use crate::process::FileRef;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub fn require_output_root(repository: &Path, supplied: &Path) -> Result<PathBuf> {
    require_exact_output_root(repository, supplied, Path::new("docs/receipts/P0A"), "P0A")
}

pub fn require_exact_output_root(
    repository: &Path,
    supplied: &Path,
    expected_relative: &Path,
    phase: &str,
) -> Result<PathBuf> {
    if supplied != expected_relative {
        return Err(XtaskError::new(
            "OUTPUT_ROOT_INVALID",
            crate::error::Category::Usage,
            format!(
                "{phase} output root must be {}, observed {}",
                expected_relative.display(),
                supplied.display()
            ),
            format!("Use --output-root {}.", expected_relative.display()),
        ));
    }
    for component in supplied.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(XtaskError::integrity(
                "OUTPUT_ROOT_UNSAFE",
                format!("{phase} output root contains an unsafe path component"),
            ));
        }
    }
    let repository = fs::canonicalize(repository).io_context(
        "OUTPUT_ROOT_INVALID",
        "could not canonicalize repository root",
    )?;
    let mut cursor = repository.clone();
    for component in supplied.components() {
        let Component::Normal(part) = component else {
            unreachable!()
        };
        cursor.push(part);
        if cursor.exists() {
            let metadata = fs::symlink_metadata(&cursor)
                .io_context("OUTPUT_ROOT_INVALID", "could not inspect output path")?;
            if is_link_or_reparse(&metadata) {
                return Err(XtaskError::integrity(
                    "OUTPUT_ROOT_SYMLINK_REJECTED",
                    format!(
                        "output path contains a symbolic link or reparse point: {}",
                        cursor.display()
                    ),
                ));
            }
            let canonical = fs::canonicalize(&cursor)
                .io_context("OUTPUT_ROOT_INVALID", "could not canonicalize output path")?;
            if !canonical.starts_with(&repository) {
                return Err(XtaskError::integrity(
                    "OUTPUT_ROOT_ESCAPE",
                    "output root escapes the repository",
                ));
            }
        }
    }
    Ok(repository.join(supplied))
}

/// Reject every pre-existing link, Windows reparse point, and special file below `root`.
///
/// P0A calls this before reading or mutating its receipt namespace so a pre-created
/// descendant cannot redirect an otherwise-contained path outside the repository.
pub fn require_no_follow_tree(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(root).io_context(
        "P0A_RECEIPT_TREE_INSPECTION_FAILED",
        format!("could not inspect {}", root.display()),
    )?;
    if is_link_or_reparse(&metadata) {
        return Err(XtaskError::integrity(
            "P0A_RECEIPT_LINK_REJECTED",
            format!(
                "receipt tree contains a symbolic link or reparse point: {}",
                root.display()
            ),
        ));
    }
    if !metadata.is_dir() {
        return Err(XtaskError::integrity(
            "P0A_RECEIPT_TREE_INVALID",
            "P0A receipt root is not a directory",
        ));
    }

    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).io_context(
            "P0A_RECEIPT_TREE_INSPECTION_FAILED",
            format!("could not enumerate {}", directory.display()),
        )? {
            let entry = entry.io_context(
                "P0A_RECEIPT_TREE_INSPECTION_FAILED",
                "could not read receipt-tree entry",
            )?;
            let metadata = fs::symlink_metadata(entry.path()).io_context(
                "P0A_RECEIPT_TREE_INSPECTION_FAILED",
                "could not inspect receipt-tree entry",
            )?;
            if is_link_or_reparse(&metadata) {
                return Err(XtaskError::integrity(
                    "P0A_RECEIPT_LINK_REJECTED",
                    format!(
                        "receipt tree contains a symbolic link or reparse point: {}",
                        entry.path().display()
                    ),
                ));
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if !metadata.is_file() {
                return Err(XtaskError::integrity(
                    "P0A_RECEIPT_SPECIAL_FILE_REJECTED",
                    format!(
                        "receipt tree contains a special file: {}",
                        entry.path().display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

pub fn create_dir(path: &Path) -> Result<()> {
    fs::create_dir(path).io_context(
        "CREATE_NEW_DIRECTORY_FAILED",
        format!("could not create {}", path.display()),
    )
}

pub fn create_dir_all(path: &Path) -> Result<()> {
    fs::create_dir_all(path).io_context(
        "DIRECTORY_CREATE_FAILED",
        format!("could not create {}", path.display()),
    )
}

pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .io_context(
            "CREATE_NEW_FILE_FAILED",
            format!("could not create {}", path.display()),
        )?;
    output.write_all(bytes).io_context(
        "FILE_WRITE_FAILED",
        format!("could not write {}", path.display()),
    )?;
    output.sync_all().io_context(
        "FILE_SYNC_FAILED",
        format!("could not sync {}", path.display()),
    )?;
    Ok(())
}

pub fn write_json_new<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        XtaskError::new(
            "JSON_SERIALIZATION_FAILED",
            crate::error::Category::Internal,
            format!("could not serialize {}: {error}", path.display()),
            "Inspect the xtask data model.",
        )
    })?;
    bytes.push(b'\n');
    write_new(path, &bytes)
}

pub fn write_new_via_owned_temp(path: &Path, bytes: &[u8], owner: &str) -> Result<()> {
    if path.exists() {
        return Err(XtaskError::integrity(
            "CREATE_NEW_FILE_FAILED",
            format!("refused to overwrite existing {}", path.display()),
        ));
    }
    let (_, _, entropy) = crate::time::now();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| XtaskError::integrity("CREATE_NEW_FILE_FAILED", "file name is not UTF-8"))?;
    let temporary = path.with_file_name(format!(
        "{file_name}.tmp-{owner}-{}-{entropy}",
        std::process::id()
    ));
    write_new(&temporary, bytes)?;
    if let Err(error) = move_new_write_through(&temporary, path) {
        return match fs::remove_file(&temporary) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(XtaskError::environment(
                "CREATE_NEW_ATOMIC_PUBLICATION_FAILED",
                format!(
                    "{}; the owned temporary {} also could not be removed: {cleanup}",
                    error.message,
                    temporary.display()
                ),
            )),
        };
    }
    if fs::read(path).ok().as_deref() != Some(bytes) {
        return Err(XtaskError::integrity(
            "CREATE_NEW_ATOMIC_PUBLICATION_INVALID",
            "create-new atomic publication bytes changed after its write-through move",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn move_new_write_through(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let mut source_wide = source.as_os_str().encode_wide().collect::<Vec<_>>();
    source_wide.push(0);
    let mut destination_wide = destination.as_os_str().encode_wide().collect::<Vec<_>>();
    destination_wide.push(0);
    // SAFETY: both UTF-16 buffers are NUL-terminated and remain live for the call.
    if unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(XtaskError::environment(
            "CREATE_NEW_ATOMIC_PUBLICATION_FAILED",
            format!(
                "could not write-through move {} to the create-new destination {}: {}",
                source.display(),
                destination.display(),
                std::io::Error::last_os_error()
            ),
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn move_new_write_through(source: &Path, destination: &Path) -> Result<()> {
    // A same-filesystem hard link is an atomic create-new publication on POSIX: it
    // cannot replace a concurrently created immutable destination. Flush the parent
    // after removing the temporary name so both directory mutations are durable.
    fs::hard_link(source, destination).io_context(
        "CREATE_NEW_ATOMIC_PUBLICATION_FAILED",
        format!(
            "could not link {} to the create-new destination {}",
            source.display(),
            destination.display()
        ),
    )?;
    fs::remove_file(source).io_context(
        "CREATE_NEW_TEMP_CLEANUP_FAILED",
        format!("could not remove owned temporary {}", source.display()),
    )?;
    let parent = destination.parent().ok_or_else(|| {
        XtaskError::integrity(
            "CREATE_NEW_ATOMIC_PUBLICATION_FAILED",
            "create-new destination has no parent directory",
        )
    })?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .io_context(
            "CREATE_NEW_PARENT_SYNC_FAILED",
            format!("could not sync publication directory {}", parent.display()),
        )
}

pub fn write_json_new_via_owned_temp<T: Serialize>(
    path: &Path,
    value: &T,
    owner: &str,
) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        XtaskError::new(
            "JSON_SERIALIZATION_FAILED",
            crate::error::Category::Internal,
            format!("could not serialize {}: {error}", path.display()),
            "Inspect the xtask data model.",
        )
    })?;
    bytes.push(b'\n');
    write_new_via_owned_temp(path, &bytes, owner)
}

pub fn file_ref_from(run_root: &Path, relative: &str) -> Result<FileRef> {
    if !safe_relative(relative) {
        return Err(XtaskError::integrity(
            "UNSAFE_FILE_REFERENCE",
            format!("unsafe file reference: {relative}"),
        ));
    }
    let path = run_root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    let metadata = fs::metadata(&path).io_context(
        "FILE_REFERENCE_FAILED",
        format!("could not inspect {relative}"),
    )?;
    Ok(FileRef {
        path: relative.to_owned(),
        sha256: hash::file(&path)?,
        bytes: metadata.len(),
    })
}

pub fn seal(run_root: &Path) -> Result<(usize, String)> {
    require_no_follow_tree(run_root)?;
    let mut files = BTreeSet::new();
    collect_regular_files(run_root, run_root, &mut files)?;
    files.remove("SHA256SUMS");
    let mut manifest = String::new();
    for relative in &files {
        let digest =
            hash::file(&run_root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR)))?;
        manifest.push_str(&digest);
        manifest.push_str("  ");
        manifest.push_str(relative);
        manifest.push('\n');
    }
    let seal_path = run_root.join("SHA256SUMS");
    write_new(&seal_path, manifest.as_bytes())?;
    Ok((files.len(), hash::bytes(manifest.as_bytes())))
}

pub fn verify_seal(run_root: &Path, expected_seal_hash: &str) -> Result<()> {
    require_no_follow_tree(run_root)?;
    let seal_path = run_root.join("SHA256SUMS");
    hash::require_file(&seal_path, expected_seal_hash, "RUN_SEAL_HASH_MISMATCH")?;
    let text = fs::read_to_string(&seal_path)
        .io_context("RUN_SEAL_READ_FAILED", "could not read run seal")?;
    if text.contains('\r') || !text.ends_with('\n') {
        return Err(XtaskError::integrity(
            "RUN_SEAL_FORMAT_INVALID",
            "run seal must be canonical LF text",
        ));
    }
    let mut listed = BTreeSet::new();
    let mut previous: Option<String> = None;
    for line in text.lines() {
        let (digest, path) = line.split_once("  ").ok_or_else(|| {
            XtaskError::integrity("RUN_SEAL_FORMAT_INVALID", "malformed run seal line")
        })?;
        if !hash::is_lower_sha256(digest)
            || !safe_relative(path)
            || previous.as_deref().is_some_and(|prior| prior >= path)
        {
            return Err(XtaskError::integrity(
                "RUN_SEAL_FORMAT_INVALID",
                format!("invalid or unordered seal line: {line}"),
            ));
        }
        previous = Some(path.to_owned());
        listed.insert(path.to_owned());
        hash::require_file(
            &run_root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR)),
            digest,
            "RUN_SEAL_ENTRY_MISMATCH",
        )?;
    }
    let mut actual = BTreeSet::new();
    collect_regular_files(run_root, run_root, &mut actual)?;
    actual.remove("SHA256SUMS");
    if listed != actual {
        return Err(XtaskError::integrity(
            "RUN_SEAL_COVERAGE_INVALID",
            "run seal does not cover exactly every run file except itself",
        ));
    }
    Ok(())
}

fn collect_regular_files(
    root: &Path,
    directory: &Path,
    output: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in fs::read_dir(directory).io_context(
        "FILE_ENUMERATION_FAILED",
        format!("could not enumerate {}", directory.display()),
    )? {
        let entry =
            entry.io_context("FILE_ENUMERATION_FAILED", "could not read directory entry")?;
        let kind = entry.file_type().io_context(
            "FILE_ENUMERATION_FAILED",
            "could not inspect directory entry",
        )?;
        if kind.is_symlink() {
            return Err(XtaskError::integrity(
                "SYMLINK_REJECTED",
                format!(
                    "publication tree contains a symbolic link: {}",
                    entry.path().display()
                ),
            ));
        }
        if kind.is_dir() {
            collect_regular_files(root, &entry.path(), output)?;
        } else if kind.is_file() {
            output.insert(
                entry
                    .path()
                    .strip_prefix(root)
                    .expect("below root")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        } else {
            return Err(XtaskError::integrity(
                "SPECIAL_FILE_REJECTED",
                format!(
                    "publication tree contains a special file: {}",
                    entry.path().display()
                ),
            ));
        }
    }
    Ok(())
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.contains(':')
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_is_sorted_complete_and_verifiable() {
        let temp = tempfile::tempdir().unwrap();
        create_dir_all(&temp.path().join("commands")).unwrap();
        write_new(&temp.path().join("z.txt"), b"z").unwrap();
        write_new(&temp.path().join("commands/a.txt"), b"a").unwrap();
        let (entries, digest) = seal(temp.path()).unwrap();
        assert_eq!(entries, 2);
        verify_seal(temp.path(), &digest).unwrap();
        let manifest = fs::read_to_string(temp.path().join("SHA256SUMS")).unwrap();
        assert!(
            manifest
                .lines()
                .next()
                .unwrap()
                .ends_with("  commands/a.txt")
        );
    }

    #[test]
    fn create_new_refuses_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("immutable");
        write_new(&path, b"one").unwrap();
        assert_eq!(
            write_new(&path, b"two").unwrap_err().code,
            "CREATE_NEW_FILE_FAILED"
        );
        assert_eq!(fs::read(path).unwrap(), b"one");
    }

    #[test]
    fn temp_publication_is_create_new_and_never_overwrites_final() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("immutable.json");
        write_new_via_owned_temp(&path, b"one\n", "test").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one\n");
        assert_eq!(
            write_new_via_owned_temp(&path, b"two\n", "test")
                .unwrap_err()
                .code,
            "CREATE_NEW_FILE_FAILED"
        );
        assert_eq!(fs::read(&path).unwrap(), b"one\n");
        assert_eq!(
            fs::read_dir(temp.path())
                .unwrap()
                .filter_map(std::result::Result::ok)
                .count(),
            1
        );
    }

    #[test]
    fn no_follow_tree_rejects_descendant_links_when_supported() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = temp.path().join("P0A");
        create_dir_all(&root).unwrap();
        let link = root.join("runs");

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        if let Err(error) = std::os::windows::fs::symlink_dir(outside.path(), &link) {
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("could not create test directory link: {error}");
        }

        assert_eq!(
            require_no_follow_tree(&root).unwrap_err().code,
            "P0A_RECEIPT_LINK_REJECTED"
        );
    }
}
