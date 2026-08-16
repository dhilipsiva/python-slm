use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::data::{
    ADAPTER_NAMESPACE, AUTHORIZATION_SCHEME, CURATE_CONFIG_SCHEMA, CurateConfigV1, FRESHNESS_BASIS,
    GOVERNED_SOURCE_METADATA_SCHEMA, GOVERNED_SOURCE_POLICY_ID, GOVERNED_SOURCE_POLICY_SCHEMA,
    GovernedRemovalSnapshot, GovernedSourceInput, HashBoundPath, IngestionLimits,
    MaterializedSourceManifestV1, Provenance, REMOVAL_MANIFEST_SCHEMA, RemovalManifestV1,
    SOURCE_MANIFEST_SCHEMA, SourceAuthorization, SourceDocument, evaluate_governed_source_metadata,
    governed_source_policy, repository_group_id, source_id,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

#[test]
fn configured_defaults_are_hash_bound_and_never_invent_external_review() {
    let (policy, binding) = governed_source_policy().unwrap();
    let policy_bytes = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/data/governed-source-policy-v1.json"),
    )
    .unwrap();
    assert_eq!(policy.schema, GOVERNED_SOURCE_POLICY_SCHEMA);
    assert_eq!(policy.policy_id, GOVERNED_SOURCE_POLICY_ID);
    assert_eq!(binding.policy_sha256, sha256(&policy_bytes));
    assert_eq!(binding.external_review_status, "UNAVAILABLE");

    let first = evaluate_governed_source_metadata(&policy, governed_input(false)).unwrap();
    let mut reversed_input = governed_input(false);
    reversed_input.removal_snapshots.reverse();
    let second = evaluate_governed_source_metadata(&policy, reversed_input).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.schema, GOVERNED_SOURCE_METADATA_SCHEMA);
    assert_eq!(first.source_status, "UNVERIFIED");
    assert_eq!(first.external_review_status, "UNAVAILABLE");
    assert_eq!(first.provenance.status, "ASSUMED");
    assert_eq!(first.license.status, "ASSUMED");
    assert_eq!(first.removal.status, "ASSUMED");
    assert_eq!(first.freshness.status, "UNVERIFIED");
    assert_eq!(first.freshness.basis, FRESHNESS_BASIS);
    assert!(!first.restricted_values_emitted);

    let public = serde_json::to_string(&first).unwrap();
    assert!(!public.contains("private-review-token"));
    assert!(!public.contains("credential"));
    assert!(!public.contains(r"C:\Users\"));
}

#[test]
fn policy_mutation_and_ambiguous_removal_metadata_fail_closed() {
    let (mut policy, _) = governed_source_policy().unwrap();
    policy.defaults.source_status = "ASSUMED".to_owned();
    let error = evaluate_governed_source_metadata(&policy, governed_input(false)).unwrap_err();
    assert_eq!(error.code, "GOVERNED_SOURCE_POLICY_UNSUPPORTED");

    let (policy, _) = governed_source_policy().unwrap();
    let mut duplicate = governed_input(false);
    duplicate.removal_snapshots[1].authority_url =
        duplicate.removal_snapshots[0].authority_url.clone();
    let error = evaluate_governed_source_metadata(&policy, duplicate).unwrap_err();
    assert_eq!(error.code, "GOVERNED_REMOVAL_AUTHORITY_DUPLICATE");

    let mut empty = governed_input(false);
    empty.removal_snapshots.clear();
    let error = evaluate_governed_source_metadata(&policy, empty).unwrap_err();
    assert_eq!(error.code, "GOVERNED_REMOVAL_EMPTY");
}

#[test]
fn curate_v4_publishes_governance_for_accepted_and_removed_documents() {
    let root = tempfile::tempdir().unwrap();
    let content_root = root.path().join("content");
    fs::create_dir(&content_root).unwrap();
    let active = padded(b"active_value = 1\n".to_vec());
    let removed = padded(b"removed_value = 2\n".to_vec());
    fs::write(content_root.join("active.py"), &active).unwrap();
    fs::write(content_root.join("removed.py"), &removed).unwrap();

    let source_path = root.path().join("source.json");
    let source = MaterializedSourceManifestV1 {
        schema: SOURCE_MANIFEST_SCHEMA.to_owned(),
        adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
        source_snapshot_id: "snapshot-p7a-1".to_owned(),
        authorization: SourceAuthorization {
            scheme: AUTHORIZATION_SCHEME.to_owned(),
            authority_url: "https://example.invalid/source-authority".to_owned(),
            authorization_id: "configured-authorization-p7a".to_owned(),
        },
        required_removal_authorities: vec!["https://example.invalid/removals".to_owned()],
        documents: vec![
            document("active", "active.py", &active),
            document("removed", "removed.py", &removed),
        ],
    };
    write_json(&source_path, &source);

    let removal_path = root.path().join("removal.json");
    let removal_bytes = write_json(
        &removal_path,
        &RemovalManifestV1 {
            schema: REMOVAL_MANIFEST_SCHEMA.to_owned(),
            adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
            authority_url: "https://example.invalid/removals".to_owned(),
            provider_snapshot_id: "removals-p7a-1".to_owned(),
            provider_order: 7,
            publication_time_utc: "2026-08-15T00:00:00Z".to_owned(),
            retrieval_time_utc: "2026-08-15T00:01:00Z".to_owned(),
            removed_provider_record_ids: vec!["record-removed".to_owned()],
            removed_provider_repository_ids: vec![],
        },
    );

    let output_root = root.path().join("generation");
    let config_path = root.path().join("config.json");
    write_json(
        &config_path,
        &CurateConfigV1 {
            schema: CURATE_CONFIG_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            source_manifest: source_path,
            content_root,
            removal_manifests: vec![HashBoundPath {
                path: removal_path,
                sha256: sha256(&removal_bytes),
            }],
            output_root: output_root.clone(),
            limits: IngestionLimits {
                maximum_documents: 2,
                maximum_total_raw_bytes: 1_000_000,
            },
        },
    );

    let result = run_curate(&config_path);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty());
    let result_json: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result_json["schema"], "python-slm-curate-result-v4");
    assert_eq!(result_json["governance_status"], "COMPLETE");
    assert_eq!(result_json["unverified_source_count"], 2);
    assert_eq!(result_json["qualification_status"], "SKIPPED");

    let manifest_bytes = fs::read(output_root.join("manifest.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["schema"], "python-slm-source-generation-v4");
    assert_eq!(manifest["governance_status"], "COMPLETE");
    assert_eq!(
        manifest["governed_source_policy"]["policy_id"],
        GOVERNED_SOURCE_POLICY_ID
    );
    let outcomes = manifest["outcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), 2);
    for outcome in outcomes {
        let governed = &outcome["governed_source_metadata"];
        assert_eq!(governed["schema"], GOVERNED_SOURCE_METADATA_SCHEMA);
        assert_eq!(governed["source_status"], "UNVERIFIED");
        assert_eq!(governed["provenance"]["status"], "ASSUMED");
        assert_eq!(governed["license"]["status"], "ASSUMED");
        assert_eq!(governed["removal"]["status"], "ASSUMED");
        assert_eq!(governed["freshness"]["status"], "UNVERIFIED");
        assert_eq!(governed["restricted_values_emitted"], false);
        let expected_removed = outcome["provider_record_id"] == "record-removed";
        assert_eq!(
            governed["removal"]["record_removed"].as_bool().unwrap(),
            expected_removed
        );
    }
    let public = String::from_utf8([manifest_bytes, result.stdout].concat()).unwrap();
    assert!(!public.contains(&root.path().display().to_string()));
    assert!(!output_root.join("receipts").exists());
}

fn governed_input(record_removed: bool) -> GovernedSourceInput {
    GovernedSourceInput {
        source_id: source_id(ADAPTER_NAMESPACE, "snapshot-p7a", "record-p7a"),
        repository_group_id: repository_group_id(
            ADAPTER_NAMESPACE,
            "snapshot-p7a",
            Some("repository-p7a"),
            "",
        ),
        source_snapshot_id: "snapshot-p7a".to_owned(),
        expected_raw_sha256: sha256(b"governed bytes"),
        expected_raw_bytes: 14,
        provenance: Provenance {
            origin_url: "https://example.invalid/credential".to_owned(),
            revision: "private-review-token".to_owned(),
            source_path: "src/example.py".to_owned(),
        },
        license_expression: "MIT".to_owned(),
        removal_snapshots: vec![
            snapshot("https://b.example.invalid/removals", "b", 2),
            snapshot("https://a.example.invalid/removals", "a", 1),
        ],
        record_removed,
    }
}

fn snapshot(authority: &str, id: &str, order: u64) -> GovernedRemovalSnapshot {
    GovernedRemovalSnapshot {
        authority_url: authority.to_owned(),
        provider_snapshot_id: format!("snapshot-{id}"),
        provider_order: order,
        manifest_sha256: sha256(id.as_bytes()),
        publication_time_utc: "2026-08-15T00:00:00Z".to_owned(),
        retrieval_time_utc: "2026-08-15T00:01:00Z".to_owned(),
    }
}

fn document(record: &str, relative: &str, bytes: &[u8]) -> SourceDocument {
    SourceDocument {
        provider_record_id: format!("record-{record}"),
        provider_repository_id: Some("repository-p7a".to_owned()),
        stable_provenance_origin_namespace: String::new(),
        relative_path: relative.to_owned(),
        expected_raw_sha256: sha256(bytes),
        expected_raw_bytes: bytes.len() as u64,
        dialect: "python3".to_owned(),
        license_expression: "MIT".to_owned(),
        provenance: Provenance {
            origin_url: format!("https://example.invalid/source/{record}"),
            revision: format!("revision-{record}"),
            source_path: relative.to_owned(),
        },
    }
}

fn padded(mut bytes: Vec<u8>) -> Vec<u8> {
    while bytes.len() < 120 {
        bytes.extend_from_slice(b"value = 1\n");
    }
    bytes
}

fn run_curate(config: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_python-slm"))
        .args(["curate", "--config"])
        .arg(config)
        .output()
        .unwrap()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    fs::write(path, &bytes).unwrap();
    bytes
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
