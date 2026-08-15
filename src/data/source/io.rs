use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_CONTROL_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn require_existing_root(path: &Path, code: &'static str) -> Result<PathBuf> {
    require_absolute(path, code)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| ProductError::environment(code, "could not inspect a required root"))?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err(ProductError::integrity(
            code,
            "a required root is not a regular non-reparse directory",
        ));
    }
    path.canonicalize()
        .map_err(|_| ProductError::environment(code, "could not canonicalize a required root"))
}

pub(crate) fn require_output_boundary(output: &Path, content_root: &Path) -> Result<()> {
    require_absolute(output, "OUTPUT_ROOT_INVALID")?;
    let components = output
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if components.iter().any(|value| value == ".git")
        || components
            .windows(2)
            .any(|pair| pair[0] == "docs" && pair[1] == "receipts")
    {
        return Err(ProductError::integrity(
            "OUTPUT_NAMESPACE_FORBIDDEN",
            "source generations cannot target repository metadata or receipt namespaces",
        ));
    }
    if output.exists() {
        return Err(ProductError::integrity(
            "OUTPUT_ALREADY_EXISTS",
            "the create-new output root already exists",
        ));
    }
    let parent = output.parent().ok_or_else(|| {
        ProductError::usage("OUTPUT_ROOT_INVALID", "the output root has no parent")
    })?;
    let parent = require_existing_root(parent, "OUTPUT_PARENT_INVALID")?;
    let leaf = output.file_name().ok_or_else(|| {
        ProductError::usage(
            "OUTPUT_ROOT_INVALID",
            "the output root has no final component",
        )
    })?;
    let target = parent.join(leaf);
    if target.starts_with(content_root) || content_root.starts_with(&target) {
        return Err(ProductError::integrity(
            "OUTPUT_CONTENT_OVERLAP",
            "the output and governed content roots overlap",
        ));
    }
    Ok(())
}

fn require_absolute(path: &Path, code: &'static str) -> Result<()> {
    if !path.is_absolute() {
        return Err(ProductError::usage(code, "a required path is not absolute"));
    }
    Ok(())
}

pub(crate) fn join_relative(root: &Path, relative: &str) -> Result<PathBuf> {
    require_portable_relative_path(relative, "DOCUMENT_PATH_INVALID")?;
    Ok(relative
        .split('/')
        .fold(root.to_path_buf(), |path, component| path.join(component)))
}

pub(crate) fn require_portable_relative_path(value: &str, code: &'static str) -> Result<()> {
    if value.is_empty()
        || value.contains('\\')
        || value.contains('\0')
        || value
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | "..") || part.contains(':'))
        || Path::new(value)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ProductError::integrity(
            code,
            "a manifest path is not a portable contained relative path",
        ));
    }
    Ok(())
}

pub(crate) fn require_contained_regular_file(root: &Path, path: &Path) -> Result<fs::Metadata> {
    let relative = path.strip_prefix(root).map_err(|_| {
        ProductError::integrity(
            "DOCUMENT_PATH_ESCAPE",
            "a document escapes the content root",
        )
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|_| {
            ProductError::environment("DOCUMENT_READ_FAILED", "could not inspect a document path")
        })?;
        if metadata.file_type().is_symlink() || is_reparse(&metadata) {
            return Err(ProductError::integrity(
                "DOCUMENT_REPARSE_REJECTED",
                "a document path contains a symlink or reparse point",
            ));
        }
    }
    let canonical = path.canonicalize().map_err(|_| {
        ProductError::environment("DOCUMENT_READ_FAILED", "could not canonicalize a document")
    })?;
    if !canonical.starts_with(root) {
        return Err(ProductError::integrity(
            "DOCUMENT_PATH_ESCAPE",
            "a document resolves outside the content root",
        ));
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        ProductError::environment("DOCUMENT_READ_FAILED", "could not inspect a document")
    })?;
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err(ProductError::integrity(
            "DOCUMENT_NOT_REGULAR",
            "a document is not a regular non-reparse file",
        ));
    }
    Ok(metadata)
}

pub(crate) fn read_stable_document(path: &Path, before: &fs::Metadata) -> Result<Vec<u8>> {
    read_stable_document_with(path, before, || {})
}

fn read_stable_document_with(
    path: &Path,
    before: &fs::Metadata,
    between: impl FnOnce(),
) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        options.share_mode(FILE_SHARE_READ);
    }
    let mut file = options.open(path).map_err(|_| {
        ProductError::environment(
            "DOCUMENT_OPEN_FAILED",
            "could not open a document read-only",
        )
    })?;
    let bound = file.metadata().map_err(|_| {
        ProductError::environment(
            "DOCUMENT_METADATA_FAILED",
            "could not bind document metadata",
        )
    })?;
    if fingerprint(before) != fingerprint(&bound) {
        return Err(ProductError::integrity(
            "DOCUMENT_MUTATED",
            "a document changed before its stable read",
        ));
    }
    let mut bytes = Vec::with_capacity(bound.len().min(1_000_003) as usize);
    file.read_to_end(&mut bytes).map_err(|_| {
        ProductError::environment("DOCUMENT_READ_FAILED", "could not read a document")
    })?;
    between();
    let after = file.metadata().map_err(|_| {
        ProductError::environment(
            "DOCUMENT_METADATA_FAILED",
            "could not recheck document metadata",
        )
    })?;
    if fingerprint(&bound) != fingerprint(&after) || after.len() != bytes.len() as u64 {
        return Err(ProductError::integrity(
            "DOCUMENT_MUTATED",
            "a document changed during its stable read",
        ));
    }
    Ok(bytes)
}

pub(crate) fn read_control_file(
    path: &Path,
    expected_sha256: Option<&str>,
    code: &'static str,
) -> Result<Vec<u8>> {
    require_absolute(path, code)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| ProductError::environment(code, "could not inspect a control file"))?;
    if !metadata.is_file() || is_reparse(&metadata) || metadata.len() > MAX_CONTROL_FILE_BYTES {
        return Err(ProductError::integrity(
            code,
            "a control file is not regular, is a reparse point, or exceeds its bound",
        ));
    }
    let bytes = read_stable_document(path, &metadata)?;
    if expected_sha256.is_some_and(|expected| expected != sha256(&bytes)) {
        return Err(ProductError::integrity(
            "CONTROL_FILE_HASH_MISMATCH",
            "a hash-bound control file differs from its declared identity",
        ));
    }
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileFingerprint {
    len: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
}

fn fingerprint(metadata: &fs::Metadata) -> FileFingerprint {
    FileFingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        created: metadata.created().ok(),
    }
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub(crate) fn parse_closed<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    code: &'static str,
) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| {
        ProductError::usage(
            code,
            "a versioned JSON document is malformed or contains unknown fields",
        )
    })
}

pub(crate) fn compact_json_line<T: Serialize>(value: &T, code: &'static str) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|_| ProductError::internal(code, "could not serialize a generation document"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(windows))]
    use std::io::Write;
    #[test]
    fn path_grammar_is_portable_and_contained() {
        assert!(require_portable_relative_path("pkg/module.py", "TEST").is_ok());
        for invalid in [
            "",
            "/absolute.py",
            "../escape.py",
            "a/./b.py",
            "C:/x.py",
            "a\\b.py",
        ] {
            assert!(require_portable_relative_path(invalid, "TEST").is_err());
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn stable_reader_detects_a_between_boundary_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.py");
        fs::write(&path, b"a".repeat(128)).unwrap();
        let before = fs::metadata(&path).unwrap();
        let error = read_stable_document_with(&path, &before, || {
            let mut file = OpenOptions::new().write(true).open(&path).unwrap();
            file.write_all(b"b").unwrap();
            file.sync_all().unwrap();
        })
        .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_MUTATED");
    }

    #[cfg(windows)]
    #[test]
    fn stable_reader_denies_concurrent_write_on_windows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.py");
        fs::write(&path, b"a".repeat(128)).unwrap();
        let before = fs::metadata(&path).unwrap();
        let mut denied = false;
        let bytes = read_stable_document_with(&path, &before, || {
            denied = OpenOptions::new().write(true).open(&path).is_err();
        })
        .unwrap();
        assert!(denied);
        assert_eq!(bytes.len(), 128);
    }
    #[test]
    fn output_boundary_rejects_git_and_receipt_namespaces() {
        let directory = tempfile::tempdir().unwrap();
        let content = directory.path().join("content");
        fs::create_dir(&content).unwrap();
        let content = content.canonicalize().unwrap();

        let receipts = directory.path().join("docs").join("receipts");
        fs::create_dir_all(&receipts).unwrap();
        let receipt_output = receipts.join("p4-generation");
        assert_eq!(
            require_output_boundary(&receipt_output, &content)
                .unwrap_err()
                .code,
            "OUTPUT_NAMESPACE_FORBIDDEN"
        );

        let git = directory.path().join(".git");
        fs::create_dir(&git).unwrap();
        let git_output = git.join("p4-generation");
        assert_eq!(
            require_output_boundary(&git_output, &content)
                .unwrap_err()
                .code,
            "OUTPUT_NAMESPACE_FORBIDDEN"
        );
    }
}
