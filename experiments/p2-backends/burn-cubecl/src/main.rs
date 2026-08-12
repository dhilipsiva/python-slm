#![forbid(unsafe_code)]

use p2_backend_common::{
    CandidateArgs, CandidateMode, CandidateResult, Diagnostic, Provenance, ResultStatus,
    write_result,
};

fn main() {
    let args = CandidateArgs::from_env().unwrap_or_else(|message| fail_usage(&message));
    let mut result = CandidateResult::empty("burn-cubecl", args.mode, args.workload, provenance());
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
        crate_name: "burn".to_owned(),
        crate_version: "0.21.0".to_owned(),
        feature_set: if cfg!(feature = "cuda") {
            vec!["autodiff", "autotune", "cuda", "fusion"]
        } else {
            vec!["autodiff", "flex"]
        }
        .into_iter()
        .map(str::to_owned)
        .collect(),
        device: if cfg!(feature = "cuda") {
            "CUDA device 0".to_owned()
        } else {
            "CPU".to_owned()
        },
        device_ordinal: cfg!(feature = "cuda").then_some(0),
        explicit_synchronization: cfg!(feature = "cuda"),
        fp32_accumulation_evidence: "unverified-until-cuda-run".to_owned(),
        framework_rng_used: false,
    }
}

fn cpu_smoke(args: &CandidateArgs, result: &mut CandidateResult) -> Result<(), String> {
    #[cfg(feature = "cpu")]
    {
        use burn::{
            backend::{Autodiff, Flex},
            prelude::Backend,
            tensor::{Tensor, TensorData},
        };

        type Cpu = Autodiff<Flex>;
        let fixture = p2_backend_common::fixture::load(&args.fixture_dir, args.workload)
            .map_err(|error| error.to_string())?;
        let device = Default::default();
        let values = fixture
            .a
            .iter()
            .take(17 * 31)
            .map(|value| value.to_f32())
            .collect::<Vec<_>>();
        let tensor = Tensor::<Cpu, 2>::from_data(TensorData::new(values, [17, 31]), &device);
        let roundtrip = tensor
            .try_into_data()
            .map_err(|error| error.to_string())?
            .to_vec::<f32>()
            .map_err(|error| error.to_string())?;
        if roundtrip.len() != 17 * 31 {
            return Err("Burn CPU tensor round-trip length mismatch".to_owned());
        }
        Cpu::sync(&device).map_err(|error| error.to_string())?;
        result.fixture_hashes = Some(fixture.hashes());
        result.status = ResultStatus::Pass;
        result.provenance.explicit_synchronization = true;
        return Ok(());
    }
    #[cfg(not(feature = "cpu"))]
    {
        let _ = (args, result);
        Err("cpu-smoke requires the cpu feature".to_owned())
    }
}

#[cfg(feature = "cuda")]
mod cuda;

#[cfg(not(feature = "cuda"))]
mod cuda {
    use super::*;

    pub fn run(_args: &CandidateArgs, _result: &mut CandidateResult) -> Result<(), String> {
        Err("CUDA qualification requires --no-default-features --features cuda".to_owned())
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
