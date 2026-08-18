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
    StackColumnsV1, StackContentEncodingV1, StackLimitsV1, StackPartitionV1, StackSourceConfigV1,
    materialize_stack_source,
};
use rust_llm_pretrain::tokenizer::HashBoundInput;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;

/// Git's blob identity, which is what Software Heritage addresses content by and
/// therefore what the adapter must reproduce to accept a blob.
fn sha1_git(bytes: &[u8]) -> String {
    let mut hasher = <sha1::Sha1 as sha1::Digest>::new();
    sha1::Digest::update(&mut hasher, format!("blob {}\0", bytes.len()).as_bytes());
    sha1::Digest::update(&mut hasher, bytes);
    hex::encode(sha1::Digest::finalize(hasher))
}

/// How a route answers, and how many times it has been asked.
///
/// Stateful because the property under test is precisely that a transient
/// failure is followed by another attempt: a route that always succeeds cannot
/// distinguish "retried once" from "never needed to".
#[derive(Clone)]
struct RouteBehavior {
    body: Vec<u8>,
    /// Answers this many requests with `failure_status` before serving the body.
    failures_before_success: u32,
    failure_status: u16,
    hits: Arc<AtomicU32>,
}

impl RouteBehavior {
    fn always(body: Vec<u8>) -> Self {
        Self {
            body,
            failures_before_success: 0,
            failure_status: 503,
            hits: Arc::new(AtomicU32::new(0)),
        }
    }

    fn failing(status: u16, times: u32, body: Vec<u8>) -> Self {
        Self {
            body,
            failures_before_success: times,
            failure_status: status,
            hits: Arc::new(AtomicU32::new(0)),
        }
    }

    fn hit_counter(&self) -> Arc<AtomicU32> {
        self.hits.clone()
    }
}

struct FixtureServer {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn start(routes: Vec<(String, Vec<u8>)>) -> Self {
        Self::with_behaviors(
            routes
                .into_iter()
                .map(|(path, body)| (path, RouteBehavior::always(body)))
                .collect(),
        )
    }

    fn with_behaviors(routes: Vec<(String, RouteBehavior)>) -> Self {
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

fn serve(mut stream: TcpStream, routes: &[(String, RouteBehavior)]) -> std::io::Result<()> {
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
        Some((_, behavior)) => {
            let seen = behavior.hits.fetch_add(1, Ordering::AcqRel);
            if seen < behavior.failures_before_success {
                write!(
                    stream,
                    "HTTP/1.1 {} Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    behavior.failure_status
                )?;
            } else {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    behavior.body.len()
                )?;
                stream.write_all(&behavior.body)?;
            }
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
        content_encoding: StackContentEncodingV1::Identity,
        language: "Python".to_owned(),
        license_allowlist: vec!["MIT".to_owned(), "Apache-2.0".to_owned()],
        source_snapshot_id: "stack-v2-fixture".to_owned(),
        authorization: SourceAuthorization {
            scheme: "operator-assertion".to_owned(),
            authority_url: "https://example.invalid/authorization".to_owned(),
            authorization_id: "fixture-001".to_owned(),
        },
        required_removal_authorities: vec!["https://example.invalid/removals".to_owned()],
        output_root,
        documents_per_generation: 2,
        blob_id_partition: None,
        limits: StackLimitsV1 {
            maximum_documents: 100,
            maximum_total_bytes: 10_000_000,
            maximum_redirects: 2,
            connect_timeout_seconds: 10,
            read_timeout_seconds: 30,
            // Short delays keep the suite fast; the schedule under test is the
            // shape, not the duration.
            retry_attempts: 0,
            retry_initial_delay_milliseconds: 1,
            retry_maximum_delay_milliseconds: 4,
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
            licenses: vec!["MIT"],
        },
        Row {
            blob_id: sha1_git(&admitted_two),
            repository: "example/two",
            path: "src/beta.py",
            revision: "rev-2",
            language: "Python",
            length_bytes: admitted_two.len() as i64,
            licenses: vec!["MIT", "Apache-2.0"],
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
            licenses: vec!["MIT", "GPL-3.0-only"],
        },
        Row {
            blob_id: sha1_git(&rejected_language),
            repository: "example/four",
            path: "src/delta.rs",
            revision: "rev-4",
            language: "Rust",
            length_bytes: rejected_language.len() as i64,
            licenses: vec!["MIT"],
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
            licenses: vec!["MIT"],
        },
        // The same blob under a second repository: transferred once, not twice.
        Row {
            blob_id: sha1_git(&admitted_one),
            repository: "example/six",
            path: "vendor/alpha.py",
            revision: "rev-6",
            language: "Python",
            length_bytes: admitted_one.len() as i64,
            licenses: vec!["MIT"],
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
    assert!(licenses.contains(&"MIT"));
    assert!(licenses.contains(&"Apache-2.0 AND MIT"));
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
        licenses: vec!["MIT"],
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
        licenses: vec!["MIT"],
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
        licenses: vec!["MIT"],
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
        licenses: vec!["MIT"],
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
            licenses: vec!["MIT"],
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

/// A single row plus the fixture wiring every retry test needs.
fn single_row_fixture(temporary: &Path, content: &[u8]) -> (PathBuf, String) {
    let rows = vec![Row {
        blob_id: sha1_git(content),
        repository: "example/one",
        path: "src/alpha.py",
        revision: "rev-1",
        language: "Python",
        length_bytes: content.len() as i64,
        licenses: vec!["MIT"],
    }];
    let shard = temporary.join("metadata-00000.parquet");
    let digest = write_shard(&shard, &rows);
    (shard, digest)
}

/// A rate limit or a server error is the origin saying "not now", and at a
/// million blobs it will happen. The run must absorb it rather than discard
/// every fetch that preceded it.
#[test]
fn a_transient_status_is_retried_until_the_blob_succeeds() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let (shard, digest) = single_row_fixture(temporary.path(), &content);

    let behavior = RouteBehavior::failing(503, 2, content.clone());
    let hits = behavior.hit_counter();
    let server =
        FixtureServer::with_behaviors(vec![(format!("/content/{}", sha1_git(&content)), behavior)]);

    let output_root = temporary.path().join("stack-source");
    let mut retrying = config(&shard, digest, server.template(), output_root.clone());
    retrying.limits.retry_attempts = 3;
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &retrying);

    let result = materialize_stack_source(&config_path).unwrap();
    assert_eq!(result["status"], "STACK_SOURCE_MATERIALIZED");
    assert_eq!(result["documents"], 1);
    assert_eq!(result["retries_performed"], 2);
    assert_eq!(
        hits.load(Ordering::Acquire),
        3,
        "two refusals then the body"
    );
    // The published bytes are the same ones a first-attempt transfer would have
    // produced, which is the property that makes retrying safe here.
    assert_eq!(
        std::fs::read(output_root.join(format!("documents/{}.py", sha1_git(&content)))).unwrap(),
        content
    );
}

/// Retrying is bounded. When the budget is spent the failure is typed and names
/// what it gave up on, rather than surfacing as the last transport error.
#[test]
fn retries_exhausted_is_a_typed_failure_that_publishes_nothing() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let (shard, digest) = single_row_fixture(temporary.path(), &content);

    let behavior = RouteBehavior::failing(503, u32::MAX, content.clone());
    let hits = behavior.hit_counter();
    let server =
        FixtureServer::with_behaviors(vec![(format!("/content/{}", sha1_git(&content)), behavior)]);

    let output_root = temporary.path().join("stack-source");
    let mut hopeless = config(&shard, digest, server.template(), output_root.clone());
    hopeless.limits.retry_attempts = 2;
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &hopeless);

    let error = materialize_stack_source(&config_path).unwrap_err();
    assert_eq!(error.code, "ACQUISITION_RETRIES_EXHAUSTED");
    assert!(error.message.contains('2'), "the budget is named");
    assert_eq!(
        hits.load(Ordering::Acquire),
        3,
        "the first try plus two retries"
    );
    assert!(
        !output_root.exists(),
        "a failed run leaves no partial generation"
    );
}

/// A 404 is the origin saying "not this". Retrying it would hide a
/// configuration error behind a delay, so it must not be retried at all.
#[test]
fn a_permanent_status_is_not_retried() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let (shard, digest) = single_row_fixture(temporary.path(), &content);

    let behavior = RouteBehavior::failing(404, u32::MAX, content.clone());
    let hits = behavior.hit_counter();
    let server =
        FixtureServer::with_behaviors(vec![(format!("/content/{}", sha1_git(&content)), behavior)]);

    let mut generous = config(
        &shard,
        digest,
        server.template(),
        temporary.path().join("stack-source"),
    );
    generous.limits.retry_attempts = 5;
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &generous);

    let error = materialize_stack_source(&config_path).unwrap_err();
    assert_eq!(error.code, "ACQUISITION_HTTP_STATUS");
    assert_eq!(
        hits.load(Ordering::Acquire),
        1,
        "asked once despite a budget of five"
    );
}

/// A backoff that starts at zero turns a rate limit into a tight loop against
/// the origin, so the configuration refuses it.
#[test]
fn a_zero_backoff_with_retries_enabled_is_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let (shard, digest) = single_row_fixture(temporary.path(), &content);
    let mut broken = config(
        &shard,
        digest,
        "https://example.invalid/content/{blob_id}".to_owned(),
        temporary.path().join("stack-source"),
    );
    broken.limits.retry_attempts = 3;
    broken.limits.retry_initial_delay_milliseconds = 0;
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &broken);

    let error = materialize_stack_source(&config_path).unwrap_err();
    assert_eq!(error.code, "STACK_CONFIG_INVALID");
}

/// Reads every `source_id` a published generation's manifests carry, sorted.
fn published_blob_ids(output_root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut index = 0;
    loop {
        let manifest = output_root.join(format!("source-manifest-{index:05}.json"));
        if !manifest.exists() {
            break;
        }
        let parsed: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        for document in parsed["documents"].as_array().unwrap() {
            found.push(document["provider_record_id"].as_str().unwrap().to_owned());
        }
        index += 1;
    }
    found.sort();
    found
}

/// Partitioning is what makes a million-blob acquisition survivable, and it is
/// only safe if the pieces reconstruct the whole. Both halves are asserted here:
/// no blob appears in two partitions, and their union is exactly what one
/// unpartitioned run selects.
#[test]
fn partitions_are_disjoint_and_their_union_matches_the_full_run() {
    let temporary = tempfile::tempdir().unwrap();
    // Enough distinct bodies that several leading hex digits are represented.
    let bodies = (0..24)
        .map(|index| format!("def f{index}():\n    return {index}\n").into_bytes())
        .collect::<Vec<_>>();
    let rows = bodies
        .iter()
        .map(|body| Row {
            blob_id: sha1_git(body),
            repository: "example/one",
            path: "src/alpha.py",
            revision: "rev-1",
            language: "Python",
            length_bytes: body.len() as i64,
            licenses: vec!["MIT"],
        })
        .collect::<Vec<_>>();
    let shard = temporary.path().join("metadata-00000.parquet");
    let digest = write_shard(&shard, &rows);
    let server = FixtureServer::start(
        bodies
            .iter()
            .map(|body| (format!("/content/{}", sha1_git(body)), body.clone()))
            .collect(),
    );

    let run = |label: &str, partition: Option<(u64, Vec<String>)>| -> Vec<String> {
        let output_root = temporary.path().join(format!("stack-{label}"));
        let mut settings = config(
            &shard,
            digest.clone(),
            server.template(),
            output_root.clone(),
        );
        settings.blob_id_partition = partition.map(|(prefix_length, include)| StackPartitionV1 {
            prefix_length,
            include,
        });
        let config_path = temporary.path().join(format!("config-{label}.json"));
        write_config(&config_path, &settings);
        materialize_stack_source(&config_path).unwrap();
        published_blob_ids(&output_root)
    };

    let whole = run("whole", None);
    assert_eq!(whole.len(), bodies.len(), "the full run selects everything");

    // Sixteen partitions on the first hex digit cover the space exactly once.
    let mut union = Vec::new();
    for digit in "0123456789abcdef".chars() {
        let part = run(&format!("p{digit}"), Some((1, vec![digit.to_string()])));
        for blob in &part {
            assert!(
                blob.starts_with(digit),
                "a partition returned a blob outside its own prefix"
            );
        }
        union.extend(part);
    }
    union.sort();
    assert_eq!(
        union, whole,
        "the union of the partitions is the unpartitioned selection"
    );
    // Disjointness follows from the union having no repeats.
    let mut deduplicated = union.clone();
    deduplicated.dedup();
    assert_eq!(deduplicated, union, "no blob appeared in two partitions");
}

/// The point of partitioning is that a late failure costs one partition rather
/// than the whole acquisition.
#[test]
fn a_failed_partition_leaves_published_partitions_intact() {
    let temporary = tempfile::tempdir().unwrap();
    let bodies = (0..16)
        .map(|index| format!("def g{index}():\n    return {index}\n").into_bytes())
        .collect::<Vec<_>>();
    let rows = bodies
        .iter()
        .map(|body| Row {
            blob_id: sha1_git(body),
            repository: "example/one",
            path: "src/alpha.py",
            revision: "rev-1",
            language: "Python",
            length_bytes: body.len() as i64,
            licenses: vec!["MIT"],
        })
        .collect::<Vec<_>>();
    let shard = temporary.path().join("metadata-00000.parquet");
    let digest = write_shard(&shard, &rows);

    // Serve only the blobs whose identifier starts with the first partition's
    // digit; every other partition will fail on a missing route.
    let served = bodies
        .iter()
        .filter(|body| sha1_git(body).starts_with('a'))
        .collect::<Vec<_>>();
    if served.is_empty() {
        return; // No blob landed in this partition; nothing to assert.
    }
    let server = FixtureServer::start(
        served
            .iter()
            .map(|body| (format!("/content/{}", sha1_git(body)), (*body).clone()))
            .collect(),
    );

    let good_root = temporary.path().join("stack-good");
    let mut good = config(&shard, digest.clone(), server.template(), good_root.clone());
    good.blob_id_partition = Some(StackPartitionV1 {
        prefix_length: 1,
        include: vec!["a".to_owned()],
    });
    let good_path = temporary.path().join("config-good.json");
    write_config(&good_path, &good);
    materialize_stack_source(&good_path).unwrap();
    let published = published_blob_ids(&good_root);
    assert_eq!(published.len(), served.len());

    // A partition whose blobs are unserved fails, and must not disturb the one
    // already on disk.
    let unserved = "0123456789bcdef"
        .chars()
        .find(|digit| bodies.iter().any(|body| sha1_git(body).starts_with(*digit)));
    if let Some(digit) = unserved {
        let bad_root = temporary.path().join("stack-bad");
        let mut bad = config(&shard, digest, server.template(), bad_root.clone());
        bad.blob_id_partition = Some(StackPartitionV1 {
            prefix_length: 1,
            include: vec![digit.to_string()],
        });
        let bad_path = temporary.path().join("config-bad.json");
        write_config(&bad_path, &bad);
        assert!(materialize_stack_source(&bad_path).is_err());
        assert!(!bad_root.exists(), "the failed partition published nothing");
    }
    assert_eq!(
        published_blob_ids(&good_root),
        published,
        "the completed partition is untouched"
    );
}

/// A prefix that cannot match anything would silently select nothing, so the
/// shape is checked rather than discovered at the end of a long run.
#[test]
fn a_malformed_partition_is_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let (shard, digest) = single_row_fixture(temporary.path(), &content);
    let cases = [
        (0_u64, vec!["a".to_owned()]),             // zero-length prefix
        (41, vec!["a".repeat(41)]),                // longer than a sha1_git
        (1, vec![]),                               // admits nothing
        (1, vec!["A".to_owned()]),                 // uppercase
        (1, vec!["z".to_owned()]),                 // not hex
        (2, vec!["a".to_owned()]),                 // wrong length for the prefix
        (1, vec!["a".to_owned(), "a".to_owned()]), // repeated
    ];
    for (prefix_length, include) in cases {
        let mut broken = config(
            &shard,
            digest.clone(),
            "https://example.invalid/content/{blob_id}".to_owned(),
            temporary.path().join("stack-source"),
        );
        broken.blob_id_partition = Some(StackPartitionV1 {
            prefix_length,
            include: include.clone(),
        });
        let config_path = temporary.path().join("config.json");
        write_config(&config_path, &broken);
        let error = materialize_stack_source(&config_path).unwrap_err();
        assert_eq!(
            error.code, "STACK_PARTITION_INVALID",
            "prefix_length {prefix_length} with {include:?} must be refused"
        );
    }
}

/// A partition nothing landed in is a success that publishes nothing, so an
/// operator loop over sixteen partitions does not have to read one failure code
/// as if it meant success.
#[test]
fn an_empty_partition_succeeds_without_publishing() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let (shard, digest) = single_row_fixture(temporary.path(), &content);
    let blob = sha1_git(&content);
    // Deliberately a prefix the one row cannot have.
    let absent = if blob.starts_with('0') { "1" } else { "0" };

    let output_root = temporary.path().join("stack-source");
    let mut empty = config(
        &shard,
        digest,
        "https://example.invalid/content/{blob_id}".to_owned(),
        output_root.clone(),
    );
    empty.blob_id_partition = Some(StackPartitionV1 {
        prefix_length: 1,
        include: vec![absent.to_owned()],
    });
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &empty);

    let result = materialize_stack_source(&config_path).unwrap();
    assert_eq!(result["status"], "STACK_PARTITION_EMPTY");
    assert_eq!(result["documents"], 0);
    assert_eq!(result["output_created"], false);
    assert_eq!(result["skipped_partition"], 1);
    assert!(!output_root.exists(), "nothing is published");
}

/// The same emptiness without a partition is a real misconfiguration and must
/// still fail, because there the filters genuinely selected nothing.
#[test]
fn an_unpartitioned_run_that_selects_nothing_still_fails() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let (shard, digest) = single_row_fixture(temporary.path(), &content);
    let mut nothing = config(
        &shard,
        digest,
        "https://example.invalid/content/{blob_id}".to_owned(),
        temporary.path().join("stack-source"),
    );
    nothing.language = "Rust".to_owned();
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &nothing);

    assert_eq!(
        materialize_stack_source(&config_path).unwrap_err().code,
        "STACK_NO_DOCUMENTS"
    );
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// The bulk mirror serves compressed bodies. Decoding must happen before every
/// check, so the published tree and every recorded digest describe the source
/// file rather than its framing — and must be byte-identical to what the
/// uncompressed route produces.
#[test]
fn gzip_content_is_decoded_before_verification() {
    let temporary = tempfile::tempdir().unwrap();
    let bodies = [
        b"def alpha():\n    return 1\n".to_vec(),
        b"def beta():\n    return 2\n".to_vec(),
    ];
    let rows = bodies
        .iter()
        .map(|body| Row {
            blob_id: sha1_git(body),
            repository: "example/one",
            path: "src/alpha.py",
            revision: "rev-1",
            language: "Python",
            length_bytes: body.len() as i64,
            licenses: vec!["MIT"],
        })
        .collect::<Vec<_>>();
    let shard = temporary.path().join("metadata-00000.parquet");
    let digest = write_shard(&shard, &rows);

    let plain = FixtureServer::start(
        bodies
            .iter()
            .map(|body| (format!("/content/{}", sha1_git(body)), body.clone()))
            .collect(),
    );
    let identity_root = temporary.path().join("stack-identity");
    let identity = config(
        &shard,
        digest.clone(),
        plain.template(),
        identity_root.clone(),
    );
    let identity_path = temporary.path().join("config-identity.json");
    write_config(&identity_path, &identity);
    let identity_result = materialize_stack_source(&identity_path).unwrap();
    drop(plain);

    let compressed = FixtureServer::start(
        bodies
            .iter()
            .map(|body| (format!("/content/{}", sha1_git(body)), gzip(body)))
            .collect(),
    );
    let gzip_root = temporary.path().join("stack-gzip");
    let mut compressed_config = config(&shard, digest, compressed.template(), gzip_root.clone());
    compressed_config.content_encoding = StackContentEncodingV1::Gzip;
    let gzip_path = temporary.path().join("config-gzip.json");
    write_config(&gzip_path, &compressed_config);
    let gzip_result = materialize_stack_source(&gzip_path).unwrap();

    assert_eq!(gzip_result["documents"], 2);
    assert_eq!(gzip_result["total_bytes"], identity_result["total_bytes"]);
    // The manifests describe the same documents by the same digests, so the two
    // routes are interchangeable as far as everything downstream can tell.
    for name in ["source-manifest-00000.json", "index.json"] {
        assert_eq!(
            std::fs::read(gzip_root.join(name)).unwrap(),
            std::fs::read(identity_root.join(name)).unwrap(),
            "{name} differs between the identity and gzip routes"
        );
    }
    for body in &bodies {
        let relative = format!("documents/{}.py", sha1_git(body));
        assert_eq!(&std::fs::read(gzip_root.join(&relative)).unwrap(), body);
    }
}

/// A stream that inflates past its declared length is either a mis-declared row
/// or a decompression bomb; either way it is refused rather than absorbed.
#[test]
fn gzip_that_inflates_past_its_declared_length_is_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let declared = b"def alpha():\n    return 1\n".to_vec();
    let oversized = b"def alpha():\n    return 1\n# padding that was never declared\n".to_vec();
    let rows = vec![Row {
        blob_id: sha1_git(&declared),
        repository: "example/one",
        path: "src/alpha.py",
        revision: "rev-1",
        language: "Python",
        length_bytes: declared.len() as i64,
        licenses: vec!["MIT"],
    }];
    let shard = temporary.path().join("metadata-00000.parquet");
    let digest = write_shard(&shard, &rows);
    let server = FixtureServer::start(vec![(
        format!("/content/{}", sha1_git(&declared)),
        gzip(&oversized),
    )]);

    let output_root = temporary.path().join("stack-source");
    let mut settings = config(&shard, digest, server.template(), output_root.clone());
    settings.content_encoding = StackContentEncodingV1::Gzip;
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &settings);

    assert_eq!(
        materialize_stack_source(&config_path).unwrap_err().code,
        "STACK_CONTENT_LENGTH_MISMATCH"
    );
    assert!(!output_root.exists());
}

/// A body that is not gzip at all under a gzip declaration is a typed decode
/// failure, not a confusing hash mismatch.
#[test]
fn a_body_that_is_not_gzip_is_a_typed_decode_failure() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let (shard, digest) = single_row_fixture(temporary.path(), &content);
    let server = FixtureServer::start(vec![(
        format!("/content/{}", sha1_git(&content)),
        content.clone(),
    )]);

    let mut settings = config(
        &shard,
        digest,
        server.template(),
        temporary.path().join("stack-source"),
    );
    settings.content_encoding = StackContentEncodingV1::Gzip;
    let config_path = temporary.path().join("config.json");
    write_config(&config_path, &settings);

    assert_eq!(
        materialize_stack_source(&config_path).unwrap_err().code,
        "STACK_CONTENT_DECODE_FAILED"
    );
}

/// Encoding is declared, never sniffed: a compressed body under an identity
/// declaration is refused rather than silently detected and inflated.
#[test]
fn gzip_bodies_are_not_sniffed_under_an_identity_declaration() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let (shard, digest) = single_row_fixture(temporary.path(), &content);
    let server = FixtureServer::start(vec![(
        format!("/content/{}", sha1_git(&content)),
        gzip(&content),
    )]);

    let config_path = temporary.path().join("config.json");
    write_config(
        &config_path,
        &config(
            &shard,
            digest,
            server.template(),
            temporary.path().join("stack-source"),
        ),
    );

    // Which check fires depends on whether the compressed form is longer or
    // shorter than the original, so any of the three is correct; what matters is
    // that the bytes never reach the content tree unexamined.
    let error = materialize_stack_source(&config_path).unwrap_err();
    assert!(
        matches!(
            error.code.as_str(),
            "ACQUISITION_LENGTH_MISMATCH"
                | "STACK_CONTENT_LENGTH_MISMATCH"
                | "STACK_CONTENT_HASH_MISMATCH"
        ),
        "unexpected code {}",
        error.code
    );
}

/// An allowlist the frozen P4 policy refuses must fail at configuration time.
///
/// The cost of getting this wrong is not a wrong answer, it is a wasted
/// acquisition: the blob is transferred, verified and written before `curate`
/// rejects its licence, so a permissive-looking identifier the policy does not
/// share would be discovered only after hours of transfer.
#[test]
fn an_allowlist_the_curation_policy_refuses_is_rejected_up_front() {
    let temporary = tempfile::tempdir().unwrap();
    let content = b"def alpha():\n    return 1\n".to_vec();
    let (shard, digest) = single_row_fixture(temporary.path(), &content);
    for refused in ["GPL-3.0-only", "CC0-1.0", "Unlicense", "mit"] {
        let mut settings = config(
            &shard,
            digest.clone(),
            "https://example.invalid/content/{blob_id}".to_owned(),
            temporary.path().join("stack-source"),
        );
        settings.license_allowlist = vec!["MIT".to_owned(), refused.to_owned()];
        let config_path = temporary.path().join("config.json");
        write_config(&config_path, &settings);
        let error = materialize_stack_source(&config_path).unwrap_err();
        assert_eq!(
            error.code, "STACK_LICENSE_NOT_PERMITTED",
            "{refused} must be refused at configuration time"
        );
        assert!(error.message.contains(refused), "the identifier is named");
    }
    // The frozen permissive set itself must pass.
    let mut permitted = config(
        &shard,
        digest,
        "https://example.invalid/content/{blob_id}".to_owned(),
        temporary.path().join("stack-permitted"),
    );
    permitted.license_allowlist = [
        "0BSD",
        "Apache-2.0",
        "BSD-2-Clause",
        "BSD-3-Clause",
        "BSL-1.0",
        "ISC",
        "MIT",
        "MIT-0",
        "Python-2.0",
        "Zlib",
    ]
    .iter()
    .map(|value| (*value).to_owned())
    .collect();
    permitted.language = "Rust".to_owned(); // selects nothing, so no transfer is attempted
    let config_path = temporary.path().join("config-permitted.json");
    write_config(&config_path, &permitted);
    assert_eq!(
        materialize_stack_source(&config_path).unwrap_err().code,
        "STACK_NO_DOCUMENTS",
        "the frozen set passes validation and fails later, on selection"
    );
}
