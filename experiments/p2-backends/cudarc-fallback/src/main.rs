#![deny(unsafe_op_in_unsafe_fn)]

use p2_backend_common::{
    CandidateArgs, CandidateMode, CandidateResult, Diagnostic, Provenance, ResultStatus,
    write_result,
};

fn main() {
    let args = CandidateArgs::from_env().unwrap_or_else(|message| fail_usage(&message));
    let mut result =
        CandidateResult::empty("cudarc-fallback", args.mode, args.workload, provenance());
    let outcome = match args.mode {
        CandidateMode::CpuSmoke => cpu_smoke(&args, &mut result),
        CandidateMode::Correctness | CandidateMode::Benchmark => cuda::run(&args, &mut result),
    };
    if let Err(message) = outcome {
        result.status = ResultStatus::Fail;
        result.diagnostics.push(Diagnostic {
            code: "CANDIDATE_EXECUTION_FAILED".to_owned(),
            message,
        });
    }
    let json =
        write_result(&args.output, &result).unwrap_or_else(|message| fail_internal(&message));
    print!("{json}");
    if result.status == ResultStatus::Fail {
        std::process::exit(5);
    }
}

fn provenance() -> Provenance {
    Provenance {
        crate_name: "cudarc".to_owned(),
        crate_version: "0.19.8".to_owned(),
        feature_set: if cfg!(feature = "cuda") {
            vec!["cublas", "cublaslt", "cuda-13010", "driver"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        } else {
            Vec::new()
        },
        device: if cfg!(feature = "cuda") {
            "CUDA device 0".to_owned()
        } else {
            "CPU".to_owned()
        },
        device_ordinal: cfg!(feature = "cuda").then_some(0),
        explicit_synchronization: cfg!(feature = "cuda"),
        fp32_accumulation_evidence: "cuBLASLt CUDA_R_16BF inputs with CUBLAS_COMPUTE_32F"
            .to_owned(),
        framework_rng_used: false,
    }
}

fn cpu_smoke(args: &CandidateArgs, result: &mut CandidateResult) -> Result<(), String> {
    let fixture = p2_backend_common::fixture::load(&args.fixture_dir, args.workload)
        .map_err(|error| error.to_string())?;
    result.fixture_hashes = Some(fixture.hashes());
    result.status = ResultStatus::Pass;
    result.provenance.explicit_synchronization = true;
    Ok(())
}

#[cfg(feature = "cuda")]
mod cuda;

#[cfg(not(feature = "cuda"))]
mod cuda {
    use super::*;

    pub fn run(_args: &CandidateArgs, _result: &mut CandidateResult) -> Result<(), String> {
        Err("CUDA diagnostic requires --features cuda".to_owned())
    }
}

fn fail_usage(message: &str) -> ! {
    eprintln!("{}", serde_json::json!({"code":"USAGE","message":message}));
    std::process::exit(2)
}

fn fail_internal(message: &str) -> ! {
    eprintln!(
        "{}",
        serde_json::json!({"code":"INTERNAL","message":message})
    );
    std::process::exit(1)
}
