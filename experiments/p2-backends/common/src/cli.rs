use std::{collections::BTreeMap, env, path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::fixture::Workload;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateMode {
    CpuSmoke,
    Correctness,
    Benchmark,
}

impl FromStr for CandidateMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "cpu-smoke" => Ok(Self::CpuSmoke),
            "correctness" => Ok(Self::Correctness),
            "benchmark" => Ok(Self::Benchmark),
            _ => Err(format!("unsupported mode {value:?}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateArgs {
    pub mode: CandidateMode,
    pub workload: Workload,
    pub fixture_dir: PathBuf,
    pub output: PathBuf,
}

impl CandidateArgs {
    pub fn from_env() -> Result<Self, String> {
        Self::parse(env::args_os().skip(1))
    }

    pub fn parse<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<std::ffi::OsString>,
    {
        let args = args
            .into_iter()
            .map(Into::into)
            .collect::<Vec<std::ffi::OsString>>();
        if args.len() != 8 {
            return Err(
                "expected exactly --mode <value> --workload <value> --fixture-dir <path> --output <path>"
                    .to_owned(),
            );
        }

        let mut values = BTreeMap::<String, std::ffi::OsString>::new();
        for pair in args.chunks_exact(2) {
            let flag = pair[0]
                .to_str()
                .ok_or_else(|| "flag is not valid UTF-8".to_owned())?;
            if !matches!(flag, "--mode" | "--workload" | "--fixture-dir" | "--output") {
                return Err(format!("unknown flag {flag:?}"));
            }
            if values.insert(flag.to_owned(), pair[1].clone()).is_some() {
                return Err(format!("duplicate flag {flag:?}"));
            }
        }

        let utf8 = |name: &str| -> Result<&str, String> {
            values
                .get(name)
                .ok_or_else(|| format!("missing {name}"))?
                .to_str()
                .ok_or_else(|| format!("{name} is not valid UTF-8"))
        };
        let mode = utf8("--mode")?.parse()?;
        let workload = utf8("--workload")?.parse()?;
        match (mode, workload) {
            (CandidateMode::CpuSmoke, Workload::Correctness)
            | (CandidateMode::Correctness, Workload::Allocation | Workload::Correctness)
            | (CandidateMode::Benchmark, Workload::Projection | Workload::FfnExpansion) => {}
            _ => {
                return Err(format!(
                    "workload {workload} is not valid for mode {}",
                    match mode {
                        CandidateMode::CpuSmoke => "cpu-smoke",
                        CandidateMode::Correctness => "correctness",
                        CandidateMode::Benchmark => "benchmark",
                    }
                ));
            }
        }

        Ok(Self {
            mode,
            workload,
            fixture_dir: PathBuf::from(values.remove("--fixture-dir").expect("validated flag")),
            output: PathBuf::from(values.remove("--output").expect("validated flag")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Vec<&'static str> {
        vec![
            "--mode",
            "correctness",
            "--workload",
            "allocation",
            "--fixture-dir",
            "fixtures",
            "--output",
            "result.json",
        ]
    }

    #[test]
    fn parses_flags_independent_of_order() {
        let parsed = CandidateArgs::parse(valid()).unwrap();
        assert_eq!(parsed.mode, CandidateMode::Correctness);
        assert_eq!(parsed.workload, Workload::Allocation);
    }

    #[test]
    fn rejects_unknown_duplicate_and_extra_flags() {
        let mut unknown = valid();
        unknown[0] = "--unknown";
        assert!(CandidateArgs::parse(unknown).is_err());

        let mut duplicate = valid();
        duplicate[2] = "--mode";
        assert!(CandidateArgs::parse(duplicate).is_err());

        let mut extra = valid();
        extra.extend(["--extra", "value"]);
        assert!(CandidateArgs::parse(extra).is_err());
    }

    #[test]
    fn rejects_incompatible_mode_and_workload() {
        let mut args = valid();
        args[1] = "benchmark";
        assert!(CandidateArgs::parse(args).is_err());
    }
}
