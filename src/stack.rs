//! The `SOURCE-001` primary source adapter: Stack v2 Python metadata plus
//! authorized Software Heritage content.
//!
//! The contract is explicit that these are two distinct sources
//! (`docs/rebuild-contract.md:111,151`): the Stack v2 shards record *identifiers*
//! rather than a content column, so a document only exists once a Software
//! Heritage blob has been resolved against the identifier the shard supplied.
//! This module keeps that separation visible — it reads metadata it never
//! fetched, and fetches content it never chose — and ends where the rest of the
//! pipeline begins, at a `MaterializedSourceManifestV1` plus a content tree that
//! `curate` already knows how to consume.
//!
//! Three things it deliberately does not do. It does not discover endpoints: the
//! metadata shards arrive as local, hash-bound bytes that `fetch` already
//! verified, so what is read is pinned rather than crawled. It does not decide
//! licensing: the allowlist is declared in configuration and applied to the
//! licence each row carries, so a row is admitted by the operator's rule rather
//! than by this module's judgement. And it does not trust an identifier: every
//! blob is verified against the content-addressed digest the shard declared
//! before it is written, which is what makes the identifier-to-content step a
//! link in the hash chain rather than a gap in it.

use crate::acquire::PartialTree;
use crate::data::source::{
    compact_json_line, is_sha256, parse_closed, read_control_file, require_output_boundary,
    require_portable_relative_path, sha256,
};
use crate::data::{MaterializedSourceManifestV1, Provenance, SourceAuthorization, SourceDocument};
use crate::error::{ProductError, Result};
use crate::tokenizer::HashBoundInput;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const IMPLEMENTATION_PHASE: &str = "E3";
pub const STACK_CONFIG_SCHEMA: &str = "python-slm-stack-v2-source-config-v1";
pub const STACK_RESULT_SCHEMA: &str = "python-slm-stack-v2-source-result-v1";
const ADAPTER_NAMESPACE: &str = "stack-v2-software-heritage";

/// Rows decoded per Arrow batch. The shards are far larger than memory at
/// production scale, so they are read in batches and every surviving row is
/// resolved and written before the next batch is decoded.
const BATCH_ROWS: usize = 4_096;

/// `docs/rebuild-contract.md:114` fixes the per-document ceiling. It is applied
/// to the length the shard declares, so an oversized blob is skipped before it
/// is ever requested rather than after it has been transferred.
pub const MAXIMUM_DOCUMENT_BYTES: u64 = 1_000_000;

/// Where each field the adapter needs lives in the shard.
///
/// Declared rather than assumed. The published Stack v2 schema is stable, but
/// this adapter is verified against fixtures rather than against the dataset, so
/// binding the column names in configuration keeps the mapping auditable and
/// lets a schema revision be handled by an operator instead of a code change. A
/// name that is absent, or present with the wrong Arrow type, is a typed failure
/// naming the column.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackColumnsV1 {
    /// Software Heritage `sha1_git` of the blob, hex. Both the content address
    /// and the verification target.
    pub blob_id: String,
    pub repository: String,
    pub path: String,
    pub revision: String,
    pub language: String,
    pub length_bytes: String,
    /// A list column; a row is admitted only if every licence it carries is
    /// allowlisted.
    pub detected_licenses: String,
}

/// How the origin encodes a blob body.
///
/// Declared, never sniffed. The bulk mirror and the archive API do not agree on
/// this, and guessing from a magic number would mean a corpus whose contents
/// depended on what a server happened to send. A body that disagrees with the
/// declaration fails the identifier check, which is the correct outcome: the
/// operator, not the transport, decides what is being read.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StackContentEncodingV1 {
    Identity,
    Gzip,
}

/// The transfer ceiling for a body that will be inflated afterwards.
///
/// Deflate cannot expand incompressible input by more than about five bytes per
/// 64 KiB stored block, and a gzip member adds an 18-byte frame, so this bounds
/// the compressed form of any blob of the declared length while still stopping a
/// response that is wildly larger than it should be.
fn compressed_ceiling(length_bytes: u64) -> u64 {
    length_bytes
        .saturating_add(length_bytes / 1000)
        .saturating_add(64)
}

/// Inflate a gzip member, refusing to produce more than the metadata promised.
///
/// The cap is one byte above the declared length so an over-long stream is
/// detected rather than absorbed — the same trick the transfer ceiling uses, and
/// the reason a decompression bomb cannot turn a 1 MB declaration into an
/// unbounded allocation.
fn inflate(compressed: &[u8], declared_bytes: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut decoder =
        flate2::read::GzDecoder::new(compressed).take(declared_bytes.saturating_add(1));
    let mut inflated = Vec::new();
    decoder.read_to_end(&mut inflated).map_err(|error| {
        ProductError::integrity(
            "STACK_CONTENT_DECODE_FAILED",
            format!("a Software Heritage blob could not be inflated: {error}"),
        )
    })?;
    Ok(inflated)
}

/// A deterministic slice of the blob space, so a long acquisition can be run in
/// pieces that fail independently.
///
/// This is the resumability the contract asks for
/// (`docs/rebuild-contract.md:113`), arranged so that it costs the create-new
/// publication rule nothing. A partial generation is never resumed — it is
/// discarded and its partition is rerun — and because the partition is a pure
/// function of `blob_id`, every occurrence of a blob falls in exactly one
/// partition. Two consequences follow without needing to be enforced: partitions
/// cannot duplicate a document between them, and the union of all of them is the
/// same set an unpartitioned run would select.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackPartitionV1 {
    /// How many leading hex characters of the identifier select the partition.
    pub prefix_length: u64,
    /// The prefixes this invocation admits. Lowercase hex, each exactly
    /// `prefix_length` long.
    pub include: Vec<String>,
}

impl StackPartitionV1 {
    fn validate(&self) -> Result<()> {
        // A `sha1_git` is 40 hex characters, so a longer prefix could never match
        // and a zero-length one would not partition anything.
        if self.prefix_length == 0 || self.prefix_length > 40 {
            return Err(ProductError::usage(
                "STACK_PARTITION_INVALID",
                "the partition prefix length must be between 1 and 40 characters",
            ));
        }
        if self.include.is_empty() {
            return Err(ProductError::usage(
                "STACK_PARTITION_INVALID",
                "a partition must admit at least one prefix",
            ));
        }
        let mut seen = BTreeSet::new();
        for prefix in &self.include {
            if prefix.len() as u64 != self.prefix_length
                || !prefix
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(ProductError::usage(
                    "STACK_PARTITION_INVALID",
                    "every partition prefix must be lowercase hex of the declared length",
                ));
            }
            // A repeated prefix would not change what is selected, but it would
            // mean the operator believes something about the split that is not
            // true, so it is refused rather than deduplicated silently.
            if !seen.insert(prefix.as_str()) {
                return Err(ProductError::usage(
                    "STACK_PARTITION_INVALID",
                    "a partition prefix is listed more than once",
                ));
            }
        }
        Ok(())
    }

    fn contains(&self, blob_id: &str) -> bool {
        blob_id
            .get(..self.prefix_length as usize)
            .is_some_and(|prefix| self.include.iter().any(|value| value == prefix))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackLimitsV1 {
    pub maximum_documents: u64,
    pub maximum_total_bytes: u64,
    pub maximum_redirects: u64,
    pub connect_timeout_seconds: u64,
    pub read_timeout_seconds: u64,
    /// Retries after the first attempt, for transient failures only. Zero is a
    /// valid explicit choice; a production run over a million blobs is not.
    pub retry_attempts: u64,
    pub retry_initial_delay_milliseconds: u64,
    pub retry_maximum_delay_milliseconds: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackSourceConfigV1 {
    pub schema: String,
    pub profile: String,
    /// Metadata shards, each already fetched and hash-bound. Parquet shards
    /// exceed the control-file bound, so they are verified by streaming rather
    /// than read through `read_control_file`.
    pub metadata_shards: Vec<HashBoundInput>,
    pub columns: StackColumnsV1,
    /// Where a blob is fetched from, with `{blob_id}` substituted. Templated
    /// rather than derived so the archive endpoint is declared, not assumed.
    pub content_url_template: String,
    /// Name of an environment variable holding the archive bearer token. The
    /// token never appears in configuration, in the result, or in an error.
    pub content_credential_env: Option<String>,
    /// Plain HTTP against a literal loopback address, for the contract's local
    /// fixture exemption (`docs/rebuild-contract.md:110`) and nothing else.
    pub allow_loopback_plain_http: bool,
    /// How the origin encodes a blob body. Declared, never sniffed.
    pub content_encoding: StackContentEncodingV1,
    pub language: String,
    /// Every licence a row carries must appear here.
    pub license_allowlist: Vec<String>,
    pub source_snapshot_id: String,
    pub authorization: SourceAuthorization,
    pub required_removal_authorities: Vec<String>,
    pub output_root: PathBuf,
    /// Absent means the whole selection; present means this invocation handles
    /// one deterministic slice of it and another invocation handles the rest.
    pub blob_id_partition: Option<StackPartitionV1>,
    pub documents_per_generation: u64,
    pub limits: StackLimitsV1,
}

impl StackSourceConfigV1 {
    pub fn validate(&self) -> Result<()> {
        if self.schema != STACK_CONFIG_SCHEMA {
            return Err(ProductError::usage(
                "STACK_CONFIG_INVALID",
                "the configuration is not the closed Stack v2 source schema",
            ));
        }
        if self.profile != crate::backend::PROTOTYPE_PROFILE {
            return Err(ProductError::gate(
                "DEFERRED_POST_P16",
                "only the prototype profile is implemented",
            ));
        }
        if self.metadata_shards.is_empty() {
            return Err(ProductError::usage(
                "STACK_CONFIG_INVALID",
                "at least one hash-bound metadata shard is required",
            ));
        }
        let mut seen = BTreeSet::new();
        for shard in &self.metadata_shards {
            if !shard.path.is_absolute() || !is_sha256(&shard.sha256) {
                return Err(ProductError::usage(
                    "STACK_CONFIG_INVALID",
                    "every metadata shard must be an absolute, hash-bound path",
                ));
            }
            if !seen.insert(shard.path.clone()) {
                return Err(ProductError::usage(
                    "STACK_CONFIG_INVALID",
                    "a metadata shard is named more than once",
                ));
            }
        }
        if !self.output_root.is_absolute() {
            return Err(ProductError::usage(
                "STACK_CONFIG_INVALID",
                "the output root must be absolute",
            ));
        }
        // A template that never varies would request one blob repeatedly, so the
        // substitution point is required rather than optional.
        if !self.content_url_template.contains("{blob_id}") {
            return Err(ProductError::usage(
                "STACK_CONFIG_INVALID",
                "the content URL template must contain the {blob_id} substitution",
            ));
        }
        if self.language.is_empty() || self.license_allowlist.is_empty() {
            return Err(ProductError::usage(
                "STACK_CONFIG_INVALID",
                "a language and a non-empty licence allowlist are required",
            ));
        }
        if self.source_snapshot_id.is_empty()
            || self.authorization.scheme.is_empty()
            || self.authorization.authority_url.is_empty()
            || self.authorization.authorization_id.is_empty()
        {
            return Err(ProductError::usage(
                "STACK_CONFIG_INVALID",
                "the snapshot identity and authorization record must be complete",
            ));
        }
        if let Some(partition) = &self.blob_id_partition {
            partition.validate()?;
        }
        if self.required_removal_authorities.is_empty() {
            return Err(ProductError::usage(
                "STACK_CONFIG_INVALID",
                "at least one removal authority is required",
            ));
        }
        if self.documents_per_generation == 0
            || self.limits.maximum_documents == 0
            || self.limits.maximum_total_bytes == 0
            || self.limits.connect_timeout_seconds == 0
            || self.limits.read_timeout_seconds == 0
        {
            return Err(ProductError::usage(
                "STACK_CONFIG_INVALID",
                "every explicit capacity must be positive",
            ));
        }
        // A backoff that starts at zero would retry instantly and turn a rate
        // limit into a tight loop against the origin.
        if self.limits.retry_attempts > 0
            && (self.limits.retry_initial_delay_milliseconds == 0
                || self.limits.retry_maximum_delay_milliseconds
                    < self.limits.retry_initial_delay_milliseconds)
        {
            return Err(ProductError::usage(
                "STACK_CONFIG_INVALID",
                "the retry backoff must start above zero and not exceed its own ceiling",
            ));
        }
        Ok(())
    }

    fn sha256(&self) -> Result<String> {
        Ok(sha256(&compact_json_line(self, "STACK_SERIALIZE_FAILED")?))
    }
}

/// One row that survived filtering: an identifier plus the governance the
/// manifest will carry.
#[derive(Clone, Debug)]
struct SelectedRow {
    blob_id: String,
    repository: String,
    path: String,
    revision: String,
    length_bytes: u64,
    license_expression: String,
}

#[derive(Clone, Debug, Default)]
struct FilterCounts {
    rows: u64,
    skipped_language: u64,
    skipped_license: u64,
    skipped_oversize: u64,
    skipped_incomplete: u64,
    skipped_duplicate: u64,
    skipped_partition: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackGenerationRef {
    pub relative_path: String,
    pub sha256: String,
    pub documents: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackIndexV1 {
    pub schema: String,
    pub profile: String,
    pub source_snapshot_id: String,
    pub generations: Vec<StackGenerationRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackSourceResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub profile: String,
    pub configuration_sha256: String,
    pub index_sha256: String,
    pub generations: u64,
    pub metadata_rows: u64,
    pub documents: u64,
    pub total_bytes: u64,
    pub skipped_language: u64,
    pub skipped_license: u64,
    pub skipped_oversize: u64,
    pub skipped_incomplete: u64,
    pub skipped_duplicate: u64,
    pub skipped_partition: u64,
    pub retries_performed: u64,
    pub output_created: bool,
    pub receipts_written: bool,
    pub limitations: Vec<String>,
}

pub fn materialize_stack_source(config_path: &Path) -> Result<serde_json::Value> {
    crate::platform::require_portable_data_host()?;
    let config_bytes = read_control_file(config_path, None, "STACK_CONFIG_READ_FAILED")?;
    let config: StackSourceConfigV1 = parse_closed(&config_bytes, "STACK_CONFIG_INVALID")?;
    config.validate()?;
    require_output_boundary(&config.output_root, &config.output_root)?;

    let allowlist = config
        .license_allowlist
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    let mut counts = FilterCounts::default();
    let mut selected = Vec::new();
    let mut blob_ids = BTreeSet::new();
    for shard in &config.metadata_shards {
        read_shard(
            shard,
            &config,
            &allowlist,
            &mut counts,
            &mut blob_ids,
            &mut selected,
        )?;
    }
    if selected.is_empty() {
        // An empty slice is a normal outcome once the work is partitioned: a
        // sixteen-way split of any real shard set will have buckets nothing
        // landed in. Reporting that as a failure would force the operator loop
        // to treat one error code as success, which is exactly how a genuine
        // misconfiguration gets missed. An unpartitioned run selecting nothing
        // is still a failure, because there the filters really are wrong.
        if config.blob_id_partition.is_some() {
            return empty_partition_result(&config, counts);
        }
        return Err(ProductError::gate(
            "STACK_NO_DOCUMENTS",
            "no metadata row survived the language, licence and size filters",
        ));
    }
    // Ordered by the content address, so a generation is a deterministic function
    // of the shards it was built from rather than of the order they were read.
    selected.sort_by(|left, right| left.blob_id.as_bytes().cmp(right.blob_id.as_bytes()));

    resolve_and_publish(&config, selected, counts)
}

fn read_shard(
    shard: &HashBoundInput,
    config: &StackSourceConfigV1,
    allowlist: &BTreeSet<&str>,
    counts: &mut FilterCounts,
    blob_ids: &mut BTreeSet<String>,
    selected: &mut Vec<SelectedRow>,
) -> Result<()> {
    verify_shard_digest(&shard.path, &shard.sha256)?;
    let file = std::fs::File::open(&shard.path).map_err(|_| {
        ProductError::environment(
            "STACK_METADATA_READ_FAILED",
            "could not open a verified metadata shard",
        )
    })?;
    let builder = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|error| parquet_failure("STACK_METADATA_INVALID", error))?
        .with_batch_size(BATCH_ROWS);
    let reader = builder
        .build()
        .map_err(|error| parquet_failure("STACK_METADATA_INVALID", error))?;

    for batch in reader {
        let batch = batch.map_err(|error| arrow_failure("STACK_METADATA_INVALID", error))?;
        project_batch(&batch, config, allowlist, counts, blob_ids, selected)?;
        if selected.len() as u64 > config.limits.maximum_documents {
            return Err(ProductError::gate(
                "STACK_DOCUMENT_LIMIT_EXCEEDED",
                "the selected metadata rows exceed the configured document bound",
            ));
        }
    }
    Ok(())
}

/// Streams the shard to confirm the digest the configuration pinned. Parquet
/// shards run past the control-file bound, so this is the read that binds them
/// rather than `read_control_file`.
fn verify_shard_digest(path: &Path, expected: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|_| {
        ProductError::environment(
            "STACK_METADATA_READ_FAILED",
            "could not open a metadata shard",
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).map_err(|_| {
            ProductError::environment(
                "STACK_METADATA_READ_FAILED",
                "could not read a metadata shard",
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if hex::encode(hasher.finalize()) != expected {
        return Err(ProductError::integrity(
            "STACK_METADATA_HASH_MISMATCH",
            "a metadata shard does not match its declared digest",
        ));
    }
    Ok(())
}

fn parquet_failure(code: &'static str, error: parquet::errors::ParquetError) -> ProductError {
    // The codec set is deliberately pure Rust, so an unsupported one is a
    // configuration fact worth naming rather than an opaque decode failure.
    ProductError::integrity(
        code,
        format!("the metadata shard could not be read: {error}"),
    )
}

fn arrow_failure(code: &'static str, error: arrow_schema::ArrowError) -> ProductError {
    ProductError::integrity(
        code,
        format!("the metadata shard could not be decoded: {error}"),
    )
}

fn missing_column(name: &str) -> ProductError {
    ProductError::integrity(
        "STACK_METADATA_COLUMN_MISSING",
        format!("the metadata shard has no usable string column named {name}"),
    )
}

fn string_column<'a>(
    batch: &'a arrow_array::RecordBatch,
    name: &str,
) -> Result<&'a arrow_array::StringArray> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<arrow_array::StringArray>())
        .ok_or_else(|| missing_column(name))
}

/// Reads one batch, keeping only rows that clear every declared filter.
///
/// The order of the checks is deliberate: identity completeness first, then the
/// cheap declared filters, and the size bound last but still before any transfer.
/// Every rejection is counted rather than silently dropped, because a run that
/// admits far fewer documents than expected should say which rule did it.
fn project_batch(
    batch: &arrow_array::RecordBatch,
    config: &StackSourceConfigV1,
    allowlist: &BTreeSet<&str>,
    counts: &mut FilterCounts,
    blob_ids: &mut BTreeSet<String>,
    selected: &mut Vec<SelectedRow>,
) -> Result<()> {
    use arrow_array::Array;
    let columns = &config.columns;
    let blob = string_column(batch, &columns.blob_id)?;
    let repository = string_column(batch, &columns.repository)?;
    let path = string_column(batch, &columns.path)?;
    let revision = string_column(batch, &columns.revision)?;
    let language = string_column(batch, &columns.language)?;
    let lengths = batch
        .column_by_name(&columns.length_bytes)
        .and_then(|column| column.as_any().downcast_ref::<arrow_array::Int64Array>())
        .ok_or_else(|| missing_column(&columns.length_bytes))?;
    let licenses = batch
        .column_by_name(&columns.detected_licenses)
        .and_then(|column| column.as_any().downcast_ref::<arrow_array::ListArray>())
        .ok_or_else(|| missing_column(&columns.detected_licenses))?;

    for row in 0..batch.num_rows() {
        counts.rows += 1;
        if blob.is_null(row)
            || repository.is_null(row)
            || path.is_null(row)
            || revision.is_null(row)
            || language.is_null(row)
            || lengths.is_null(row)
            || licenses.is_null(row)
        {
            counts.skipped_incomplete += 1;
            continue;
        }
        let blob_id = blob.value(row);
        // The identifier is the content address every later check depends on, so
        // a malformed one is refused rather than requested.
        if blob_id.len() != 40 || !blob_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            counts.skipped_incomplete += 1;
            continue;
        }
        // Filtered here, before any other rule, so a partitioned run does the
        // same work per surviving row as an unpartitioned one and the counts
        // below stay comparable between them.
        if let Some(partition) = &config.blob_id_partition
            && !partition.contains(blob_id)
        {
            counts.skipped_partition += 1;
            continue;
        }
        if language.value(row) != config.language {
            counts.skipped_language += 1;
            continue;
        }
        let Some(license_expression) = row_license(licenses, row, allowlist)? else {
            counts.skipped_license += 1;
            continue;
        };
        let declared = lengths.value(row);
        if declared <= 0 || declared as u64 > MAXIMUM_DOCUMENT_BYTES {
            counts.skipped_oversize += 1;
            continue;
        }
        // The same blob can appear under many repositories; the corpus wants it
        // once, and deduplicating here avoids transferring it more than once.
        if !blob_ids.insert(blob_id.to_owned()) {
            counts.skipped_duplicate += 1;
            continue;
        }
        selected.push(SelectedRow {
            blob_id: blob_id.to_owned(),
            repository: repository.value(row).to_owned(),
            path: path.value(row).to_owned(),
            revision: revision.value(row).to_owned(),
            length_bytes: declared as u64,
            license_expression,
        });
    }
    Ok(())
}

/// A partition that nothing landed in: a success that publishes nothing.
///
/// Deliberately its own status rather than a materialized generation with zero
/// documents. `curate` would reject an empty generation, and an operator sweeping
/// a directory of outputs should find only generations that carry documents; an
/// empty one would be a trap. `output_created: false` says plainly that there is
/// nothing here to feed forward.
fn empty_partition_result(
    config: &StackSourceConfigV1,
    counts: FilterCounts,
) -> Result<serde_json::Value> {
    let result = StackSourceResultV1 {
        schema: STACK_RESULT_SCHEMA.to_owned(),
        status: "STACK_PARTITION_EMPTY".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        profile: config.profile.clone(),
        configuration_sha256: config.sha256()?,
        index_sha256: String::new(),
        generations: 0,
        metadata_rows: counts.rows,
        documents: 0,
        total_bytes: 0,
        skipped_language: counts.skipped_language,
        skipped_license: counts.skipped_license,
        skipped_oversize: counts.skipped_oversize,
        skipped_incomplete: counts.skipped_incomplete,
        skipped_duplicate: counts.skipped_duplicate,
        skipped_partition: counts.skipped_partition,
        retries_performed: 0,
        output_created: false,
        receipts_written: false,
        limitations: vec!["partition-selected-no-rows-so-nothing-was-published".to_owned()],
    };
    serde_json::to_value(result).map_err(|_| {
        ProductError::internal(
            "STACK_RESULT_SERIALIZE_FAILED",
            "could not serialize the closed Stack v2 source result",
        )
    })
}
/// Resolves every selected identifier to bytes, then publishes the content tree
/// and the sharded source manifests `curate` consumes.
///
/// The whole generation is create-new: it is assembled in a partial tree that is
/// removed on any failure, so an interrupted run leaves nothing that could be
/// mistaken for a complete one.
fn resolve_and_publish(
    config: &StackSourceConfigV1,
    selected: Vec<SelectedRow>,
    counts: FilterCounts,
) -> Result<serde_json::Value> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(std::time::Duration::from_secs(
            config.limits.connect_timeout_seconds,
        ))
        .timeout_read(std::time::Duration::from_secs(
            config.limits.read_timeout_seconds,
        ))
        .build();

    let policy = crate::acquire::RetryPolicy {
        attempts: config.limits.retry_attempts,
        initial_delay: std::time::Duration::from_millis(
            config.limits.retry_initial_delay_milliseconds,
        ),
        maximum_delay: std::time::Duration::from_millis(
            config.limits.retry_maximum_delay_milliseconds,
        ),
    };

    let mut generation = PartialTree::create(&config.output_root)?;
    let mut documents = Vec::with_capacity(selected.len());
    let mut total_bytes = 0_u64;
    let mut retries_performed = 0_u64;
    for row in &selected {
        let relative_path = format!("documents/{}.py", row.blob_id);
        require_portable_relative_path(&relative_path, "STACK_CONTENT_PATH_INVALID")?;
        let url = config
            .content_url_template
            .replace("{blob_id}", &row.blob_id);
        let (fetched, retries) = crate::acquire::fetch_with_retry(
            &agent,
            crate::acquire::FetchRequest {
                url: &url,
                credential_env: config.content_credential_env.as_deref(),
                // The shard's declared length is the transfer ceiling, so a blob
                // that disagrees with its metadata is stopped mid-stream rather
                // than after it has all arrived. A body that will be inflated is
                // bounded by what its compressed form could plausibly be.
                ceiling_bytes: match config.content_encoding {
                    StackContentEncodingV1::Identity => row.length_bytes,
                    StackContentEncodingV1::Gzip => compressed_ceiling(row.length_bytes),
                },
            },
            config.limits.maximum_redirects,
            config.allow_loopback_plain_http,
            &policy,
        )?;
        retries_performed += retries;
        let _ = fetched.redirects_followed;
        // Decoding happens before every check, so length, identifier and the
        // recorded digest all describe the bytes that reach the content tree
        // rather than whatever framing carried them.
        let content = match config.content_encoding {
            StackContentEncodingV1::Identity => fetched.bytes,
            StackContentEncodingV1::Gzip => inflate(&fetched.bytes, row.length_bytes)?,
        };
        if content.len() as u64 != row.length_bytes {
            return Err(ProductError::integrity(
                "STACK_CONTENT_LENGTH_MISMATCH",
                "a Software Heritage blob differs in length from its metadata row",
            ));
        }
        // This is the link that makes an identifier into content: the archive is
        // content-addressed by `sha1_git`, so recomputing it locally proves the
        // bytes are the ones the shard named rather than whatever the endpoint
        // chose to return.
        let observed = sha1_git(&content);
        if observed != row.blob_id {
            return Err(ProductError::integrity(
                "STACK_CONTENT_HASH_MISMATCH",
                "a Software Heritage blob does not match the identifier that selected it",
            ));
        }
        total_bytes = total_bytes.checked_add(row.length_bytes).ok_or_else(|| {
            ProductError::gate("STACK_ACCOUNTING_OVERFLOW", "byte accounting overflowed")
        })?;
        if total_bytes > config.limits.maximum_total_bytes {
            return Err(ProductError::gate(
                "STACK_TOTAL_BYTES_EXCEEDED",
                "the resolved content exceeds the configured total byte bound",
            ));
        }
        generation.write(&relative_path, &content)?;
        documents.push(SourceDocument {
            provider_record_id: row.blob_id.clone(),
            // The repository is what dedup and splitting group by, so it comes
            // from the row rather than from the path.
            provider_repository_id: Some(row.repository.clone()),
            stable_provenance_origin_namespace: config.source_snapshot_id.clone(),
            relative_path: relative_path.clone(),
            expected_raw_sha256: sha256(&content),
            expected_raw_bytes: row.length_bytes,
            dialect: "python3".to_owned(),
            license_expression: row.license_expression.clone(),
            provenance: Provenance {
                origin_url: format!("https://softwareheritage.org/swh:1:cnt:{}", row.blob_id),
                revision: row.revision.clone(),
                source_path: row.path.clone(),
            },
        });
    }

    let shard_size = usize::try_from(config.documents_per_generation).map_err(|_| {
        ProductError::usage(
            "STACK_CONFIG_INVALID",
            "the per-generation bound does not fit this host's address width",
        )
    })?;
    let mut generations = Vec::new();
    for (index, shard) in documents.chunks(shard_size).enumerate() {
        let manifest = MaterializedSourceManifestV1 {
            schema: crate::data::SOURCE_MANIFEST_SCHEMA.to_owned(),
            adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
            source_snapshot_id: config.source_snapshot_id.clone(),
            authorization: config.authorization.clone(),
            required_removal_authorities: config.required_removal_authorities.clone(),
            documents: shard.to_vec(),
        };
        let bytes = compact_json_line(&manifest, "STACK_SERIALIZE_FAILED")?;
        let relative_path = format!("source-manifest-{index:05}.json");
        generation.write(&relative_path, &bytes)?;
        generations.push(StackGenerationRef {
            relative_path,
            sha256: sha256(&bytes),
            documents: shard.len() as u64,
            bytes: shard
                .iter()
                .map(|document| document.expected_raw_bytes)
                .sum(),
        });
    }

    let index = StackIndexV1 {
        schema: crate::materialize::MATERIALIZE_INDEX_SCHEMA.to_owned(),
        profile: config.profile.clone(),
        source_snapshot_id: config.source_snapshot_id.clone(),
        generations,
    };
    let index_bytes = compact_json_line(&index, "STACK_SERIALIZE_FAILED")?;
    generation.write("index.json", &index_bytes)?;
    generation.publish()?;

    let result = StackSourceResultV1 {
        schema: STACK_RESULT_SCHEMA.to_owned(),
        status: "STACK_SOURCE_MATERIALIZED".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        profile: config.profile.clone(),
        configuration_sha256: config.sha256()?,
        index_sha256: sha256(&index_bytes),
        generations: index.generations.len() as u64,
        metadata_rows: counts.rows,
        documents: documents.len() as u64,
        total_bytes,
        skipped_language: counts.skipped_language,
        skipped_license: counts.skipped_license,
        skipped_oversize: counts.skipped_oversize,
        skipped_incomplete: counts.skipped_incomplete,
        skipped_duplicate: counts.skipped_duplicate,
        skipped_partition: counts.skipped_partition,
        retries_performed,
        output_created: true,
        receipts_written: false,
        limitations: vec![
            // Said plainly because each is a real boundary of what this command
            // establishes, and none of them is visible from the result alone.
            "licence-comes-from-the-shard-and-is-not-independently-reviewed".to_owned(),
            "authorization-is-operator-declared-not-verified".to_owned(),
            "content-identity-is-verified-but-provenance-metadata-is-not".to_owned(),
        ],
    };
    serde_json::to_value(result).map_err(|_| {
        ProductError::internal(
            "STACK_RESULT_SERIALIZE_FAILED",
            "could not serialize the closed Stack v2 source result",
        )
    })
}

/// Git's blob object identity, which is what Software Heritage addresses content
/// by: SHA-1 over `blob <len>\0` followed by the bytes.
fn sha1_git(bytes: &[u8]) -> String {
    let mut hasher = <sha1::Sha1 as sha1::Digest>::new();
    sha1::Digest::update(&mut hasher, format!("blob {}\0", bytes.len()).as_bytes());
    sha1::Digest::update(&mut hasher, bytes);
    hex::encode(sha1::Digest::finalize(hasher))
}

/// A row is admitted only when it declares at least one licence and *every*
/// licence it declares is allowlisted. Dual-licensed rows therefore need both
/// terms approved, which is the conservative reading and the one a licence
/// allowlist exists to enforce.
fn row_license(
    licenses: &arrow_array::ListArray,
    row: usize,
    allowlist: &BTreeSet<&str>,
) -> Result<Option<String>> {
    use arrow_array::Array;
    let values = licenses.value(row);
    let values = values
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .ok_or_else(|| {
            ProductError::integrity(
                "STACK_METADATA_COLUMN_MISSING",
                "the detected-licence column is not a list of strings",
            )
        })?;
    if values.is_empty() {
        return Ok(None);
    }
    let mut admitted = BTreeSet::new();
    for index in 0..values.len() {
        if values.is_null(index) {
            return Ok(None);
        }
        let license = values.value(index);
        if !allowlist.contains(license) {
            return Ok(None);
        }
        admitted.insert(license);
    }
    Ok(Some(admitted.into_iter().collect::<Vec<_>>().join(" AND ")))
}
