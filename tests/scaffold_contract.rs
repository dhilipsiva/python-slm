use serde_json::Value;
use std::process::Command;

#[test]
fn canonical_plan_is_one_compact_json_line() {
    let output = Command::new(env!("CARGO_BIN_EXE_python-slm"))
        .arg("plan")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let value: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["schema"], "python-slm-plan-result-v1");
    assert_eq!(value["qualification_status"], "SKIPPED");
    assert_eq!(value["model"]["identity"], "gqa-135m-v1");
    assert_eq!(value["model"]["parameters"], 135_285_504);
    assert_eq!(value["tokens"]["valid_training_targets"], 2_000_000_000_u64);
    assert_eq!(value["tokens"]["total_updates"], 30_518);
}

#[test]
fn deferred_command_does_not_read_config_or_write_the_working_directory() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing.json");
    let output = Command::new(env!("CARGO_BIN_EXE_python-slm"))
        .current_dir(directory.path())
        .args(["tokenize", "--config"])
        .arg(&missing)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1);
    let value: Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["schema"], "python-slm-error-v1");
    assert_eq!(value["code"], "PHASE_NOT_IMPLEMENTED");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn invalid_arguments_are_typed_usage_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_python-slm"))
        .arg("does-not-exist")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["code"], "ARGUMENTS_INVALID");
    assert_eq!(value["category"], "usage");
}
