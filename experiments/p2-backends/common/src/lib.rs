#![forbid(unsafe_code)]

pub mod cli;
pub mod fixture;
pub mod metrics;
pub mod oracle;
pub mod result;
pub mod timing;

pub use cli::{CandidateArgs, CandidateMode};
pub use fixture::{Fixture, FixtureHashes, FixtureManifest, Workload, WorkloadShape, sha256_bytes};
pub use metrics::{ElementMetrics, ScalarMetrics, Tolerance};
pub use oracle::{OracleResult, evaluate_oracle};
pub use result::{
    AllocationResult, CandidateResult, CorrectnessResult, Diagnostic, MemoryResult, Provenance,
    ResultStatus, ShapeResult, TimingMeasurement, TimingResult, assess_correctness, write_result,
};
