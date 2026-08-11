use super::{CorpusReader, corpus::require_nonempty_corpus};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashSet},
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::Path,
};
use tokenizers::{
    AddedToken, Tokenizer,
    decoders::byte_level::ByteLevel as ByteLevelDecoder,
    models::{
        TrainerWrapper,
        bpe::{BPE, BpeTrainerBuilder},
    },
    pre_tokenizers::byte_level::ByteLevel,
    processors::byte_level::ByteLevel as ByteLevelProcessor,
};

pub const SPECIAL_TOKENS: [&str; 4] = ["<pad>", "<s>", "</s>", "<unk>"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TokenizerTrainConfig {
    pub vocab_size: usize,
    /// Decimal bytes, intentionally not GiB. This avoids the BPE trainer's i32
    /// pair-count overflow edge near 2^31 observations.
    pub subset_bytes: u64,
    pub minimum_frequency: u64,
    pub show_progress: bool,
}

impl Default for TokenizerTrainConfig {
    fn default() -> Self {
        Self {
            vocab_size: 32_000,
            subset_bytes: 2_000_000_000,
            minimum_frequency: 2,
            show_progress: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenizerTrainStats {
    pub subset_documents: u64,
    pub subset_bytes: u64,
    pub vocabulary_size: usize,
}

type SubsetCandidate = Reverse<(u32, u64, u64, u64)>;

pub fn train_bpe(
    corpus_path: &Path,
    subset_path: &Path,
    tokenizer_path: &Path,
    config: &TokenizerTrainConfig,
) -> Result<TokenizerTrainStats> {
    ensure!(
        config.vocab_size > SPECIAL_TOKENS.len(),
        "vocabulary is too small"
    );
    ensure!(config.vocab_size <= 65_536, "vocabulary does not fit u16");
    ensure!(config.subset_bytes > 0, "subset_bytes must be positive");
    require_nonempty_corpus(corpus_path)?;
    ensure!(
        subset_path != tokenizer_path,
        "subset and tokenizer output paths must differ"
    );
    ensure!(
        !subset_path.exists(),
        "refusing to overwrite existing subset {}",
        subset_path.display()
    );
    ensure!(
        !tokenizer_path.exists(),
        "refusing to overwrite existing tokenizer {}",
        tokenizer_path.display()
    );
    if let Some(parent) = subset_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = tokenizer_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }

    // Retain the highest-scoring documents over the entire scrubbed corpus.
    // The stable hash breaks score ties without making shard order a quality
    // signal; selected records are emitted in corpus order for reproducibility.
    let mut candidates = BinaryHeap::<SubsetCandidate>::new();
    let mut selected_bytes = 0_u64;
    for (index, document) in CorpusReader::open(corpus_path)?.enumerate() {
        let document = document?;
        let record_bytes = document.text.len() as u64 + 1;
        if record_bytes > config.subset_bytes {
            continue;
        }
        let score = quality_score(&document.text);
        let tie_breaker = xxhash_rust::xxh3::xxh3_64(document.id.as_bytes())
            ^ xxhash_rust::xxh3::xxh3_64(document.text.as_bytes()).rotate_left(17);
        candidates.push(Reverse((score, tie_breaker, index as u64, record_bytes)));
        selected_bytes += record_bytes;
        while selected_bytes > config.subset_bytes {
            let Reverse((_, _, _, evicted_bytes)) = candidates
                .pop()
                .expect("a positive selected byte count has a candidate");
            selected_bytes -= evicted_bytes;
        }
    }
    ensure!(
        !candidates.is_empty(),
        "no complete document fit in the tokenizer subset"
    );
    let selected_indices: HashSet<u64> = candidates
        .into_iter()
        .map(|Reverse((_, _, index, _))| index)
        .collect();

    let subset_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(subset_path)
        .with_context(|| {
            format!(
                "creating {} (refusing to overwrite an existing subset)",
                subset_path.display()
            )
        })?;
    let mut subset = BufWriter::with_capacity(8 * 1024 * 1024, subset_file);
    let mut subset_documents = 0_u64;
    let mut subset_bytes = 0_u64;
    for (index, document) in CorpusReader::open(corpus_path)?.enumerate() {
        let document = document?;
        if !selected_indices.contains(&(index as u64)) {
            continue;
        }
        subset.write_all(document.text.as_bytes())?;
        subset.write_all(b"\n")?;
        let record_bytes = document.text.len() as u64 + 1;
        subset_bytes += record_bytes;
        subset_documents += 1;
    }
    subset.flush()?;
    subset.get_ref().sync_all()?;
    drop(subset);
    ensure!(
        subset_bytes == selected_bytes && subset_documents == selected_indices.len() as u64,
        "tokenizer subset selection changed between corpus passes"
    );
    ensure!(
        subset_documents > 0,
        "no complete document fit in the tokenizer subset"
    );

    let model = BPE::builder()
        .unk_token("<unk>".into())
        .build()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let byte_level = ByteLevel::new(false, false, true);
    let alphabet: HashSet<char> = ByteLevel::alphabet().into_iter().collect();
    let mut tokenizer = Tokenizer::new(model);
    tokenizer.with_pre_tokenizer(Some(byte_level));
    tokenizer.with_post_processor(Some(ByteLevelProcessor::new(false, false, true)));
    tokenizer.with_decoder(Some(ByteLevelDecoder::new(false, false, true)));

    let mut trainer: TrainerWrapper = BpeTrainerBuilder::new()
        .show_progress(config.show_progress)
        .vocab_size(config.vocab_size)
        .min_frequency(config.minimum_frequency)
        .initial_alphabet(alphabet)
        .special_tokens(
            SPECIAL_TOKENS
                .iter()
                .map(|token| AddedToken::from((*token).to_owned(), true))
                .collect(),
        )
        .build()
        .into();
    let subset_file = subset_path
        .to_str()
        .context("tokenizers currently requires a Unicode subset path")?
        .to_owned();
    tokenizer
        .train_from_files(&mut trainer, vec![subset_file])
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let vocabulary_size = tokenizer.get_vocab_size(true);
    ensure!(
        vocabulary_size == config.vocab_size,
        "BPE trainer produced {vocabulary_size} tokens, expected {}; use a larger/more varied subset",
        config.vocab_size
    );
    tokenizer
        .save(tokenizer_path, false)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    Ok(TokenizerTrainStats {
        subset_documents,
        subset_bytes,
        vocabulary_size,
    })
}

/// Deterministic information-density proxy for already syntax-validated code.
/// It rewards non-whitespace density, non-repeated lines, and useful file sizes;
/// it is not a substitute for provenance, license, or benchmark-leakage labels.
fn quality_score(text: &str) -> u32 {
    let byte_len = text.len().max(1);
    let non_whitespace = text
        .as_bytes()
        .iter()
        .filter(|byte| !byte.is_ascii_whitespace())
        .count();
    let density = (non_whitespace as u64 * 1_000 / byte_len as u64) as u32;

    let mut nonempty_lines = 0_u64;
    let mut unique_lines = HashSet::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        nonempty_lines += 1;
        unique_lines.insert(xxhash_rust::xxh3::xxh3_64(line.as_bytes()));
    }
    let uniqueness = (unique_lines.len() as u64 * 1_000)
        .checked_div(nonempty_lines)
        .unwrap_or(0) as u32;
    let length = match text.len() {
        1_024..=65_536 => 1_000,
        512..=262_144 => 800,
        128..=1_048_576 => 500,
        _ => 200,
    };
    density + uniqueness + length
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{CorpusWriter, Document, TokenDataset, TokenizeConfig, tokenize_corpus};

    #[test]
    fn trains_serializes_and_reloads_byte_level_bpe() {
        let temp = tempfile::tempdir().unwrap();
        let corpus_path = temp.path().join("sample.corpus");
        let subset_path = temp.path().join("subset.txt");
        let tokenizer_path = temp.path().join("tokenizer.json");
        let mut corpus = CorpusWriter::create(&corpus_path).unwrap();
        for index in 0..64 {
            corpus
                .write(&Document {
                    id: format!("doc-{index}"),
                    text: format!(
                        "def function_{index}(value):\n    result = value * {}\n    return result\n",
                        index + 1
                    ),
                })
                .unwrap();
        }
        corpus.finish().unwrap();

        let stats = train_bpe(
            &corpus_path,
            &subset_path,
            &tokenizer_path,
            &TokenizerTrainConfig {
                vocab_size: 300,
                subset_bytes: 1_000_000,
                minimum_frequency: 1,
                show_progress: false,
            },
        )
        .unwrap();
        assert_eq!(stats.vocabulary_size, 300);

        let tokenizer = Tokenizer::from_file(&tokenizer_path).unwrap();
        for token in SPECIAL_TOKENS {
            assert!(tokenizer.token_to_id(token).is_some());
        }
        assert!(
            !tokenizer
                .encode("def useful(): return 1", false)
                .unwrap()
                .is_empty()
        );

        let token_dir = temp.path().join("tokens");
        let manifest = tokenize_corpus(
            &corpus_path,
            &tokenizer_path,
            &token_dir,
            &TokenizeConfig {
                target_tokens: 128,
                shard_tokens: 31,
                add_eos_between_documents: true,
            },
        )
        .unwrap();
        assert_eq!(manifest.total_tokens, 128);
        assert_eq!(manifest.files.len(), 5);
        let mapped = TokenDataset::open(&token_dir.join("tokens.manifest.json"), true).unwrap();
        assert_eq!(mapped.len(), 128);
    }

    #[test]
    fn quality_proxy_penalizes_repeated_lines() {
        let repeated = "value = 1\n".repeat(200);
        let varied: String = (0..200)
            .map(|index| format!("value_{index} = {index}\n"))
            .collect();
        assert!(quality_score(&varied) > quality_score(&repeated));
    }
}
