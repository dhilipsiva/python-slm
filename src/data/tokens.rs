use super::{CorpusReader, tokenizer::SPECIAL_TOKENS};
use anyhow::{Context, Result, bail, ensure};
use memmap2::{Mmap, MmapOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
};
use tokenizers::Tokenizer;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TokenizeConfig {
    pub target_tokens: u64,
    pub shard_tokens: u64,
    pub add_eos_between_documents: bool,
}

impl Default for TokenizeConfig {
    fn default() -> Self {
        Self {
            target_tokens: 2_000_000_000,
            shard_tokens: 100_000_000,
            add_eos_between_documents: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenFileEntry {
    pub file: String,
    pub tokens: u64,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenManifest {
    pub format_version: u32,
    pub dtype: String,
    pub byte_order: String,
    pub total_tokens: u64,
    pub vocabulary_size: usize,
    pub eos_id: u16,
    pub pad_id: u16,
    pub eos_between_documents: bool,
    pub source_corpus_sha256: String,
    pub tokenizer_sha256: String,
    pub files: Vec<TokenFileEntry>,
}

pub fn tokenize_corpus(
    corpus_path: &Path,
    tokenizer_path: &Path,
    output_dir: &Path,
    config: &TokenizeConfig,
) -> Result<TokenManifest> {
    ensure!(
        config.target_tokens > 0,
        "target token count must be positive"
    );
    ensure!(
        config.shard_tokens > 0,
        "shard token count must be positive"
    );
    std::fs::create_dir_all(output_dir)?;
    let manifest_path = output_dir.join("tokens.manifest.json");
    ensure!(
        !manifest_path.exists(),
        "refusing to overwrite {}",
        manifest_path.display()
    );

    let tokenizer = Tokenizer::from_file(tokenizer_path)
        .map_err(|error| anyhow::anyhow!(error))
        .with_context(|| format!("loading {}", tokenizer_path.display()))?;
    let vocabulary_size = tokenizer.get_vocab_size(true);
    ensure!(
        vocabulary_size <= 65_536,
        "tokenizer vocabulary does not fit u16"
    );
    let pad_id = special_id(&tokenizer, SPECIAL_TOKENS[0])?;
    let eos_id = special_id(&tokenizer, SPECIAL_TOKENS[2])?;
    let source_corpus_sha256 = hash_file(corpus_path)?;
    let tokenizer_sha256 = hash_file(tokenizer_path)?;

    let mut writer = TokenShardWriter::new(output_dir, config.shard_tokens)?;
    'documents: for document in CorpusReader::open(corpus_path)? {
        let document = document?;
        let encoding = tokenizer
            .encode(document.text, false)
            .map_err(|error| anyhow::anyhow!(error))?;
        for &id in encoding.get_ids() {
            let id = u16::try_from(id).context("token id exceeded u16")?;
            writer.push(id)?;
            if writer.total_tokens == config.target_tokens {
                break 'documents;
            }
        }
        if config.add_eos_between_documents {
            writer.push(eos_id)?;
            if writer.total_tokens == config.target_tokens {
                break;
            }
        }
    }
    let files = writer.finish()?;
    let total_tokens = files.iter().map(|file| file.tokens).sum();
    ensure!(
        total_tokens == config.target_tokens,
        "corpus produced {total_tokens} tokens, below target {} (partial shards were retained, but no manifest was written)",
        config.target_tokens
    );
    let manifest = TokenManifest {
        format_version: 1,
        dtype: "u16".into(),
        byte_order: "little".into(),
        total_tokens,
        vocabulary_size,
        eos_id,
        pad_id,
        eos_between_documents: config.add_eos_between_documents,
        source_corpus_sha256,
        tokenizer_sha256,
        files,
    };
    write_manifest(&manifest_path, &manifest)?;
    Ok(manifest)
}

fn special_id(tokenizer: &Tokenizer, token: &str) -> Result<u16> {
    let id = tokenizer
        .token_to_id(token)
        .with_context(|| format!("tokenizer has no required special token {token:?}"))?;
    u16::try_from(id).context("special token id exceeded u16")
}

struct TokenShardWriter {
    output_dir: PathBuf,
    shard_tokens: u64,
    shard_index: usize,
    current: Option<OpenShard>,
    entries: Vec<TokenFileEntry>,
    total_tokens: u64,
}

struct OpenShard {
    name: String,
    file: BufWriter<File>,
    hasher: Sha256,
    tokens: u64,
}

impl TokenShardWriter {
    fn new(output_dir: &Path, shard_tokens: u64) -> Result<Self> {
        Ok(Self {
            output_dir: output_dir.to_owned(),
            shard_tokens,
            shard_index: 0,
            current: None,
            entries: Vec::new(),
            total_tokens: 0,
        })
    }

    fn push(&mut self, token: u16) -> Result<()> {
        if self.current.is_none() {
            self.open_shard()?;
        }
        let bytes = token.to_le_bytes();
        let shard = self.current.as_mut().expect("shard was opened");
        shard.file.write_all(&bytes)?;
        shard.hasher.update(bytes);
        shard.tokens += 1;
        self.total_tokens += 1;
        if shard.tokens == self.shard_tokens {
            self.close_shard()?;
        }
        Ok(())
    }

    fn open_shard(&mut self) -> Result<()> {
        let name = format!("tokens-{:05}.u16le", self.shard_index);
        let path = self.output_dir.join(&name);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("creating {} without overwrite", path.display()))?;
        self.current = Some(OpenShard {
            name,
            file: BufWriter::with_capacity(8 * 1024 * 1024, file),
            hasher: Sha256::new(),
            tokens: 0,
        });
        self.shard_index += 1;
        Ok(())
    }

    fn close_shard(&mut self) -> Result<()> {
        let Some(mut shard) = self.current.take() else {
            return Ok(());
        };
        if shard.tokens == 0 {
            bail!("internal error: empty token shard");
        }
        shard.file.flush()?;
        shard.file.get_ref().sync_all()?;
        self.entries.push(TokenFileEntry {
            file: shard.name,
            tokens: shard.tokens,
            bytes: shard.tokens * 2,
            sha256: hex::encode(shard.hasher.finalize()),
        });
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<TokenFileEntry>> {
        self.close_shard()?;
        ensure!(!self.entries.is_empty(), "no tokens were written");
        Ok(self.entries)
    }
}

fn write_manifest(path: &Path, manifest: &TokenManifest) -> Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut file = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut file, manifest)?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.get_ref().sync_all()?;
    Ok(())
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

struct MappedShard {
    start_token: u64,
    end_token: u64,
    mmap: Mmap,
}

/// Read-only CPU mapping of immutable u16 shards. This removes an extra disk-to-
/// heap copy, but it is not CUDA-pinned memory and not GPU zero-copy. Each batch
/// is deliberately widened to integer tensor input before H2D transfer.
pub struct TokenDataset {
    manifest: TokenManifest,
    shards: Vec<MappedShard>,
}

impl TokenDataset {
    pub fn open(manifest_path: &Path, verify_hashes: bool) -> Result<Self> {
        let bytes = std::fs::read(manifest_path)?;
        let manifest: TokenManifest = serde_json::from_slice(&bytes)?;
        validate_manifest(&manifest)?;
        let base = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let mut shards = Vec::with_capacity(manifest.files.len());
        let mut start_token = 0_u64;
        for entry in &manifest.files {
            ensure_safe_filename(&entry.file)?;
            let path = base.join(&entry.file);
            let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
            let actual_bytes = file.metadata()?.len();
            ensure!(
                actual_bytes == entry.bytes && actual_bytes == entry.tokens * 2,
                "token shard length mismatch for {}",
                path.display()
            );
            if verify_hashes {
                let actual = hash_file(&path)?;
                ensure!(
                    actual == entry.sha256,
                    "token shard hash mismatch for {}",
                    path.display()
                );
            }
            // SAFETY: the file is opened read-only after length/hash validation. The
            // dataset contract requires finalized shards not be replaced or truncated
            // while mapped; this inherent external-mutation precondition is why memmap2
            // marks mapping unsafe.
            let mmap = unsafe { MmapOptions::new().map(&file) }?;
            let end_token = start_token + entry.tokens;
            shards.push(MappedShard {
                start_token,
                end_token,
                mmap,
            });
            start_token = end_token;
        }
        ensure!(
            start_token == manifest.total_tokens,
            "manifest total token mismatch"
        );
        Ok(Self { manifest, shards })
    }

    pub fn len(&self) -> u64 {
        self.manifest.total_tokens
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn manifest(&self) -> &TokenManifest {
        &self.manifest
    }

    pub fn token_at(&self, index: u64) -> Result<u16> {
        ensure!(index < self.len(), "token index {index} is out of range");
        let shard_index = self
            .shards
            .partition_point(|shard| shard.end_token <= index);
        let shard = &self.shards[shard_index];
        let local = (index - shard.start_token) as usize;
        let offset = local * 2;
        Ok(u16::from_le_bytes([
            shard.mmap[offset],
            shard.mmap[offset + 1],
        ]))
    }

    /// Produces row-major next-token inputs and labels. The immutable token stream
    /// is treated as circular, so a final batch crosses the corpus boundary instead
    /// of jumping backward and oversampling an earlier window.
    pub fn batch(&self, start: u64, batch: usize, sequence: usize) -> Result<(Vec<i64>, Vec<i64>)> {
        ensure!(
            batch > 0 && sequence > 0,
            "batch and sequence must be positive"
        );
        let predicted = (batch as u64)
            .checked_mul(sequence as u64)
            .context("batch token count overflow")?;
        ensure!(self.len() >= 2, "dataset requires at least two tokens");
        let mut index = start % self.len();
        let capacity = usize::try_from(predicted).context("batch does not fit address space")?;
        let mut inputs = Vec::with_capacity(capacity);
        let mut labels = Vec::with_capacity(capacity);
        for _ in 0..predicted {
            let next = if index + 1 == self.len() {
                0
            } else {
                index + 1
            };
            inputs.push(self.token_at(index)? as i64);
            labels.push(self.token_at(next)? as i64);
            index = next;
        }
        Ok((inputs, labels))
    }
}

fn validate_manifest(manifest: &TokenManifest) -> Result<()> {
    ensure!(
        manifest.format_version == 1,
        "unsupported token manifest version"
    );
    ensure!(manifest.dtype == "u16", "unsupported token dtype");
    ensure!(
        manifest.byte_order == "little",
        "unsupported token byte order"
    );
    ensure!(manifest.total_tokens > 0, "empty token dataset");
    ensure!(manifest.vocabulary_size <= 65_536, "vocabulary exceeds u16");
    ensure!(
        usize::from(manifest.eos_id) < manifest.vocabulary_size
            && usize::from(manifest.pad_id) < manifest.vocabulary_size,
        "special token id is outside the vocabulary"
    );
    ensure!(
        is_sha256(&manifest.source_corpus_sha256) && is_sha256(&manifest.tokenizer_sha256),
        "corpus/tokenizer hashes must be 64 hexadecimal characters"
    );
    ensure!(
        !manifest.files.is_empty(),
        "token manifest contains no files"
    );
    ensure!(
        manifest.files.iter().all(|entry| is_sha256(&entry.sha256)),
        "token shard hashes must be 64 hexadecimal characters"
    );
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn ensure_safe_filename(name: &str) -> Result<()> {
    let path = Path::new(name);
    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_)))
            && path.components().count() == 1,
        "token shard must be a single relative filename: {name:?}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_little_endian_shards_across_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let path_a = temp.path().join("tokens-00000.u16le");
        let path_b = temp.path().join("tokens-00001.u16le");
        std::fs::write(
            &path_a,
            [1_u16, 2, 3]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        std::fs::write(
            &path_b,
            [4_u16, 5, 6]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let manifest = TokenManifest {
            format_version: 1,
            dtype: "u16".into(),
            byte_order: "little".into(),
            total_tokens: 6,
            vocabulary_size: 32_000,
            eos_id: 2,
            pad_id: 0,
            eos_between_documents: true,
            source_corpus_sha256: "00".repeat(32),
            tokenizer_sha256: "00".repeat(32),
            files: vec![
                TokenFileEntry {
                    file: "tokens-00000.u16le".into(),
                    tokens: 3,
                    bytes: 6,
                    sha256: hash_file(&path_a).unwrap(),
                },
                TokenFileEntry {
                    file: "tokens-00001.u16le".into(),
                    tokens: 3,
                    bytes: 6,
                    sha256: hash_file(&path_b).unwrap(),
                },
            ],
        };
        let manifest_path = temp.path().join("tokens.manifest.json");
        write_manifest(&manifest_path, &manifest).unwrap();
        let dataset = TokenDataset::open(&manifest_path, true).unwrap();
        assert_eq!(dataset.token_at(3).unwrap(), 4);
        assert_eq!(
            dataset.batch(1, 1, 4).unwrap(),
            (vec![2, 3, 4, 5], vec![3, 4, 5, 6])
        );
        assert_eq!(
            dataset.batch(4, 1, 4).unwrap(),
            (vec![5, 6, 1, 2], vec![6, 1, 2, 3])
        );
    }
}
