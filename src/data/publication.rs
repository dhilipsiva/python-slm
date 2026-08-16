use crate::error::{ProductError, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static PARTIAL_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) struct PartialGeneration {
    final_path: PathBuf,
    partial_path: PathBuf,
    published: bool,
}

impl PartialGeneration {
    pub(crate) fn create(final_path: &Path) -> Result<Self> {
        let parent = final_path.parent().ok_or_else(|| {
            ProductError::usage("OUTPUT_ROOT_INVALID", "the output root has no parent")
        })?;
        let leaf = final_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                ProductError::usage(
                    "OUTPUT_ROOT_INVALID",
                    "the output root has no portable name",
                )
            })?;
        for _ in 0..64 {
            let sequence = PARTIAL_COUNTER.fetch_add(1, Ordering::Relaxed);
            let partial_path = parent.join(format!(
                ".{leaf}.p4-partial-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&partial_path) {
                Ok(()) => {
                    return Ok(Self {
                        final_path: final_path.to_path_buf(),
                        partial_path,
                        published: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    continue;
                }
                Err(_) => {
                    return Err(ProductError::environment(
                        "OUTPUT_PARTIAL_CREATE_FAILED",
                        "could not create a unique partial generation",
                    ));
                }
            }
        }
        Err(ProductError::environment(
            "OUTPUT_PARTIAL_CREATE_FAILED",
            "could not allocate a unique partial generation",
        ))
    }

    pub(crate) fn create_documents_directory(&mut self) -> Result<()> {
        self.create_directory("documents")
    }

    pub(crate) fn create_parser_directory(&mut self) -> Result<()> {
        self.create_directory("parser")
    }

    pub(crate) fn create_policy_directory(&mut self) -> Result<()> {
        self.create_directory("policy")
    }

    fn create_directory(&mut self, name: &str) -> Result<()> {
        fs::create_dir(self.partial_path.join(name)).map_err(|_| {
            ProductError::environment(
                "OUTPUT_DIRECTORY_CREATE_FAILED",
                "could not create an output generation directory",
            )
        })
    }

    pub(crate) fn write_file(&mut self, relative: &Path, bytes: &[u8]) -> Result<()> {
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(ProductError::internal(
                "OUTPUT_PATH_INVALID",
                "an internal output path is not contained",
            ));
        }
        let path = self.partial_path.join(relative);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| {
                ProductError::environment(
                    "OUTPUT_FILE_CREATE_FAILED",
                    "could not create an immutable generation file",
                )
            })?;
        file.write_all(bytes).map_err(|_| {
            ProductError::environment(
                "OUTPUT_FILE_WRITE_FAILED",
                "could not write a generation file",
            )
        })?;
        file.sync_all().map_err(|_| {
            ProductError::environment(
                "OUTPUT_FILE_SYNC_FAILED",
                "could not sync a generation file",
            )
        })
    }

    pub(crate) fn publish(&mut self) -> Result<()> {
        if self.final_path.exists() {
            return Err(ProductError::integrity(
                "OUTPUT_ALREADY_EXISTS",
                "the create-new output root appeared before publication",
            ));
        }
        crate::platform::publish_create_new(&self.partial_path, &self.final_path)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for PartialGeneration {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.partial_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpublished_partial_generation_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("generation");
        let partial;
        {
            let generation = PartialGeneration::create(&output).unwrap();
            partial = generation.partial_path.clone();
            assert!(partial.is_dir());
        }
        assert!(!partial.exists());
        assert!(!output.exists());
    }

    #[test]
    fn publication_is_create_new() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("generation");
        let mut generation = PartialGeneration::create(&output).unwrap();
        generation.create_documents_directory().unwrap();
        generation
            .write_file(Path::new("manifest.json"), b"{}\n")
            .unwrap();
        generation.publish().unwrap();
        assert!(output.join("manifest.json").is_file());

        let mut second = PartialGeneration::create(&output).unwrap();
        let error = second.publish().unwrap_err();
        assert_eq!(error.code, "OUTPUT_ALREADY_EXISTS");
    }
}
