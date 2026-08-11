use anyhow::{Context, Result, bail, ensure};
use futures_util::{StreamExt, TryStreamExt, stream};
use reqwest::{Client, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteManifest {
    pub format_version: u32,
    /// Name of an environment variable containing a bearer token. The token
    /// itself is intentionally never accepted in this file.
    pub bearer_token_env: Option<String>,
    pub shards: Vec<RemoteShard>,
}

impl Default for RemoteManifest {
    fn default() -> Self {
        Self {
            format_version: 1,
            bearer_token_env: None,
            shards: Vec::new(),
        }
    }
}

impl RemoteManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.format_version == 1,
            "unsupported remote manifest version"
        );
        ensure!(
            !self.shards.is_empty(),
            "remote manifest contains no shards"
        );
        for (index, shard) in self.shards.iter().enumerate() {
            Url::parse(&shard.url)
                .with_context(|| format!("invalid shard URL at index {index}"))?;
            ensure!(
                !shard.content_column.is_empty(),
                "empty content column at index {index}"
            );
            if let Some(hash) = &shard.sha256 {
                ensure!(
                    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "sha256 at index {index} must contain 64 hexadecimal characters"
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteShard {
    pub url: String,
    pub sha256: Option<String>,
    pub content_column: String,
    pub id_column: Option<String>,
}

impl Default for RemoteShard {
    fn default() -> Self {
        Self {
            url: String::new(),
            sha256: None,
            content_column: "content".into(),
            id_column: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadConfig {
    pub concurrency: usize,
    pub maximum_shard_bytes: u64,
    pub require_sha256: bool,
    pub allow_plain_http: bool,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            maximum_shard_bytes: 8 * 1024 * 1024 * 1024,
            require_sha256: true,
            allow_plain_http: false,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DownloadedShard {
    pub manifest_index: usize,
    pub path: PathBuf,
}

pub(crate) async fn download_manifest(
    manifest: &RemoteManifest,
    work_dir: &Path,
    config: &DownloadConfig,
) -> Result<Vec<DownloadedShard>> {
    ensure!(
        config.concurrency > 0,
        "download concurrency must be positive"
    );
    ensure!(
        config.maximum_shard_bytes > 0,
        "maximum shard size must be positive"
    );
    if config.require_sha256 {
        ensure!(
            manifest.shards.iter().all(|shard| shard.sha256.is_some()),
            "every shard must declare sha256 when require_sha256=true"
        );
    }
    let download_dir = work_dir.join("downloads");
    tokio::fs::create_dir_all(&download_dir)
        .await
        .with_context(|| format!("creating {}", download_dir.display()))?;

    let bearer = match &manifest.bearer_token_env {
        Some(name) => Some(
            env::var(name)
                .with_context(|| format!("environment variable {name} is required for access"))?,
        ),
        None => None,
    };
    ensure!(
        bearer.is_none() || !config.allow_plain_http,
        "bearer authentication requires strict HTTPS; disable allow_plain_http"
    );
    let client = Client::builder()
        .user_agent(concat!("rust-llm-pretrain/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(120))
        .https_only(!config.allow_plain_http)
        .redirect(Policy::limited(5))
        .build()?;

    let jobs = manifest
        .shards
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, shard)| {
            let client = client.clone();
            let bearer = bearer.clone();
            let download_dir = download_dir.clone();
            let config = config.clone();
            async move {
                download_one(
                    index,
                    shard,
                    client,
                    bearer.as_deref(),
                    &download_dir,
                    &config,
                )
                .await
            }
        });
    let mut downloaded: Vec<_> = stream::iter(jobs)
        .buffer_unordered(config.concurrency)
        .try_collect()
        .await?;
    downloaded.sort_by_key(|entry| entry.manifest_index);
    Ok(downloaded)
}

async fn download_one(
    manifest_index: usize,
    shard: RemoteShard,
    client: Client,
    bearer: Option<&str>,
    directory: &Path,
    config: &DownloadConfig,
) -> Result<DownloadedShard> {
    let url = Url::parse(&shard.url)?;
    ensure!(
        url.host_str().is_some(),
        "shard URL must contain a network host"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "credentials must not be embedded in a shard URL"
    );
    if url.scheme() != "https" && !(config.allow_plain_http && url.scheme() == "http") {
        bail!("refusing non-HTTPS shard URL: {url}");
    }
    let url_digest = Sha256::digest(url.as_str().as_bytes());
    let name = format!(
        "{}-{}.parquet",
        manifest_index,
        hex::encode(&url_digest[..12])
    );
    let final_path = directory.join(name);
    if final_path.is_file() {
        if let Some(expected) = &shard.sha256 {
            let actual = hash_file(&final_path).await?;
            ensure!(
                actual.eq_ignore_ascii_case(expected),
                "cached shard checksum mismatch"
            );
        } else {
            tracing::warn!(path = %final_path.display(), "using cached shard without a declared checksum");
        }
        return Ok(DownloadedShard {
            manifest_index,
            path: final_path,
        });
    }

    let part_path = final_path.with_extension("parquet.part");
    let mut request = client.get(url.clone());
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let response = request.send().await?.error_for_status()?;
    let advertised_length = response.content_length();
    if let Some(length) = advertised_length {
        ensure!(
            length <= config.maximum_shard_bytes,
            "shard advertises {length} bytes, above the configured limit"
        );
    }
    let mut file = tokio::fs::File::create(&part_path)
        .await
        .with_context(|| format!("creating {}", part_path.display()))?;
    let mut hasher = Sha256::new();
    let mut received = 0_u64;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        received = received
            .checked_add(chunk.len() as u64)
            .context("shard byte count overflow")?;
        ensure!(
            received <= config.maximum_shard_bytes,
            "shard exceeded configured maximum while streaming"
        );
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    ensure!(received > 0, "shard response was empty");
    if let Some(expected) = advertised_length {
        ensure!(
            received == expected,
            "shard response length mismatch: advertised {expected}, received {received}"
        );
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);

    let actual = hex::encode(hasher.finalize());
    if let Some(expected) = &shard.sha256 {
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = tokio::fs::remove_file(&part_path).await;
            bail!(
                "checksum mismatch for shard {manifest_index}: expected {expected}, got {actual}"
            );
        }
    } else {
        tracing::warn!(shard = manifest_index, sha256 = %actual, "manifest omitted a reproducibility checksum");
    }
    tokio::fs::rename(&part_path, &final_path)
        .await
        .with_context(|| format!("finalizing {}", final_path.display()))?;
    Ok(DownloadedShard {
        manifest_index,
        path: final_path,
    })
}

async fn hash_file(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}
