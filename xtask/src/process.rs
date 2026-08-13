use crate::error::{Result, XtaskError};
use crate::hash;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output, Stdio};

const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileRef {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug)]
pub struct RecordedCommand {
    pub id: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub exit_code: i32,
    pub status: &'static str,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl RecordedCommand {
    pub fn stdout_ref(&self) -> FileRef {
        FileRef {
            path: format!("commands/{}.stdout.txt", self.id),
            sha256: hash::bytes(&self.stdout),
            bytes: self.stdout.len() as u64,
        }
    }

    pub fn stderr_ref(&self) -> FileRef {
        FileRef {
            path: format!("commands/{}.stderr.txt", self.id),
            sha256: hash::bytes(&self.stderr),
            bytes: self.stderr.len() as u64,
        }
    }
}

#[derive(Default)]
pub struct Recorder {
    commands: Vec<RecordedCommand>,
}

impl Recorder {
    pub fn run_git(&mut self, repository: &Path, args: &[&str]) -> Result<Output> {
        let id = format!("C{:02}", self.commands.len() + 1);
        let output = Command::new("git")
            .args(args)
            .current_dir(repository)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .output()
            .map_err(|error| {
                XtaskError::environment(
                    "GIT_EXEC_FAILED",
                    format!("could not execute git directly: {error}"),
                )
            })?;
        if output.stdout.len() > MAX_CAPTURE_BYTES || output.stderr.len() > MAX_CAPTURE_BYTES {
            return Err(XtaskError::environment(
                "COMMAND_OUTPUT_LIMIT_EXCEEDED",
                format!("git {} exceeded the bounded capture limit", args.join(" ")),
            ));
        }
        let exit_code = output.status.code().unwrap_or(-1);
        self.commands.push(RecordedCommand {
            id,
            argv: std::iter::once("git".to_owned())
                .chain(args.iter().map(|value| (*value).to_owned()))
                .collect(),
            cwd: "${REPO}".to_owned(),
            exit_code,
            status: if output.status.success() {
                "PASS"
            } else {
                "FAIL"
            },
            stdout: redact(&output.stdout, repository),
            stderr: redact(&output.stderr, repository),
        });
        Ok(output)
    }

    pub fn mark_locked_cargo_metadata_audit_pass(&mut self, raw_stdout: &[u8]) -> Result<()> {
        let command = self.commands.last_mut().ok_or_else(|| {
            XtaskError::integrity(
                "CARGO_METADATA_AUDIT_STATE_INVALID",
                "Cargo metadata audit has no recorded command",
            )
        })?;
        let expected = [
            "cargo",
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
        ];
        if command.argv.iter().map(String::as_str).ne(expected)
            || command.exit_code != 0
            || command.status != "PASS"
        {
            return Err(XtaskError::integrity(
                "CARGO_METADATA_AUDIT_STATE_INVALID",
                "last command is not the successful fixed Cargo metadata probe",
            ));
        }
        command.stdout = cargo_metadata_summary(raw_stdout, "PASS");
        Ok(())
    }

    /// Run the one non-Git child process admitted by the P0A verifier.
    ///
    /// The argv is deliberately fixed here rather than accepted from a caller. Cargo
    /// metadata resolves the locked dependency graph without running build scripts.
    pub fn run_locked_cargo_metadata(&mut self, repository: &Path) -> Result<Output> {
        const ARGS: &[&str] = &["metadata", "--locked", "--offline", "--format-version", "1"];
        let id = format!("C{:02}", self.commands.len() + 1);
        let output = Command::new("cargo")
            .args(ARGS)
            .current_dir(repository)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| {
                XtaskError::environment(
                    "CARGO_METADATA_EXEC_FAILED",
                    format!("could not execute locked Cargo metadata directly: {error}"),
                )
            })?;
        if output.stdout.len() > MAX_CAPTURE_BYTES || output.stderr.len() > MAX_CAPTURE_BYTES {
            return Err(XtaskError::environment(
                "COMMAND_OUTPUT_LIMIT_EXCEEDED",
                "cargo metadata exceeded the bounded capture limit",
            ));
        }
        let exit_code = output.status.code().unwrap_or(-1);
        self.commands.push(RecordedCommand {
            id,
            argv: std::iter::once("cargo".to_owned())
                .chain(ARGS.iter().map(|value| (*value).to_owned()))
                .collect(),
            cwd: "${REPO}".to_owned(),
            exit_code,
            status: if output.status.success() {
                "PASS"
            } else {
                "FAIL"
            },
            stdout: cargo_metadata_summary(&output.stdout, "PENDING"),
            stderr: redact(&output.stderr, repository),
        });
        if !output.status.success() {
            return Err(XtaskError::integrity(
                "CARGO_METADATA_FAILED",
                format!(
                    "locked offline Cargo metadata failed with exit {:?}",
                    output.status.code()
                ),
            ));
        }
        Ok(output)
    }

    pub fn git_success(
        &mut self,
        repository: &Path,
        args: &[&str],
        code: &'static str,
    ) -> Result<Output> {
        let output = self.run_git(repository, args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(XtaskError::integrity(
                code,
                format!(
                    "git {} failed with exit {:?}: {stderr}",
                    args.join(" "),
                    output.status.code()
                ),
            ));
        }
        Ok(output)
    }

    pub fn git_text(
        &mut self,
        repository: &Path,
        args: &[&str],
        code: &'static str,
    ) -> Result<String> {
        let output = self.git_success(repository, args, code)?;
        String::from_utf8(output.stdout).map_err(|_| {
            XtaskError::integrity(
                code,
                format!("git {} returned non-UTF-8 output", args.join(" ")),
            )
        })
    }

    pub fn commands(&self) -> &[RecordedCommand] {
        &self.commands
    }
}

fn redact(bytes: &[u8], repository: &Path) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let native = repository.as_os_str().to_string_lossy();
    let slashed = native.replace('\\', "/");
    let escaped = native.replace('\\', "\\\\");
    let mut redacted = text
        .replace(&escaped, "${REPO}")
        .replace(native.as_ref(), "${REPO}")
        .replace(&slashed, "${REPO}");
    for (name, replacement) in [
        ("USERPROFILE", "${HOME}"),
        ("HOME", "${HOME}"),
        ("TEMP", "${TEMP}"),
        ("TMP", "${TEMP}"),
    ] {
        if let Some(value) = std::env::var_os(name) {
            let native = value.to_string_lossy();
            if !native.is_empty() {
                let escaped = native.replace('\\', "\\\\");
                redacted = redacted
                    .replace(&escaped, replacement)
                    .replace(native.as_ref(), replacement)
                    .replace(&native.replace('\\', "/"), replacement);
            }
        }
    }
    redacted.into_bytes()
}

fn cargo_metadata_summary(raw_stdout: &[u8], audit_result: &str) -> Vec<u8> {
    let parsed = serde_json::from_slice::<Value>(raw_stdout).ok();
    let package_count = parsed
        .as_ref()
        .and_then(|value| value["packages"].as_array())
        .map(Vec::len);
    let resolve_node_count = parsed
        .as_ref()
        .and_then(|value| value["resolve"]["nodes"].as_array())
        .map(Vec::len);
    let mut bytes = serde_json::to_vec_pretty(&json!({
        "schema": "python-slm-cargo-metadata-audit-summary-v1",
        "raw_stdout_sha256": hash::bytes(raw_stdout),
        "raw_stdout_bytes": raw_stdout.len(),
        "metadata_json_parse": if parsed.is_some() { "PASS" } else { "FAIL" },
        "package_count": package_count,
        "resolve_node_count": resolve_node_count,
        "audit_scope": "xtask_locked_runtime_and_build_dependency_closure",
        "audit_result": audit_result,
        "raw_metadata_retained": false
    }))
    .expect("fixed audit summary serializes");
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_native_and_slashed_repository_paths() {
        let root = Path::new(r"C:\work\repo");
        let value = redact(br"C:\work\repo and C:/work/repo", root);
        assert_eq!(String::from_utf8(value).unwrap(), "${REPO} and ${REPO}");
    }

    #[test]
    fn redacts_json_escaped_windows_paths() {
        let root = Path::new(r"C:\work\repo");
        let value = redact(
            br#"{"manifest_path":"C:\\work\\repo\\xtask\\Cargo.toml"}"#,
            root,
        );
        assert_eq!(
            String::from_utf8(value).unwrap(),
            r#"{"manifest_path":"${REPO}\\xtask\\Cargo.toml"}"#
        );
    }

    #[test]
    fn cargo_metadata_summary_retains_no_paths_or_author_identity() {
        let raw = br#"{"packages":[{"manifest_path":"C:\\Users\\private\\Cargo.toml","authors":["Private Person <private@example.com>"]}],"resolve":{"nodes":[{}]}}"#;
        let summary = String::from_utf8(cargo_metadata_summary(raw, "PASS")).unwrap();
        assert!(summary.contains("\"package_count\": 1"));
        assert!(summary.contains("\"resolve_node_count\": 1"));
        assert!(summary.contains("\"audit_result\": \"PASS\""));
        assert!(!summary.contains("C:"));
        assert!(!summary.contains("private"));
        assert!(!summary.contains('@'));
        assert!(!summary.contains("\\\\"));
    }
}
