use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::corpus::{
    BENCHMARK_MANIFEST_SCHEMA, BenchmarkAssetV1, BenchmarkContentKind,
    BenchmarkProtectionManifestV1, BenchmarkRecordV1, CorpusPolicyConfigV1,
    CorpusPolicyGenerationV1, CorpusPolicyLimits, DECONTAMINATION_MANIFEST_SCHEMA,
    DecontaminationManifestV1, GENERATION_SCHEMA, GOVERNED_CORPUS_SCHEMA, GovernedCorpusManifestV2,
    PREPARE_CONFIG_SCHEMA, prepare,
};
use rust_llm_pretrain::tokenizer::HashBoundInput;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    fs::write(path, &bytes).unwrap();
    bytes
}

fn outcome(label: &str, repository: &str, content: &[u8]) -> serde_json::Value {
    let digest = hash(content);
    json!({
        "source_id": hash(label.as_bytes()),
        "repository_group_id": hash(repository.as_bytes()),
        "provider_record_id": label,
        "status": "POLICY_ACCEPTED",
        "reasons": [],
        "raw_sha256": digest,
        "raw_bytes": content.len(),
        "canonical_decoded_sha256": digest,
        "canonical_decoded_bytes": content.len(),
        "bom_removed": false,
        "license_expression": "MIT",
        "provenance": {
            "origin_url": format!("https://example.invalid/{label}"),
            "revision": "r1",
            "source_path": format!("{label}.py")
        },
        "parser_result_path": null,
        "parser_result_sha256": null,
        "policy_result_path": null,
        "policy_result_sha256": null,
        "content_path": format!("documents/{label}.py"),
        "governed_source_metadata": {}
    })
}

#[test]
fn prepare_corpus_excludes_benchmark_duplicate_cluster_and_publishes_once() {
    let temporary = tempfile::tempdir().unwrap();
    let source_root = temporary.path().join("source");
    let source_documents = source_root.join("documents");
    let benchmark_root = temporary.path().join("benchmark");
    fs::create_dir_all(&source_documents).unwrap();
    fs::create_dir(&benchmark_root).unwrap();

    let duplicate = b"def benchmark_copy(value):\n    return value + 41\n";
    let retained = b"def retained(value):\n    return value * 3\n";
    fs::write(source_documents.join("a.py"), duplicate).unwrap();
    fs::write(source_documents.join("b.py"), duplicate).unwrap();
    fs::write(source_documents.join("c.py"), retained).unwrap();
    let source_manifest = json!({
        "schema": "python-slm-source-generation-v4",
        "profile": PROTOTYPE_PROFILE,
        "adapter_namespace": "stack-v2-swh-materialized-v1",
        "source_snapshot_id": "fixture",
        "source_manifest_sha256": hash(b"source"),
        "authorization": {},
        "license_policy": "permissive-v1",
        "generated_marker_policy": "generated-v1",
        "parser_status": "COMPLETE",
        "parser_bundle": {},
        "policy_status": "COMPLETE",
        "sensitive_policy": {},
        "governed_source_policy": {},
        "governance_status": "UNVERIFIED",
        "removal_snapshots": [],
        "outcomes": [
            outcome("a", "repo-a", duplicate),
            outcome("b", "repo-b", duplicate),
            outcome("c", "repo-c", retained)
        ]
    });
    let source_manifest_path = source_root.join("manifest.json");
    let source_manifest_bytes = write_json(&source_manifest_path, &source_manifest);

    let benchmark_path = benchmark_root.join("humaneval.py");
    fs::write(&benchmark_path, duplicate).unwrap();
    let benchmark = BenchmarkProtectionManifestV1 {
        schema: BENCHMARK_MANIFEST_SCHEMA.to_owned(),
        registry_id: "evalplus-v0.3.1".to_owned(),
        registry_commit: "e5d0ed0bab96280b60b637ec7f15b5e4841b0cb2".to_owned(),
        assets: vec![
            // The frozen DECONTAM-001 identities. These are compared against
            // compiled-in constants now, not merely checked for hash shape, so a
            // synthetic corpus fixture still has to name the real assets.
            BenchmarkAssetV1 {
                dataset: "humanevalplus".to_owned(),
                release_asset: "HumanEvalPlus.jsonl.gz".to_owned(),
                release_version: "v0.1.10".to_owned(),
                asset_sha256: "272720b90ac375502c8ed23cd791c2a93dfb22a911641a494da74a426c09f101"
                    .to_owned(),
                decoded_sha256: "42526ec0e7d5f3ee0b06d6ced98f8c8bae3d76519151bfb3d36f79010645bd7f"
                    .to_owned(),
            },
            BenchmarkAssetV1 {
                dataset: "mbppplus".to_owned(),
                release_asset: "MbppPlus.jsonl.gz".to_owned(),
                release_version: "v0.2.0".to_owned(),
                asset_sha256: "af43697e8791c4c149bdfd6b489d8b5412507551ac20e28a439f650b8225db63"
                    .to_owned(),
                decoded_sha256: "b54e762755248ca411b523c917fa9f93c07b5ff2966bf60b3917b853926a3dad"
                    .to_owned(),
            },
        ],
        records: vec![BenchmarkRecordV1 {
            dataset: "humanevalplus".to_owned(),
            task_id: "HumanEval/0".to_owned(),
            json_pointer: "/prompt".to_owned(),
            role: "prompt".to_owned(),
            content_kind: BenchmarkContentKind::PythonModule,
            relative_path: "humaneval.py".to_owned(),
            sha256: hash(duplicate),
            bytes: duplicate.len() as u64,
        }],
    };
    let benchmark_manifest_path = benchmark_root.join("manifest.json");
    let benchmark_manifest_bytes = write_json(&benchmark_manifest_path, &benchmark);

    let output_root = temporary.path().join("corpus-policy");
    let config = CorpusPolicyConfigV1 {
        schema: PREPARE_CONFIG_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        source_generation_manifest: HashBoundInput {
            path: source_manifest_path,
            sha256: hash(&source_manifest_bytes),
        },
        source_content_root: source_root,
        benchmark_manifest: HashBoundInput {
            path: benchmark_manifest_path,
            sha256: hash(&benchmark_manifest_bytes),
        },
        benchmark_content_root: benchmark_root,
        output_root: output_root.clone(),
        limits: CorpusPolicyLimits {
            maximum_documents: 10,
            maximum_total_canonical_bytes: 10_000,
            maximum_total_shingles: 10_000,
            maximum_candidate_pairs: 100,
            maximum_benchmark_records: 10,
        },
    };
    let config_path = temporary.path().join("config.json");
    write_json(&config_path, &config);

    let result = prepare(&config_path).unwrap();
    assert_eq!(result["status"], "CORPUS_POLICY_MATERIALIZED");
    assert_eq!(result["qualification_status"], "SKIPPED");
    assert_eq!(result["representative_documents"], 1);
    assert_eq!(result["excluded_documents"], 2);

    let generation: CorpusPolicyGenerationV1 =
        serde_json::from_slice(&fs::read(output_root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(generation.schema, GENERATION_SCHEMA);
    assert_eq!(generation.representative_documents, 1);
    let decontamination: DecontaminationManifestV1 = serde_json::from_slice(
        &fs::read(output_root.join("decontamination-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(decontamination.schema, DECONTAMINATION_MANIFEST_SCHEMA);
    assert_eq!(decontamination.rejected_clusters, 1);
    assert_eq!(decontamination.rejected_documents.len(), 2);
    let governed: GovernedCorpusManifestV2 = serde_json::from_slice(
        &fs::read(output_root.join("governed-corpus-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(governed.schema, GOVERNED_CORPUS_SCHEMA);
    assert_eq!(governed.documents.len(), 1);
    assert_eq!(governed.documents[0].source_id, hash(b"c"));
    assert!(
        output_root
            .join("documents")
            .join(format!("{}.py", hash(b"c")))
            .is_file()
    );

    assert_eq!(
        prepare(&config_path).unwrap_err().code,
        "OUTPUT_ALREADY_EXISTS"
    );
    assert!(!temporary.path().join("docs/receipts").exists());

    // A benchmark manifest carrying a plausible but wrong digest used to pass
    // every gate here, because the asset hashes were only checked for shape and
    // never compared to a known value — so a manifest protecting nothing could
    // certify decontamination. They are bound to the frozen DECONTAM-001
    // identities now, and this is the assertion that says so.
    let mut forged = benchmark.clone();
    forged.assets[0].asset_sha256 = hash(b"a plausible but fabricated digest");
    let forged_manifest_path = temporary.path().join("forged-benchmark.json");
    let forged_bytes = write_json(&forged_manifest_path, &forged);
    let mut forged_config = config.clone();
    forged_config.output_root = temporary.path().join("corpus-policy-forged");
    forged_config.benchmark_manifest = HashBoundInput {
        path: forged_manifest_path,
        sha256: hash(&forged_bytes),
    };
    let forged_config_path = temporary.path().join("forged-config.json");
    write_json(&forged_config_path, &forged_config);
    assert_eq!(
        prepare(&forged_config_path).unwrap_err().code,
        "BENCHMARK_ASSET_DIGEST_MISMATCH"
    );
    assert!(!forged_config.output_root.exists());
}
