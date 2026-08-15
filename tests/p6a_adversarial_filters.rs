#![cfg(windows)]

use base64::{Engine as _, engine::general_purpose::STANDARD};
use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::data::{
    ADAPTER_NAMESPACE, AUTHORIZATION_SCHEME, CURATE_CONFIG_SCHEMA, CurateConfigV1, HashBoundPath,
    IngestionLimits, MaterializedSourceManifestV1, Provenance, REMOVAL_MANIFEST_SCHEMA,
    RemovalManifestV1, SOURCE_MANIFEST_SCHEMA, SourceAuthorization, SourceDocument,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

const CORPUS_PATH: &str = "tests/fixtures/p6a/adversarial-filter-cases-v1.json";
const CORPUS_SHA256: &str = "a7eefb3e4f4abca90ea4b686e68175de00c63a102fc467bdff66017a26e8517c";
const REMOVAL_AUTHORITY: &str = "https://example.invalid/p6a/removals";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdversarialCorpus {
    schema: String,
    document_cases: Vec<DocumentCase>,
    path_cases: Vec<PathCase>,
    mutation_cases: Vec<MutationCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentCase {
    id: String,
    source_base64: String,
    license_expression: String,
    expected_status: String,
    expected_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathCase {
    id: String,
    relative_path: String,
    expected_error: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationCase {
    id: String,
    operation: String,
    expected: String,
}

#[test]
fn adversarial_corpus_is_closed_frozen_and_ordered() {
    let (corpus, bytes) = load_corpus();
    assert_eq!(corpus.schema, "python-slm-p6a-adversarial-filter-cases-v1");
    assert_eq!(sha256(&bytes), CORPUS_SHA256);
    require_sorted_unique(corpus.document_cases.iter().map(|case| case.id.as_str()));
    require_sorted_unique(corpus.path_cases.iter().map(|case| case.id.as_str()));
    require_sorted_unique(corpus.mutation_cases.iter().map(|case| case.id.as_str()));
    assert_eq!(corpus.document_cases.len(), 25);
    assert_eq!(corpus.path_cases.len(), 8);
    assert_eq!(corpus.mutation_cases.len(), 3);
    for case in &corpus.document_cases {
        assert!(matches!(
            case.expected_status.as_str(),
            "POLICY_ACCEPTED" | "QUARANTINED" | "REJECTED"
        ));
        assert_eq!(
            case.expected_status == "POLICY_ACCEPTED",
            case.expected_reason.is_none()
        );
        STANDARD.decode(&case.source_base64).unwrap();
    }
}

#[test]
fn adversarial_documents_have_deterministic_fail_closed_outcomes() {
    let (corpus, _) = load_corpus();
    let root = tempfile::tempdir().unwrap();
    let content_root = root.path().join("content");
    fs::create_dir(&content_root).unwrap();

    let mut documents = Vec::new();
    for case in &corpus.document_cases {
        let bytes = padded(STANDARD.decode(&case.source_base64).unwrap());
        let relative = format!("{}.py", case.id);
        fs::write(content_root.join(&relative), &bytes).unwrap();
        documents.push(source_document(
            &case.id,
            &relative,
            &bytes,
            &case.license_expression,
        ));
    }
    let source_path = root.path().join("source.json");
    write_json(&source_path, &source_manifest(documents));
    let (removal_path, removal_sha256) = write_removal(root.path());

    let first_output = root.path().join("generation-first");
    let first_config = root.path().join("config-first.json");
    write_config(
        &first_config,
        &source_path,
        &content_root,
        &removal_path,
        &removal_sha256,
        &first_output,
        corpus.document_cases.len() as u64,
    );
    let first = run_curate(&first_config);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(first.stderr.is_empty());

    let result: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(result["schema"], "python-slm-curate-result-v4");
    assert_eq!(result["qualification_status"], "SKIPPED");
    assert_eq!(result["document_count"], corpus.document_cases.len());
    assert_eq!(
        result["policy_accepted_count"],
        corpus
            .document_cases
            .iter()
            .filter(|case| case.expected_status == "POLICY_ACCEPTED")
            .count()
    );
    assert_eq!(
        result["quarantined_count"],
        corpus
            .document_cases
            .iter()
            .filter(|case| case.expected_status == "QUARANTINED")
            .count()
    );
    assert_eq!(
        result["rejected_count"],
        corpus
            .document_cases
            .iter()
            .filter(|case| case.expected_status == "REJECTED")
            .count()
    );
    assert_eq!(result["receipts_written"], false);

    let manifest_bytes = fs::read(first_output.join("manifest.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let outcomes = manifest["outcomes"].as_array().unwrap();
    for case in &corpus.document_cases {
        let outcome = outcomes
            .iter()
            .find(|outcome| outcome["provider_record_id"] == case.id)
            .unwrap_or_else(|| panic!("missing outcome {}", case.id));
        assert_eq!(
            outcome["status"].as_str(),
            Some(case.expected_status.as_str()),
            "{}",
            case.id
        );
        let reasons = outcome["reasons"].as_array().unwrap();
        if let Some(expected) = &case.expected_reason {
            assert!(
                reasons.iter().any(|reason| reason == expected),
                "{} missing {expected}",
                case.id
            );
        } else {
            assert!(reasons.is_empty(), "unexpected reasons for {}", case.id);
        }
        assert_eq!(
            outcome["content_path"].is_string(),
            case.expected_status == "POLICY_ACCEPTED"
        );
    }

    let mut public = manifest_bytes;
    public.extend_from_slice(&first.stdout);
    append_json_tree(&first_output.join("parser"), &mut public);
    append_json_tree(&first_output.join("policy"), &mut public);
    let public = String::from_utf8(public).unwrap();
    for restricted in [
        "aB3!dE5@gH7#jK9$mN2%qR4&",
        "Alice@Corp.Test",
        "gb82west12345698765432",
    ] {
        assert!(!public.contains(restricted));
    }
    assert!(!public.contains(&root.path().display().to_string()));
    assert!(!first_output.join("receipts").exists());

    let second_output = root.path().join("generation-second");
    let second_config = root.path().join("config-second.json");
    write_config(
        &second_config,
        &source_path,
        &content_root,
        &removal_path,
        &removal_sha256,
        &second_output,
        corpus.document_cases.len() as u64,
    );
    let second = run_curate(&second_config);
    assert!(second.status.success());
    assert_eq!(
        generation_inventory(&first_output),
        generation_inventory(&second_output)
    );
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn adversarial_paths_fail_before_output_creation_without_echoing_values() {
    let (corpus, _) = load_corpus();
    for case in &corpus.path_cases {
        let root = tempfile::tempdir().unwrap();
        let content_root = root.path().join("content");
        fs::create_dir(&content_root).unwrap();
        let bytes = padded(b"value = 1\n".to_vec());
        let source_path = root.path().join("source.json");
        write_json(
            &source_path,
            &source_manifest(vec![source_document(
                &case.id,
                &case.relative_path,
                &bytes,
                "MIT",
            )]),
        );
        let (removal_path, removal_sha256) = write_removal(root.path());
        let output = root.path().join("generation");
        let config = root.path().join("config.json");
        write_config(
            &config,
            &source_path,
            &content_root,
            &removal_path,
            &removal_sha256,
            &output,
            1,
        );
        let result = run_curate(&config);
        assert_eq!(result.status.code(), Some(3), "{}", case.id);
        let error = error_json(&result);
        assert_eq!(error["code"], case.expected_error, "{}", case.id);
        let stderr = String::from_utf8(result.stderr).unwrap();
        if !case.relative_path.is_empty() {
            assert!(!stderr.contains(&case.relative_path));
        }
        assert!(!output.exists());
        require_no_partial(root.path());
    }
}

#[test]
fn bound_document_denies_every_adversarial_mutation_operation() {
    let (corpus, _) = load_corpus();
    let root = tempfile::tempdir().unwrap();
    let content_root = root.path().join("content");
    fs::create_dir(&content_root).unwrap();
    let bytes = padded(b"value = 1\n".to_vec());
    let document_path = content_root.join("document.py");
    fs::write(&document_path, &bytes).unwrap();

    let source_path = root.path().join("source.json");
    write_json(
        &source_path,
        &source_manifest(vec![source_document(
            "mutation-target",
            "document.py",
            &bytes,
            "MIT",
        )]),
    );
    let (removal_path, removal_sha256) = write_removal(root.path());
    let output = root.path().join("generation");
    let config = root.path().join("config.json");
    write_config(
        &config,
        &source_path,
        &content_root,
        &removal_path,
        &removal_sha256,
        &output,
        1,
    );

    let mut options = OpenOptions::new();
    options.read(true).share_mode(FILE_SHARE_READ);
    let guard = options.open(&document_path).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_python-slm"))
        .args(["curate", "--config"])
        .arg(&config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    for case in &corpus.mutation_cases {
        assert_eq!(case.expected, "DENIED");
        let denied = match case.operation.as_str() {
            "write" => OpenOptions::new().write(true).open(&document_path).is_err(),
            "delete" => fs::remove_file(&document_path).is_err(),
            "rename" => fs::rename(&document_path, content_root.join("moved.py")).is_err(),
            other => panic!("unknown mutation operation {other}"),
        };
        assert!(denied, "{} was not denied", case.id);
    }
    let child_output = child.wait_with_output().unwrap();
    drop(guard);
    assert!(
        child_output.status.success(),
        "{}",
        String::from_utf8_lossy(&child_output.stderr)
    );
    assert!(child_output.stderr.is_empty());
    serde_json::from_slice::<Value>(&child_output.stdout).unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["outcomes"][0]["status"], "POLICY_ACCEPTED");
    let relative = manifest["outcomes"][0]["content_path"].as_str().unwrap();
    assert_eq!(fs::read(output.join(relative)).unwrap(), bytes);
    assert!(!output.join("receipts").exists());
}

fn load_corpus() -> (AdversarialCorpus, Vec<u8>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CORPUS_PATH);
    let bytes = fs::read(path).unwrap();
    let corpus = serde_json::from_slice(&bytes).unwrap();
    let canonical_bytes = String::from_utf8(bytes)
        .unwrap()
        .replace("\r\n", "\n")
        .into_bytes();
    (corpus, canonical_bytes)
}

fn source_manifest(documents: Vec<SourceDocument>) -> MaterializedSourceManifestV1 {
    MaterializedSourceManifestV1 {
        schema: SOURCE_MANIFEST_SCHEMA.to_owned(),
        adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
        source_snapshot_id: "snapshot-p6a-adversarial-v1".to_owned(),
        authorization: SourceAuthorization {
            scheme: AUTHORIZATION_SCHEME.to_owned(),
            authority_url: "https://example.invalid/authorized-source".to_owned(),
            authorization_id: "owner-authorized-p6a-fixtures".to_owned(),
        },
        required_removal_authorities: vec![REMOVAL_AUTHORITY.to_owned()],
        documents,
    }
}

fn source_document(
    id: &str,
    relative_path: &str,
    bytes: &[u8],
    license_expression: &str,
) -> SourceDocument {
    SourceDocument {
        provider_record_id: id.to_owned(),
        provider_repository_id: Some("repository-p6a-adversarial-v1".to_owned()),
        stable_provenance_origin_namespace: String::new(),
        relative_path: relative_path.to_owned(),
        expected_raw_sha256: sha256(bytes),
        expected_raw_bytes: bytes.len() as u64,
        dialect: "python3".to_owned(),
        license_expression: license_expression.to_owned(),
        provenance: Provenance {
            origin_url: "https://example.invalid/authorized-source".to_owned(),
            revision: format!("revision-{id}"),
            source_path: "src/fixture.py".to_owned(),
        },
    }
}

fn write_removal(root: &Path) -> (PathBuf, String) {
    let path = root.join("removal.json");
    let bytes = write_json(
        &path,
        &RemovalManifestV1 {
            schema: REMOVAL_MANIFEST_SCHEMA.to_owned(),
            adapter_namespace: ADAPTER_NAMESPACE.to_owned(),
            authority_url: REMOVAL_AUTHORITY.to_owned(),
            provider_snapshot_id: "removals-p6a-v1".to_owned(),
            provider_order: 1,
            publication_time_utc: "2026-08-15T00:00:00Z".to_owned(),
            retrieval_time_utc: "2026-08-15T00:01:00Z".to_owned(),
            removed_provider_record_ids: vec![],
            removed_provider_repository_ids: vec![],
        },
    );
    (path, sha256(&bytes))
}

fn write_config(
    path: &Path,
    source_manifest: &Path,
    content_root: &Path,
    removal_manifest: &Path,
    removal_sha256: &str,
    output_root: &Path,
    maximum_documents: u64,
) {
    write_json(
        path,
        &CurateConfigV1 {
            schema: CURATE_CONFIG_SCHEMA.to_owned(),
            profile: PROTOTYPE_PROFILE.to_owned(),
            source_manifest: source_manifest.to_owned(),
            content_root: content_root.to_owned(),
            removal_manifests: vec![HashBoundPath {
                path: removal_manifest.to_owned(),
                sha256: removal_sha256.to_owned(),
            }],
            output_root: output_root.to_owned(),
            limits: IngestionLimits {
                maximum_documents,
                maximum_total_raw_bytes: 2_000_000,
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

fn error_json(output: &Output) -> Value {
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert_eq!(stderr.lines().count(), 1);
    serde_json::from_str(stderr.trim()).unwrap()
}

fn padded(mut bytes: Vec<u8>) -> Vec<u8> {
    while bytes.len() < 120 {
        bytes.extend_from_slice(b"\nvalue = 1\n");
    }
    bytes
}

fn append_json_tree(root: &Path, output: &mut Vec<u8>) {
    let mut paths = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        output.extend_from_slice(&fs::read(path).unwrap());
    }
}

fn generation_inventory(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, current: &Path, output: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(base, &path, output);
            } else {
                let relative = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                output.insert(relative, fs::read(path).unwrap());
            }
        }
    }
    let mut output = BTreeMap::new();
    walk(root, root, &mut output);
    output
}

fn require_sorted_unique<'a>(values: impl Iterator<Item = &'a str>) {
    let values = values.collect::<Vec<_>>();
    assert!(!values.is_empty());
    assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
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
