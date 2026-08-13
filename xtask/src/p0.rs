use crate::error::{IoContext, Result, XtaskError};
use crate::hash;
use crate::process::Recorder;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const BASELINE_COMMIT: &str = "b1ebb455cdae94bbb9fc54f246cdf2758eedf1d1";
pub const RECEIPT_COMMIT: &str = "86fb1e4cc68efeb651e5362c4aca85c2827d8e4d";
pub const RECONCILIATION_COMMIT: &str = "245f5f71eb3a76b3ce8e7c42228c00167803947c";
pub const RECONCILIATION_TREE: &str = "c7a322bc77aebab6a5117708f2b74d34365c7336";
pub const RECEIPT_SHA256: &str = "f08c6a41658ff287e238d6a96c4f2c874975964202c3eeced2bc0bc21f308904";
pub const CONTRACT_SHA256: &str =
    "fc2c60b52fdd7c524e0da06bb03972a4d523c21ad5536cba536185435bd44ad4";
pub const LEDGER_SHA256: &str = "8349d8a3e06d96d6921889de5534715e7b2f7439caf7e06558a97652a8890c8d";
pub const HISTORICAL_CARGO_LOCK_SHA256: &str =
    "c0a5e1afe02e252a22cca8bf07ab37fb2a25844210d2d5ce2e1e6082e497a28c";
pub const RUN_ID: &str = "20260811T074740Z-d5008e94";
pub const SEAL_SHA256: &str = "184dc926bb9e5e2963a61182398580f7dedbf5aa5992f062dacfc6db6f1430f5";
pub const MACHINE_EVIDENCE_SHA256: &str =
    "5516db126cd1b74d3daf7da535d2b0780191837926a5117c3e0e6a1135b03dd7";
pub const ORACLE_COMMIT: &str = "804cdcec24996895c469f54593a5aa79a4cd706a";
pub const ORACLE_NORMALIZED_SHA256: &str =
    "ffaa3b309e31782c95bfed1d4c37eb4b56427bc878c3d796f99d8663de53fe46";

const SEALED_PATHS: &[&str] = &[
    "docs/rebuild-contract.md",
    "docs/receipts/P0/capture.ps1",
    "docs/receipts/P0/evidence.json",
    "docs/receipts/P0/runs",
];

#[derive(Clone, Debug, Serialize)]
pub struct Identity {
    pub status: &'static str,
    pub baseline_commit: &'static str,
    pub receipt_commit: &'static str,
    pub receipt_sha256: &'static str,
    pub contract_sha256: &'static str,
    pub decision_ledger_sha256: &'static str,
    pub run_id: &'static str,
    pub seal_sha256: &'static str,
    pub oracle_commit: &'static str,
    pub oracle_normalized_sha256: &'static str,
}

impl Identity {
    pub fn approved() -> Self {
        Self {
            status: "PASS",
            baseline_commit: BASELINE_COMMIT,
            receipt_commit: RECEIPT_COMMIT,
            receipt_sha256: RECEIPT_SHA256,
            contract_sha256: CONTRACT_SHA256,
            decision_ledger_sha256: LEDGER_SHA256,
            run_id: RUN_ID,
            seal_sha256: SEAL_SHA256,
            oracle_commit: ORACLE_COMMIT,
            oracle_normalized_sha256: ORACLE_NORMALIZED_SHA256,
        }
    }
}

pub fn verify(repository: &Path, recorder: &mut Recorder) -> Result<Identity> {
    require_repository_root(repository, recorder)?;
    require_exact_commit_graph(repository, recorder)?;
    require_sealed_git_state(repository, recorder)?;
    require_receipt(repository)?;
    require_contract(repository)?;
    require_machine_evidence(repository)?;
    require_seal(repository)?;
    require_historical_lock(repository, recorder)?;
    require_historical_oracle(repository, recorder)?;
    Ok(Identity::approved())
}

fn require_repository_root(repository: &Path, recorder: &mut Recorder) -> Result<()> {
    let top = recorder.git_text(
        repository,
        &["rev-parse", "--show-toplevel"],
        "REPOSITORY_ROOT_FAILED",
    )?;
    let expected = fs::canonicalize(repository).io_context(
        "REPOSITORY_ROOT_FAILED",
        "could not canonicalize the current directory",
    )?;
    let observed = fs::canonicalize(top.trim()).io_context(
        "REPOSITORY_ROOT_FAILED",
        "could not canonicalize Git's repository root",
    )?;
    if observed != expected {
        return Err(XtaskError::environment(
            "REPOSITORY_ROOT_REQUIRED",
            format!(
                "run xtask from the repository root, not {}",
                repository.display()
            ),
        ));
    }
    Ok(())
}

fn require_exact_commit_graph(repository: &Path, recorder: &mut Recorder) -> Result<()> {
    recorder.git_success(
        repository,
        &["cat-file", "-e", &format!("{BASELINE_COMMIT}^{{commit}}")],
        "P0_BASELINE_MISSING",
    )?;
    recorder.git_success(
        repository,
        &["cat-file", "-e", &format!("{RECEIPT_COMMIT}^{{commit}}")],
        "P0_RECEIPT_COMMIT_MISSING",
    )?;
    recorder.git_success(
        repository,
        &[
            "cat-file",
            "-e",
            &format!("{RECONCILIATION_COMMIT}^{{commit}}"),
        ],
        "P0_RECONCILIATION_COMMIT_MISSING",
    )?;
    recorder.git_success(
        repository,
        &["cat-file", "-e", &format!("{ORACLE_COMMIT}^{{commit}}")],
        "P0_ORACLE_COMMIT_MISSING",
    )?;

    let receipt_line = recorder.git_text(
        repository,
        &["rev-list", "--parents", "-n", "1", RECEIPT_COMMIT],
        "P0_RECEIPT_GRAPH_INVALID",
    )?;
    require_exact_line(
        &receipt_line,
        &format!("{RECEIPT_COMMIT} {RECONCILIATION_COMMIT}"),
        "P0_RECEIPT_GRAPH_INVALID",
    )?;
    let reconciliation_line = recorder.git_text(
        repository,
        &["rev-list", "--parents", "-n", "1", RECONCILIATION_COMMIT],
        "P0_RECONCILIATION_GRAPH_INVALID",
    )?;
    require_exact_line(
        &reconciliation_line,
        &format!("{RECONCILIATION_COMMIT} {BASELINE_COMMIT}"),
        "P0_RECONCILIATION_GRAPH_INVALID",
    )?;
    let tree = recorder.git_text(
        repository,
        &["rev-parse", &format!("{RECONCILIATION_COMMIT}^{{tree}}")],
        "P0_RECONCILIATION_TREE_INVALID",
    )?;
    require_exact_line(&tree, RECONCILIATION_TREE, "P0_RECONCILIATION_TREE_INVALID")?;
    recorder.git_success(
        repository,
        &["merge-base", "--is-ancestor", RECEIPT_COMMIT, "HEAD"],
        "P0_RECEIPT_NOT_ANCESTOR",
    )?;
    recorder.git_success(
        repository,
        &["merge-base", "--is-ancestor", ORACLE_COMMIT, "HEAD"],
        "P0_ORACLE_NOT_ANCESTOR",
    )?;
    Ok(())
}

fn require_sealed_git_state(repository: &Path, recorder: &mut Recorder) -> Result<()> {
    let mut diff_args = vec!["diff", "--exit-code", BASELINE_COMMIT, "--"];
    diff_args.extend_from_slice(SEALED_PATHS);
    recorder.git_success(repository, &diff_args, "P0_SEALED_BYTES_CHANGED")?;

    let mut status_args = vec!["status", "--porcelain=v1", "--untracked-files=all", "--"];
    status_args.extend_from_slice(SEALED_PATHS);
    let status = recorder.git_text(repository, &status_args, "P0_SEALED_PATH_DIRTY")?;
    if !status.trim().is_empty() {
        return Err(XtaskError::integrity(
            "P0_SEALED_PATH_DIRTY",
            "sealed Phase 0 paths are dirty or contain additions",
        ));
    }
    recorder.git_success(
        repository,
        &[
            "diff",
            "--exit-code",
            RECEIPT_COMMIT,
            "--",
            "docs/receipts/P0.md",
        ],
        "P0_RECEIPT_CHANGED",
    )?;
    let status = recorder.git_text(
        repository,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            "docs/receipts/P0.md",
        ],
        "P0_RECEIPT_DIRTY",
    )?;
    if !status.trim().is_empty() {
        return Err(XtaskError::integrity(
            "P0_RECEIPT_DIRTY",
            "the signed Phase 0 receipt is dirty",
        ));
    }
    Ok(())
}

fn require_receipt(repository: &Path) -> Result<()> {
    let path = repository.join("docs/receipts/P0.md");
    hash::require_file(&path, RECEIPT_SHA256, "P0_RECEIPT_HASH_MISMATCH")?;
    let text = fs::read_to_string(&path).io_context(
        "P0_RECEIPT_READ_FAILED",
        "could not read the signed P0 receipt",
    )?;
    require_receipt_text(&text)
}

fn require_receipt_text(text: &str) -> Result<()> {
    require_unique_line(text, "Status: **PASS**", "P0_STATUS_INVALID")?;
    require_unique_line(
        text,
        "Machine evidence: **PASS**",
        "P0_MACHINE_STATUS_INVALID",
    )?;
    require_unique_line(
        text,
        "Technical approval: **APPROVED**",
        "P0_TECHNICAL_SUMMARY_INVALID",
    )?;
    require_unique_line(
        text,
        "Data-governance approval: **APPROVED**",
        "P0_GOVERNANCE_SUMMARY_INVALID",
    )?;
    require_unique_line(
        text,
        &format!("| Reviewed reconciliation commit | `{RECONCILIATION_COMMIT}` |"),
        "P0_RECONCILIATION_REFERENCE_INVALID",
    )?;
    require_unique_line(
        text,
        &format!("| Reviewed reconciliation tree | `{RECONCILIATION_TREE}` |"),
        "P0_RECONCILIATION_REFERENCE_INVALID",
    )?;
    require_unique_line(
        text,
        &format!("| Contract SHA-256 | `{CONTRACT_SHA256}` |"),
        "P0_CONTRACT_REFERENCE_INVALID",
    )?;
    require_unique_line(
        text,
        &format!("| Decision-ledger SHA-256 | `{LEDGER_SHA256}` |"),
        "P0_LEDGER_REFERENCE_INVALID",
    )?;
    require_unique_line(
        text,
        &format!("| `Cargo.lock` SHA-256 | `{HISTORICAL_CARGO_LOCK_SHA256}` |"),
        "P0_LOCK_REFERENCE_INVALID",
    )?;
    require_owner_section(text, "Technical approval", "### Data-governance approval")?;
    require_owner_section(text, "Data-governance approval", "Phase 0 is `PASS`")?;
    Ok(())
}

fn require_owner_section(text: &str, heading: &str, end_marker: &str) -> Result<()> {
    let marker = format!("### {heading}");
    if text.match_indices(&marker).count() != 1 {
        return Err(XtaskError::integrity(
            "P0_OWNER_SECTION_INVALID",
            format!("owner section is not unique: {heading}"),
        ));
    }
    let start = text.find(&marker).expect("count checked");
    let rest = &text[start + marker.len()..];
    let end = rest.find(end_marker).unwrap_or(rest.len());
    let section = &rest[..end];
    require_unique_line(section, "Decision: `APPROVE`", "P0_OWNER_DECISION_INVALID")?;
    require_unique_line(
        section,
        "Owner name: dhilipsiva",
        "P0_OWNER_IDENTITY_INVALID",
    )?;
    let expected_reference = format!(
        "Review reference/signature: Codex task 019fee98-6d38-7253-90a9-189b1fe7f04d; owner approval of reviewed commit `{RECONCILIATION_COMMIT}` and sealed run `{RUN_ID}`"
    );
    require_unique_line(section, &expected_reference, "P0_OWNER_SIGNATURE_INVALID")?;
    let timestamps: Vec<&str> = section
        .lines()
        .filter_map(|line| line.strip_prefix("UTC timestamp: "))
        .collect();
    if timestamps.len() != 1 || !is_utc_timestamp(timestamps[0]) {
        return Err(XtaskError::integrity(
            "P0_OWNER_TIMESTAMP_INVALID",
            format!("owner timestamp is missing, duplicated, or malformed: {heading}"),
        ));
    }
    Ok(())
}

fn require_contract(repository: &Path) -> Result<()> {
    let path = repository.join("docs/rebuild-contract.md");
    hash::require_file(&path, CONTRACT_SHA256, "P0_CONTRACT_HASH_MISMATCH")?;
    let bytes =
        fs::read(&path).io_context("P0_CONTRACT_READ_FAILED", "could not read the P0 contract")?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        XtaskError::integrity("P0_CONTRACT_ENCODING_INVALID", "P0 contract is not UTF-8")
    })?;
    if text.contains('\r') {
        return Err(XtaskError::integrity(
            "P0_CONTRACT_ENCODING_INVALID",
            "P0 contract is not canonical LF text",
        ));
    }
    let start_marker = "## Frozen Decision Ledger\n";
    let end_marker = "\n## Deferred Qualification Facts";
    let start = text.find(start_marker).ok_or_else(|| {
        XtaskError::integrity(
            "P0_LEDGER_RANGE_INVALID",
            "decision-ledger start marker is missing",
        )
    })?;
    let end = text[start..]
        .find(end_marker)
        .map(|offset| start + offset)
        .ok_or_else(|| {
            XtaskError::integrity(
                "P0_LEDGER_RANGE_INVALID",
                "decision-ledger end marker is missing",
            )
        })?;
    if hash::bytes(&bytes[start..end]) != LEDGER_SHA256 {
        return Err(XtaskError::integrity(
            "P0_LEDGER_HASH_MISMATCH",
            "the frozen decision-ledger byte-range hash changed",
        ));
    }
    Ok(())
}

fn require_machine_evidence(repository: &Path) -> Result<()> {
    let path = repository.join("docs/receipts/P0/evidence.json");
    hash::require_file(
        &path,
        MACHINE_EVIDENCE_SHA256,
        "P0_MACHINE_EVIDENCE_HASH_MISMATCH",
    )?;
    let value: Value = serde_json::from_slice(&fs::read(&path).io_context(
        "P0_MACHINE_EVIDENCE_READ_FAILED",
        "could not read P0 machine evidence",
    )?)
    .map_err(|error| {
        XtaskError::integrity(
            "P0_MACHINE_EVIDENCE_JSON_INVALID",
            format!("invalid P0 evidence JSON: {error}"),
        )
    })?;
    require_json_string(&value, "/schema", "python-slm-phase-evidence-v1")?;
    require_json_string(&value, "/phase", "P0")?;
    require_json_string(&value, "/status", "AWAITING_REVIEW")?;
    require_json_string(&value, "/authority/machine_evidence", "pass")?;
    require_json_string(
        &value,
        "/authority/phase_acceptance",
        "pending_owner_approval",
    )?;
    require_json_string(&value, "/capture/run_id", RUN_ID)?;
    require_json_string(&value, "/capture/seal/sha256", SEAL_SHA256)?;
    require_json_string(&value, "/contract/sha256_raw_file_bytes", CONTRACT_SHA256)?;
    require_json_string(&value, "/contract/decision_ledger_sha256", LEDGER_SHA256)?;
    require_json_string(
        &value,
        "/source/cargo_lock_sha256",
        HISTORICAL_CARGO_LOCK_SHA256,
    )?;
    Ok(())
}

fn require_json_string(value: &Value, pointer: &str, expected: &str) -> Result<()> {
    if value.pointer(pointer).and_then(Value::as_str) != Some(expected) {
        return Err(XtaskError::integrity(
            "P0_MACHINE_EVIDENCE_FIELD_INVALID",
            format!("P0 evidence field {pointer} is not {expected:?}"),
        ));
    }
    Ok(())
}

fn require_seal(repository: &Path) -> Result<()> {
    let run_root = repository.join("docs/receipts/P0/runs").join(RUN_ID);
    verify_run_seal(&run_root, SEAL_SHA256, 68)
}

fn verify_run_seal(
    run_root: &Path,
    expected_seal_sha256: &str,
    expected_entries: usize,
) -> Result<()> {
    let seal_path = run_root.join("SHA256SUMS");
    hash::require_file(&seal_path, expected_seal_sha256, "P0_SEAL_HASH_MISMATCH")?;
    let text = fs::read_to_string(&seal_path)
        .io_context("P0_SEAL_READ_FAILED", "could not read P0 SHA256SUMS")?;
    if text.contains('\r') || !text.ends_with('\n') {
        return Err(XtaskError::integrity(
            "P0_SEAL_FORMAT_INVALID",
            "P0 SHA256SUMS is not canonical LF text",
        ));
    }
    let mut manifest_paths = BTreeSet::new();
    let mut previous: Option<String> = None;
    for line in text.lines() {
        let (expected, relative) = line.split_once("  ").ok_or_else(|| {
            XtaskError::integrity("P0_SEAL_FORMAT_INVALID", "malformed P0 SHA256SUMS line")
        })?;
        if !hash::is_lower_sha256(expected)
            || relative.contains("  ")
            || !is_safe_relative(relative)
        {
            return Err(XtaskError::integrity(
                "P0_SEAL_FORMAT_INVALID",
                format!("invalid P0 SHA256SUMS entry: {line}"),
            ));
        }
        if previous.as_deref().is_some_and(|value| value >= relative) {
            return Err(XtaskError::integrity(
                "P0_SEAL_ORDER_INVALID",
                "P0 SHA256SUMS paths are not strictly sorted",
            ));
        }
        previous = Some(relative.to_owned());
        if !manifest_paths.insert(relative.to_owned()) {
            return Err(XtaskError::integrity(
                "P0_SEAL_DUPLICATE_PATH",
                format!("duplicate P0 seal path: {relative}"),
            ));
        }
        let path = run_root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        let canonical_root = fs::canonicalize(run_root)
            .io_context("P0_SEAL_PATH_INVALID", "could not canonicalize P0 run root")?;
        let canonical = fs::canonicalize(&path).io_context(
            "P0_SEAL_PATH_INVALID",
            format!("could not canonicalize sealed path {relative}"),
        )?;
        if !canonical.starts_with(&canonical_root) {
            return Err(XtaskError::integrity(
                "P0_SEAL_PATH_ESCAPE",
                format!("sealed path escapes the run root: {relative}"),
            ));
        }
        if fs::symlink_metadata(&path)
            .io_context("P0_SEAL_PATH_INVALID", "could not inspect sealed path")?
            .file_type()
            .is_symlink()
        {
            return Err(XtaskError::integrity(
                "P0_SEAL_SYMLINK_REJECTED",
                format!("sealed path is a symbolic link: {relative}"),
            ));
        }
        hash::require_file(&path, expected, "P0_SEAL_ENTRY_MISMATCH")?;
    }
    if manifest_paths.len() != expected_entries {
        return Err(XtaskError::integrity(
            "P0_SEAL_ENTRY_COUNT_INVALID",
            format!(
                "expected {expected_entries} P0 seal entries, observed {}",
                manifest_paths.len()
            ),
        ));
    }
    let mut actual_paths = BTreeSet::new();
    collect_files(run_root, run_root, &mut actual_paths)?;
    actual_paths.remove("SHA256SUMS");
    if actual_paths != manifest_paths {
        return Err(XtaskError::integrity(
            "P0_SEAL_COVERAGE_INVALID",
            "P0 SHA256SUMS does not cover exactly every run file except itself",
        ));
    }
    Ok(())
}

fn collect_files(root: &Path, directory: &Path, output: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(directory).io_context(
        "P0_SEAL_ENUMERATION_FAILED",
        format!("could not enumerate {}", directory.display()),
    )? {
        let entry = entry.io_context(
            "P0_SEAL_ENUMERATION_FAILED",
            "could not read a P0 run directory entry",
        )?;
        let metadata = entry.file_type().io_context(
            "P0_SEAL_ENUMERATION_FAILED",
            "could not inspect a P0 run entry",
        )?;
        if metadata.is_symlink() {
            return Err(XtaskError::integrity(
                "P0_SEAL_SYMLINK_REJECTED",
                format!(
                    "P0 run contains a symbolic link: {}",
                    entry.path().display()
                ),
            ));
        }
        if metadata.is_dir() {
            collect_files(root, &entry.path(), output)?;
        } else if metadata.is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .expect("entry is below root")
                .to_string_lossy()
                .replace('\\', "/");
            output.insert(relative);
        } else {
            return Err(XtaskError::integrity(
                "P0_SEAL_SPECIAL_FILE_REJECTED",
                format!(
                    "P0 run contains a non-regular file: {}",
                    entry.path().display()
                ),
            ));
        }
    }
    Ok(())
}

fn require_historical_lock(repository: &Path, recorder: &mut Recorder) -> Result<()> {
    let output = recorder.git_success(
        repository,
        &["show", &format!("{BASELINE_COMMIT}:Cargo.lock")],
        "P0_HISTORICAL_LOCK_MISSING",
    )?;
    let actual = hash::bytes(&output.stdout);
    if actual != HISTORICAL_CARGO_LOCK_SHA256 {
        return Err(XtaskError::integrity(
            "P0_HISTORICAL_LOCK_HASH_MISMATCH",
            format!("historical Cargo.lock hash changed: observed {actual}"),
        ));
    }
    Ok(())
}

fn require_historical_oracle(repository: &Path, recorder: &mut Recorder) -> Result<()> {
    let output = recorder.git_success(
        repository,
        &["show", &format!("{ORACLE_COMMIT}:TODO.md")],
        "P0_ORACLE_SOURCE_MISSING",
    )?;
    let text = std::str::from_utf8(&output.stdout).map_err(|_| {
        XtaskError::integrity(
            "P0_ORACLE_ENCODING_INVALID",
            "historical TODO.md is not UTF-8",
        )
    })?;
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    if lines.len() < 179 {
        return Err(XtaskError::integrity(
            "P0_ORACLE_RANGE_INVALID",
            format!(
                "historical TODO.md has only {} normalized lines",
                lines.len()
            ),
        ));
    }
    let oracle = format!("{}\n", lines[137..179].join("\n"));
    if oracle.len() != 3_388 || hash::bytes(oracle.as_bytes()) != ORACLE_NORMALIZED_SHA256 {
        return Err(XtaskError::integrity(
            "P0_ORACLE_HASH_MISMATCH",
            "historical TODO.md normalized lines 138..=179 do not match the approved verifier oracle",
        ));
    }
    if !oracle.starts_with("VERIFY:\n") || !oracle.ends_with("```\n") {
        return Err(XtaskError::integrity(
            "P0_ORACLE_RANGE_INVALID",
            "historical verifier oracle does not contain the exact VERIFY block boundary",
        ));
    }
    Ok(())
}

fn require_exact_line(actual: &str, expected: &str, code: &'static str) -> Result<()> {
    if actual.trim_end_matches(['\r', '\n']) != expected {
        return Err(XtaskError::integrity(
            code,
            format!("expected {expected:?}, observed {:?}", actual.trim()),
        ));
    }
    Ok(())
}

fn require_unique_line(text: &str, expected: &str, code: &'static str) -> Result<()> {
    if text.lines().filter(|line| *line == expected).count() != 1 {
        return Err(XtaskError::integrity(
            code,
            format!("required line is missing or duplicated: {expected}"),
        ));
    }
    Ok(())
}

fn is_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || *bytes.last().unwrap() != b'Z'
    {
        return false;
    }
    let whole_seconds = bytes.len() == 20 && bytes[19] == b'Z';
    let fractional = bytes.len() > 21
        && bytes[19] == b'.'
        && bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit);
    (whole_seconds || fractional)
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[11..13].iter().all(u8::is_ascii_digit)
        && bytes[14..16].iter().all(u8::is_ascii_digit)
        && bytes[17..19].iter().all(u8::is_ascii_digit)
}

fn is_safe_relative(value: &str) -> bool {
    if value.is_empty() || value.contains('\\') || value.starts_with('/') || value.contains(':') {
        return false;
    }
    let path = PathBuf::from(value);
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn rejects_unsafe_seal_paths() {
        for value in ["", "/absolute", "../escape", "a/../b", r"a\b", "C:/escape"] {
            assert!(!is_safe_relative(value), "accepted {value:?}");
        }
        assert!(is_safe_relative("commands/C01.stdout.txt"));
    }

    #[test]
    fn timestamps_are_strict_utc_with_optional_fraction() {
        assert!(is_utc_timestamp("2026-08-11T11:04:55Z"));
        assert!(is_utc_timestamp("2026-08-11T11:04:55.123456789Z"));
        assert!(!is_utc_timestamp("2026-08-11T11:04:55+00:00"));
        assert!(!is_utc_timestamp("2026-08-11T11:04:55.Z"));
        assert!(!is_utc_timestamp("2026-08-11T11:04Z"));
    }

    #[test]
    fn current_historical_chain_passes_the_rust_port() {
        let mut recorder = Recorder::default();
        verify(&repository(), &mut recorder).unwrap();
        assert!(recorder.commands().len() >= 16);
    }

    #[test]
    fn duplicate_owner_or_status_claim_is_rejected() {
        let receipt = fs::read_to_string(repository().join("docs/receipts/P0.md")).unwrap();
        let duplicated_status = format!("{receipt}\nStatus: **PASS**\n");
        assert_eq!(
            require_receipt_text(&duplicated_status).unwrap_err().code,
            "P0_STATUS_INVALID"
        );
        let duplicated_owner = receipt.replacen(
            "### Data-governance approval",
            "### Technical approval\n\n### Data-governance approval",
            1,
        );
        assert_eq!(
            require_receipt_text(&duplicated_owner).unwrap_err().code,
            "P0_OWNER_SECTION_INVALID"
        );
    }

    #[test]
    fn seal_rejects_single_byte_mutation_and_extra_file() {
        let source = repository().join("docs/receipts/P0/runs").join(RUN_ID);
        let temp = tempfile::tempdir().unwrap();
        copy_tree(&source, temp.path());
        verify_run_seal(temp.path(), SEAL_SHA256, 68).unwrap();
        let evidence = temp.path().join("command-results.json");
        let mut bytes = fs::read(&evidence).unwrap();
        bytes[0] ^= 1;
        fs::write(&evidence, bytes).unwrap();
        assert_eq!(
            verify_run_seal(temp.path(), SEAL_SHA256, 68)
                .unwrap_err()
                .code,
            "P0_SEAL_ENTRY_MISMATCH"
        );
        fs::remove_dir_all(temp.path()).unwrap();
        fs::create_dir_all(temp.path()).unwrap();
        copy_tree(&source, temp.path());
        fs::write(temp.path().join("extra.txt"), b"extra").unwrap();
        assert_eq!(
            verify_run_seal(temp.path(), SEAL_SHA256, 68)
                .unwrap_err()
                .code,
            "P0_SEAL_COVERAGE_INVALID"
        );
    }

    fn copy_tree(source: &Path, destination: &Path) {
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                fs::create_dir_all(&target).unwrap();
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}
