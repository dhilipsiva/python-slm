mod corpus;
mod dedup;
mod filter;
mod parquet_source;
mod remote;
mod tokenizer;
mod tokens;

pub use corpus::{CorpusReader, CorpusWriter, Document};
pub use dedup::{DedupConfig, DedupIndex, MinHashSignature};
pub use filter::{FilterConfig, FilterDecision, FilterRejection, PythonFilter};
pub use parquet_source::{ParquetBatch, ParquetColumns, RawDocument, read_parquet_batches};
pub use remote::{DownloadConfig, RemoteManifest, RemoteShard};
pub use tokenizer::{TokenizerTrainConfig, train_bpe};
pub use tokens::{TokenDataset, TokenFileEntry, TokenManifest, TokenizeConfig, tokenize_corpus};

use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use remote::download_manifest;
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CurateConfig {
    pub download: DownloadConfig,
    pub filter: FilterConfig,
    pub dedup: DedupConfig,
    pub parquet_batch_rows: usize,
}

impl Default for CurateConfig {
    fn default() -> Self {
        Self {
            download: DownloadConfig::default(),
            filter: FilterConfig::default(),
            dedup: DedupConfig::default(),
            // Combined with the default 16 MiB document cap, this bounds the
            // cloned/filtering batch to at most 256 MiB. Arrow's own decoder
            // allocations remain controlled by Parquet page/row-group layout.
            parquet_batch_rows: 16,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CurationStats {
    pub shards: u64,
    pub input_documents: u64,
    pub input_bytes: u64,
    pub accepted_documents: u64,
    pub accepted_bytes: u64,
    pub duplicate_documents: u64,
    pub rejected_too_small: u64,
    pub rejected_too_large: u64,
    pub rejected_invalid_utf8: u64,
    pub rejected_syntax: u64,
    pub rejected_comments: u64,
    pub rejected_generated: u64,
}

impl CurationStats {
    fn record_rejection(&mut self, rejection: FilterRejection) {
        match rejection {
            FilterRejection::TooSmall => self.rejected_too_small += 1,
            FilterRejection::TooLarge => self.rejected_too_large += 1,
            FilterRejection::InvalidUtf8 => self.rejected_invalid_utf8 += 1,
            FilterRejection::SyntaxError => self.rejected_syntax += 1,
            FilterRejection::CommentRatio => self.rejected_comments += 1,
            FilterRejection::GeneratedBoilerplate => self.rejected_generated += 1,
        }
    }
}

/// Downloads direct, content-bearing Parquet shards and writes a deterministic,
/// length-prefixed corpus. The Stack v2 ID-only layout is deliberately not
/// misrepresented as content-bearing input; callers must first obtain authorized
/// source blobs and produce a flat content column or presigned content URLs.
pub async fn curate_remote_parquet(
    manifest_path: &Path,
    work_dir: &Path,
    output_corpus: &Path,
    config: CurateConfig,
) -> Result<CurationStats> {
    ensure!(
        config.parquet_batch_rows > 0,
        "parquet_batch_rows must be positive"
    );
    ensure!(
        config.filter.maximum_bytes >= config.filter.minimum_bytes,
        "filter maximum_bytes must be at least minimum_bytes"
    );
    ensure!(
        config.filter.maximum_bytes <= corpus::MAX_DOCUMENT_TEXT_BYTES,
        "filter maximum_bytes exceeds the corpus format's {} byte record limit",
        corpus::MAX_DOCUMENT_TEXT_BYTES
    );
    let maximum_cloned_batch_bytes = config
        .parquet_batch_rows
        .checked_mul(config.filter.maximum_bytes)
        .context("Parquet batch clone budget overflow")?;
    ensure!(
        maximum_cloned_batch_bytes <= parquet_source::MAX_CLONED_BATCH_BYTES,
        "configured Parquet batch could clone {maximum_cloned_batch_bytes} bytes, above the hard {} byte budget",
        parquet_source::MAX_CLONED_BATCH_BYTES
    );
    let manifest_bytes = tokio::fs::read(manifest_path)
        .await
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: RemoteManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
    manifest.validate()?;

    let downloads = download_manifest(&manifest, work_dir, &config.download).await?;
    let output = output_corpus.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut writer = CorpusWriter::create(&output)?;
        let mut dedup = DedupIndex::new(config.dedup.clone())?;
        let mut stats = CurationStats {
            shards: downloads.len() as u64,
            ..CurationStats::default()
        };

        for downloaded in downloads {
            let shard = &manifest.shards[downloaded.manifest_index];
            let columns = ParquetColumns {
                content: shard.content_column.clone(),
                id: shard.id_column.clone(),
            };
            let source_name = downloaded.path.display().to_string();
            read_parquet_batches(
                &downloaded.path,
                &columns,
                config.parquet_batch_rows,
                config.filter.maximum_bytes,
                |batch| {
                    stats.input_documents += batch.input_documents;
                    stats.input_bytes += batch.input_bytes;
                    stats.rejected_too_large += batch.rejected_too_large;
                    curate_batch(
                        batch.documents,
                        &config.filter,
                        &mut dedup,
                        &mut writer,
                        &mut stats,
                    )
                },
            )
            .with_context(|| format!("decoding Parquet shard {source_name}"))?;
        }
        writer.finish()?;
        Ok(stats)
    })
    .await
    .context("curation worker panicked")?
}

fn curate_batch(
    documents: Vec<RawDocument>,
    filter_config: &FilterConfig,
    dedup: &mut DedupIndex,
    writer: &mut CorpusWriter,
    stats: &mut CurationStats,
) -> Result<()> {
    let filter_config = Arc::new(filter_config.clone());
    let decisions: Vec<(RawDocument, FilterDecision)> = documents
        .into_par_iter()
        .map_init(
            || PythonFilter::new((*filter_config).clone()),
            |filter, document| {
                let decision = filter.evaluate(&document.content);
                (document, decision)
            },
        )
        .collect();

    let mut survivors = Vec::with_capacity(decisions.len());
    for (document, decision) in decisions {
        match decision {
            FilterDecision::Accept { .. } => survivors.push(Document {
                id: document.id,
                text: String::from_utf8(document.content)
                    .context("filter accepted invalid UTF-8")?,
            }),
            FilterDecision::Reject(reason) => stats.record_rejection(reason),
        }
    }

    let signatures: Vec<MinHashSignature> = survivors
        .par_iter()
        .map(|document| MinHashSignature::from_code(&document.text, dedup.config()))
        .collect();

    for (document, signature) in survivors.into_iter().zip(signatures) {
        if dedup.insert_if_unique(signature) {
            stats.accepted_documents += 1;
            stats.accepted_bytes += document.text.len() as u64;
            writer.write(&document)?;
        } else {
            stats.duplicate_documents += 1;
        }
    }
    Ok(())
}
