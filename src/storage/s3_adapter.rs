//! Vendor adapters. No SDK type crosses the application ports.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use object_store::aws::{AmazonS3Builder, S3ConditionalPut};
use object_store::path::Path;
use object_store::{ClientOptions, ObjectStore, ObjectStoreExt, PutMode, PutOptions, RetryConfig, UpdateVersion};
use reqwest::{header, StatusCode};

use super::object_port::{BlobPort, MetadataPort, MetadataRead};
use super::settings::{S3Config, MAX_BLOB_BYTES, MAX_METADATA_BYTES, REQUEST_TIMEOUT_SECONDS};

type Ports = (Arc<dyn MetadataPort>, Arc<dyn BlobPort>);

pub fn connect(config: &S3Config, user: &str) -> Result<Ports> {
    let timeout = Duration::from_secs(REQUEST_TIMEOUT_SECONDS);
    let mut builder = AmazonS3Builder::new()
        .with_endpoint(&config.endpoint)
        .with_region(&config.region)
        .with_bucket_name(&config.bucket)
        .with_virtual_hosted_style_request(false)
        .with_access_key_id(secret(&config.access_key_env)?)
        .with_secret_access_key(secret(&config.secret_key_env)?)
        .with_allow_http(config.allow_http)
        .with_client_options(
            ClientOptions::new()
                .with_allow_http(config.allow_http)
                .with_timeout(timeout),
        )
        .with_conditional_put(S3ConditionalPut::ETagMatch)
        // The application reads current state before every retry.
        .with_retry(RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        });
    if let Some(name) = &config.session_token_env {
        builder = builder.with_token(secret(name)?);
    }
    let store = Arc::new(builder.build().context("cannot configure S3 client")?);
    let adapter = Arc::new(S3Adapter {
        store,
        prefix: format!("{}/users/{user}", config.prefix),
    });
    let metadata: Arc<dyn MetadataPort> = match &config.celld_endpoint {
        Some(endpoint) => Arc::new(CelldAdapter {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            url: format!("{}/state", endpoint.trim_end_matches('/')),
            user: user.into(),
            token: secret(&config.celld_token_env)?,
        }),
        None => adapter.clone(),
    };
    Ok((metadata, adapter))
}

fn secret(name: &str) -> Result<String> {
    let value = std::env::var(name)
        .with_context(|| format!("required credential environment variable {name} is absent"))?;
    ensure!(
        !value.trim().is_empty(),
        "credential environment variable {name} is empty"
    );
    Ok(value)
}

struct S3Adapter {
    store: Arc<dyn ObjectStore>,
    prefix: String,
}

impl S3Adapter {
    fn state_path(&self) -> Path {
        Path::from(format!("{}/state.json", self.prefix))
    }

    fn blob_path(&self, hash: &str) -> Result<Path> {
        blake3::Hash::from_hex(hash).context("invalid blob identity")?;
        Ok(Path::from(format!("{}/blobs/{hash}", self.prefix)))
    }
}

async fn bounded_object(result: object_store::GetResult, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        result.meta.size <= limit as u64,
        "object exceeds its byte limit"
    );
    let mut stream = result.into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.try_next().await? {
        ensure!(
            chunk.len() <= limit.saturating_sub(bytes.len()),
            "object exceeded its byte limit during download"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[async_trait]
impl MetadataPort for S3Adapter {
    async fn load(&self) -> Result<MetadataRead> {
        match self.store.get(&self.state_path()).await {
            Ok(result) => {
                let revision = result
                    .meta
                    .e_tag
                    .clone()
                    .context("S3 metadata response lacks an ETag")?;
                let bytes = bounded_object(result, MAX_METADATA_BYTES).await?;
                Ok(MetadataRead {
                    bytes: Some(bytes),
                    revision: Some(revision),
                })
            }
            Err(object_store::Error::NotFound { .. }) => Ok(MetadataRead {
                bytes: None,
                revision: None,
            }),
            Err(error) => Err(error).context("S3 metadata read failed"),
        }
    }

    async fn compare_and_swap(&self, expected: Option<&str>, bytes: Vec<u8>) -> Result<bool> {
        ensure!(
            bytes.len() <= MAX_METADATA_BYTES,
            "metadata exceeds its byte limit"
        );
        let mode = match expected {
            None => PutMode::Create,
            Some(revision) => PutMode::Update(UpdateVersion {
                e_tag: Some(revision.into()),
                version: None,
            }),
        };
        match self
            .store
            .put_opts(
                &self.state_path(),
                bytes.into(),
                PutOptions {
                    mode,
                    ..PutOptions::default()
                },
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(
                object_store::Error::Precondition { .. }
                | object_store::Error::AlreadyExists { .. },
            ) => Ok(false),
            Err(error) => Err(error).context("S3 metadata outcome is not confirmed"),
        }
    }
}

#[async_trait]
impl BlobPort for S3Adapter {
    async fn put(&self, hash: &str, bytes: Vec<u8>) -> Result<()> {
        ensure!(bytes.len() <= MAX_BLOB_BYTES, "blob exceeds its byte limit");
        ensure!(
            blake3::hash(&bytes).to_hex().as_str() == hash,
            "blob bytes do not match their identity"
        );
        self.store
            .put(&self.blob_path(hash)?, bytes.into())
            .await
            .context("S3 blob write is not confirmed")?;
        Ok(())
    }

    async fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        match self.store.get(&self.blob_path(hash)?).await {
            Ok(result) => Ok(Some(bounded_object(result, MAX_BLOB_BYTES).await?)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(error).context("S3 blob read failed"),
        }
    }
}

struct CelldAdapter {
    client: reqwest::Client,
    url: String,
    user: String,
    token: String,
}

impl CelldAdapter {
    fn request(&self, method: reqwest::Method) -> reqwest::RequestBuilder {
        self.client
            .request(method, &self.url)
            .bearer_auth(&self.token)
            .header("x-drift-user", &self.user)
    }
}

#[async_trait]
impl MetadataPort for CelldAdapter {
    async fn load(&self) -> Result<MetadataRead> {
        let response = self
            .request(reqwest::Method::GET)
            .send()
            .await
            .context("Celld read failed")?;
        // Only the worker's typed absence response means an empty account.
        if response.status() == StatusCode::NOT_FOUND {
            ensure!(
                response
                    .headers()
                    .get("x-drift-state")
                    .is_some_and(|value| value == "absent"),
                "unrecognized Celld absence response"
            );
            return Ok(MetadataRead {
                bytes: None,
                revision: None,
            });
        }
        ensure!(
            response.status() == StatusCode::OK,
            "Celld read rejected with status {}",
            response.status()
        );
        let revision = response
            .headers()
            .get(header::ETAG)
            .context("Celld response lacks an ETag")?
            .to_str()?
            .to_owned();
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.try_next().await? {
            ensure!(
                chunk.len() <= MAX_METADATA_BYTES.saturating_sub(bytes.len()),
                "Celld metadata exceeds its byte limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(MetadataRead {
            bytes: Some(bytes),
            revision: Some(revision),
        })
    }

    async fn compare_and_swap(&self, expected: Option<&str>, bytes: Vec<u8>) -> Result<bool> {
        ensure!(
            bytes.len() <= MAX_METADATA_BYTES,
            "metadata exceeds its byte limit"
        );
        let request = self
            .request(reqwest::Method::PUT)
            .header(header::CONTENT_TYPE, "application/json");
        let request = match expected {
            Some(revision) => request.header(header::IF_MATCH, revision),
            None => request.header(header::IF_NONE_MATCH, "*"),
        };
        let response = request
            .body(bytes)
            .send()
            .await
            .context("Celld write outcome is not confirmed")?;
        match response.status() {
            StatusCode::NO_CONTENT => {
                ensure!(
                    response
                        .headers()
                        .get("x-drift-state")
                        .is_some_and(|value| value == "committed"),
                    "Celld response lacks a commit acknowledgement"
                );
                Ok(true)
            }
            StatusCode::PRECONDITION_FAILED => Ok(false),
            status => anyhow::bail!("Celld write rejected with status {status}"),
        }
    }
}

#[cfg(test)]
mod tests;
