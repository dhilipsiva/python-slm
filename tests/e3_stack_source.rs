//! The `SOURCE-001` Stack v2 adapter: metadata projection, governance filters,
//! and the identifier-to-content link.
//!
//! Everything here runs the real command over a real Parquet shard and a real
//! socket. The contract's local-fixture exemption (`docs/rebuild-contract.md:110`)
//! is what makes plain HTTP admissible for the transport half, and writing an
//! actual Parquet file rather than mocking the reader is what makes the
//! projection half meaningful — a hand-stubbed row set would prove nothing about
//! whether the column binding works.

use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use rust_llm_pretrain::data::SourceAuthorization;
use rust_llm_pretrain::stack::{
    StackColumnsV1, StackLimitsV1, StackSourceConfigV1, materialize_stack_source,
};
use rust_llm_pretrain::tokenizer::HashBoundInput;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

/// Git's blob identity, which is what Software Heritage addresses content by and
/// therefore what the adapter must reproduce to accept a blob.
fn sha1_git(bytes: &[u8]) -> String {
    let mut hasher = <sha1::Sha1 as sha1::Digest>::new();
    sha1::Digest::update(&mut hasher, format!("blob {}\0", bytes.len()).as_bytes());
    sha1::Digest::update(&mut hasher, bytes);
    hex::encode(sha1::Digest::finalize(hasher))
}

struct FixtureServer {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn start(routes: Vec<(String, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = serve(stream, &routes);
                    }
                    Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            stop,
            handle: Some(handle),
        }
    }

    fn template(&self) -> String {
        format!("http://127.0.0.1:{}/content/{{blob_id}}", self.port)
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(mut stream: TcpStream, routes: &[(String, Vec<u8>)]) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    match routes.iter().find(|(route, _)| route == path) {
        Some((_, body)) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body)?;
        }
        None => write!(
            stream,
            "HTTP/1.1 404 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?,
    }
    stream.flush()
}

/// One metadata row, in the shape the published Stack v2 schema uses.
struct Row {
    blob_id: String,
    repository: &'static str,
    path: &'static str,
    revision: &'static str,
    language: &'static str,
    length_bytes: i64,
    licenses: Vec<&'static str>,
}

fn write_shard(path: &Path, rows: &[Row]) -> String {
    let schema = Arc::new(Schema::new(vec![
        Field::new("blob_id", DataType::Utf8, false),
        Field::new("repo_name", DataType::Utf8, false),
        Field::new("path", DataType::Utf8, false),
        Field::new("revision_id", DataType::Utf8, false),
        Field::new("language", DataType::Utf8, false),
        Field::new("length_bytes", DataType::Int64, false),
        Field::new(
            "detected_licenses",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            false,
        ),
    ]));
    let licenses = {
        let mut builder =
            arrow_array::builder::ListBuilder::new(arrow_array::builder::StringBuilder::new());
        for row in rows {
            for license in &row.licenses {
                builder.values().append_value(license);
            }
            builder.append(true);
        }
        builder.finish()
    };
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|row| row.blob_id.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.repository).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.path).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.revision).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.language).collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                rows.iter().map(|row| row.length_bytes).collect::<Vec<_>>(),
            )),
            Arc::new(licenses),
        ],
    )
    .expect("record batch");
    let file = std::fs::File::create(path).expect("create shard");
    let mut writer = ArrowWriter::try_new(file, schema, None).expect("writer");
    writer.write(&batch).expect("write batch");
    writer.close().expect("close writer");
    hex::encode(Sha256::digest(std::fs::read(path).expect("read shard")))
}

fn columns() -> StackColumnsV1 {
    StackColumnsV1 {
        blob_id: "blob_id".to_owned(),
        repository: "repo_name".to_owned(),
        path: "path".to_owned(),
        revision: "revision_id".to_owned(),
        language: "language".to_owned(),
        length_bytes: "length_bytes".to_owned(),
        detected_licenses: "detected_licenses".to_owned(),
    }
}

fn config(
    shard: &Path,
    shard_digest: String,
    template: String,
    output_root: PathBuf,
) -> StackSourceConfigV1 {
    StackSourceConfigV1 {
        schema: "python-slm-stack-v2-source-config-v1".to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        metadata_shards: vec![HashBoundInput {
            path: shard.to_path_buf(),
            sha256: shard_digest,
        }],
        columns: columns(),
        content_url_template: template,
        content_credential_env: None,
        allow_loopback_plain_http: true,
        language: "Python".to_owned(),
        license_allowlist: vec!["mit".to_owned(), "apache-2.0".to_owned()],
        source_snapshot_id: "stack-v2-fixture".to_owned(),
        authorization: SourceAuthorization {
            scheme: "operator-assertion".to_owned(),
            authority_url: "https://example.invalid/authorization".to_owned(),
            authorization_id: "fixture-001".to_owned(),
        },
        required_removal_authorities: vec!["https://example.invalid/removals".to_owned()],
        output_root,
        documents_per_generation: 2,
        limits: StackLimitsV1 {
            maximum_documents: 100,
            maximum_total_bytes: 10_000_000,
            maximum_redirects: 2,
            connect_timeout_seconds: 10,
            read_timeout_seconds: 30,
        },
    }
}

fn write_config(path: &Path, config: &StackSourceConfigV1) {
    std::fs::write(path, serde_json::to_vec(config).expect("serialize")).expect("write config");
}

/// The whole adapter on one shard: what each filter rejects, and what survives.
///
/// The rows are chosen so every rejection rule fires exactly once, which is why
/// the counts in the result are asserted individually rather than only the
/// admitted total — a single number cannot distinguish "the licence filter
/// worked" from "the language filter rejected everything".
#[test]
fn stack_metadata_and_content_become_a_governed_source_generation() {
    let temporary = tempfile::tempdir().unwrap();
    let admitted_one = b"def alpha():\n    return 1\n".to_vec();
    let admitted_two = b"def beta():\n    return 2\n".to_vec();
    let rejected_license = b"def gamma():\n    return 3\n".to_vec();
    let rejected_language = b"fn delta() -> i32 { 4 }\n".to_vec();

    let rows = vec![
        Row {
            blob_id: sha1_git(&admitted_one),
            repository: "example/one",
            path: "src/alpha.py",
            revision: "rev-1",
            language: "Python",
            length_bytes: admitted_one.len() as i64,
            licenses: vec!["mit"],
        },
        Row {
            blob_id: sha1_git(&admitted_two),
            repository: "example/two",
            path: "src/beta.py",
            revision: "rev-2",
            language: "Python",
            length_bytes: admitted_two.len() as i64,
            licenses: vec!["mit", "apache-2.0"],
        },
        // Dual-licensed with one term outside the allowlist: the conservative
        // reading rejects it, and that is the rule being pinned here.
        Row {
            blob_id: sha1_git(&rejected_license),
            repository: "example/three",
            path: "src/gamma.py",
            revision: "rev-3",
            language: "Python",
            length_bytes: rejected_license.len() as i64,
            licenses: vec!["mit", "gpl-3.0"],
        },
        Row {
            blob_id: sha1_git(&rejected_language),
            repository: "example/four",
            path: "src/delta.rs",
            revision: "rev-4",
            language: "Rust",
            length_bytes: rejected_language.len() as i64,
            licenses: vec!["mit"],
        },
        // Above the frozen 1,000,000-byte document ceiling, so it must be
        // skipped from its declared length without ever being requested.
        Row {
            blob_id: sha1_git(b"oversize"),
            repository: "example/five",
            path: "src/huge.py",
            revision: "rev-5",
            language: "Python",
            length_bytes: 1_000_001,
            licenses: vec!["mit"],
        },
        // The same blob under a second repository: transferred once, not twice.
        Row {
            blob_id: sha1_git(&admitted_one),
            repository: "example/six",
            path: "vendor/alpha.py",
            revision: "rev-6",
            language: "Python",
            length_bytes: admitted_one.len() as i64,
            licenses: vec!["mit"],
        },
    ];

    let shard = temporary.path().join("metadata-00000.parquet");
    let shard_digest = write_shard(&shard, &rows);
    let server = FixtureServer::start(vec![
        (
            format!("/content/{}", sha1_git(&admitted_one)),
            admitted_one.clone(),
        ),
        (
            format!("/content/{}", sha1_git(&admitted_two)),
            admitted_two.clone(),
        ),
    ]);

    let output_root = temporary.path().join("stack-source");
    let config = config(&shard, shard_digest, server.template(), output_root.clone());
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &config);

    let result = materialize_stack_source(&config_path).unwrap();
    assert_eq!(result["status"], "STACK_SOURCE_MATERIALIZED");
    assert_eq!(result["documents"], 2);
    assert_eq!(result["metadata_rows"], 6);
    assert_eq!(result["skipped_license"], 1);
    assert_eq!(result["skipped_language"], 1);
    assert_eq!(result["skipped_oversize"], 1);
    assert_eq!(result["skipped_duplicate"], 1);
    assert_eq!(
        result["total_bytes"],
        (admitted_one.len() + admitted_two.len()) as u64
    );

    // The content tree holds exactly the admitted blobs, addressed by identifier.
    for blob in [&admitted_one, &admitted_two] {
        let path = output_root.join(format!("documents/{}.py", sha1_git(blob)));
        assert_eq!(&std::fs::read(&path).unwrap(), blob);
    }
    assert!(
        !output_root
            .join(format!("documents/{}.py", sha1_git(&rejected_license)))
            .exists(),
        "a rejected row must not reach the content tree"
    );

    // One document per generation shard beyond the first, and each manifest
    // carries the per-row licence rather than one blanket declaration.
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output_root.join("index.json")).unwrap()).unwrap();
    assert_eq!(index["generations"].as_array().unwrap().len(), 1);
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(output_root.join("source-manifest-00000.json")).unwrap(),
    )
    .unwrap();
    let documents = manifest["documents"].as_array().unwrap();
    assert_eq!(documents.len(), 2);
    let licenses = documents
        .iter()
        .map(|document| document["license_expression"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(licenses.contains(&"mit"));
    assert!(licenses.contains(&"apache-2.0 AND mit"));
    assert_eq!(manifest["adapter_namespace"], "stack-v2-software-heritage");

    // Create-new: a second run against the same root must refuse.
    assert!(materialize_stack_source(&config_path).is_err());
}

/// The identifier is the only thing binding a metadata row to bytes, so an
/// endpoint returning something else must fail the run rather than quietly
/// publish whatever arrived.
#[test]
fn content_that_does_not_match_its_identifier_is_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let declared = b"def alpha():\n    return 1\n".to_vec();
    // Same length on purpose: otherwise the declared-length check would fire
    // first and the identifier check would go untested.
    let substituted = b"def alpha():\n    return 2\n".to_vec();
    assert_eq!(declared.len(), substituted.len());

    let rows = vec![Row {
        blob_id: sha1_git(&declared),
        repository: "example/one",
        path: "src/alpha.py",
        revision: "rev-1",
        language: "Python",
        length_bytes: declared.len() as i64,
        licenses: vec!["mit"],
    }];
    let shard = temporary.path().join("metadata-00000.parquet");
    let shard_digest = write_shard(&shard, &rows);
    let server = FixtureServer::start(vec![(
        format!("/content/{}", sha1_git(&declared)),
        substituted,
    )]);

    let output_root = temporary.path().join("stack-source");
    let config_path = temporary.path().join("config.json");
    write_config(
        &config_path,
        &config(&shard, shard_digest, server.template(), output_root.clone()),
    );

    let error = materialize_stack_source(&config_path).unwrap_err();
    assert_eq!(error.code, "STACK_CONTENT_HASH_MISMATCH");
    assert!(
        !output_root.exists(),
        "a failed run must leave no partial generation"
    );
}

/// The shards are pinned by the configuration, so a shard that changed after it
/// was pinned must not be read at all.
#[test]
fn a_metadata_shard_that_does_not_match_its_digest_is_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let rows = vec![Row {
        blob_id: sha1_git(&content),
        repository: "example/one",
        path: "src/alpha.py",
        revision: "rev-1",
        language: "Python",
        length_bytes: content.len() as i64,
        licenses: vec!["mit"],
    }];
    let shard = temporary.path().join("metadata-00000.parquet");
    write_shard(&shard, &rows);
    let output_root = temporary.path().join("stack-source");
    let config_path = temporary.path().join("config.json");
    write_config(
        &config_path,
        &config(
            &shard,
            "b".repeat(64),
            "https://example.invalid/content/{blob_id}".to_owned(),
            output_root.clone(),
        ),
    );

    let error = materialize_stack_source(&config_path).unwrap_err();
    assert_eq!(error.code, "STACK_METADATA_HASH_MISMATCH");
    assert!(!output_root.exists());
}

/// A column the operator named but the shard does not carry must be a legible
/// failure naming the column, not a panic or a silently empty projection.
#[test]
fn a_missing_declared_column_names_itself() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let rows = vec![Row {
        blob_id: sha1_git(&content),
        repository: "example/one",
        path: "src/alpha.py",
        revision: "rev-1",
        language: "Python",
        length_bytes: content.len() as i64,
        licenses: vec!["mit"],
    }];
    let shard = temporary.path().join("metadata-00000.parquet");
    let shard_digest = write_shard(&shard, &rows);
    let mut broken = config(
        &shard,
        shard_digest,
        "https://example.invalid/content/{blob_id}".to_owned(),
        temporary.path().join("stack-source"),
    );
    broken.columns.blob_id = "swhid".to_owned();
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &broken);

    let error = materialize_stack_source(&config_path).unwrap_err();
    assert_eq!(error.code, "STACK_METADATA_COLUMN_MISSING");
    assert!(error.message.contains("swhid"), "the column must be named");
}

/// Credentials come from the environment and only from there, so a config naming
/// a variable that is not set must fail before any transfer.
#[test]
fn a_missing_credential_variable_fails_before_transfer() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let rows = vec![Row {
        blob_id: sha1_git(&content),
        repository: "example/one",
        path: "src/alpha.py",
        revision: "rev-1",
        language: "Python",
        length_bytes: content.len() as i64,
        licenses: vec!["mit"],
    }];
    let shard = temporary.path().join("metadata-00000.parquet");
    let shard_digest = write_shard(&shard, &rows);
    let server = FixtureServer::start(vec![(
        format!("/content/{}", sha1_git(&content)),
        content.clone(),
    )]);
    let mut with_credential = config(
        &shard,
        shard_digest,
        server.template(),
        temporary.path().join("stack-source"),
    );
    with_credential.content_credential_env =
        Some("PYTHON_SLM_STACK_FIXTURE_TOKEN_ABSENT".to_owned());
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &with_credential);

    let error = materialize_stack_source(&config_path).unwrap_err();
    assert_eq!(error.code, "ACQUISITION_CREDENTIAL_MISSING");
    assert!(
        error
            .message
            .contains("PYTHON_SLM_STACK_FIXTURE_TOKEN_ABSENT"),
        "the variable is named"
    );
    assert!(
        !error.message.contains("token"),
        "no value is ever echoed back"
    );
}

/// A template without the substitution would request one blob repeatedly, which
/// is a configuration error rather than something to discover at transfer time.
#[test]
fn a_template_without_the_substitution_is_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let shard = temporary.path().join("metadata-00000.parquet");
    let shard_digest = write_shard(
        &shard,
        &[Row {
            blob_id: sha1_git(b"x"),
            repository: "example/one",
            path: "src/alpha.py",
            revision: "rev-1",
            language: "Python",
            length_bytes: 1,
            licenses: vec!["mit"],
        }],
    );
    let mut broken = config(
        &shard,
        shard_digest,
        "https://example.invalid/content/fixed".to_owned(),
        temporary.path().join("stack-source"),
    );
    broken.documents_per_generation = 2;
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &broken);

    let error = materialize_stack_source(&config_path).unwrap_err();
    assert_eq!(error.code, "STACK_CONFIG_INVALID");
}
