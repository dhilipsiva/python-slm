mod io;

pub(crate) use io::*;

use super::{
    ADAPTER_NAMESPACE, AUTHORIZATION_SCHEME, CurateConfigV1, IngestionLimits,
    MaterializedSourceManifestV1, RemovalManifestV1, SourceDocument,
};
use crate::backend::PROTOTYPE_PROFILE;
use crate::error::{ProductError, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub(crate) struct SelectedRemovalSnapshot {
    pub(crate) manifest: RemovalManifestV1,
    pub(crate) sha256: String,
    pub(crate) removed_records: BTreeSet<String>,
    pub(crate) removed_repositories: BTreeSet<String>,
}

#[derive(Debug)]
pub(crate) struct PreparedDocument {
    pub(crate) source_id: String,
    pub(crate) repository_group_id: String,
    pub(crate) document: SourceDocument,
}

pub(crate) fn validate_config(config: &CurateConfigV1) -> Result<()> {
    if config.schema != super::CURATE_CONFIG_SCHEMA {
        return Err(ProductError::usage(
            "CONFIG_SCHEMA_UNSUPPORTED",
            "the curate configuration schema is unsupported",
        ));
    }
    if config.profile != PROTOTYPE_PROFILE {
        return Err(ProductError::gate(
            "DEFERRED_POST_P16",
            "the requested profile is designed but not implemented",
        ));
    }
    for path in [
        &config.source_manifest,
        &config.content_root,
        &config.output_root,
    ] {
        if !path.is_absolute() {
            return Err(ProductError::usage(
                "CONFIG_PATH_NOT_ABSOLUTE",
                "curate configuration paths must be absolute",
            ));
        }
    }
    if config.removal_manifests.is_empty()
        || config
            .removal_manifests
            .iter()
            .any(|entry| !entry.path.is_absolute() || !is_sha256(&entry.sha256))
    {
        return Err(ProductError::usage(
            "REMOVAL_INPUT_INVALID",
            "removal manifests must be absolute and hash-bound",
        ));
    }
    if config.limits.maximum_documents == 0 || config.limits.maximum_total_raw_bytes == 0 {
        return Err(ProductError::usage(
            "INGESTION_LIMIT_INVALID",
            "ingestion limits must be positive",
        ));
    }
    Ok(())
}

pub(crate) fn validate_source_manifest(
    source: &MaterializedSourceManifestV1,
    limits: &IngestionLimits,
) -> Result<()> {
    if source.schema != super::SOURCE_MANIFEST_SCHEMA
        || source.adapter_namespace != ADAPTER_NAMESPACE
        || source.authorization.scheme != AUTHORIZATION_SCHEME
    {
        return Err(ProductError::integrity(
            "SOURCE_AUTHORIZATION_INVALID",
            "the source manifest does not use the authorized materialized adapter",
        ));
    }
    require_bounded_text(&source.source_snapshot_id, false, "SOURCE_SNAPSHOT_INVALID")?;
    require_https(
        &source.authorization.authority_url,
        "SOURCE_AUTHORITY_INVALID",
    )?;
    require_bounded_text(
        &source.authorization.authorization_id,
        false,
        "SOURCE_AUTHORIZATION_INVALID",
    )?;
    if source.required_removal_authorities.is_empty()
        || source.documents.len() as u64 > limits.maximum_documents
        || source.documents.is_empty()
    {
        return Err(ProductError::integrity(
            "SOURCE_MANIFEST_BOUND_INVALID",
            "the source manifest violates an explicit ingestion bound",
        ));
    }
    let authorities = source
        .required_removal_authorities
        .iter()
        .map(|authority| {
            require_https(authority, "REMOVAL_AUTHORITY_INVALID")?;
            Ok(authority.clone())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if authorities.len() != source.required_removal_authorities.len() {
        return Err(ProductError::integrity(
            "REMOVAL_AUTHORITY_DUPLICATE",
            "required removal authorities must be unique",
        ));
    }
    let mut total = 0_u64;
    for document in &source.documents {
        total = total
            .checked_add(document.expected_raw_bytes)
            .ok_or_else(|| {
                ProductError::integrity(
                    "SOURCE_BYTE_COUNT_OVERFLOW",
                    "declared source bytes overflowed",
                )
            })?;
        if total > limits.maximum_total_raw_bytes {
            return Err(ProductError::integrity(
                "SOURCE_TOTAL_LIMIT_EXCEEDED",
                "declared source bytes exceed the explicit aggregate limit",
            ));
        }
    }
    Ok(())
}

pub(crate) fn prepare_documents(
    source: &MaterializedSourceManifestV1,
) -> Result<Vec<PreparedDocument>> {
    let mut provider_ids = BTreeSet::new();
    let mut source_ids = BTreeSet::new();
    let mut prepared = Vec::with_capacity(source.documents.len());
    for document in &source.documents {
        require_bounded_text(
            &document.provider_record_id,
            false,
            "PROVIDER_RECORD_ID_INVALID",
        )?;
        if let Some(repository) = &document.provider_repository_id {
            require_bounded_text(repository, false, "PROVIDER_REPOSITORY_ID_INVALID")?;
        }
        require_bounded_text(
            &document.stable_provenance_origin_namespace,
            true,
            "PROVENANCE_NAMESPACE_INVALID",
        )?;
        require_portable_relative_path(&document.relative_path, "DOCUMENT_PATH_INVALID")?;
        require_portable_relative_path(
            &document.provenance.source_path,
            "PROVENANCE_PATH_INVALID",
        )?;
        require_https(&document.provenance.origin_url, "PROVENANCE_URL_INVALID")?;
        require_bounded_text(&document.provenance.revision, false, "PROVENANCE_INVALID")?;
        if !is_sha256(&document.expected_raw_sha256) || document.expected_raw_bytes == 0 {
            return Err(ProductError::integrity(
                "DOCUMENT_IDENTITY_INVALID",
                "a document has an invalid expected hash or length",
            ));
        }
        let source_id = source_id(
            &source.adapter_namespace,
            &source.source_snapshot_id,
            &document.provider_record_id,
        );
        let repository_group_id = repository_group_id(
            &source.adapter_namespace,
            &source.source_snapshot_id,
            document.provider_repository_id.as_deref(),
            &document.stable_provenance_origin_namespace,
        );
        if !provider_ids.insert(document.provider_record_id.clone())
            || !source_ids.insert(source_id.clone())
        {
            return Err(ProductError::integrity(
                "DOCUMENT_IDENTITY_DUPLICATE",
                "the source manifest contains a duplicate provider or source identity",
            ));
        }
        prepared.push(PreparedDocument {
            source_id,
            repository_group_id,
            document: document.clone(),
        });
    }
    prepared.sort_by(|left, right| left.source_id.as_bytes().cmp(right.source_id.as_bytes()));
    Ok(prepared)
}
pub(crate) fn canonical_source_manifest_sha256(
    source: &MaterializedSourceManifestV1,
) -> Result<String> {
    let mut canonical = source.clone();
    canonical
        .required_removal_authorities
        .sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    canonical.documents.sort_by(|left, right| {
        source_id(
            &canonical.adapter_namespace,
            &canonical.source_snapshot_id,
            &left.provider_record_id,
        )
        .as_bytes()
        .cmp(
            source_id(
                &canonical.adapter_namespace,
                &canonical.source_snapshot_id,
                &right.provider_record_id,
            )
            .as_bytes(),
        )
    });
    let bytes = serde_json::to_vec(&canonical).map_err(|_| {
        ProductError::internal(
            "SOURCE_MANIFEST_CANONICALIZATION_FAILED",
            "could not canonicalize the source manifest",
        )
    })?;
    Ok(sha256(&bytes))
}

pub(crate) fn load_removal_snapshots(
    config: &CurateConfigV1,
    source: &MaterializedSourceManifestV1,
) -> Result<BTreeMap<String, SelectedRemovalSnapshot>> {
    let required = source
        .required_removal_authorities
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut candidates: BTreeMap<String, Vec<SelectedRemovalSnapshot>> = BTreeMap::new();
    let mut seen_paths = BTreeSet::new();
    for input in &config.removal_manifests {
        let canonical = input.path.canonicalize().map_err(|_| {
            ProductError::environment(
                "REMOVAL_MANIFEST_READ_FAILED",
                "could not resolve a removal manifest",
            )
        })?;
        if !seen_paths.insert(canonical) {
            return Err(ProductError::integrity(
                "REMOVAL_MANIFEST_DUPLICATE",
                "the same removal manifest was supplied more than once",
            ));
        }
        let bytes = read_control_file(
            &input.path,
            Some(&input.sha256),
            "REMOVAL_MANIFEST_READ_FAILED",
        )?;
        let manifest: RemovalManifestV1 = parse_closed(&bytes, "REMOVAL_MANIFEST_INVALID")?;
        validate_removal_manifest(&manifest, source)?;
        if !required.contains(&manifest.authority_url) {
            return Err(ProductError::integrity(
                "REMOVAL_AUTHORITY_UNEXPECTED",
                "a removal manifest names an undeclared authority",
            ));
        }
        let removed_records = unique_values(
            &manifest.removed_provider_record_ids,
            "REMOVAL_RECORD_DUPLICATE",
        )?;
        let removed_repositories = unique_values(
            &manifest.removed_provider_repository_ids,
            "REMOVAL_REPOSITORY_DUPLICATE",
        )?;
        candidates
            .entry(manifest.authority_url.clone())
            .or_default()
            .push(SelectedRemovalSnapshot {
                manifest,
                sha256: sha256(&bytes),
                removed_records,
                removed_repositories,
            });
    }

    let mut selected = BTreeMap::new();
    for authority in required {
        let mut snapshots = candidates.remove(&authority).ok_or_else(|| {
            ProductError::integrity(
                "REMOVAL_AUTHORITY_MISSING",
                "a required removal authority has no supplied snapshot",
            )
        })?;
        let snapshot_ids = snapshots
            .iter()
            .map(|snapshot| snapshot.manifest.provider_snapshot_id.as_str())
            .collect::<BTreeSet<_>>();
        if snapshot_ids.len() != snapshots.len() {
            return Err(ProductError::integrity(
                "REMOVAL_SNAPSHOT_DUPLICATE",
                "a removal authority repeats a provider snapshot identity",
            ));
        }
        snapshots.sort_by_key(|snapshot| snapshot.manifest.provider_order);
        if snapshots
            .windows(2)
            .any(|pair| pair[0].manifest.provider_order == pair[1].manifest.provider_order)
        {
            return Err(ProductError::integrity(
                "REMOVAL_ORDER_AMBIGUOUS",
                "a removal authority has duplicate provider ordering values",
            ));
        }
        let newest = snapshots.pop().ok_or_else(|| {
            ProductError::internal(
                "REMOVAL_SELECTION_EMPTY",
                "a required removal authority lost its supplied snapshots",
            )
        })?;
        selected.insert(authority, newest);
    }
    Ok(selected)
}

fn validate_removal_manifest(
    manifest: &RemovalManifestV1,
    source: &MaterializedSourceManifestV1,
) -> Result<()> {
    if manifest.schema != super::REMOVAL_MANIFEST_SCHEMA
        || manifest.adapter_namespace != source.adapter_namespace
    {
        return Err(ProductError::integrity(
            "REMOVAL_MANIFEST_IDENTITY_INVALID",
            "a removal manifest has the wrong schema or adapter namespace",
        ));
    }
    require_https(&manifest.authority_url, "REMOVAL_AUTHORITY_INVALID")?;
    require_bounded_text(
        &manifest.provider_snapshot_id,
        false,
        "REMOVAL_SNAPSHOT_INVALID",
    )?;
    if !is_utc_second(&manifest.publication_time_utc)
        || !is_utc_second(&manifest.retrieval_time_utc)
        || manifest.retrieval_time_utc < manifest.publication_time_utc
    {
        return Err(ProductError::integrity(
            "REMOVAL_TIMESTAMP_INVALID",
            "removal timestamps must use canonical UTC-second syntax",
        ));
    }
    Ok(())
}

pub fn source_id(
    adapter_namespace: &str,
    source_snapshot_id: &str,
    provider_record_id: &str,
) -> String {
    domain_hash(
        b"python-slm/source-id/v1\0",
        &[adapter_namespace, source_snapshot_id, provider_record_id],
    )
}

pub fn repository_group_id(
    adapter_namespace: &str,
    source_snapshot_id: &str,
    provider_repository_id: Option<&str>,
    stable_provenance_origin_namespace: &str,
) -> String {
    match provider_repository_id {
        Some(repository) => domain_hash(
            b"python-slm/repository-group/v1\0",
            &[adapter_namespace, repository],
        ),
        None => domain_hash(
            b"python-slm/repository-group-fallback/v1\0",
            &[
                adapter_namespace,
                source_snapshot_id,
                stable_provenance_origin_namespace,
            ],
        ),
    }
}

fn domain_hash(domain: &[u8], values: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    for value in values {
        let bytes = value.as_bytes();
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    hex::encode(digest.finalize())
}

fn require_https(value: &str, code: &'static str) -> Result<()> {
    let remainder = value.strip_prefix("https://");
    let authority = remainder
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default();
    if value.len() > 4096
        || !value.is_ascii()
        || authority.is_empty()
        || authority.contains('@')
        || value.contains(['\\', '?', '#'])
        || value.chars().any(char::is_whitespace)
    {
        return Err(ProductError::integrity(
            code,
            "an authority or provenance URL is not a closed credential-free HTTPS identity",
        ));
    }
    Ok(())
}

fn require_bounded_text(value: &str, allow_empty: bool, code: &'static str) -> Result<()> {
    if (!allow_empty && value.is_empty())
        || value.len() > 4096
        || value.contains('\0')
        || value.chars().any(char::is_control)
    {
        return Err(ProductError::integrity(
            code,
            "a governed identity is invalid",
        ));
    }
    Ok(())
}

fn unique_values(values: &[String], code: &'static str) -> Result<BTreeSet<String>> {
    let mut unique = BTreeSet::new();
    for value in values {
        require_bounded_text(value, false, code)?;
        if !unique.insert(value.clone()) {
            return Err(ProductError::integrity(
                code,
                "a removal identity is duplicated",
            ));
        }
    }
    Ok(unique)
}

fn is_utc_second(value: &str) -> bool {
    if value.len() != 20 || !value.ends_with('Z') {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || !bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        })
    {
        return false;
    }
    let number = |start: usize, end: usize| {
        std::str::from_utf8(&bytes[start..end])
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0, 4),
        number(5, 7),
        number(8, 10),
        number(11, 13),
        number(14, 16),
        number(17, 19),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    year >= 1970 && (1..=days).contains(&day) && hour <= 23 && minute <= 59 && second <= 59
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_and_repository_id_vectors_are_stable() {
        assert_eq!(
            source_id("adapter-v1", "snapshot-1", "record-7"),
            "efd26d68e16677ff77f87161b630d414eac63abf5869e71be63f3bd69245ac73"
        );
        assert_eq!(
            repository_group_id("adapter-v1", "snapshot-1", Some("repo-2"), "ignored"),
            "5a505c4185a0a09f74c65a1328ba6ba259e25df53659f3034a55f4782255f7c8"
        );
    }

    #[test]
    fn removal_timestamp_syntax_is_closed() {
        assert!(is_utc_second("2026-08-15T12:00:00Z"));
        assert!(!is_utc_second("2026-08-15T12:00:00.1Z"));
        assert!(!is_utc_second("2026-02-29T12:00:00Z"));
        assert!(is_utc_second("2028-02-29T12:00:00Z"));
        assert!(!is_utc_second("2026-08-15T25:00:00Z"));
        assert!(!is_utc_second("2026-08-15 12:00:00Z"));
    }
    #[test]
    fn governed_urls_reject_credentials_queries_and_fragments() {
        assert!(require_https("https://example.invalid/path", "TEST").is_ok());
        for invalid in [
            "http://example.invalid/path",
            "https://user:secret@example.invalid/path",
            "https://example.invalid/path?token=secret",
            "https://example.invalid/path#fragment",
            "https://",
        ] {
            assert!(require_https(invalid, "TEST").is_err());
        }
    }
}
