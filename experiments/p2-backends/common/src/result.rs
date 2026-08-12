use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::{
    CandidateMode, ElementMetrics, FixtureHashes, OracleResult, ScalarMetrics, Tolerance, Workload,
};

pub const CANDIDATE_RESULT_SCHEMA: &str = "python-slm-backend-candidate-result-v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ResultStatus {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "FAIL")]
    Fail,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateResult {
    pub schema: String,
    pub candidate_id: String,
    pub mode: CandidateMode,
    pub status: ResultStatus,
    pub workload: Workload,
    pub fixture_hashes: Option<FixtureHashes>,
    pub allocation: Option<AllocationResult>,
    pub correctness: Option<CorrectnessResult>,
    pub timing: Option<TimingResult>,
    pub memory: Option<MemoryResult>,
    pub provenance: Provenance,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShapeResult {
    pub m: u64,
    pub k: u64,
    pub n: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationResult {
    pub shape: [u64; 3],
    pub elements: u64,
    pub input_sha256: String,
    pub output_sha256: String,
    pub bitwise_equal: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectnessResult {
    pub shape: ShapeResult,
    pub accumulation: String,
    pub output_dtype: String,
    pub loss_dtype: String,
    pub forward: ElementMetrics,
    pub loss: ScalarMetrics,
    pub grad_a: ElementMetrics,
    pub grad_b: ElementMetrics,
    pub nan_count: u64,
    pub infinite_count: u64,
    pub envelope_violation_count: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimingResult {
    pub shape: ShapeResult,
    pub warmup_iterations: u64,
    pub forward: TimingMeasurement,
    pub forward_backward: TimingMeasurement,
    pub context_ns: u64,
    pub jit_ns: u64,
    pub first_result_ns: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimingMeasurement {
    pub samples_ns: Vec<u64>,
    pub sample_count: u64,
    pub elapsed_ns: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub flop_count: u64,
    pub gflops: f64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryResult {
    pub free_bytes_after_context: Option<u64>,
    pub free_bytes_after_allocation: Option<u64>,
    pub free_bytes_after_forward: Option<u64>,
    pub free_bytes_after_backward: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub crate_name: String,
    pub crate_version: String,
    pub feature_set: Vec<String>,
    pub device: String,
    pub device_ordinal: Option<u64>,
    pub explicit_synchronization: bool,
    pub fp32_accumulation_evidence: String,
    pub framework_rng_used: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
}

impl CandidateResult {
    pub fn empty(
        candidate_id: &str,
        mode: CandidateMode,
        workload: Workload,
        provenance: Provenance,
    ) -> Self {
        Self {
            schema: CANDIDATE_RESULT_SCHEMA.to_owned(),
            candidate_id: candidate_id.to_owned(),
            mode,
            status: ResultStatus::Fail,
            workload,
            fixture_hashes: None,
            allocation: None,
            correctness: None,
            timing: None,
            memory: None,
            provenance,
            diagnostics: Vec::new(),
        }
    }
}

pub fn assess_correctness(
    shape: ShapeResult,
    actual_y: &[f64],
    actual_loss: f64,
    actual_grad_a: &[f64],
    actual_grad_b: &[f64],
    oracle: &OracleResult,
) -> CorrectnessResult {
    let forward = ElementMetrics::evaluate(actual_y, &oracle.y, Tolerance::FORWARD);
    let loss = ScalarMetrics::loss(actual_loss, oracle.loss);
    let grad_a = ElementMetrics::evaluate(actual_grad_a, &oracle.grad_a, Tolerance::GRADIENT);
    let grad_b = ElementMetrics::evaluate(actual_grad_b, &oracle.grad_b, Tolerance::GRADIENT);
    let values = actual_y
        .iter()
        .chain(actual_grad_a)
        .chain(actual_grad_b)
        .copied()
        .chain(std::iter::once(actual_loss));
    let mut nan_count = 0_u64;
    let mut infinite_count = 0_u64;
    for value in values {
        nan_count += u64::from(value.is_nan());
        infinite_count += u64::from(value.is_infinite());
    }
    let envelope_violation_count = forward
        .envelope_violation_count
        .saturating_add(grad_a.envelope_violation_count)
        .saturating_add(grad_b.envelope_violation_count)
        .saturating_add(u64::from(!loss.passed));
    CorrectnessResult {
        shape,
        accumulation: "fp32".to_owned(),
        output_dtype: "bf16".to_owned(),
        loss_dtype: "fp32".to_owned(),
        forward,
        loss,
        grad_a,
        grad_b,
        nan_count,
        infinite_count,
        envelope_violation_count,
    }
}

impl CorrectnessResult {
    pub fn passes(&self) -> bool {
        self.nan_count == 0
            && self.infinite_count == 0
            && self.envelope_violation_count == 0
            && self.forward.passes(Tolerance::FORWARD)
            && self.loss.passed
            && self.grad_a.passes(Tolerance::GRADIENT)
            && self.grad_b.passes(Tolerance::GRADIENT)
    }
}

pub fn write_result(path: &Path, result: &CandidateResult) -> Result<String, String> {
    if path.exists() {
        return Err(format!("refusing to overwrite {}", path.display()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "output path has no parent".to_owned())?;
    if !parent.is_dir() {
        return Err(format!(
            "output parent {} is not a directory",
            parent.display()
        ));
    }
    let mut bytes = serde_json::to_vec(result).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "output filename must be valid UTF-8".to_owned())?;
    let temporary = parent.join(format!(".{filename}.tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    drop(file);
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_rejects_unknown_fields_when_deserializing() {
        let json = r#"{
          "schema":"python-slm-backend-candidate-result-v1",
          "candidate_id":"candidate","mode":"cpu-smoke","status":"FAIL",
          "workload":"correctness","fixture_hashes":null,"allocation":null,
          "correctness":null,"timing":null,"memory":null,
          "provenance":{"crate_name":"x","crate_version":"1","feature_set":[],
            "device":"cpu","device_ordinal":null,"explicit_synchronization":false,
            "fp32_accumulation_evidence":"none","framework_rng_used":false},
          "diagnostics":[],"unknown":true
        }"#;
        assert!(serde_json::from_str::<CandidateResult>(json).is_err());
    }
}
