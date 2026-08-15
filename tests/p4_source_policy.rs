#![cfg(windows)]

use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::data::{
    ADAPTER_NAMESPACE, AUTHORIZATION_SCHEME, CURATE_CONFIG_SCHEMA, CurateConfigV1, HashBoundPath,
    IngestionLimits, MaterializedSourceManifestV1, Provenance, REMOVAL_MANIFEST_SCHEMA,
    RemovalManifestV1, SOURCE_MANIFEST_SCHEMA, SourceAuthorization, SourceDocument,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const REMOVAL_AUTHORITY: &str = "https://example.invalid/stack-v2/removals";

struct Fixture {
    _root: tempfile::TempDir,
    first_config: PathBuf,
    second_config: PathBuf,
    first_output: PathBuf,
    second_output: PathBuf,
    content_root: PathBuf,
    documents: Vec<SourceDocument>,
    removal_path: PathBuf,
    removal_sha256: String,
}

#[test]
fn materializes_deterministic_parser_and_rejected_outcomes() {
    let fixture = build_fixture();

    let first = run_curate(&fixture.first_config);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(first.stderr.is_empty());
    let first_result: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first_result["schema"], "python-slm-curate-result-v3");
    assert_eq!(first_result["status"], "SOURCE_MATERIALIZED");
    assert_eq!(first_result["qualification_status"], "SKIPPED");
    assert_eq!(first_result["document_count"], 5);
    assert_eq!(first_result["parser_accepted_count"], 2);
    assert_eq!(first_result["policy_accepted_count"], 2);
    assert_eq!(first_result["quarantined_count"], 0);
    assert_eq!(first_result["rejected_count"], 3);
    assert_eq!(first_result["receipts_written"], false);

    let second = run_curate(&fixture.second_config);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_result: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(
        first_result["generation_manifest_sha256"],
        second_result["generation_manifest_sha256"]
    );

    let first_manifest = fs::read(fixture.first_output.join("manifest.json")).unwrap();
    let second_manifest = fs::read(fixture.second_output.join("manifest.json")).unwrap();
    assert_eq!(first_manifest, second_manifest);
    assert_eq!(String::from_utf8_lossy(&first_manifest).lines().count(), 1);
    let public = String::from_utf8(first_manifest.clone()).unwrap();
    assert!(!public.contains(&fixture._root.path().display().to_string()));
    assert!(!public.contains("C:\\"));
    let manifest: Value = serde_json::from_slice(&first_manifest).unwrap();
    assert_eq!(manifest["schema"], "python-slm-source-generation-v3");
    assert_eq!(manifest["parser_status"], "COMPLETE");
    assert_eq!(manifest["policy_status"], "COMPLETE");

    let outcomes = manifest["outcomes"].as_array().unwrap();
    let source_ids = outcomes
        .iter()
        .map(|outcome| outcome["source_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(source_ids.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome["status"] == "POLICY_ACCEPTED")
            .count(),
        2
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "REMOVED_BY_AUTHORITY"))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "LICENSE_NOT_ALLOWED"))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "SIZE_OUT_OF_RANGE"))
            .count(),
        1
    );

    let generated_documents = fs::read_dir(fixture.first_output.join("documents"))
        .unwrap()
        .count();
    assert_eq!(generated_documents, 2);
    for outcome in outcomes
        .iter()
        .filter(|outcome| outcome["status"] == "POLICY_ACCEPTED")
    {
        let relative = outcome["content_path"].as_str().unwrap();
        assert_eq!(
            fs::read(fixture.first_output.join(relative)).unwrap(),
            accepted_source()
        );
    }
}

#[test]
fn newest_removal_snapshot_is_selected_and_missing_authority_is_fatal() {
    let fixture = build_fixture();
    let older = RemovalManifestV1 {
        schema: REMOVAL_MANIFEST_SCHEMA.to_owned(),
        adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
        authority_url: REMOVAL_AUTHORITY.to_owned(),
        provider_snapshot_id: "removals-2026-08-14".to_owned(),
        provider_order: 6,
        publication_time_utc: "2026-08-14T01:00:00Z".to_owned(),
        retrieval_time_utc: "2026-08-14T02:00:00Z".to_owned(),
        removed_provider_record_ids: vec!["record-a".to_owned()],
        removed_provider_repository_ids: vec![],
    };
    let older_path = fixture._root.path().join("removal-older.json");
    let older_sha256 = sha256(&write_json(&older_path, &older));
    let source_path = fixture._root.path().join("ordered-removal-source.json");
    write_json(
        &source_path,
        &source_manifest(fixture.documents[..2].to_vec()),
    );
    let output = fixture._root.path().join("ordered-removal-output");
    let config_path = fixture._root.path().join("ordered-removal-config.json");
    write_json(
        &config_path,
        &CurateConfigV1 {
            schema: CURATE_CONFIG_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            source_manifest: source_path,
            content_root: fixture.content_root.clone(),
            removal_manifests: vec![
                HashBoundPath {
                    path: older_path,
                    sha256: older_sha256,
                },
                HashBoundPath {
                    path: fixture.removal_path.clone(),
                    sha256: fixture.removal_sha256.clone(),
                },
            ],
            output_root: output.clone(),
            limits: IngestionLimits {
                maximum_documents: 10,
                maximum_total_raw_bytes: 2_000_000,
            },
        },
    );
    let ordered = run_curate(&config_path);
    assert!(
        ordered.status.success(),
        "{}",
        String::from_utf8_lossy(&ordered.stderr)
    );
    let result: Value = serde_json::from_slice(&ordered.stdout).unwrap();
    assert_eq!(result["parser_accepted_count"], 2);
    let manifest: Value =
        serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["removal_snapshots"][0]["provider_snapshot_id"],
        "removals-2026-08-15"
    );

    let missing_source = fixture._root.path().join("missing-authority-source.json");
    let mut missing = source_manifest(vec![fixture.documents[0].clone()]);
    missing
        .required_removal_authorities
        .push("https://example.invalid/second-removal-authority".to_owned());
    write_json(&missing_source, &missing);
    let missing_output = fixture._root.path().join("missing-authority-output");
    let missing_config = fixture._root.path().join("missing-authority-config.json");
    write_config(
        &missing_config,
        &missing_source,
        &fixture.content_root,
        &fixture.removal_path,
        &fixture.removal_sha256,
        &missing_output,
    );
    let failed = run_curate(&missing_config);
    assert_eq!(failed.status.code(), Some(3));
    assert_eq!(error_code(&failed), "REMOVAL_AUTHORITY_MISSING");
    assert!(!missing_output.exists());
    require_no_partial(fixture._root.path());
}

#[test]
fn duplicate_identity_and_hash_mutation_abort_without_publication() {
    let fixture = build_fixture();
    let duplicate_manifest = MaterializedSourceManifestV1 {
        schema: SOURCE_MANIFEST_SCHEMA.to_owned(),
        adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
        source_snapshot_id: "snapshot-2026-08-15".to_owned(),
        authorization: authorization(),
        required_removal_authorities: vec![REMOVAL_AUTHORITY.to_owned()],
        documents: vec![fixture.documents[0].clone(), fixture.documents[0].clone()],
    };
    let duplicate_source = fixture._root.path().join("duplicate-source.json");
    write_json(&duplicate_source, &duplicate_manifest);
    let duplicate_output = fixture._root.path().join("duplicate-output");
    let duplicate_config = fixture._root.path().join("duplicate-config.json");
    write_config(
        &duplicate_config,
        &duplicate_source,
        &fixture.content_root,
        &fixture.removal_path,
        &fixture.removal_sha256,
        &duplicate_output,
    );
    let duplicate = run_curate(&duplicate_config);
    assert_eq!(duplicate.status.code(), Some(3));
    assert_eq!(error_code(&duplicate), "DOCUMENT_IDENTITY_DUPLICATE");
    assert!(!duplicate_output.exists());
    require_no_partial(fixture._root.path());

    let mutated_source = fixture._root.path().join("mutated-source.json");
    let manifest = MaterializedSourceManifestV1 {
        schema: SOURCE_MANIFEST_SCHEMA.to_owned(),
        adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
        source_snapshot_id: "snapshot-2026-08-15".to_owned(),
        authorization: authorization(),
        required_removal_authorities: vec![REMOVAL_AUTHORITY.to_owned()],
        documents: vec![fixture.documents[0].clone()],
    };
    write_json(&mutated_source, &manifest);
    let document_path = fixture
        .content_root
        .join(&fixture.documents[0].relative_path);
    let mut changed = accepted_source();
    let changed_index = changed.len() - 2;
    changed[changed_index] ^= 1;
    fs::write(&document_path, changed).unwrap();
    let mutated_output = fixture._root.path().join("mutated-output");
    let mutated_config = fixture._root.path().join("mutated-config.json");
    write_config(
        &mutated_config,
        &mutated_source,
        &fixture.content_root,
        &fixture.removal_path,
        &fixture.removal_sha256,
        &mutated_output,
    );
    let mutated = run_curate(&mutated_config);
    assert_eq!(mutated.status.code(), Some(3));
    assert_eq!(error_code(&mutated), "DOCUMENT_HASH_MISMATCH");
    assert!(!mutated_output.exists());
    require_no_partial(fixture._root.path());
}

#[test]
fn closed_configuration_and_create_new_output_fail_typed() {
    let fixture = build_fixture();
    let mut config: Value =
        serde_json::from_slice(&fs::read(&fixture.first_config).unwrap()).unwrap();
    config["unexpected"] = Value::Bool(true);
    let open_config = fixture._root.path().join("open-config.json");
    fs::write(&open_config, serde_json::to_vec(&config).unwrap()).unwrap();
    let open = run_curate(&open_config);
    assert_eq!(open.status.code(), Some(2));
    assert_eq!(error_code(&open), "CONFIG_INVALID");
    assert!(!fixture.first_output.exists());

    let first = run_curate(&fixture.first_config);
    assert!(first.status.success());
    let repeated = run_curate(&fixture.first_config);
    assert_eq!(repeated.status.code(), Some(3));
    assert_eq!(error_code(&repeated), "OUTPUT_ALREADY_EXISTS");
}

fn build_fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let content_root = root.path().join("content");
    fs::create_dir(&content_root).unwrap();

    let accepted = accepted_source();
    let oversized = vec![b'x'; 1_000_004];
    let specifications = [
        ("record-b", "same-b.py", accepted.clone(), "MIT"),
        ("record-a", "same-a.py", accepted.clone(), "Apache-2.0"),
        ("record-removed", "removed.py", accepted.clone(), "MIT"),
        (
            "record-license",
            "license.py",
            accepted.clone(),
            "GPL-3.0-only",
        ),
        ("record-large", "large.py", oversized, "MIT"),
    ];
    let mut documents = Vec::new();
    for (record, relative, bytes, license) in specifications {
        fs::write(content_root.join(relative), &bytes).unwrap();
        documents.push(SourceDocument {
            provider_record_id: record.to_owned(),
            provider_repository_id: Some(format!("repository-{record}")),
            stable_provenance_origin_namespace: String::new(),
            relative_path: relative.to_owned(),
            expected_raw_sha256: sha256(&bytes),
            expected_raw_bytes: bytes.len() as u64,
            dialect: "python3".to_owned(),
            license_expression: license.to_owned(),
            provenance: Provenance {
                origin_url: format!("https://archive.softwareheritage.org/{record}"),
                revision: format!("revision-{record}"),
                source_path: format!("src/{relative}"),
            },
        });
    }

    let removal = RemovalManifestV1 {
        schema: REMOVAL_MANIFEST_SCHEMA.to_owned(),
        adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
        authority_url: REMOVAL_AUTHORITY.to_owned(),
        provider_snapshot_id: "removals-2026-08-15".to_owned(),
        provider_order: 7,
        publication_time_utc: "2026-08-15T01:00:00Z".to_owned(),
        retrieval_time_utc: "2026-08-15T02:00:00Z".to_owned(),
        removed_provider_record_ids: vec!["record-removed".to_owned()],
        removed_provider_repository_ids: vec![],
    };
    let removal_path = root.path().join("removal.json");
    let removal_bytes = write_json(&removal_path, &removal);
    let removal_sha256 = sha256(&removal_bytes);

    let first_source = root.path().join("source-first.json");
    write_json(&first_source, &source_manifest(documents.clone()));
    let mut reversed = documents.clone();
    reversed.reverse();
    let second_source = root.path().join("source-second.json");
    write_json(&second_source, &source_manifest(reversed));

    let first_config = root.path().join("curate-first.json");
    let second_config = root.path().join("curate-second.json");
    let first_output = root.path().join("generation-first");
    let second_output = root.path().join("generation-second");
    write_config(
        &first_config,
        &first_source,
        &content_root,
        &removal_path,
        &removal_sha256,
        &first_output,
    );
    write_config(
        &second_config,
        &second_source,
        &content_root,
        &removal_path,
        &removal_sha256,
        &second_output,
    );
    Fixture {
        _root: root,
        first_config,
        second_config,
        first_output,
        second_output,
        content_root,
        documents,
        removal_path,
        removal_sha256,
    }
}

fn source_manifest(documents: Vec<SourceDocument>) -> MaterializedSourceManifestV1 {
    MaterializedSourceManifestV1 {
        schema: SOURCE_MANIFEST_SCHEMA.to_owned(),
        adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
        source_snapshot_id: "snapshot-2026-08-15".to_owned(),
        authorization: authorization(),
        required_removal_authorities: vec![REMOVAL_AUTHORITY.to_owned()],
        documents,
    }
}

fn authorization() -> SourceAuthorization {
    SourceAuthorization {
        scheme: AUTHORIZATION_SCHEME.to_owned(),
        authority_url: "https://huggingface.co/datasets/bigcode/the-stack-v2".to_owned(),
        authorization_id: "owner-authorized-stack-v2-swh-v1".to_owned(),
    }
}

fn write_config(
    path: &Path,
    source_manifest: &Path,
    content_root: &Path,
    removal_manifest: &Path,
    removal_sha256: &str,
    output_root: &Path,
) {
    write_json(
        path,
        &CurateConfigV1 {
            schema: CURATE_CONFIG_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            source_manifest: source_manifest.to_path_buf(),
            content_root: content_root.to_path_buf(),
            removal_manifests: vec![HashBoundPath {
                path: removal_manifest.to_path_buf(),
                sha256: removal_sha256.to_owned(),
            }],
            output_root: output_root.to_path_buf(),
            limits: IngestionLimits {
                maximum_documents: 10,
                maximum_total_raw_bytes: 2_000_000,
            },
        },
    );
}

fn accepted_source() -> Vec<u8> {
    let mut bytes = b"# coding: utf-8\ndef square(value):\n    return value * value\n\n".to_vec();
    bytes.extend_from_slice(b"def cube(value):\n    return value * value * value\n");
    while bytes.len() < 180 {
        bytes.extend_from_slice(b"\nvalue = square(7)\n");
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

fn error_code(output: &Output) -> String {
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert_eq!(stderr.lines().count(), 1);
    serde_json::from_str::<Value>(stderr.trim()).unwrap()["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn require_no_partial(root: &Path) {
    assert!(fs::read_dir(root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".p4-partial-")
    }));
}

fn write_json(path: &Path, value: &impl Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    fs::write(path, &bytes).unwrap();
    bytes
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
