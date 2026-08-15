use crate::error::{Category, Result, XtaskError};
use crate::{p0, p0a, p1a, p1b};
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
    /// Compile, inspect, and execute the non-publishing prototype CUDA probe.
    ProbeCuda {
        #[arg(long)]
        cuda_root: Option<PathBuf>,
        #[arg(long)]
        vs_instance_id: Option<String>,
        #[arg(long)]
        device_uuid: Option<String>,
    },
    /// Qualify an implemented host or accelerator environment profile.
    VerifyEnv {
        #[arg(long)]
        mode: String,
        #[arg(long)]
        profile: String,
        #[arg(long)]
        provider: Option<String>,
        /// Select one exact VS 2022 instance when discovery is otherwise ambiguous.
        #[arg(long, conflicts_with = "check_selected")]
        vs_instance_id: Option<String>,
        /// Revalidate the committed immutable selected P1A chain without probing the host.
        #[arg(long)]
        check_selected: bool,
        #[arg(long, default_value = "docs/receipts/P1A-prototype-v2")]
        output_root: PathBuf,
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
    match arguments.command {
        Command::VerifyP0 => {
            let repository = current_repository()?;
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
            let repository = current_repository()?;
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
        Command::ProbeCuda {
            cuda_root,
            vs_instance_id,
            device_uuid,
        } => p1b::probe(p1b::ProbeOptions {
            cuda_root,
            vs_instance_id,
            device_uuid,
        }),
        Command::VerifyEnv {
            mode,
            profile,
            provider,
            vs_instance_id,
            check_selected,
            output_root,
        } => {
            require_p1a_host_selection(&mode, &profile, provider.as_deref())?;
            let repository = current_repository()?;
            if check_selected {
                p1a::check_selected(&repository, &output_root)
            } else {
                p1a::qualify(&repository, &output_root, vs_instance_id.as_deref())
            }
        }
    }
}

fn current_repository() -> Result<PathBuf> {
    std::env::current_dir().map_err(|error| {
        XtaskError::environment(
            "CURRENT_DIRECTORY_FAILED",
            format!("could not read the current directory: {error}"),
        )
    })
}

fn require_p1a_host_selection(mode: &str, profile: &str, provider: Option<&str>) -> Result<()> {
    if mode == "host" && profile == PROTOTYPE_PROFILE && provider.is_none() {
        return Ok(());
    }

    let provider = provider.unwrap_or("none");
    Err(XtaskError::gate(
        "DEFERRED_POST_P16",
        format!(
            "environment tuple mode={mode}, profile={profile}, provider={provider} is designed but not implemented by P1A"
        ),
        "Use --mode host --profile prototype-windows-5090-v1 without --provider, or wait for the phase that implements the requested tuple.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_cuda_cli_has_only_nonpublishing_overrides() {
        let parsed = Arguments::try_parse_from([
            "xtask",
            "probe-cuda",
            "--cuda-root",
            r"C:\CUDA",
            "--vs-instance-id",
            "vs-17",
            "--device-uuid",
            "GPU-00000000-0000-0000-0000-000000000001",
        ])
        .unwrap();
        match parsed.command {
            Command::ProbeCuda {
                cuda_root,
                vs_instance_id,
                device_uuid,
            } => {
                assert_eq!(cuda_root.unwrap(), PathBuf::from(r"C:\CUDA"));
                assert_eq!(vs_instance_id.as_deref(), Some("vs-17"));
                assert_eq!(
                    device_uuid.as_deref(),
                    Some("GPU-00000000-0000-0000-0000-000000000001")
                );
            }
            _ => panic!("probe-cuda parsed as the wrong command"),
        }
    }

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

    #[test]
    fn defers_accelerator_mode_before_output_root_validation() {
        let error = run([
            OsString::from("xtask"),
            OsString::from("verify-env"),
            OsString::from("--mode"),
            OsString::from("accelerator"),
            OsString::from("--profile"),
            OsString::from(PROTOTYPE_PROFILE),
            OsString::from("--provider"),
            OsString::from("cuda"),
            OsString::from("--output-root"),
            OsString::from("not-the-p1a-root"),
        ])
        .unwrap_err();
        assert_eq!(error.code, "DEFERRED_POST_P16");
        assert_eq!(error.exit_code(), 5);
    }

    #[test]
    fn defers_nonprototype_host_before_output_root_validation() {
        let error = run([
            OsString::from("xtask"),
            OsString::from("verify-env"),
            OsString::from("--mode"),
            OsString::from("host"),
            OsString::from("--profile"),
            OsString::from("portable-interface-v2"),
            OsString::from("--output-root"),
            OsString::from("not-the-p1a-root"),
        ])
        .unwrap_err();
        assert_eq!(error.code, "DEFERRED_POST_P16");
    }

    #[test]
    fn defers_provider_on_host_mode_before_output_root_validation() {
        let error = run([
            OsString::from("xtask"),
            OsString::from("verify-env"),
            OsString::from("--mode"),
            OsString::from("host"),
            OsString::from("--profile"),
            OsString::from(PROTOTYPE_PROFILE),
            OsString::from("--provider"),
            OsString::from("cuda"),
            OsString::from("--output-root"),
            OsString::from("not-the-p1a-root"),
        ])
        .unwrap_err();
        assert_eq!(error.code, "DEFERRED_POST_P16");
    }

    #[test]
    fn exact_host_tuple_reaches_p1a_output_root_validation() {
        let error = run([
            OsString::from("xtask"),
            OsString::from("verify-env"),
            OsString::from("--mode"),
            OsString::from("host"),
            OsString::from("--profile"),
            OsString::from(PROTOTYPE_PROFILE),
            OsString::from("--output-root"),
            OsString::from("not-the-p1a-root"),
        ])
        .unwrap_err();
        assert_eq!(error.code, "OUTPUT_ROOT_INVALID");
        assert_eq!(error.exit_code(), 2);
    }
}
