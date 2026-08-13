use crate::error::{Category, Result, XtaskError};
use crate::{p0, p0a};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::PathBuf;

const PROTOTYPE_PROFILE: &str = "prototype-windows-5090-v1";

#[derive(Debug, Parser)]
#[command(name = "xtask", disable_help_subcommand = true)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Verify the immutable, owner-approved Phase 0 chain.
    VerifyP0,
    /// Prepare, finalize, or validate a phase receipt.
    VerifyPhase {
        #[arg(long)]
        phase: String,
        #[arg(long, default_value = "docs/receipts/P0A")]
        output_root: PathBuf,
        #[arg(long, conflicts_with = "finalize")]
        check_selected: bool,
        #[arg(long, conflicts_with = "check_selected")]
        finalize: bool,
        #[arg(long)]
        profile: Option<String>,
    },
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<Value> {
    let arguments = Arguments::try_parse_from(args).map_err(|error| {
        XtaskError::new(
            "ARGUMENTS_INVALID",
            Category::Usage,
            error.to_string(),
            "Use `cargo run --locked -p xtask --bin xtask -- --help`.",
        )
    })?;
    let repository = std::env::current_dir().map_err(|error| {
        XtaskError::environment(
            "CURRENT_DIRECTORY_FAILED",
            format!("could not read the current directory: {error}"),
        )
    })?;
    match arguments.command {
        Command::VerifyP0 => {
            let mut recorder = crate::process::Recorder::default();
            let identity = p0::verify(&repository, &mut recorder)?;
            Ok(json!({
                "schema": "python-slm-verify-p0-result-v1",
                "phase_id": "P0",
                "status": "PASS",
                "identity": identity,
                "commands_executed": recorder.commands().len()
            }))
        }
        Command::VerifyPhase {
            phase,
            output_root,
            check_selected,
            finalize,
            profile,
        } => {
            if phase != "P0A" {
                return Err(XtaskError::gate(
                    "PHASE_NOT_INSTALLED",
                    format!("phase {phase} is rejected until its implementing phase installs it"),
                    "Complete the preceding phase; do not substitute a historical verifier.",
                ));
            }
            if let Some(profile) = profile
                && profile != PROTOTYPE_PROFILE
            {
                return Err(XtaskError::gate(
                    "DEFERRED_POST_P16",
                    format!("profile {profile} is designed but not implemented before P16A"),
                    "Use prototype-windows-5090-v1 or wait for the post-P16 portability phases.",
                ));
            }
            if check_selected {
                p0a::check_selected(&repository, &output_root)
            } else if finalize {
                p0a::finalize(&repository, &output_root)
            } else {
                p0a::prepare(&repository, &output_root)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_uninstalled_phase() {
        let error = run([
            OsString::from("xtask"),
            OsString::from("verify-phase"),
            OsString::from("--phase"),
            OsString::from("P1A"),
        ])
        .unwrap_err();
        assert_eq!(error.code, "PHASE_NOT_INSTALLED");
    }

    #[test]
    fn defers_nonprototype_profile_without_discovery() {
        let error = run([
            OsString::from("xtask"),
            OsString::from("verify-phase"),
            OsString::from("--phase"),
            OsString::from("P0A"),
            OsString::from("--profile"),
            OsString::from("linux-amd"),
        ])
        .unwrap_err();
        assert_eq!(error.code, "DEFERRED_POST_P16");
        assert_eq!(error.exit_code(), 5);
    }

    #[test]
    fn invalid_output_root_is_usage_error() {
        let error = run([
            OsString::from("xtask"),
            OsString::from("verify-phase"),
            OsString::from("--phase"),
            OsString::from("P0A"),
            OsString::from("--output-root"),
            OsString::from("elsewhere"),
        ])
        .unwrap_err();
        assert_eq!(error.code, "OUTPUT_ROOT_INVALID");
        assert_eq!(error.exit_code(), 2);
    }
}
