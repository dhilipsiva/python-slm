#![cfg(windows)]

use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::storage::{
    CONFIG_SCHEMA, CORPUS_MANIFEST_SCHEMA, CorpusSplit, GovernedCorpusDocumentV1,
    GovernedCorpusManifestV1, MaterializationLimits, TokenMaterializeConfigV1, VerifiedTokenCorpus,
    tokenize,
};
use rust_llm_pretrain::tokenizer::{
    ARTIFACT_SCHEMA, ByteAlphabet, ByteBpeTokenizer, HashBoundInput, MergeRule, SampleBinding,
    SpecialTokenIds, TokenizerArtifactV1, TokenizerSampleDocumentV1, TokenizerSampleManifestV1,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn json_line<T: serde::Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    bytes
}

fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
}

struct Fixture {
    root: tempfile::TempDir,
    config: std::path::PathBuf,
    output: std::path::PathBuf,
    source_bytes: std::collections::BTreeMap<String, Vec<u8>>,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let content = root.path().join("content");
    fs::create_dir(&content).unwrap();

    let source_generation = hash(b"source-generation");
    let documents = [
        (
            "train-z.py",
            CorpusSplit::Train,
            "component-z",
            b"print('train z')\n".repeat(9),
        ),
        (
            "train-a.py",
            CorpusSplit::Train,
            "component-a",
            b"print('train a')\n".repeat(8),
        ),
        (
            "validation.py",
            CorpusSplit::Validation,
            "component-validation",
            b"print('validation')\n".repeat(7),
        ),
        (
            "test.py",
            CorpusSplit::Test,
            "component-test",
            b"print('test')\n".repeat(7),
        ),
    ];
    let mut source_bytes = std::collections::BTreeMap::new();
    let mut corpus_documents = Vec::new();
    let mut sample_documents = Vec::new();
    for (name, split, component, bytes) in documents {
        write(&content.join(name), &bytes);
        let source_id = hash(format!("source-{name}").as_bytes());
        let canonical_sha256 = hash(&bytes);
        source_bytes.insert(source_id.clone(), bytes.clone());
        corpus_documents.push(GovernedCorpusDocumentV1 {
            component_id: hash(component.as_bytes()),
            repository_group_id: hash(format!("repository-{name}").as_bytes()),
            source_id: source_id.clone(),
            curated_sha256_raw: canonical_sha256.clone(),
            canonical_sha256: canonical_sha256.clone(),
            canonical_bytes: bytes.len() as u64,
            relative_path: name.to_owned(),
            split,
        });
        sample_documents.push(TokenizerSampleDocumentV1 {
            repository_group_id: hash(format!("repository-{name}").as_bytes()),
            source_id,
            curated_sha256_raw: canonical_sha256.clone(),
            canonical_sha256,
            canonical_bytes: bytes.len() as u64,
            relative_path: name.to_owned(),
        });
    }

    let sample = TokenizerSampleManifestV1 {
        schema: "python-slm-tokenizer-sample-manifest-v1".to_owned(),
        source_generation_manifest_sha256: source_generation.clone(),
        documents: sample_documents,
    };
    let sample_bytes = json_line(&sample);
    let sample_path = root.path().join("sample.json");
    write(&sample_path, &sample_bytes);
    let sample_sha = hash(&sample_bytes);

    let merges = (0..31_740_u32)
        .map(|offset| {
            let id = 260 + offset;
            MergeRule {
                id,
                left: if offset == 0 { 4 } else { id - 1 },
                right: 5 + (offset % 255),
                training_frequency: 2,
            }
        })
        .collect();
    let selected_bytes = source_bytes.values().map(|bytes| bytes.len() as u64).sum();
    let artifact = TokenizerArtifactV1 {
        schema: ARTIFACT_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        algorithm: "deterministic-byte-bpe-v1".to_owned(),
        vocabulary_size: 32_000,
        maximum_token_id: 31_999,
        special_tokens: SpecialTokenIds {
            pad: 0,
            bos: 1,
            eos: 2,
            unk: 3,
        },
        byte_alphabet: ByteAlphabet {
            first_id: 4,
            count: 256,
        },
        normalization: "none-byte-preserving-v1".to_owned(),
        minimum_merge_frequency: 2,
        tie_breaker: "highest-frequency-then-lowest-token-id-pair-v1".to_owned(),
        sample: SampleBinding {
            manifest_sha256: sample_sha.clone(),
            source_generation_manifest_sha256: source_generation.clone(),
            selected_document_count: 4,
            selected_bytes,
            skipped_document_count: 0,
            skipped_bytes: 0,
            repository_byte_cap: 10_000_000,
            global_byte_cap: 2_000_000_000,
            qualified_minimum_bytes: 1_999_000_000,
            qualified_range_satisfied: false,
        },
        merges,
    };
    let artifact_bytes = json_line(&artifact);
    ByteBpeTokenizer::from_artifact_bytes(&artifact_bytes).unwrap();
    let artifact_path = root.path().join("tokenizer.json");
    write(&artifact_path, &artifact_bytes);
    let artifact_sha = hash(&artifact_bytes);

    let corpus = GovernedCorpusManifestV1 {
        schema: CORPUS_MANIFEST_SCHEMA.to_owned(),
        source_generation_manifest_sha256: source_generation,
        split_manifest_sha256: hash(b"synthetic-split"),
        tokenizer_sample_manifest_sha256: sample_sha.clone(),
        tokenizer_artifact_sha256: artifact_sha.clone(),
        documents: corpus_documents,
    };
    let corpus_bytes = json_line(&corpus);
    let corpus_path = root.path().join("corpus.json");
    write(&corpus_path, &corpus_bytes);

    let output = root.path().join("tokens");
    let config = TokenMaterializeConfigV1 {
        schema: CONFIG_SCHEMA.to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        corpus_manifest: HashBoundInput {
            path: corpus_path,
            sha256: hash(&corpus_bytes),
        },
        content_root: content,
        tokenizer_sample_manifest: HashBoundInput {
            path: sample_path,
            sha256: sample_sha,
        },
        tokenizer_artifact: HashBoundInput {
            path: artifact_path,
            sha256: artifact_sha,
        },
        output_root: output.clone(),
        limits: MaterializationLimits {
            maximum_documents: 4,
            maximum_total_canonical_bytes: 10_000,
            maximum_total_stored_ids: 10_000,
            shard_maximum_ids: 31,
        },
    };
    let config_path = root.path().join("config.json");
    write(&config_path, &json_line(&config));
    Fixture {
        root,
        config: config_path,
        output,
        source_bytes,
    }
}

#[test]
fn materializes_hash_bound_shards_indexes_and_round_trips_documents() {
    let fixture = fixture();
    let result = tokenize(&fixture.config).unwrap();
    assert_eq!(result["schema"], "python-slm-tokenize-result-v1");
    assert_eq!(result["status"], "TOKEN_CORPUS_MATERIALIZED");
    assert_eq!(result["qualification_status"], "SKIPPED");
    assert_eq!(result["training_target_satisfied"], false);
    assert_eq!(result["receipts_written"], false);
    let serialized_result = serde_json::to_string(&result).unwrap();
    assert!(!serialized_result.contains(&fixture.root.path().display().to_string()));
    let mut top_level = fs::read_dir(&fixture.output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    top_level.sort();
    assert_eq!(top_level, ["indexes", "inputs", "manifest.json", "shards"]);

    let corpus = VerifiedTokenCorpus::open(&fixture.output).unwrap();
    assert!(corpus.manifest.shards.len() > 4);
    assert_eq!(corpus.manifest.accounting.stored_pad_ids, 0);
    assert_eq!(corpus.manifest.accounting.stored_bos_ids, 0);
    assert_eq!(corpus.manifest.accounting.stored_unk_ids, 0);

    let tokenizer_bytes = fs::read(fixture.output.join("inputs/tokenizer.json")).unwrap();
    let tokenizer = ByteBpeTokenizer::from_artifact_bytes(&tokenizer_bytes).unwrap();
    for entry in &corpus.documents.entries {
        let tokens = corpus.read_document_tokens(&entry.source_id).unwrap();
        assert_eq!(tokens.last().copied(), Some(2));
        assert_eq!(tokens.iter().filter(|id| **id == 2).count(), 1);
        let source = tokens[..tokens.len() - 1]
            .iter()
            .map(|id| *id as u32)
            .collect::<Vec<_>>();
        assert_eq!(
            tokenizer.decode_source(&source).unwrap(),
            fixture.source_bytes[&entry.source_id]
        );
    }

    let train = corpus
        .documents
        .entries
        .iter()
        .filter(|entry| entry.split == CorpusSplit::Train)
        .collect::<Vec<_>>();
    assert!(
        train[0].component_id.as_bytes() < train[1].component_id.as_bytes(),
        "training documents must use canonical component-first ordering"
    );
}

#[test]
fn publication_is_create_new_and_shard_mutation_is_detected() {
    let fixture = fixture();
    tokenize(&fixture.config).unwrap();
    assert_eq!(
        tokenize(&fixture.config).unwrap_err().code,
        "OUTPUT_ALREADY_EXISTS"
    );

    let corpus = VerifiedTokenCorpus::open(&fixture.output).unwrap();
    let shard = corpus.manifest.shards.first().unwrap().path.clone();
    let shard_path = fixture.output.join(shard);
    let mut bytes = fs::read(&shard_path).unwrap();
    bytes[0] ^= 1;
    fs::write(&shard_path, bytes).unwrap();
    assert_eq!(
        corpus
            .read_document_tokens(&corpus.documents.entries[0].source_id)
            .unwrap_err()
            .code,
        "TOKEN_ARTIFACT_IDENTITY_MISMATCH"
    );
}

#[test]
fn output_is_byte_identical_for_the_same_inputs() {
    let first = fixture();
    tokenize(&first.config).unwrap();
    let first_manifest = fs::read(first.output.join("manifest.json")).unwrap();

    let second = fixture();
    tokenize(&second.config).unwrap();
    let second_manifest = fs::read(second.output.join("manifest.json")).unwrap();
    assert_eq!(first_manifest, second_manifest);
}
