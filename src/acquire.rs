//! Governed HTTPS acquisition.
//!
//! Every other artifact in this pipeline is reproducible because it is bound to a
//! digest declared in advance, and acquisition is held to the same standard: the
//! configuration names each asset's URL, its expected SHA-256, and its expected
//! length, and nothing is published unless the bytes that arrived match all
//! three. There is deliberately no discovery or crawling â€” an endpoint listing
//! that can change between runs would make the corpus irreproducible, which is
//! the one property the rest of the repository is built to preserve.
//!
//! The rules implemented here are the contract's, not new policy.
//! `docs/rebuild-contract.md:109` marks HTTPS, checksums, and environment-only
//! credentials `REIMPLEMENT` and "mandatory in production"; `:110` drops generic
//! plain HTTP; `:113` drops the download-all flow in favour of bounded streaming
//! and resumable verified generations. `AGENTS.md` requires the same HTTPS,
//! redirect, path-containment, and hash-chain rules.

use crate::data::source::{
    compact_json_line, is_sha256, join_relative, parse_closed, require_output_boundary,
    require_portable_relative_path, sha256,
};
use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const IMPLEMENTATION_PHASE: &str = "E3";
pub const ACQUISITION_CONFIG_SCHEMA: &str = "python-slm-acquisition-config-v1";
pub const ACQUISITION_RESULT_SCHEMA: &str = "python-slm-acquisition-result-v1";
pub const ACQUISITION_MANIFEST_SCHEMA: &str = "python-slm-acquisition-manifest-v1";
pub const DISCOVERY_CONFIG_SCHEMA: &str = "python-slm-acquisition-discovery-v1";
pub const DISCOVERY_RESULT_SCHEMA: &str = "python-slm-acquisition-discovery-result-v1";

/// Bytes read per streaming chunk. Bounded so a hostile or misdeclared response
/// cannot force an unbounded allocation before the length check runs.
const STREAM_CHUNK_BYTES: usize = 64 * 1024;

/// The largest asset this command will accept a declaration for, independent of
/// configured limits, so a single typo cannot commit the host to an unbounded
/// transfer.
const MAXIMUM_DECLARED_ASSET_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcquisitionConfigV1 {
    pub schema: String,
    pub profile: String,
    pub output_root: PathBuf,
    /// The contract's local-fixture exemption (`docs/rebuild-contract.md:110`):
    /// plain HTTP is permitted only here, and only to a literal loopback address.
    /// It exists so the transport itself is testable; a generation acquired this
    /// way is labelled in its own manifest so it can never be mistaken for a
    /// production one.
    pub allow_loopback_plain_http: bool,
    pub assets: Vec<AcquisitionAssetV1>,
    pub limits: AcquisitionLimits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcquisitionAssetV1 {
    /// Operator label, carried into the result so a downstream stage can identify
    /// which file is which without re-deriving it from the URL.
    pub role: String,
    pub url: String,
    /// Portable, contained destination beneath `output_root`.
    pub relative_path: String,
    pub expected_sha256: String,
    pub expected_bytes: u64,
    /// Name of an environment variable holding a bearer token. The token itself
    /// never appears in configuration, in the result, or in an error.
    pub credential_env: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcquisitionLimits {
    pub maximum_assets: u64,
    pub maximum_total_bytes: u64,
    pub maximum_redirects: u64,
    pub connect_timeout_seconds: u64,
    pub read_timeout_seconds: u64,
}

/// The published generation's own manifest, so a fetched tree is verifiable
/// afterwards without the configuration that produced it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcquisitionManifestV1 {
    pub schema: String,
    pub profile: String,
    pub qualification_status: String,
    pub transport: String,
    pub assets: Vec<AcquiredAssetV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcquiredAssetV1 {
    pub role: String,
    pub relative_path: String,
    pub sha256: String,
    pub bytes: u64,
    pub redirects_followed: u64,
    pub credential_supplied: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcquisitionResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub profile: String,
    pub transport: String,
    pub configuration_sha256: String,
    pub manifest_sha256: String,
    pub acquired_assets: u64,
    pub acquired_bytes: u64,
    pub output_created: bool,
    pub receipts_written: bool,
    pub limitations: Vec<String>,
}

impl AcquisitionConfigV1 {
    pub fn validate(&self) -> Result<()> {
        if self.schema != ACQUISITION_CONFIG_SCHEMA {
            return Err(ProductError::usage(
                "ACQUISITION_CONFIG_SCHEMA_UNSUPPORTED",
                "the acquisition configuration is not the closed acquisition schema",
            ));
        }
        if self.profile != crate::backend::PROTOTYPE_PROFILE {
            return Err(ProductError::gate(
                "DEFERRED_POST_P16",
                "only the prototype profile is implemented",
            ));
        }
        if !self.output_root.is_absolute() {
            return Err(ProductError::usage(
                "ACQUISITION_PATH_NOT_ABSOLUTE",
                "the acquisition output root must be absolute",
            ));
        }
        if self.limits.maximum_assets == 0
            || self.limits.maximum_total_bytes == 0
            || self.limits.connect_timeout_seconds == 0
            || self.limits.read_timeout_seconds == 0
        {
            return Err(ProductError::usage(
                "ACQUISITION_LIMIT_INVALID",
                "every acquisition limit except the redirect bound must be nonzero",
            ));
        }
        if self.assets.is_empty() || self.assets.len() as u64 > self.limits.maximum_assets {
            return Err(ProductError::usage(
                "ACQUISITION_ASSET_COUNT_INVALID",
                "the acquisition set is empty or exceeds its configured bound",
            ));
        }

        let mut total = 0_u64;
        let mut seen_paths = std::collections::BTreeSet::new();
        for asset in &self.assets {
            require_acquisition_url(&asset.url, self.allow_loopback_plain_http)?;
            require_bounded_text(&asset.role, "ACQUISITION_ROLE_INVALID")?;
            require_portable_relative_path(&asset.relative_path, "ACQUISITION_PATH_INVALID")?;
            if !is_sha256(&asset.expected_sha256) {
                return Err(ProductError::usage(
                    "ACQUISITION_DIGEST_INVALID",
                    "an acquisition asset digest is not lowercase SHA-256",
                ));
            }
            if asset.expected_bytes == 0 || asset.expected_bytes > MAXIMUM_DECLARED_ASSET_BYTES {
                return Err(ProductError::usage(
                    "ACQUISITION_LENGTH_INVALID",
                    "an acquisition asset declares zero or an implausible length",
                ));
            }
            if let Some(variable) = &asset.credential_env {
                require_bounded_text(variable, "ACQUISITION_CREDENTIAL_ENV_INVALID")?;
                if variable.contains('=') {
                    return Err(ProductError::usage(
                        "ACQUISITION_CREDENTIAL_ENV_INVALID",
                        "a credential environment variable name may not contain '='",
                    ));
                }
            }
            if !seen_paths.insert(asset.relative_path.clone()) {
                return Err(ProductError::usage(
                    "ACQUISITION_PATH_DUPLICATE",
                    "two acquisition assets declare the same destination",
                ));
            }
            total = total.checked_add(asset.expected_bytes).ok_or_else(|| {
                ProductError::integrity(
                    "ACQUISITION_ACCOUNTING_OVERFLOW",
                    "declared acquisition bytes overflowed",
                )
            })?;
        }
        if total > self.limits.maximum_total_bytes {
            return Err(ProductError::gate(
                "ACQUISITION_TOTAL_LIMIT_EXCEEDED",
                "the declared acquisition set exceeds its configured total byte bound",
            ));
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String> {
        Ok(sha256(&compact_json_line(
            self,
            "ACQUISITION_CONFIG_SERIALIZE_FAILED",
        )?))
    }
}

/// The contract's URL grammar: HTTPS only, credential-free, and free of the
/// characters that make a URL ambiguous to split or log.
///
/// This mirrors the rule already enforced on provenance URLs in
/// `crate::data::source`, deliberately rather than by import, because that one is
/// about recording where a document came from and this one is about deciding what
/// this process will actually connect to.
pub fn require_acquisition_url(value: &str, allow_loopback_plain_http: bool) -> Result<()> {
    let invalid =
        |message: &'static str| Err(ProductError::usage("ACQUISITION_URL_INVALID", message));
    let authority_start = if let Some(rest) = value.strip_prefix("https://") {
        value.len() - rest.len()
    } else if allow_loopback_plain_http && value.starts_with("http://") {
        // The exemption is narrow on purpose: a literal loopback address, never a
        // name. `localhost` is a resolver lookup and can be pointed anywhere, so
        // permitting it would turn a test affordance into a real bypass.
        let rest = &value["http://".len()..];
        let authority = rest.split(['/', '?']).next().unwrap_or("");
        let host = authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host);
        if host != "127.0.0.1" && host != "[::1]" {
            return invalid("plain HTTP is permitted only to a literal loopback address");
        }
        "http://".len()
    } else {
        return invalid("plain HTTP and non-HTTPS schemes are rejected outright");
    };
    if value.len() > 4096 || !value.is_ascii() {
        return invalid("an acquisition URL is oversized or not ASCII");
    }
    if value
        .bytes()
        .any(|byte| byte.is_ascii_whitespace() || byte < 0x20 || byte == 0x7f)
    {
        return invalid("an acquisition URL contains whitespace or control characters");
    }
    if value.contains('\\') || value.contains('#') {
        return invalid("an acquisition URL contains a backslash or fragment");
    }
    let rest = &value[authority_start..];
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    if authority.is_empty() {
        return invalid("an acquisition URL has no authority");
    }
    if authority.contains('@') {
        return invalid("credentials must not be embedded in an acquisition URL");
    }
    Ok(())
}

fn require_bounded_text(value: &str, code: &'static str) -> Result<()> {
    if value.is_empty()
        || value.len() > 4096
        || value.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
    {
        return Err(ProductError::usage(
            code,
            "a bounded text field is empty, oversized, or contains control characters",
        ));
    }
    Ok(())
}

/// A fixture-mode generation names itself, so a tree acquired over loopback plain
/// HTTP can never be read back as a production acquisition.
fn transport_label(allow_loopback_plain_http: bool) -> &'static str {
    if allow_loopback_plain_http {
        "loopback-fixture-pinned-digest-v1"
    } else {
        "https-pinned-digest-v1"
    }
}

pub(crate) struct FetchedAsset {
    pub(crate) bytes: Vec<u8>,
    pub(crate) sha256: String,
    pub(crate) redirects_followed: u64,
}

/// One transfer to perform. Discovery has no declared digest or length, so the
/// only bound available to it is an explicit ceiling; the publishing path passes
/// its declaration as that ceiling and checks the result afterwards.
pub(crate) struct FetchRequest<'a> {
    pub(crate) url: &'a str,
    pub(crate) credential_env: Option<&'a str>,
    pub(crate) ceiling_bytes: u64,
}

/// Why a transfer failed, and therefore whether repeating it could help.
///
/// The distinction is the whole point: retrying a `403` or a `404` is noise that
/// hides a configuration error behind a delay, while retrying a `429` or a reset
/// connection is the difference between a run that finishes and one that does
/// not. Nothing about the classification touches bytes — a retried transfer is
/// verified against the same digest as a first-attempt one.
pub(crate) enum FetchFailure {
    Transient(ProductError),
    Permanent(ProductError),
}

impl FetchFailure {
    /// Deliberately an inherent method rather than a `From` impl: a second
    /// conversion into `ProductError` makes the error type of every `?` in the
    /// crate ambiguous, which surfaces as inference failures in code that has
    /// nothing to do with acquisition.
    fn into_error(self) -> ProductError {
        match self {
            FetchFailure::Transient(error) | FetchFailure::Permanent(error) => error,
        }
    }
}

/// A bounded, purely arithmetic backoff schedule.
///
/// No jitter and no randomness. This lane pins its RNG so that two runs of one
/// configuration can be compared, and a retry schedule that varied per run would
/// be one more thing to explain away when they differed. The schedule moves wall
/// clock only.
pub(crate) struct RetryPolicy {
    pub(crate) attempts: u64,
    pub(crate) initial_delay: Duration,
    pub(crate) maximum_delay: Duration,
}

impl RetryPolicy {
    /// The delay before the retry that follows attempt `index`, counting from
    /// zero, doubling until it reaches the ceiling and staying there.
    pub(crate) fn delay(&self, index: u32) -> Duration {
        let factor = 1_u32.checked_shl(index.min(31)).unwrap_or(u32::MAX);
        self.initial_delay
            .checked_mul(factor)
            .unwrap_or(self.maximum_delay)
            .min(self.maximum_delay)
    }
}

/// Stream one asset, retrying only the failures that repeating could fix.
///
/// Determinism is unaffected, and it is worth saying why rather than assuming
/// it: every published byte is verified against the digest its metadata declared
/// before it is written, so a transfer that needed three attempts produces the
/// same artifact as one that needed none. Attempt counts and elapsed time reach
/// the one-shot result object and are inputs to no hash.
pub(crate) fn fetch_with_retry(
    agent: &ureq::Agent,
    request: FetchRequest<'_>,
    maximum_redirects: u64,
    allow_loopback_plain_http: bool,
    policy: &RetryPolicy,
) -> Result<(FetchedAsset, u64)> {
    let mut retries = 0_u64;
    loop {
        let attempt = FetchRequest {
            url: request.url,
            credential_env: request.credential_env,
            ceiling_bytes: request.ceiling_bytes,
        };
        match fetch_one_classified(agent, attempt, maximum_redirects, allow_loopback_plain_http) {
            Ok(asset) => return Ok((asset, retries)),
            Err(FetchFailure::Permanent(error)) => return Err(error),
            Err(FetchFailure::Transient(error)) => {
                if retries >= policy.attempts {
                    return Err(ProductError::environment(
                        "ACQUISITION_RETRIES_EXHAUSTED",
                        format!(
                            "the transfer still failed after {} retries; last failure {}: {}",
                            policy.attempts, error.code, error.message
                        ),
                    ));
                }
                std::thread::sleep(policy.delay(retries as u32));
                retries += 1;
            }
        }
    }
}

/// Stream one asset, following redirects manually so every hop is re-validated
/// against the HTTPS rule rather than trusted to the client's own policy.
///
/// Shared with the Stack v2 adapter so that resolving a Software Heritage blob
/// goes through exactly the transport rules the contract sets for any other
/// acquisition, rather than a second implementation of them.
pub(crate) fn fetch_one(
    agent: &ureq::Agent,
    request: FetchRequest<'_>,
    maximum_redirects: u64,
    allow_loopback_plain_http: bool,
) -> Result<FetchedAsset> {
    fetch_one_classified(agent, request, maximum_redirects, allow_loopback_plain_http)
        .map_err(FetchFailure::into_error)
}

fn fetch_one_classified(
    agent: &ureq::Agent,
    request: FetchRequest<'_>,
    maximum_redirects: u64,
    allow_loopback_plain_http: bool,
) -> std::result::Result<FetchedAsset, FetchFailure> {
    let asset = request;
    let credential = match &asset.credential_env {
        Some(variable) => {
            let token = std::env::var(variable).map_err(|_| {
                // Name the variable, never its value.
                FetchFailure::Permanent(ProductError::environment(
                    "ACQUISITION_CREDENTIAL_MISSING",
                    format!("the environment variable {variable} is not set"),
                ))
            })?;
            if token.trim().is_empty() {
                return Err(FetchFailure::Permanent(ProductError::environment(
                    "ACQUISITION_CREDENTIAL_MISSING",
                    format!("the environment variable {variable} is empty"),
                )));
            }
            Some(token)
        }
        None => None,
    };

    let mut url = asset.url.to_owned();
    let mut redirects_followed = 0_u64;
    let response = loop {
        let mut request = agent.get(&url);
        if let Some(token) = &credential {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(status, _)) => {
                let error = ProductError::environment(
                    "ACQUISITION_HTTP_STATUS",
                    format!("the origin answered status {status}"),
                );
                // Too Many Requests and the server-error class are the origin
                // saying "not now"; every other status is it saying "not this",
                // and repeating that only hides a configuration error in a delay.
                return Err(if status == 429 || (500..600).contains(&status) {
                    FetchFailure::Transient(error)
                } else {
                    FetchFailure::Permanent(error)
                });
            }
            Err(ureq::Error::Transport(transport)) => {
                return Err(FetchFailure::Transient(ProductError::environment(
                    "ACQUISITION_TRANSPORT_FAILED",
                    format!("the transfer failed: {transport}"),
                )));
            }
        };
        let status = response.status();
        if !(300..400).contains(&status) {
            break response;
        }
        let location = response.header("location").ok_or_else(|| {
            FetchFailure::Permanent(ProductError::integrity(
                "ACQUISITION_REDIRECT_INVALID",
                "the origin returned a redirect without a location",
            ))
        })?;
        if redirects_followed >= maximum_redirects {
            return Err(FetchFailure::Permanent(ProductError::gate(
                "ACQUISITION_REDIRECT_LIMIT_EXCEEDED",
                "the origin exceeded the configured redirect bound",
            )));
        }
        // A redirect to plain HTTP is exactly what the HTTPS rule exists to stop,
        // so every hop is re-validated here rather than delegated to the client's
        // own policy.
        require_acquisition_url(location, allow_loopback_plain_http)
            .map_err(FetchFailure::Permanent)?;
        url = location.to_owned();
        redirects_followed += 1;
    };

    // Read against a hard ceiling one byte above what the caller will accept, so
    // an over-long body is detected rather than absorbed.
    let mut reader = response
        .into_reader()
        .take(asset.ceiling_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    let mut chunk = vec![0_u8; STREAM_CHUNK_BYTES];
    loop {
        let read = reader.read(&mut chunk).map_err(|error| {
            FetchFailure::Transient(ProductError::environment(
                "ACQUISITION_TRANSPORT_FAILED",
                format!("the transfer failed while streaming: {error}"),
            ))
        })?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() as u64 > asset.ceiling_bytes {
            return Err(FetchFailure::Permanent(ProductError::integrity(
                "ACQUISITION_LENGTH_MISMATCH",
                "the origin returned more bytes than the caller will accept",
            )));
        }
    }
    let sha256 = hex::encode(Sha256::digest(&bytes));
    Ok(FetchedAsset {
        bytes,
        sha256,
        redirects_followed,
    })
}

/// A create-new directory built beside its destination and renamed into place, so
/// a failed or interrupted acquisition never leaves a half-populated generation
/// at the published path.
pub(crate) struct PartialTree {
    partial_path: PathBuf,
    final_path: PathBuf,
    published: bool,
}

impl PartialTree {
    pub(crate) fn create(final_path: &Path) -> Result<Self> {
        let parent = final_path.parent().ok_or_else(|| {
            ProductError::usage(
                "ACQUISITION_OUTPUT_PARENT_INVALID",
                "the acquisition output root has no parent directory",
            )
        })?;
        let leaf = final_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                ProductError::usage(
                    "ACQUISITION_OUTPUT_PARENT_INVALID",
                    "the acquisition output root has no readable name",
                )
            })?;
        let partial_path = parent.join(format!(".{leaf}.acquire-partial-{}", std::process::id()));
        if partial_path.exists() {
            fs::remove_dir_all(&partial_path).map_err(|_| {
                ProductError::environment(
                    "ACQUISITION_OUTPUT_PARTIAL_FAILED",
                    "could not clear a stale acquisition partial directory",
                )
            })?;
        }
        fs::create_dir_all(&partial_path).map_err(|_| {
            ProductError::environment(
                "ACQUISITION_OUTPUT_PARTIAL_FAILED",
                "could not create the acquisition partial directory",
            )
        })?;
        Ok(Self {
            partial_path,
            final_path: final_path.to_path_buf(),
            published: false,
        })
    }

    pub(crate) fn write(&self, relative: &str, bytes: &[u8]) -> Result<()> {
        let destination = join_relative(&self.partial_path, relative)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|_| {
                ProductError::environment(
                    "ACQUISITION_WRITE_FAILED",
                    "could not create an acquisition destination directory",
                )
            })?;
        }
        fs::write(&destination, bytes).map_err(|_| {
            ProductError::environment(
                "ACQUISITION_WRITE_FAILED",
                "could not write an acquired asset",
            )
        })
    }

    pub(crate) fn publish(&mut self) -> Result<()> {
        crate::platform::publish_create_new(&self.partial_path, &self.final_path)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for PartialTree {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.partial_path);
        }
    }
}

pub fn acquire(config_path: &Path) -> Result<serde_json::Value> {
    crate::platform::require_portable_data_host()?;
    let config_bytes = crate::data::source::read_control_file(
        config_path,
        None,
        "ACQUISITION_CONFIG_READ_FAILED",
    )?;
    let config: AcquisitionConfigV1 = parse_closed(&config_bytes, "ACQUISITION_CONFIG_INVALID")?;
    config.validate()?;
    require_output_boundary(&config.output_root, &config.output_root)?;

    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(config.limits.connect_timeout_seconds))
        .timeout_read(Duration::from_secs(config.limits.read_timeout_seconds))
        .build();

    let mut generation = PartialTree::create(&config.output_root)?;
    let mut acquired = Vec::with_capacity(config.assets.len());
    let mut acquired_bytes = 0_u64;
    for asset in &config.assets {
        let fetched = fetch_one(
            &agent,
            FetchRequest {
                url: &asset.url,
                credential_env: asset.credential_env.as_deref(),
                ceiling_bytes: asset.expected_bytes,
            },
            config.limits.maximum_redirects,
            config.allow_loopback_plain_http,
        )?;
        // Verification happens here rather than inside the transfer, because
        // discovery performs the same transfer with nothing to verify against.
        if fetched.bytes.len() as u64 != asset.expected_bytes {
            return Err(ProductError::integrity(
                "ACQUISITION_LENGTH_MISMATCH",
                "the origin returned fewer bytes than the asset declares",
            ));
        }
        if fetched.sha256 != asset.expected_sha256 {
            return Err(ProductError::integrity(
                "ACQUISITION_DIGEST_MISMATCH",
                "the acquired bytes do not match the declared digest",
            ));
        }
        generation.write(&asset.relative_path, &fetched.bytes)?;
        acquired_bytes = acquired_bytes
            .checked_add(fetched.bytes.len() as u64)
            .ok_or_else(|| {
                ProductError::integrity(
                    "ACQUISITION_ACCOUNTING_OVERFLOW",
                    "acquired byte accounting overflowed",
                )
            })?;
        acquired.push(AcquiredAssetV1 {
            role: asset.role.clone(),
            relative_path: asset.relative_path.clone(),
            sha256: asset.expected_sha256.clone(),
            bytes: fetched.bytes.len() as u64,
            redirects_followed: fetched.redirects_followed,
            credential_supplied: asset.credential_env.is_some(),
        });
    }

    let manifest = AcquisitionManifestV1 {
        schema: ACQUISITION_MANIFEST_SCHEMA.to_owned(),
        profile: config.profile.clone(),
        qualification_status: "SKIPPED".to_owned(),
        transport: transport_label(config.allow_loopback_plain_http).to_owned(),
        assets: acquired,
    };
    let manifest_bytes = compact_json_line(&manifest, "ACQUISITION_MANIFEST_SERIALIZE_FAILED")?;
    generation.write("manifest.json", &manifest_bytes)?;
    generation.publish()?;

    let result = AcquisitionResultV1 {
        schema: ACQUISITION_RESULT_SCHEMA.to_owned(),
        status: "ASSETS_ACQUIRED".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        profile: config.profile.clone(),
        transport: transport_label(config.allow_loopback_plain_http).to_owned(),
        configuration_sha256: config.sha256()?,
        manifest_sha256: sha256(&manifest_bytes),
        acquired_assets: manifest.assets.len() as u64,
        acquired_bytes,
        output_created: true,
        receipts_written: false,
        limitations: vec![
            "acquisition-is-not-license-review".to_owned(),
            "digests-are-declared-by-the-operator-not-independently-witnessed".to_owned(),
        ],
    };
    serde_json::to_value(result).map_err(|_| {
        ProductError::internal(
            "ACQUISITION_RESULT_SERIALIZE_FAILED",
            "could not serialize the closed acquisition result",
        )
    })
}

/// Report what an origin currently serves, without publishing anything.
///
/// Pinning needs a digest, but a digest can only be obtained from the bytes, so
/// the very first pin is unavoidably trust-on-first-use. This makes that step
/// explicit and visible rather than a quiet exception inside the normal path:
/// discovery writes **no artifact at all**, so nothing downstream can ever come
/// to depend on a byte that was never verified. The operator reads the reported
/// digest, pins it, and from then on the publishing path is strictly checked.
///
/// It has its own closed schema so a discovery configuration can never be run as
/// an acquisition, or the reverse.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryConfigV1 {
    pub schema: String,
    pub profile: String,
    pub allow_loopback_plain_http: bool,
    pub assets: Vec<DiscoveryAssetV1>,
    pub limits: DiscoveryLimits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryAssetV1 {
    pub role: String,
    pub url: String,
    pub credential_env: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryLimits {
    pub maximum_assets: u64,
    /// With nothing declared, this ceiling is the only bound on a transfer, so it
    /// is mandatory rather than advisory.
    pub maximum_asset_bytes: u64,
    pub maximum_redirects: u64,
    pub connect_timeout_seconds: u64,
    pub read_timeout_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAssetV1 {
    pub role: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    pub redirects_followed: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryResultV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub profile: String,
    pub configuration_sha256: String,
    pub assets: Vec<DiscoveredAssetV1>,
    pub artifacts_written: bool,
    pub receipts_written: bool,
    pub limitations: Vec<String>,
}

impl DiscoveryConfigV1 {
    pub fn validate(&self) -> Result<()> {
        if self.schema != DISCOVERY_CONFIG_SCHEMA {
            return Err(ProductError::usage(
                "ACQUISITION_CONFIG_SCHEMA_UNSUPPORTED",
                "the discovery configuration is not the closed discovery schema",
            ));
        }
        if self.profile != crate::backend::PROTOTYPE_PROFILE {
            return Err(ProductError::gate(
                "DEFERRED_POST_P16",
                "only the prototype profile is implemented",
            ));
        }
        if self.limits.maximum_assets == 0
            || self.limits.maximum_asset_bytes == 0
            || self.limits.connect_timeout_seconds == 0
            || self.limits.read_timeout_seconds == 0
        {
            return Err(ProductError::usage(
                "ACQUISITION_LIMIT_INVALID",
                "every discovery limit except the redirect bound must be nonzero",
            ));
        }
        if self.limits.maximum_asset_bytes > MAXIMUM_DECLARED_ASSET_BYTES {
            return Err(ProductError::usage(
                "ACQUISITION_LENGTH_INVALID",
                "the discovery ceiling exceeds the implausible-transfer bound",
            ));
        }
        if self.assets.is_empty() || self.assets.len() as u64 > self.limits.maximum_assets {
            return Err(ProductError::usage(
                "ACQUISITION_ASSET_COUNT_INVALID",
                "the discovery set is empty or exceeds its configured bound",
            ));
        }
        for asset in &self.assets {
            require_acquisition_url(&asset.url, self.allow_loopback_plain_http)?;
            require_bounded_text(&asset.role, "ACQUISITION_ROLE_INVALID")?;
            if let Some(variable) = &asset.credential_env {
                require_bounded_text(variable, "ACQUISITION_CREDENTIAL_ENV_INVALID")?;
                if variable.contains('=') {
                    return Err(ProductError::usage(
                        "ACQUISITION_CREDENTIAL_ENV_INVALID",
                        "a credential environment variable name may not contain '='",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String> {
        Ok(sha256(&compact_json_line(
            self,
            "ACQUISITION_CONFIG_SERIALIZE_FAILED",
        )?))
    }
}

pub fn discover(config_path: &Path) -> Result<serde_json::Value> {
    crate::platform::require_portable_data_host()?;
    let config_bytes = crate::data::source::read_control_file(
        config_path,
        None,
        "ACQUISITION_CONFIG_READ_FAILED",
    )?;
    let config: DiscoveryConfigV1 = parse_closed(&config_bytes, "ACQUISITION_CONFIG_INVALID")?;
    config.validate()?;

    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(config.limits.connect_timeout_seconds))
        .timeout_read(Duration::from_secs(config.limits.read_timeout_seconds))
        .build();

    let mut discovered = Vec::with_capacity(config.assets.len());
    for asset in &config.assets {
        let fetched = fetch_one(
            &agent,
            FetchRequest {
                url: &asset.url,
                credential_env: asset.credential_env.as_deref(),
                ceiling_bytes: config.limits.maximum_asset_bytes,
            },
            config.limits.maximum_redirects,
            config.allow_loopback_plain_http,
        )?;
        discovered.push(DiscoveredAssetV1 {
            role: asset.role.clone(),
            url: asset.url.clone(),
            sha256: fetched.sha256,
            bytes: fetched.bytes.len() as u64,
            redirects_followed: fetched.redirects_followed,
        });
    }

    let result = DiscoveryResultV1 {
        schema: DISCOVERY_RESULT_SCHEMA.to_owned(),
        status: "ASSETS_DISCOVERED".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        profile: config.profile.clone(),
        configuration_sha256: config.sha256()?,
        assets: discovered,
        artifacts_written: false,
        receipts_written: false,
        limitations: vec![
            "discovery-is-trust-on-first-use-and-verifies-nothing".to_owned(),
            "no-artifact-is-written-so-nothing-downstream-can-depend-on-this".to_owned(),
        ],
    };
    serde_json::to_value(result).map_err(|_| {
        ProductError::internal(
            "ACQUISITION_RESULT_SERIALIZE_FAILED",
            "could not serialize the closed discovery result",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_urls_are_accepted() {
        require_acquisition_url("https://example.invalid/a.gz", false).unwrap();
        for rejected in [
            "http://example.invalid/a.gz",
            "ftp://example.invalid/a.gz",
            "https://user:pass@example.invalid/a.gz",
            "https:///a.gz",
            "https://example.invalid/a b.gz",
            "https://example.invalid/a.gz#frag",
            "https://example.invalid\\a.gz",
        ] {
            assert_eq!(
                require_acquisition_url(rejected, false).unwrap_err().code,
                "ACQUISITION_URL_INVALID",
                "expected {rejected} to be rejected"
            );
        }
    }

    /// The fixture exemption must not become a general plain-HTTP bypass.
    #[test]
    fn the_loopback_exemption_is_narrow() {
        require_acquisition_url("http://127.0.0.1:8080/a.gz", true).unwrap();
        require_acquisition_url("http://[::1]:8080/a.gz", true).unwrap();
        require_acquisition_url("https://example.invalid/a.gz", true).unwrap();

        for rejected in [
            // A name is a resolver lookup and can be pointed anywhere.
            "http://localhost:8080/a.gz",
            "http://example.invalid/a.gz",
            "http://127.0.0.1.example.invalid/a.gz",
            "http://10.0.0.1/a.gz",
            "http://user@127.0.0.1/a.gz",
        ] {
            assert_eq!(
                require_acquisition_url(rejected, true).unwrap_err().code,
                "ACQUISITION_URL_INVALID",
                "expected {rejected} to be rejected even in fixture mode"
            );
        }

        // And with the exemption off, loopback is no more permitted than anything else.
        assert_eq!(
            require_acquisition_url("http://127.0.0.1:8080/a.gz", false)
                .unwrap_err()
                .code,
            "ACQUISITION_URL_INVALID"
        );
    }

    fn config() -> AcquisitionConfigV1 {
        AcquisitionConfigV1 {
            schema: ACQUISITION_CONFIG_SCHEMA.to_owned(),
            profile: crate::backend::PROTOTYPE_PROFILE.to_owned(),
            output_root: if cfg!(windows) {
                PathBuf::from("C:\\python-slm\\acquired")
            } else {
                PathBuf::from("/python-slm/acquired")
            },
            allow_loopback_plain_http: false,
            assets: vec![AcquisitionAssetV1 {
                role: "evalplus-humanevalplus".to_owned(),
                url: "https://example.invalid/HumanEvalPlus.jsonl.gz".to_owned(),
                relative_path: "assets/HumanEvalPlus.jsonl.gz".to_owned(),
                expected_sha256: "ab".repeat(32),
                expected_bytes: 1_024,
                credential_env: None,
            }],
            limits: AcquisitionLimits {
                maximum_assets: 8,
                maximum_total_bytes: 1_000_000,
                maximum_redirects: 4,
                connect_timeout_seconds: 30,
                read_timeout_seconds: 300,
            },
        }
    }

    #[test]
    fn the_configuration_is_closed_and_bounded() {
        config().validate().unwrap();

        let mut plain_http = config();
        plain_http.assets[0].url = "http://example.invalid/a.gz".to_owned();
        assert_eq!(
            plain_http.validate().unwrap_err().code,
            "ACQUISITION_URL_INVALID"
        );

        let mut bad_digest = config();
        bad_digest.assets[0].expected_sha256 = "NOTAHASH".to_owned();
        assert_eq!(
            bad_digest.validate().unwrap_err().code,
            "ACQUISITION_DIGEST_INVALID"
        );

        let mut zero_length = config();
        zero_length.assets[0].expected_bytes = 0;
        assert_eq!(
            zero_length.validate().unwrap_err().code,
            "ACQUISITION_LENGTH_INVALID"
        );

        // An escaping destination must never be reachable from a manifest.
        let mut escaping = config();
        escaping.assets[0].relative_path = "../outside.gz".to_owned();
        assert_eq!(
            escaping.validate().unwrap_err().code,
            "ACQUISITION_PATH_INVALID"
        );

        let mut over_total = config();
        over_total.limits.maximum_total_bytes = 512;
        assert_eq!(
            over_total.validate().unwrap_err().code,
            "ACQUISITION_TOTAL_LIMIT_EXCEEDED"
        );

        let mut duplicated = config();
        let first = duplicated.assets[0].clone();
        duplicated.assets.push(first);
        assert_eq!(
            duplicated.validate().unwrap_err().code,
            "ACQUISITION_PATH_DUPLICATE"
        );

        // Unknown fields are rejected rather than ignored.
        let json = serde_json::to_string(&config()).unwrap();
        let widened = json.replace('{', "{\"extra\":1,");
        assert_eq!(
            parse_closed::<AcquisitionConfigV1>(widened.as_bytes(), "ACQUISITION_CONFIG_INVALID")
                .unwrap_err()
                .code,
            "ACQUISITION_CONFIG_INVALID"
        );
    }

    /// A credential is named, never carried. The configuration holds a variable
    /// name and the result records only whether one was supplied.
    #[test]
    fn credentials_are_named_not_embedded() {
        let mut named = config();
        named.assets[0].credential_env = Some("PYTHON_SLM_TEST_TOKEN".to_owned());
        named.validate().unwrap();

        let serialized = serde_json::to_string(&named).unwrap();
        assert!(serialized.contains("PYTHON_SLM_TEST_TOKEN"));
        assert!(!serialized.to_lowercase().contains("bearer"));

        let mut malformed = config();
        malformed.assets[0].credential_env = Some("BAD=NAME".to_owned());
        assert_eq!(
            malformed.validate().unwrap_err().code,
            "ACQUISITION_CREDENTIAL_ENV_INVALID"
        );
    }

    /// The schedule is a pure function of the attempt index, with no clock and no
    /// randomness, so two runs of one configuration wait the same way. It doubles
    /// until it reaches the ceiling and then stays there rather than overflowing.
    #[test]
    fn the_backoff_schedule_doubles_and_then_holds_at_its_ceiling() {
        let policy = RetryPolicy {
            attempts: 8,
            initial_delay: Duration::from_millis(100),
            maximum_delay: Duration::from_millis(800),
        };
        let observed = (0..6).map(|index| policy.delay(index)).collect::<Vec<_>>();
        assert_eq!(
            observed,
            vec![
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::from_millis(400),
                Duration::from_millis(800),
                Duration::from_millis(800),
                Duration::from_millis(800),
            ]
        );
        // A far-out index must clamp rather than overflow the shift or the
        // multiplication.
        assert_eq!(policy.delay(64), Duration::from_millis(800));
        assert_eq!(policy.delay(u32::MAX), Duration::from_millis(800));
    }
}
