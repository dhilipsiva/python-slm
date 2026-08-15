//! Deterministic governed-source metadata with explicit non-review defaults.

use super::{LICENSE_POLICY, Provenance};
use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const GOVERNED_SOURCE_POLICY_SCHEMA: &str = "python-slm-governed-source-policy-v1";
pub const GOVERNED_SOURCE_METADATA_SCHEMA: &str = "python-slm-governed-source-metadata-v1";
pub const GOVERNED_SOURCE_POLICY_ID: &str = "governed-source-defaults-v1";
pub const FRESHNESS_BASIS: &str = "provider-ordered-snapshots-without-external-review-v1";

const POLICY_BYTES: &[u8] = include_bytes!("governed-source-policy-v1.json");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedSourcePolicy {
    pub schema: String,
    pub policy_id: String,
    pub external_review_status: String,
    pub defaults: GovernedSourceDefaults,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedSourceDefaults {
    pub provenance_status: String,
    pub license_status: String,
    pub removal_status: String,
    pub freshness_status: String,
    pub source_status: String,
    pub freshness_basis: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedSourcePolicyBinding {
    pub schema: &'static str,
    pub policy_id: String,
    pub policy_sha256: String,
    pub external_review_status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedSourceMetadata {
    pub schema: &'static str,
    pub policy_id: String,
    pub policy_sha256: String,
    pub source_status: String,
    pub external_review_status: String,
    pub source_identity: GovernedSourceIdentity,
    pub provenance: GovernedPolicyFact,
    pub license: GovernedLicenseFact,
    pub removal: GovernedRemovalFact,
    pub freshness: GovernedFreshnessFact,
    pub restricted_values_emitted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedSourceIdentity {
    pub source_id: String,
    pub repository_group_id: String,
    pub source_snapshot_id: String,
    pub expected_raw_sha256: String,
    pub expected_raw_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedPolicyFact {
    pub status: String,
    pub binding_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedLicenseFact {
    pub status: String,
    pub expression: String,
    pub policy_id: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedRemovalFact {
    pub status: String,
    pub selected_snapshot_count: u64,
    pub selected_snapshots_sha256: String,
    pub record_removed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedFreshnessFact {
    pub status: String,
    pub basis: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedRemovalSnapshot {
    pub authority_url: String,
    pub provider_snapshot_id: String,
    pub provider_order: u64,
    pub manifest_sha256: String,
    pub publication_time_utc: String,
    pub retrieval_time_utc: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GovernedSourceInput {
    pub source_id: String,
    pub repository_group_id: String,
    pub source_snapshot_id: String,
    pub expected_raw_sha256: String,
    pub expected_raw_bytes: u64,
    pub provenance: Provenance,
    pub license_expression: String,
    pub removal_snapshots: Vec<GovernedRemovalSnapshot>,
    pub record_removed: bool,
}

pub fn governed_source_policy() -> Result<(GovernedSourcePolicy, GovernedSourcePolicyBinding)> {
    let policy: GovernedSourcePolicy = serde_json::from_slice(POLICY_BYTES).map_err(|_| {
        ProductError::internal(
            "GOVERNED_SOURCE_POLICY_INVALID",
            "the checked-in governed-source policy is malformed",
        )
    })?;
    validate_policy(&policy)?;
    let binding = GovernedSourcePolicyBinding {
        schema: GOVERNED_SOURCE_POLICY_SCHEMA,
        policy_id: policy.policy_id.clone(),
        policy_sha256: sha256(POLICY_BYTES),
        external_review_status: policy.external_review_status.clone(),
    };
    Ok((policy, binding))
}

pub fn evaluate_governed_source_metadata(
    policy: &GovernedSourcePolicy,
    input: GovernedSourceInput,
) -> Result<GovernedSourceMetadata> {
    validate_policy(policy)?;
    require_sha256(&input.source_id, "SOURCE_ID_INVALID")?;
    require_sha256(&input.repository_group_id, "REPOSITORY_GROUP_ID_INVALID")?;
    require_sha256(&input.expected_raw_sha256, "SOURCE_RAW_IDENTITY_INVALID")?;
    if input.expected_raw_bytes == 0 || input.source_snapshot_id.is_empty() {
        return Err(ProductError::integrity(
            "SOURCE_IDENTITY_INVALID",
            "governed source identity fields must be nonempty",
        ));
    }

    let provenance_binding_sha256 = hash_fields(
        b"python-slm/governed-provenance/v1\0",
        &[
            input.provenance.origin_url.as_str(),
            input.provenance.revision.as_str(),
            input.provenance.source_path.as_str(),
        ],
    );
    let mut snapshots = input.removal_snapshots;
    snapshots.sort_by(|left, right| {
        left.authority_url
            .as_bytes()
            .cmp(right.authority_url.as_bytes())
    });
    let mut authorities = BTreeSet::new();
    for snapshot in &snapshots {
        if !authorities.insert(snapshot.authority_url.as_str()) {
            return Err(ProductError::integrity(
                "GOVERNED_REMOVAL_AUTHORITY_DUPLICATE",
                "governed removal metadata repeats an authority",
            ));
        }
        require_sha256(&snapshot.manifest_sha256, "GOVERNED_REMOVAL_HASH_INVALID")?;
    }
    if snapshots.is_empty() {
        return Err(ProductError::integrity(
            "GOVERNED_REMOVAL_EMPTY",
            "governed source metadata requires selected removal snapshots",
        ));
    }
    let snapshot_bytes = serde_json::to_vec(&snapshots).map_err(|_| {
        ProductError::internal(
            "GOVERNED_REMOVAL_SERIALIZATION_FAILED",
            "could not canonicalize governed removal metadata",
        )
    })?;
    let policy_sha256 = sha256(POLICY_BYTES);
    Ok(GovernedSourceMetadata {
        schema: GOVERNED_SOURCE_METADATA_SCHEMA,
        policy_id: policy.policy_id.clone(),
        policy_sha256,
        source_status: policy.defaults.source_status.clone(),
        external_review_status: policy.external_review_status.clone(),
        source_identity: GovernedSourceIdentity {
            source_id: input.source_id,
            repository_group_id: input.repository_group_id,
            source_snapshot_id: input.source_snapshot_id,
            expected_raw_sha256: input.expected_raw_sha256,
            expected_raw_bytes: input.expected_raw_bytes,
        },
        provenance: GovernedPolicyFact {
            status: policy.defaults.provenance_status.clone(),
            binding_sha256: provenance_binding_sha256,
        },
        license: GovernedLicenseFact {
            status: policy.defaults.license_status.clone(),
            expression: input.license_expression,
            policy_id: LICENSE_POLICY,
        },
        removal: GovernedRemovalFact {
            status: policy.defaults.removal_status.clone(),
            selected_snapshot_count: snapshots.len() as u64,
            selected_snapshots_sha256: sha256(&snapshot_bytes),
            record_removed: input.record_removed,
        },
        freshness: GovernedFreshnessFact {
            status: policy.defaults.freshness_status.clone(),
            basis: policy.defaults.freshness_basis.clone(),
        },
        restricted_values_emitted: false,
    })
}

fn validate_policy(policy: &GovernedSourcePolicy) -> Result<()> {
    if policy.schema != GOVERNED_SOURCE_POLICY_SCHEMA
        || policy.policy_id != GOVERNED_SOURCE_POLICY_ID
        || policy.external_review_status != "UNAVAILABLE"
        || policy.defaults.provenance_status != "ASSUMED"
        || policy.defaults.license_status != "ASSUMED"
        || policy.defaults.removal_status != "ASSUMED"
        || policy.defaults.freshness_status != "UNVERIFIED"
        || policy.defaults.source_status != "UNVERIFIED"
        || policy.defaults.freshness_basis != FRESHNESS_BASIS
    {
        return Err(ProductError::integrity(
            "GOVERNED_SOURCE_POLICY_UNSUPPORTED",
            "the governed-source policy changes a frozen default",
        ));
    }
    Ok(())
}

fn require_sha256(value: &str, code: &'static str) -> Result<()> {
    if value.len() != 64
        || !value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ProductError::integrity(
            code,
            "a governed metadata hash is not lowercase SHA-256",
        ));
    }
    Ok(())
}

fn hash_fields(domain: &[u8], values: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    for value in values {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
