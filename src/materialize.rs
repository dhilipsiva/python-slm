//! Turn an authorized local tree into `curate` inputs.
//!
//! `curate` consumes a `MaterializedSourceManifestV1` naming every document with
//! its license, provenance, and expected SHA-256. At production scale that is
//! millions of entries, and even a modest proof run is thousands, so it cannot be
//! written by hand. This performs the mechanical half — enumerate, hash, order —
//! and leaves the governance half where it belongs: the operator declares the
//! authorization, the license expression, and the removal authorities in the
//! configuration, and this asserts nothing it was not told.
//!
//! Output is sharded into as many generations as the operator asks for, because a
//! single generation manifest is capped near 28,000 accepted documents by the
//! 64 MiB control-file bound. `curate` runs once per shard and the resulting
//! generations compose through `prepare-corpus`.

use crate::data::source::{compact_json_line, sha256};
use crate::data::{
    ADAPTER_NAMESPACE, MaterializedSourceManifestV1, Provenance, SourceAuthorization,
    SourceDocument,
};
use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const IMPLEMENTATION_PHASE: &str = "E3";
pub const MATERIALIZE_CONFIG_SCHEMA: &str = "python-slm-materialize-source-config-v1";
pub const MATERIALIZE_INDEX_SCHEMA: &str = "python-slm-materialize-source-index-v1";
pub const MATERIALIZE_RESULT_SCHEMA: &str = "python-slm-materialize-source-result-v1";

/// Bound on tree walking, so a cyclic or pathological layout cannot run forever.
const MAXIMUM_TREE_DEPTH: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializeSourceConfigV1 {
    pub schema: String,
    pub profile: String,
    /// The authorized tree. It is read, never modified, and becomes `curate`'s
    /// `content_root` unchanged, so nothing is copied.
    pub source_root: PathBuf,
    pub output_root: PathBuf,
    pub source_snapshot_id: String,
    /// Declared by the operator. Nothing here verifies it against anything
    /// external; it is the operator's assertion that these bytes are authorized.
    pub authorization: SourceAuthorization,
    pub required_removal_authorities: Vec<String>,
    /// Applied to every document in this tree. A tree with mixed licensing needs
    /// more than one run.
    pub license_expression: String,
    /// Provenance base; each document's origin URL is this plus its relative path.
    pub origin_url: String,
    pub revision: String,
    pub maximum_documents: u64,
    pub documents_per_generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedGenerationRef {
    pub relative_path: String,
    pub sha256: String,
    pub documents: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializeIndexV1 {
    pub schema: String,
    pub profile: String,
    pub source_snapshot_id: String,
    pub generations: Vec<MaterializedGenerationRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializeSourceResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub profile: String,
    pub configuration_sha256: String,
    pub index_sha256: String,
    pub generations: u64,
    pub documents: u64,
    pub total_bytes: u64,
    pub skipped_empty: u64,
    pub skipped_not_regular: u64,
    pub output_created: bool,
    pub receipts_written: bool,
    pub limitations: Vec<String>,
}

impl MaterializeSourceConfigV1 {
    pub fn validate(&self) -> Result<()> {
        if self.schema != MATERIALIZE_CONFIG_SCHEMA {
            return Err(ProductError::usage(
                "MATERIALIZE_CONFIG_INVALID",
                "the configuration is not the closed materialize schema",
            ));
        }
        if self.profile != crate::backend::PROTOTYPE_PROFILE {
            return Err(ProductError::gate(
                "DEFERRED_POST_P16",
                "only the prototype profile is implemented",
            ));
        }
        if !self.source_root.is_absolute() || !self.output_root.is_absolute() {
            return Err(ProductError::usage(
                "MATERIALIZE_PATH_NOT_ABSOLUTE",
                "every materialize path must be absolute",
            ));
        }
        if self.authorization.scheme != crate::data::AUTHORIZATION_SCHEME {
            return Err(ProductError::usage(
                "MATERIALIZE_AUTHORIZATION_INVALID",
                "the authorization scheme is not the frozen materialized-source scheme",
            ));
        }
        if self.required_removal_authorities.is_empty() {
            return Err(ProductError::usage(
                "MATERIALIZE_REMOVAL_AUTHORITY_MISSING",
                "at least one removal authority must be declared; curate requires one",
            ));
        }
        if self.maximum_documents == 0 || self.documents_per_generation == 0 {
            return Err(ProductError::usage(
                "MATERIALIZE_LIMIT_INVALID",
                "both document limits must be positive",
            ));
        }
        crate::acquire::require_acquisition_url(&self.origin_url, false)?;
        Ok(())
    }

    pub fn sha256(&self) -> Result<String> {
        Ok(sha256(&compact_json_line(
            self,
            "MATERIALIZE_SERIALIZE_FAILED",
        )?))
    }
}

struct Enumerated {
    relative: String,
    bytes: u64,
    sha256: String,
}

struct Counts {
    skipped_empty: u64,
    skipped_not_regular: u64,
}

/// Collect every `.py` file beneath `root`, in a deterministic order.
///
/// Entries that `curate` would abort on are left out rather than listed and then
/// fatally rejected: a reparse point trips `DOCUMENT_REPARSE_REJECTED` and a
/// zero-length file trips `DOCUMENT_IDENTITY_INVALID`, and both abort the whole
/// generation rather than skipping one document.
fn enumerate(root: &Path) -> Result<(Vec<Enumerated>, Counts)> {
    let mut found = Vec::new();
    let mut counts = Counts {
        skipped_empty: 0,
        skipped_not_regular: 0,
    };
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    while let Some((directory, depth)) = stack.pop() {
        if depth > MAXIMUM_TREE_DEPTH {
            return Err(ProductError::integrity(
                "MATERIALIZE_TREE_TOO_DEEP",
                "the source tree exceeded its depth bound",
            ));
        }
        let entries = std::fs::read_dir(&directory).map_err(|_| {
            ProductError::environment(
                "MATERIALIZE_READ_FAILED",
                format!("could not read {}", directory.display()),
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|_| {
                ProductError::environment(
                    "MATERIALIZE_READ_FAILED",
                    "could not read a directory entry",
                )
            })?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|_| {
                ProductError::environment(
                    "MATERIALIZE_READ_FAILED",
                    "could not inspect a directory entry",
                )
            })?;
            // Never follow a reparse point; curate would reject it fatally.
            if metadata.file_type().is_symlink() {
                counts.skipped_not_regular += 1;
                continue;
            }
            if metadata.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            if !metadata.is_file() || path.extension().is_none_or(|ext| ext != "py") {
                continue;
            }
            if metadata.len() == 0 {
                counts.skipped_empty += 1;
                continue;
            }
            let relative = path.strip_prefix(root).map_err(|_| {
                ProductError::integrity(
                    "MATERIALIZE_PATH_INVALID",
                    "a discovered path escaped the source root",
                )
            })?;
            // Portable relative paths are forward-slash separated and contain no
            // drive or component separators of their own.
            let relative = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if relative.is_empty() || relative.contains('\\') || relative.contains(':') {
                counts.skipped_not_regular += 1;
                continue;
            }
            let content = std::fs::read(&path).map_err(|_| {
                ProductError::environment(
                    "MATERIALIZE_READ_FAILED",
                    format!("could not read {}", path.display()),
                )
            })?;
            found.push(Enumerated {
                relative,
                bytes: content.len() as u64,
                sha256: sha256(&content),
            });
        }
    }
    // Deterministic order, so the same tree always yields the same manifests and
    // the same sharding.
    found.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok((found, counts))
}

pub fn materialize_source(config_path: &Path) -> Result<serde_json::Value> {
    crate::platform::require_portable_data_host()?;
    let config_bytes = crate::data::source::read_control_file(
        config_path,
        None,
        "MATERIALIZE_CONFIG_READ_FAILED",
    )?;
    let config: MaterializeSourceConfigV1 =
        crate::data::source::parse_closed(&config_bytes, "MATERIALIZE_CONFIG_INVALID")?;
    config.validate()?;
    let root = crate::data::source::require_existing_root(
        &config.source_root,
        "MATERIALIZE_SOURCE_ROOT_INVALID",
    )?;
    crate::data::source::require_output_boundary(&config.output_root, &root)?;

    let (documents, counts) = enumerate(&root)?;
    if documents.is_empty() {
        return Err(ProductError::gate(
            "MATERIALIZE_NO_DOCUMENTS",
            "the authorized tree contains no eligible Python documents",
        ));
    }
    if documents.len() as u64 > config.maximum_documents {
        return Err(ProductError::gate(
            "MATERIALIZE_DOCUMENT_LIMIT_EXCEEDED",
            "the authorized tree exceeds the configured document bound",
        ));
    }

    let mut generation = crate::acquire::PartialTree::create(&config.output_root)?;
    let shard_size = usize::try_from(config.documents_per_generation).map_err(|_| {
        ProductError::usage(
            "MATERIALIZE_LIMIT_INVALID",
            "the per-generation bound does not fit this host's address width",
        )
    })?;
    let mut references = Vec::new();
    let mut total_bytes = 0_u64;
    for (index, shard) in documents.chunks(shard_size).enumerate() {
        let mut shard_bytes = 0_u64;
        let entries = shard
            .iter()
            .map(|item| {
                shard_bytes += item.bytes;
                SourceDocument {
                    // The relative path is unique within the tree, which is what
                    // makes it usable as the provider record identity.
                    provider_record_id: item.relative.clone(),
                    // The first path component groups documents the way a
                    // repository would, which is what dedup and splitting use.
                    provider_repository_id: item.relative.split('/').next().map(str::to_owned),
                    stable_provenance_origin_namespace: config.source_snapshot_id.clone(),
                    relative_path: item.relative.clone(),
                    expected_raw_sha256: item.sha256.clone(),
                    expected_raw_bytes: item.bytes,
                    dialect: "python3".to_owned(),
                    license_expression: config.license_expression.clone(),
                    provenance: Provenance {
                        origin_url: format!(
                            "{}/{}",
                            config.origin_url.trim_end_matches('/'),
                            item.relative
                        ),
                        revision: config.revision.clone(),
                        source_path: item.relative.clone(),
                    },
                }
            })
            .collect::<Vec<_>>();
        let manifest = MaterializedSourceManifestV1 {
            schema: crate::data::SOURCE_MANIFEST_SCHEMA.to_owned(),
            adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
            source_snapshot_id: config.source_snapshot_id.clone(),
            authorization: config.authorization.clone(),
            required_removal_authorities: config.required_removal_authorities.clone(),
            documents: entries,
        };
        let bytes = compact_json_line(&manifest, "MATERIALIZE_SERIALIZE_FAILED")?;
        let relative_path = format!("source-manifest-{index:05}.json");
        generation.write(&relative_path, &bytes)?;
        references.push(MaterializedGenerationRef {
            relative_path,
            sha256: sha256(&bytes),
            documents: shard.len() as u64,
            bytes: shard_bytes,
        });
        total_bytes += shard_bytes;
    }

    let index = MaterializeIndexV1 {
        schema: MATERIALIZE_INDEX_SCHEMA.to_owned(),
        profile: config.profile.clone(),
        source_snapshot_id: config.source_snapshot_id.clone(),
        generations: references,
    };
    let index_bytes = compact_json_line(&index, "MATERIALIZE_SERIALIZE_FAILED")?;
    generation.write("index.json", &index_bytes)?;
    generation.publish()?;

    let result = MaterializeSourceResultV1 {
        schema: MATERIALIZE_RESULT_SCHEMA.to_owned(),
        status: "SOURCE_ENUMERATED".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        profile: config.profile.clone(),
        configuration_sha256: config.sha256()?,
        index_sha256: sha256(&index_bytes),
        generations: index.generations.len() as u64,
        documents: documents.len() as u64,
        total_bytes,
        skipped_empty: counts.skipped_empty,
        skipped_not_regular: counts.skipped_not_regular,
        output_created: true,
        receipts_written: false,
        limitations: vec![
            "enumeration-is-not-license-review".to_owned(),
            "authorization-and-license-are-operator-declared-not-verified".to_owned(),
            "one-license-expression-is-applied-to-the-whole-tree".to_owned(),
        ],
    };
    serde_json::to_value(result).map_err(|_| {
        ProductError::internal(
            "MATERIALIZE_RESULT_SERIALIZE_FAILED",
            "could not serialize the closed materialize result",
        )
    })
}
