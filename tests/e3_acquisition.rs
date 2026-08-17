//! E3 governed-acquisition transport contracts.
//!
//! These drive the real `fetch` path — sockets, redirects, streaming, and digest
//! verification — against a loopback fixture server, which is the contract's own
//! provision for testing this (`docs/rebuild-contract.md:110` permits plain HTTP
//! "only inside an explicit local-fixture mode with address restrictions").
//! Validating the transport by unit-testing the URL grammar alone would leave the
//! part that actually touches the network unproven.

use rust_llm_pretrain::acquire::{
    AcquisitionAssetV1, AcquisitionConfigV1, AcquisitionLimits, acquire,
};
use rust_llm_pretrain::backend::PROTOTYPE_PROFILE;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

/// How the fixture should answer a request for a given path.
#[derive(Clone)]
enum Reply {
    Body(Vec<u8>),
    Redirect(String),
    Status(u16),
}

/// A minimal single-purpose HTTP/1.1 origin. Hand-rolled rather than pulled in,
/// because the point is to exercise our client, not someone else's server.
struct FixtureServer {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn start(routes: Vec<(String, Reply)>) -> Self {
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
                        let routes = routes.clone();
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

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
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

fn serve(mut stream: TcpStream, routes: &[(String, Reply)]) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    // Drain headers so the client sees a well-formed exchange.
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let reply = routes
        .iter()
        .find(|(route, _)| route == path)
        .map(|(_, reply)| reply.clone())
        .unwrap_or(Reply::Status(404));

    match reply {
        Reply::Body(body) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(&body)?;
        }
        Reply::Redirect(location) => {
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
        Reply::Status(code) => {
            write!(
                stream,
                "HTTP/1.1 {code} Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
    }
    stream.flush()
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn config(output_root: &Path, assets: Vec<AcquisitionAssetV1>) -> AcquisitionConfigV1 {
    AcquisitionConfigV1 {
        schema: "python-slm-acquisition-config-v1".to_owned(),
        profile: PROTOTYPE_PROFILE.to_owned(),
        output_root: output_root.to_path_buf(),
        allow_loopback_plain_http: true,
        assets,
        limits: AcquisitionLimits {
            maximum_assets: 8,
            maximum_total_bytes: 10_000_000,
            maximum_redirects: 2,
            connect_timeout_seconds: 10,
            read_timeout_seconds: 30,
        },
    }
}

fn asset(url: String, sha256: String, bytes: u64) -> AcquisitionAssetV1 {
    AcquisitionAssetV1 {
        role: "fixture-asset".to_owned(),
        url,
        relative_path: "assets/payload.bin".to_owned(),
        expected_sha256: sha256,
        expected_bytes: bytes,
        credential_env: None,
    }
}

/// Write the config where `fetch` will read it, and return the output root.
fn run(
    directory: &Path,
    config: &AcquisitionConfigV1,
) -> rust_llm_pretrain::error::Result<serde_json::Value> {
    let config_path = directory.join("acquire.json");
    std::fs::write(&config_path, serde_json::to_vec(config).unwrap()).unwrap();
    acquire(&config_path)
}

fn output_root(directory: &Path) -> PathBuf {
    directory.join("acquired")
}

#[test]
fn a_pinned_asset_is_fetched_verified_and_published() {
    let payload = b"# a small fixture payload\nvalue = 1\n".to_vec();
    let server = FixtureServer::start(vec![(
        "/payload.bin".to_owned(),
        Reply::Body(payload.clone()),
    )]);
    let directory = tempfile::tempdir().unwrap();
    let root = output_root(directory.path());
    let configuration = config(
        &root,
        vec![asset(
            server.url("/payload.bin"),
            digest(&payload),
            payload.len() as u64,
        )],
    );

    let result = run(directory.path(), &configuration).unwrap();
    assert_eq!(result["status"], "ASSETS_ACQUIRED");
    assert_eq!(result["acquired_assets"], 1);
    assert_eq!(result["acquired_bytes"], payload.len());
    assert_eq!(result["output_created"], true);
    assert_eq!(result["receipts_written"], false);
    // A fixture-mode generation names itself so it cannot be read back as
    // production acquisition.
    assert_eq!(result["transport"], "loopback-fixture-pinned-digest-v1");

    // The bytes landed where they were declared to, unmodified.
    assert_eq!(
        std::fs::read(root.join("assets/payload.bin")).unwrap(),
        payload
    );

    // The generation carries its own manifest.
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["assets"][0]["sha256"], digest(&payload));
    assert_eq!(manifest["assets"][0]["redirects_followed"], 0);
    assert_eq!(manifest["assets"][0]["credential_supplied"], false);
}

#[test]
fn a_digest_mismatch_publishes_nothing() {
    let server = FixtureServer::start(vec![(
        "/payload.bin".to_owned(),
        Reply::Body(b"the bytes that actually arrive".to_vec()),
    )]);
    let directory = tempfile::tempdir().unwrap();
    let root = output_root(directory.path());
    let configuration = config(
        &root,
        vec![asset(
            server.url("/payload.bin"),
            digest(b"the bytes that were promised"),
            "the bytes that actually arrive".len() as u64,
        )],
    );

    assert_eq!(
        run(directory.path(), &configuration).unwrap_err().code,
        "ACQUISITION_DIGEST_MISMATCH"
    );
    // Nothing is left behind: not the generation, not a partial beside it.
    assert!(!root.exists());
    let strays = std::fs::read_dir(directory.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().contains("partial"))
        .count();
    assert_eq!(strays, 0, "a partial acquisition directory survived");
}

#[test]
fn a_length_mismatch_is_detected_in_both_directions() {
    let body = b"0123456789".to_vec();
    let server = FixtureServer::start(vec![("/payload.bin".to_owned(), Reply::Body(body.clone()))]);

    for declared in [body.len() as u64 - 1, body.len() as u64 + 1] {
        let directory = tempfile::tempdir().unwrap();
        let root = output_root(directory.path());
        let configuration = config(
            &root,
            vec![asset(server.url("/payload.bin"), digest(&body), declared)],
        );
        assert_eq!(
            run(directory.path(), &configuration).unwrap_err().code,
            "ACQUISITION_LENGTH_MISMATCH",
            "declared {declared} against a {}-byte body",
            body.len()
        );
        assert!(!root.exists());
    }
}

#[test]
fn redirects_are_followed_and_bounded() {
    let payload = b"redirected payload".to_vec();
    let listener_probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener_probe.local_addr().unwrap().port();
    drop(listener_probe);
    // Bind the real server on a known port so a redirect target can name it.
    let routes = vec![
        (
            "/one".to_owned(),
            Reply::Redirect(format!("http://127.0.0.1:{port}/two")),
        ),
        (
            "/two".to_owned(),
            Reply::Redirect(format!("http://127.0.0.1:{port}/final")),
        ),
        ("/final".to_owned(), Reply::Body(payload.clone())),
        (
            "/loop".to_owned(),
            Reply::Redirect(format!("http://127.0.0.1:{port}/loop")),
        ),
    ];
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();
    listener.set_nonblocking(true).unwrap();
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

    // Two hops is exactly the configured bound and must succeed.
    let directory = tempfile::tempdir().unwrap();
    let root = output_root(directory.path());
    let configuration = config(
        &root,
        vec![asset(
            format!("http://127.0.0.1:{port}/one"),
            digest(&payload),
            payload.len() as u64,
        )],
    );
    let result = run(directory.path(), &configuration).unwrap();
    assert_eq!(result["acquired_assets"], 1);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["assets"][0]["redirects_followed"], 2);

    // An endless redirect must terminate at the bound rather than spin.
    let directory = tempfile::tempdir().unwrap();
    let looping = config(
        &output_root(directory.path()),
        vec![asset(
            format!("http://127.0.0.1:{port}/loop"),
            digest(&payload),
            payload.len() as u64,
        )],
    );
    assert_eq!(
        run(directory.path(), &looping).unwrap_err().code,
        "ACQUISITION_REDIRECT_LIMIT_EXCEEDED"
    );

    stop.store(true, Ordering::Release);
    let _ = handle.join();
}

/// The reason every hop is re-validated rather than delegated to the client.
#[test]
fn a_redirect_to_a_non_loopback_host_is_rejected() {
    let server = FixtureServer::start(vec![(
        "/downgrade".to_owned(),
        Reply::Redirect("http://example.invalid/payload.bin".to_owned()),
    )]);
    let directory = tempfile::tempdir().unwrap();
    let root = output_root(directory.path());
    let configuration = config(
        &root,
        vec![asset(server.url("/downgrade"), "ab".repeat(32), 16)],
    );

    assert_eq!(
        run(directory.path(), &configuration).unwrap_err().code,
        "ACQUISITION_URL_INVALID"
    );
    assert!(!root.exists());
}

#[test]
fn an_origin_error_status_fails_closed() {
    let server = FixtureServer::start(vec![("/missing".to_owned(), Reply::Status(404))]);
    let directory = tempfile::tempdir().unwrap();
    let root = output_root(directory.path());
    let configuration = config(
        &root,
        vec![asset(server.url("/missing"), "cd".repeat(32), 16)],
    );

    assert_eq!(
        run(directory.path(), &configuration).unwrap_err().code,
        "ACQUISITION_HTTP_STATUS"
    );
    assert!(!root.exists());
}

/// Without the explicit fixture flag the same loopback URL is refused, so the
/// exemption cannot be reached by accident.
#[test]
fn loopback_plain_http_requires_the_explicit_flag() {
    let payload = b"payload".to_vec();
    let server = FixtureServer::start(vec![(
        "/payload.bin".to_owned(),
        Reply::Body(payload.clone()),
    )]);
    let directory = tempfile::tempdir().unwrap();
    let mut configuration = config(
        &output_root(directory.path()),
        vec![asset(
            server.url("/payload.bin"),
            digest(&payload),
            payload.len() as u64,
        )],
    );
    configuration.allow_loopback_plain_http = false;

    assert_eq!(
        run(directory.path(), &configuration).unwrap_err().code,
        "ACQUISITION_URL_INVALID"
    );
}

#[test]
fn a_named_credential_is_read_from_the_environment_and_never_serialized() {
    let payload = b"authorized payload".to_vec();
    let server = FixtureServer::start(vec![(
        "/private.bin".to_owned(),
        Reply::Body(payload.clone()),
    )]);
    let directory = tempfile::tempdir().unwrap();
    let root = output_root(directory.path());
    let mut declared = asset(
        server.url("/private.bin"),
        digest(&payload),
        payload.len() as u64,
    );
    declared.credential_env = Some("PYTHON_SLM_E3_FIXTURE_TOKEN".to_owned());
    let configuration = config(&root, vec![declared]);

    // Absent variable fails closed before any socket is opened.
    unsafe { std::env::remove_var("PYTHON_SLM_E3_FIXTURE_TOKEN") };
    assert_eq!(
        run(directory.path(), &configuration).unwrap_err().code,
        "ACQUISITION_CREDENTIAL_MISSING"
    );

    unsafe { std::env::set_var("PYTHON_SLM_E3_FIXTURE_TOKEN", "s3cret-value") };
    let result = run(directory.path(), &configuration).unwrap();
    assert_eq!(result["acquired_assets"], 1);

    // The token reaches the request but never the artifacts.
    let manifest = std::fs::read_to_string(root.join("manifest.json")).unwrap();
    assert!(manifest.contains("\"credential_supplied\":true"));
    assert!(!manifest.contains("s3cret-value"));
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("s3cret-value")
    );
    unsafe { std::env::remove_var("PYTHON_SLM_E3_FIXTURE_TOKEN") };
}

#[test]
fn acquisition_refuses_to_overwrite_an_existing_generation() {
    let payload = b"payload".to_vec();
    let server = FixtureServer::start(vec![(
        "/payload.bin".to_owned(),
        Reply::Body(payload.clone()),
    )]);
    let directory = tempfile::tempdir().unwrap();
    let root = output_root(directory.path());
    std::fs::create_dir_all(&root).unwrap();
    let configuration = config(
        &root,
        vec![asset(
            server.url("/payload.bin"),
            digest(&payload),
            payload.len() as u64,
        )],
    );

    assert_eq!(
        run(directory.path(), &configuration).unwrap_err().code,
        "OUTPUT_ALREADY_EXISTS"
    );
}
