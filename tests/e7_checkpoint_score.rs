#![cfg(feature = "cuda")]
//! Score a published checkpoint against held-out data.
//!
//! Diagnostic, not a gate: it reads a checkpoint and reports a loss. The bounded
//! diagnostic run reports its own, so this exists for checkpoints published before
//! that landed, and for any checkpoint a later binary can no longer resume.
//!
//!   cargo test --release --locked --features cuda --test e7_checkpoint_score -- --ignored --nocapture

#[test]
#[ignore = "diagnostic; needs the prototype device and a published checkpoint"]
fn score() {
    let generation = std::env::var("RUST_LLM_CHECKPOINT").expect("RUST_LLM_CHECKPOINT");
    let tokens = std::env::var("RUST_LLM_TOKENS").expect("RUST_LLM_TOKENS");
    let value = rust_llm_pretrain::train::launch::score_published_checkpoint(
        std::path::Path::new(&generation),
        std::path::Path::new(&tokens),
        0,
    )
    .expect("scoring");
    println!("\n{}", serde_json::to_string_pretty(&value).unwrap());
}
