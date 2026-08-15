use crate::error::{ProductError, Result};
use crate::model::canonical_plan;
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::ffi::OsString;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "python-slm", version, disable_help_subcommand = true)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the immutable canonical arithmetic for the prototype.
    Plan,
    /// Emit deterministic canonical initialization and CPU-oracle diagnostics.
    ModelOracle,
    /// Curate the governed source corpus. Implemented by Phase 4.
    Curate {
        #[arg(long)]
        config: PathBuf,
    },
    /// Train the canonical deterministic byte-level tokenizer.
    TrainTokenizer {
        #[arg(long)]
        config: PathBuf,
    },
    /// Materialize the governed token corpus. Implemented by Phase 8.
    Tokenize {
        #[arg(long)]
        config: PathBuf,
    },
    /// Deduplicate, decontaminate, split, and prepare governed corpus manifests.
    PrepareCorpus {
        #[arg(long)]
        config: PathBuf,
    },
    /// Materialize the deterministic SPAN-001 order over a verified token corpus.
    PlanSpans {
        #[arg(long)]
        config: PathBuf,
    },
    /// Inspect an immutable artifact. Implemented by the owning artifact phase.
    Inspect {
        #[arg(long)]
        config: PathBuf,
    },
    /// Run the bounded benchmark suite. Implemented by the performance phases.
    Bench {
        #[arg(long)]
        config: PathBuf,
    },
    /// Run canonical training. Implemented by the training phases.
    Train {
        #[arg(long)]
        config: PathBuf,
    },
}

pub fn entry(args: impl IntoIterator<Item = OsString>) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| run(args))).unwrap_or_else(|_| {
        Err(ProductError::internal(
            "UNEXPECTED_PANIC",
            "the product command panicked before producing a result",
        ))
    });
    match result {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string(&value).expect("JSON Value serialization cannot fail")
            );
            0
        }
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::to_string(&error).unwrap_or_else(|_| {
                    "{\"schema\":\"python-slm-error-v1\",\"code\":\"ERROR_SERIALIZATION_FAILED\",\"category\":\"internal\",\"message\":\"error serialization failed\",\"remediation\":\"Inspect the Rust implementation.\"}".to_owned()
                })
            );
            error.exit_code()
        }
    }
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<Value> {
    let arguments = Arguments::try_parse_from(args)
        .map_err(|error| ProductError::usage("ARGUMENTS_INVALID", error.to_string()))?;
    match arguments.command {
        Command::Plan => serde_json::to_value(canonical_plan()).map_err(|error| {
            ProductError::internal(
                "RESULT_SERIALIZATION_FAILED",
                format!("canonical plan serialization failed: {error}"),
            )
        }),
        Command::ModelOracle => crate::model::oracle_result_value(),
        Command::Curate { config } => crate::data::curate(&config),
        Command::TrainTokenizer { config } => crate::tokenizer::train_tokenizer(&config),
        Command::Tokenize { config } => crate::storage::tokenize(&config),
        Command::PrepareCorpus { config } => crate::corpus::prepare(&config),
        Command::PlanSpans { config } => crate::corpus::plan_spans(&config),
        Command::Inspect { config } => deferred("future artifact phase", "inspect", config),
        Command::Bench { config } => deferred("performance phase", "bench", config),
        Command::Train { config } => deferred("training phase", "train", config),
    }
}

fn deferred(phase: &str, command: &str, _config: PathBuf) -> Result<Value> {
    Err(ProductError::gate(
        "PHASE_NOT_IMPLEMENTED",
        format!("{command} is installed but remains unavailable until {phase}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_remains_implemented() {
        let value = run(["python-slm".into(), "plan".into()]).unwrap();
        assert_eq!(value["schema"], "python-slm-plan-result-v1");
        assert_eq!(value["status"], "PLANNED");
        assert_eq!(value["model"]["identity"], "gqa-135m-v1");
        assert_eq!(value["model"]["parameters"], 135_285_504);
    }

    #[test]
    fn model_oracle_command_is_installed_without_running_the_full_stream() {
        let arguments = Arguments::try_parse_from(["python-slm", "model-oracle"]).unwrap();
        assert!(matches!(arguments.command, Command::ModelOracle));
    }

    #[test]
    fn every_future_command_fails_with_the_same_typed_gate() {
        for command in ["inspect", "bench", "train"] {
            let error = run([
                "python-slm".into(),
                command.into(),
                "--config".into(),
                "must-not-be-read.json".into(),
            ])
            .unwrap_err();
            assert_eq!(error.code, "PHASE_NOT_IMPLEMENTED");
            assert_eq!(error.exit_code(), 5);
        }
    }

    #[cfg(windows)]
    #[test]
    fn tokenize_is_active_and_reads_its_explicit_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.json").into_os_string();
        let error = run([
            "python-slm".into(),
            "tokenize".into(),
            "--config".into(),
            missing,
        ])
        .unwrap_err();
        assert_eq!(error.code, "TOKEN_CONFIG_READ_FAILED");
        assert_eq!(error.exit_code(), 4);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn clap_errors_use_the_product_usage_contract() {
        let error = run(["python-slm".into(), "unknown".into()]).unwrap_err();
        assert_eq!(error.code, "ARGUMENTS_INVALID");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn panic_boundary_maps_to_the_internal_category() {
        let result = catch_unwind(AssertUnwindSafe(|| panic!("synthetic")));
        let error = result
            .map(|_| ())
            .map_err(|_| ProductError::internal("UNEXPECTED_PANIC", "synthetic"))
            .unwrap_err();
        assert_eq!(error.exit_code(), 1);
    }
}
