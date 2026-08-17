use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::corpus::{
    BENCHMARK_MANIFEST_SCHEMA, BenchmarkAssetV1, BenchmarkContentKind,
    BenchmarkProtectionManifestV1, BenchmarkRecordV1, CorpusPolicyConfigV1,
    CorpusPolicyGenerationV1, CorpusPolicyLimits, DECONTAMINATION_MANIFEST_SCHEMA,
    DecontaminationManifestV1, GENERATION_SCHEMA, GOVERNED_CORPUS_SCHEMA, GovernedCorpusManifestV2,
    PREPARE_CONFIG_SCHEMA, SourceGenerationInput, prepare,
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

/// Write one P4 generation and return its manifest path and bytes.
fn generation(root: &Path, documents: &[(&str, &str, &[u8])]) -> (std::path::PathBuf, Vec<u8>) {
    let content = root.join("documents");
    fs::create_dir_all(&content).unwrap();
    let mut outcomes = Vec::new();
    for (label, repository, bytes) in documents {
        fs::write(content.join(format!("{label}.py")), bytes).unwrap();
        outcomes.push(outcome(label, repository, bytes));
    }
    let manifest = json!({
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
        "outcomes": outcomes
    });
    let path = root.join("manifest.json");
    let bytes = write_json(&path, &manifest);
    (path, bytes)
}

/// A structurally valid benchmark manifest that protects one unrelated module.
fn benchmark_inputs(root: &Path) -> (std::path::PathBuf, Vec<u8>) {
    fs::create_dir_all(root).unwrap();
    let protected = b"def unrelated_benchmark_case(value):\n    return value - 7\n";
    fs::write(root.join("bench.py"), protected).unwrap();
    let manifest = BenchmarkProtectionManifestV1 {
        schema: BENCHMARK_MANIFEST_SCHEMA.to_owned(),
        registry_id: "evalplus-v0.3.1".to_owned(),
        registry_commit: "e5d0ed0bab96280b60b637ec7f15b5e4841b0cb2".to_owned(),
        assets: vec![
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
            relative_path: "bench.py".to_owned(),
            sha256: hash(protected),
            bytes: protected.len() as u64,
        }],
    };
    let path = root.join("manifest.json");
    let bytes = write_json(&path, &manifest);
    (path, bytes)
}

/// Several P4 generations compose into one corpus.
///
/// A single generation manifest is capped near 28,000 documents by the 64 MiB
/// control-file bound, so production scale needs many. The property that makes
/// that safe is that everything downstream sees the union: a duplicate spanning
/// two generations has to collapse exactly as it would inside one, or composing
/// would silently admit the duplicates that deduplication exists to remove.
#[test]
fn generations_compose_into_one_corpus_and_deduplicate_across_them() {
    let temporary = tempfile::tempdir().unwrap();
    let shared = b"def shared_helper(value):\n    return value * 11\n";
    let only_first = b"def first_only(value):\n    return value + 101\n";
    let only_second = b"def second_only(value):\n    return value - 202\n";

    // The duplicate straddles the two generations.
    let first_root = temporary.path().join("gen-a");
    let (first_manifest, first_bytes) = generation(
        &first_root,
        &[("a1", "repo-a", shared), ("a2", "repo-a", only_first)],
    );
    let second_root = temporary.path().join("gen-b");
    let (second_manifest, second_bytes) = generation(
        &second_root,
        &[("b1", "repo-b", shared), ("b2", "repo-b", only_second)],
    );
    let benchmark_root = temporary.path().join("benchmark");
    let (benchmark_manifest, benchmark_bytes) = benchmark_inputs(&benchmark_root);

    let output_root = temporary.path().join("corpus-policy");
    let config = CorpusPolicyConfigV1 {
        schema: PREPARE_CONFIG_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        source_generations: vec![
            SourceGenerationInput {
                manifest: HashBoundInput {
                    path: first_manifest,
                    sha256: hash(&first_bytes),
                },
                content_root: first_root,
            },
            SourceGenerationInput {
                manifest: HashBoundInput {
                    path: second_manifest,
                    sha256: hash(&second_bytes),
                },
                content_root: second_root,
            },
        ],
        benchmark_manifest: HashBoundInput {
            path: benchmark_manifest,
            sha256: hash(&benchmark_bytes),
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
    let config_path = temporary.path().join("multi-config.json");
    write_json(&config_path, &config);

    let result = prepare(&config_path).unwrap();
    assert_eq!(result["status"], "CORPUS_POLICY_MATERIALIZED");
    // Four accepted documents across two generations; the shared pair collapses
    // to one representative, leaving three.
    assert_eq!(result["representative_documents"], 3);

    let dedup: serde_json::Value =
        serde_json::from_slice(&fs::read(output_root.join("dedup-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(dedup["input_documents"], 4);
    assert_eq!(
        dedup["exact_duplicate_edges"], 1,
        "the cross-generation duplicate must be found"
    );
    // Clusters include singletons, so four documents with one duplicate pair give
    // three: the pair plus two singletons. The pair is the one that matters, and
    // its members must be the two documents that came from different generations.
    let clusters = dedup["clusters"].as_array().unwrap();
    assert_eq!(clusters.len(), 3);
    let multi: Vec<_> = clusters
        .iter()
        .filter(|cluster| cluster["member_source_ids"].as_array().unwrap().len() > 1)
        .collect();
    assert_eq!(
        multi.len(),
        1,
        "exactly one cluster should have two members"
    );
    let members = multi[0]["member_source_ids"].as_array().unwrap();
    assert_eq!(members.len(), 2);
    let member_ids: Vec<&str> = members.iter().map(|id| id.as_str().unwrap()).collect();
    assert!(member_ids.contains(&hash(b"a1").as_str()));
    assert!(member_ids.contains(&hash(b"b1").as_str()));

    // The composed identity covers both generations, and is not either digest.
    let generation_manifest: CorpusPolicyGenerationV1 =
        serde_json::from_slice(&fs::read(output_root.join("manifest.json")).unwrap()).unwrap();
    assert_ne!(
        generation_manifest.source_generation_manifest_sha256,
        hash(&first_bytes)
    );
    assert_ne!(
        generation_manifest.source_generation_manifest_sha256,
        hash(&second_bytes)
    );
}

/// Identity must be unique across the composed set, not merely within one
/// generation, or the same document could enter the corpus twice.
#[test]
fn a_source_identity_repeated_across_generations_fails_closed() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def repeated(value):\n    return value + 3\n";
    let first_root = temporary.path().join("gen-a");
    let (first_manifest, first_bytes) = generation(&first_root, &[("same", "repo-a", content)]);
    let second_root = temporary.path().join("gen-b");
    // The same label yields the same derived source_id in both generations.
    let (second_manifest, second_bytes) = generation(&second_root, &[("same", "repo-b", content)]);
    let benchmark_root = temporary.path().join("benchmark");
    let (benchmark_manifest, benchmark_bytes) = benchmark_inputs(&benchmark_root);

    let config = CorpusPolicyConfigV1 {
        schema: PREPARE_CONFIG_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        source_generations: vec![
            SourceGenerationInput {
                manifest: HashBoundInput {
                    path: first_manifest,
                    sha256: hash(&first_bytes),
                },
                content_root: first_root,
            },
            SourceGenerationInput {
                manifest: HashBoundInput {
                    path: second_manifest,
                    sha256: hash(&second_bytes),
                },
                content_root: second_root,
            },
        ],
        benchmark_manifest: HashBoundInput {
            path: benchmark_manifest,
            sha256: hash(&benchmark_bytes),
        },
        benchmark_content_root: benchmark_root,
        output_root: temporary.path().join("corpus-policy"),
        limits: CorpusPolicyLimits {
            maximum_documents: 10,
            maximum_total_canonical_bytes: 10_000,
            maximum_total_shingles: 10_000,
            maximum_candidate_pairs: 100,
            maximum_benchmark_records: 10,
        },
    };
    let config_path = temporary.path().join("duplicate-identity.json");
    write_json(&config_path, &config);
    assert_eq!(
        prepare(&config_path).unwrap_err().code,
        "SOURCE_ID_DUPLICATE"
    );

    // Naming one generation twice is refused before anything is read.
    let mut repeated = config.clone();
    repeated.source_generations[1] = repeated.source_generations[0].clone();
    let repeated_path = temporary.path().join("repeated-generation.json");
    write_json(&repeated_path, &repeated);
    assert_eq!(
        prepare(&repeated_path).unwrap_err().code,
        "CONFIG_SOURCE_GENERATION_DUPLICATE"
    );

    // And an empty set is a usage error rather than an empty corpus.
    let mut empty = config;
    empty.source_generations.clear();
    let empty_path = temporary.path().join("empty-generations.json");
    write_json(&empty_path, &empty);
    assert_eq!(
        prepare(&empty_path).unwrap_err().code,
        "CONFIG_INPUT_INVALID"
    );
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
        source_generations: vec![SourceGenerationInput {
            manifest: HashBoundInput {
                path: source_manifest_path,
                sha256: hash(&source_manifest_bytes),
            },
            content_root: source_root,
        }],
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
