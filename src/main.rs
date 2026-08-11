use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rust_llm_pretrain::{
    config::{LlamaConfig, MemoryEstimate, TrainConfig},
    data::{curate_remote_parquet, tokenize_corpus, train_bpe},
};
use serde::{Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Zero-Python Rust code-corpus and Llama pre-training reference"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print exact parameter, memory, FLOP, and wall-clock arithmetic.
    Plan {
        #[arg(long)]
        model_config: Option<PathBuf>,
        #[arg(long)]
        train_config: Option<PathBuf>,
        /// Use d_ff=2432, retaining GQA, for 135,285,504 parameters.
        #[arg(long, conflicts_with = "model_config")]
        gqa_135m: bool,
    },
    /// Download direct, content-bearing Parquet shards and curate Python.
    Curate {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        work_dir: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Train the 32k byte-level BPE on a bounded scrubbed-corpus subset.
    TrainTokenizer {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        subset: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Convert a curated corpus to immutable little-endian u16 token shards.
    Tokenize {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        tokenizer: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Run the differentiable reference trainer. The production gate is closed by default.
    Train {
        #[arg(long)]
        tokens: PathBuf,
        #[arg(long)]
        model_config: Option<PathBuf>,
        #[arg(long)]
        train_config: Option<PathBuf>,
        #[arg(long, default_value_t = 0)]
        device: usize,
        #[arg(long)]
        verify_hashes: bool,
        /// Final model checkpoint base; Burn writes `<base>.mpk` atomically.
        #[arg(long)]
        checkpoint: Option<PathBuf>,
        /// Permit the O(L^2) correctness path. This does not certify the 8-hour target.
        #[arg(long)]
        allow_reference_attention: bool,
    },
}

#[derive(Debug, Serialize)]
struct PlanReport {
    model: LlamaConfig,
    training: TrainConfig,
    parameters: u64,
    training_flops_per_token_lower: u64,
    training_flops_per_token_upper: u64,
    exact_minimum_tokens_per_second_for_eight_hours: f64,
    configured_target_tokens_per_second: f64,
    compute_required_tflops_lower_at_target: f64,
    compute_required_tflops_upper_at_target: f64,
    optimizer_steps: u64,
    memory: MemoryEstimate,
    acceptance_gate: &'static str,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rust_llm_pretrain=info".into()),
        )
        .init();

    match Cli::parse().command {
        Command::Plan {
            model_config,
            train_config,
            gqa_135m,
        } => {
            let model = if gqa_135m {
                LlamaConfig::gqa_135m()
            } else {
                load_or_default(model_config.as_deref())?
            };
            let training: TrainConfig = load_or_default(train_config.as_deref())?;
            training.validate(&model)?;
            let (flops_lower, flops_upper) = model.training_flops_per_token();
            let target = training.target_tokens_per_second;
            print_json(&PlanReport {
                parameters: model.parameter_count(),
                training_flops_per_token_lower: flops_lower,
                training_flops_per_token_upper: flops_upper,
                exact_minimum_tokens_per_second_for_eight_hours: 2_000_000_000.0 / 28_800.0,
                configured_target_tokens_per_second: target,
                compute_required_tflops_lower_at_target: flops_lower as f64 * target / 1e12,
                compute_required_tflops_upper_at_target: flops_upper as f64 * target / 1e12,
                optimizer_steps: training.total_optimizer_steps(&model),
                memory: MemoryEstimate::bf16(&model, &training),
                model,
                training,
                acceptance_gate: "closed until an SM120 fused attention backward, chunked/fused cross-entropy, mixed-precision optimizer, and an end-to-end >=75k token/s benchmark exist",
            })?;
        }
        Command::Curate {
            manifest,
            work_dir,
            output,
            config,
        } => {
            let config = load_or_default(config.as_deref())?;
            let stats = curate_remote_parquet(&manifest, &work_dir, &output, config).await?;
            print_json(&stats)?;
        }
        Command::TrainTokenizer {
            corpus,
            subset,
            output,
            config,
        } => {
            let config = load_or_default(config.as_deref())?;
            let report = train_bpe(&corpus, &subset, &output, &config)?;
            print_json(&report)?;
        }
        Command::Tokenize {
            corpus,
            tokenizer,
            output_dir,
            config,
        } => {
            let config = load_or_default(config.as_deref())?;
            let manifest = tokenize_corpus(&corpus, &tokenizer, &output_dir, &config)?;
            print_json(&manifest)?;
        }
        Command::Train {
            tokens,
            model_config,
            train_config,
            device,
            verify_hashes,
            checkpoint,
            allow_reference_attention,
        } => {
            let model = load_or_default(model_config.as_deref())?;
            let mut training: TrainConfig = load_or_default(train_config.as_deref())?;
            training.allow_reference_attention |= allow_reference_attention;
            run_train(
                tokens,
                model,
                training,
                device,
                verify_hashes,
                checkpoint.as_deref(),
            )?;
        }
    }
    Ok(())
}

fn load_or_default<T>(path: Option<&Path>) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    match path {
        Some(path) => {
            let bytes = std::fs::read(path)
                .with_context(|| format!("reading configuration {}", path.display()))?;
            serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing configuration {}", path.display()))
        }
        None => Ok(T::default()),
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(feature = "cuda")]
fn run_train(
    tokens: PathBuf,
    model: LlamaConfig,
    training: TrainConfig,
    device: usize,
    verify_hashes: bool,
    checkpoint: Option<&Path>,
) -> Result<()> {
    let summary = rust_llm_pretrain::train::run_cuda_training_with_checkpoint(
        &tokens,
        model,
        training,
        device,
        verify_hashes,
        checkpoint,
    )?;
    print_json(&summary)
}

#[cfg(not(feature = "cuda"))]
fn run_train(
    _tokens: PathBuf,
    _model: LlamaConfig,
    _training: TrainConfig,
    _device: usize,
    _verify_hashes: bool,
    _checkpoint: Option<&Path>,
) -> Result<()> {
    anyhow::bail!(
        "the train command requires `cargo run --no-default-features --features cuda --release -- train ...`"
    )
}
