#![cfg(windows)]

use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::data::{
    ADAPTER_NAMESPACE, AUTHORIZATION_SCHEME, CURATE_CONFIG_SCHEMA, CurateConfigV1, HashBoundPath,
    IngestionLimits, MaterializedSourceManifestV1, Provenance, REMOVAL_MANIFEST_SCHEMA,
    RemovalManifestV1, SENSITIVE_POLICY_ID, SENSITIVE_REGISTRY_ID, SENSITIVE_RESULT_SCHEMA,
    SOURCE_MANIFEST_SCHEMA, SourceAuthorization, SourceDocument, evaluate_sensitive_policy,
    policy_registry, sensitive_policy_binding,
};
use rust_llm_pretrain::parser::CancellationToken;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[test]
fn frozen_registry_identity_and_result_shape_are_stable() {
    let registry = policy_registry().unwrap();
    let binding = sensitive_policy_binding().unwrap();
    assert_eq!(registry.policy_id, SENSITIVE_POLICY_ID);
    assert_eq!(registry.registry_id, SENSITIVE_REGISTRY_ID);
    assert_eq!(binding.policy_id, SENSITIVE_POLICY_ID);
    assert_eq!(binding.registry_id, SENSITIVE_REGISTRY_ID);
    let bytes = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/data/sensitive-rules-v1.json"),
    )
    .unwrap();
    assert_eq!(binding.registry_sha256, sha256(&bytes));

    let result = evaluate_sensitive_policy(
        b"value = 'ordinary deterministic fixture'\n",
        &CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(result.schema, SENSITIVE_RESULT_SCHEMA);
    assert_eq!(result.status, "ACCEPTED");
    assert_eq!(result.finding_count, 0);
    assert!(!result.restricted_values_emitted);
    assert!(!result.source_rewritten);
}

#[test]
fn confirmed_sensitive_categories_reject_without_emitting_values() {
    let cases: &[(&[u8], &str)] = &[
        (
            b"value = '-----BEGIN PRIVATE KEY-----'\n",
            "PRIVATE_KEY_PEM",
        ),
        (b"value = 'AKIAABCDEFGHIJKLMNOP'\n", "PROVIDER_CREDENTIAL"),
        (
            b"value = 'https://user:pass@host.test/path'\n",
            "CREDENTIALED_URL",
        ),
        (
            b"client_secret = 'aB3!dE5@gH7#jK9$mN2%qR4&'\n",
            "HIGH_ENTROPY_NAMED_SECRET",
        ),
        (b"value = 'alice@corp.test'\n", "PERSONAL_EMAIL"),
        (b"value = '+1 (415) 555-2671'\n", "TELEPHONE_NUMBER"),
        (b"phone: str = '415-555-2671'\n", "TELEPHONE_NUMBER"),
        (b"value = '123-45-6789'\n", "GOVERNMENT_IDENTIFIER"),
        (
            b"value = '4000000000000002'\n",
            "PAYMENT_ACCOUNT_IDENTIFIER",
        ),
        (
            b"value = 'GB82WEST12345698765432'\n",
            "PAYMENT_ACCOUNT_IDENTIFIER",
        ),
        (
            b"value = '1600 Amphitheatre Parkway, Mountain View, 94043'\n",
            "POSTAL_ADDRESS",
        ),
    ];
    for (source, expected_rule) in cases {
        let result = evaluate_sensitive_policy(source, &CancellationToken::default()).unwrap();
        assert_eq!(result.status, "REJECTED", "missing {expected_rule}");
        assert!(result.rule_ids.iter().any(|rule| rule == expected_rule));
        assert_eq!(result.reason, Some("SENSITIVE_CONTENT_DETECTED"));
        let public = serde_json::to_string(&result).unwrap();
        assert!(!public.contains(&String::from_utf8_lossy(source).to_string()));
        assert!(!result.restricted_values_emitted);
        assert!(!result.source_rewritten);
    }
}

#[test]
fn reserved_and_non_sensitive_lookalikes_pass() {
    let source = br#"
email = "developer@example.com"
password = "changeme"
test_card = "4242424242424242"
checksum = "A1B2C3D4E5F60718293A4B5C6D7E8F90"
url = "https://example.invalid/path"
integer = 14155552671
lookalike = "prefixAKIAABCDEFGHIJKLMNOP"
"#;
    let result = evaluate_sensitive_policy(source, &CancellationToken::default()).unwrap();
    assert_eq!(result.status, "ACCEPTED");
    assert!(result.rule_ids.is_empty());
}

#[test]
fn uncertain_labeled_values_are_quarantined() {
    let cases: &[(&[u8], &str)] = &[
        (b"password = 'customer-choice'\n", "POSSIBLE_NAMED_SECRET"),
        (
            b"passport = 'AB12345678'\n",
            "POSSIBLE_GOVERNMENT_IDENTIFIER",
        ),
        (b"location = '123 Main Street'\n", "POSSIBLE_POSTAL_ADDRESS"),
    ];
    for (source, expected_rule) in cases {
        let result = evaluate_sensitive_policy(source, &CancellationToken::default()).unwrap();
        assert_eq!(result.status, "QUARANTINED");
        assert!(result.rule_ids.iter().any(|rule| rule == expected_rule));
        assert_eq!(result.reason, Some("SENSITIVE_CONTENT_UNCERTAIN"));
    }
}

#[test]
fn provider_token_and_reserved_domain_boundaries_are_exact() {
    let short = evaluate_sensitive_policy(
        b"value = 'AKIAABCDEFGHIJKLMNO'\n",
        &CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(short.status, "ACCEPTED");
    let exact = evaluate_sensitive_policy(
        b"value = 'AKIAABCDEFGHIJKLMNOP'\n",
        &CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(exact.status, "REJECTED");
    let reserved = evaluate_sensitive_policy(
        b"value = 'dev@sub.example.org'\n",
        &CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(reserved.status, "ACCEPTED");
}

#[test]
fn cancellation_is_fatal_and_fresh_state_remains_usable() {
    let cancelled = CancellationToken::default();
    cancelled.cancel();
    let error = evaluate_sensitive_policy(b"value = 1\n", &cancelled).unwrap_err();
    assert_eq!(error.code, "SENSITIVE_POLICY_CANCELLED");

    let fresh = evaluate_sensitive_policy(b"value = 1\n", &CancellationToken::default()).unwrap();
    assert_eq!(fresh.status, "ACCEPTED");
}

#[test]
fn curate_excludes_rejected_and_quarantined_bytes_and_detects_mutation() {
    let root = tempfile::tempdir().unwrap();
    let content_root = root.path().join("content");
    fs::create_dir(&content_root).unwrap();
    let accepted = padded(b"value = 'ordinary source'\n".to_vec());
    let rejected = padded(b"contact = 'alice@corp.test'\n".to_vec());
    let quarantined = padded(b"password = 'customer-choice'\n".to_vec());
    for (name, bytes) in [
        ("accepted.py", &accepted),
        ("rejected.py", &rejected),
        ("quarantined.py", &quarantined),
    ] {
        fs::write(content_root.join(name), bytes).unwrap();
    }

    let source_path = root.path().join("source.json");
    let source = MaterializedSourceManifestV1 {
        schema: SOURCE_MANIFEST_SCHEMA.to_owned(),
        adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
        source_snapshot_id: "snapshot-p6-1".to_owned(),
        authorization: SourceAuthorization {
            scheme: AUTHORIZATION_SCHEME.to_owned(),
            authority_url: "https://huggingface.co/datasets/bigcode/the-stack-v2".to_owned(),
            authorization_id: "owner-authorized-p6".to_owned(),
        },
        required_removal_authorities: vec!["https://example.invalid/removals".to_owned()],
        documents: vec![
            document("accepted", "accepted.py", &accepted),
            document("rejected", "rejected.py", &rejected),
            document("quarantined", "quarantined.py", &quarantined),
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
            provider_snapshot_id: "removals-p6-1".to_owned(),
            provider_order: 1,
            publication_time_utc: "2026-08-15T00:00:00Z".to_owned(),
            retrieval_time_utc: "2026-08-15T00:01:00Z".to_owned(),
            removed_provider_record_ids: vec![],
            removed_provider_repository_ids: vec![],
        },
    );
    let output = root.path().join("generation");
    let config_path = root.path().join("config.json");
    write_config(
        &config_path,
        &source_path,
        &content_root,
        &removal_path,
        &removal_bytes,
        &output,
    );

    let result = run_curate(&config_path);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty());
    let result_json: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result_json["schema"], "python-slm-curate-result-v3");
    assert_eq!(result_json["parser_accepted_count"], 3);
    assert_eq!(result_json["policy_accepted_count"], 1);
    assert_eq!(result_json["quarantined_count"], 1);
    assert_eq!(result_json["rejected_count"], 1);
    assert_eq!(fs::read_dir(output.join("documents")).unwrap().count(), 1);
    assert_eq!(fs::read_dir(output.join("parser")).unwrap().count(), 3);
    assert_eq!(fs::read_dir(output.join("policy")).unwrap().count(), 3);

    let manifest_bytes = fs::read(output.join("manifest.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let outcomes = manifest["outcomes"].as_array().unwrap();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome["status"] == "POLICY_ACCEPTED")
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome["status"] == "QUARANTINED")
            .count(),
        1
    );
    let mut public = manifest_bytes;
    public.extend_from_slice(&result.stdout);
    for entry in fs::read_dir(output.join("policy")).unwrap() {
        public.extend_from_slice(&fs::read(entry.unwrap().path()).unwrap());
    }
    let public = String::from_utf8(public).unwrap();
    assert!(!public.contains("alice@corp.test"));
    assert!(!public.contains("customer-choice"));
    assert!(!public.contains(&root.path().display().to_string()));
    assert!(!output.join("receipts").exists());

    let mutated_output = root.path().join("mutated-generation");
    let mutated_config = root.path().join("mutated-config.json");
    write_config(
        &mutated_config,
        &source_path,
        &content_root,
        &removal_path,
        &removal_bytes,
        &mutated_output,
    );
    let mut changed = accepted.clone();
    let changed_index = changed.len() - 2;
    changed[changed_index] ^= 1;
    fs::write(content_root.join("accepted.py"), changed).unwrap();
    let mutated = run_curate(&mutated_config);
    assert_eq!(mutated.status.code(), Some(3));
    let error: Value = serde_json::from_slice(&mutated.stderr).unwrap();
    assert_eq!(error["code"], "DOCUMENT_HASH_MISMATCH");
    assert!(!mutated_output.exists());
    assert!(fs::read_dir(root.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".partial-")
    }));
}

fn document(record: &str, relative: &str, bytes: &[u8]) -> SourceDocument {
    SourceDocument {
        provider_record_id: format!("record-{record}"),
        provider_repository_id: Some("repository-p6".to_owned()),
        stable_provenance_origin_namespace: String::new(),
        relative_path: relative.to_owned(),
        expected_raw_sha256: sha256(bytes),
        expected_raw_bytes: bytes.len() as u64,
        dialect: "python3".to_owned(),
        license_expression: "MIT".to_owned(),
        provenance: Provenance {
            origin_url: format!("https://archive.softwareheritage.org/{record}"),
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

fn write_config(
    path: &Path,
    source: &Path,
    content: &Path,
    removal: &Path,
    removal_bytes: &[u8],
    output: &Path,
) {
    write_json(
        path,
        &CurateConfigV1 {
            schema: CURATE_CONFIG_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            source_manifest: source.to_owned(),
            content_root: content.to_owned(),
            removal_manifests: vec![HashBoundPath {
                path: removal.to_owned(),
                sha256: sha256(removal_bytes),
            }],
            output_root: output.to_owned(),
            limits: IngestionLimits {
                maximum_documents: 3,
                maximum_total_raw_bytes: 1_000_000,
            },
        },
    );
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
