//! Non-secret configuration for S3 blobs and optional Celld metadata.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

pub const REQUEST_TIMEOUT_SECONDS: u64 = 30;
pub const MAX_METADATA_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_BLOB_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_CAS_ATTEMPTS: usize = 8;
pub const POLL_INTERVAL_SECONDS: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub prefix: String,
    pub access_key_env: String,
    pub secret_key_env: String,
    pub session_token_env: Option<String>,
    pub allow_http: bool,
    /// Without this endpoint, metadata uses conditional S3 object writes.
    pub celld_endpoint: Option<String>,
    pub celld_token_env: String,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            bucket: String::new(),
            region: "us-east-1".into(),
            prefix: "drift/v1".into(),
            access_key_env: "DRIFT_S3_ACCESS_KEY_ID".into(),
            secret_key_env: "DRIFT_S3_SECRET_ACCESS_KEY".into(),
            session_token_env: None,
            allow_http: false,
            celld_endpoint: None,
            celld_token_env: "DRIFT_CELLD_TOKEN".into(),
        }
    }
}

pub fn safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

impl S3Config {
    pub fn validate(&self, user: &str) -> Result<()> {
        ensure!(
            safe_segment(user),
            "S3 sync requires a safe, explicit storage.user_id"
        );
        ensure!(
            safe_segment(&self.bucket),
            "S3 bucket is missing or invalid"
        );
        ensure!(!self.region.trim().is_empty(), "S3 region is missing");
        ensure!(
            self.prefix.split('/').all(safe_segment),
            "S3 prefix contains an invalid segment"
        );
        validate_endpoint(&self.endpoint, self.allow_http)?;
        if let Some(endpoint) = &self.celld_endpoint {
            validate_endpoint(endpoint, self.allow_http)?;
        }
        for name in [
            &self.access_key_env,
            &self.secret_key_env,
            &self.celld_token_env,
        ] {
            ensure!(
                !name.is_empty(),
                "credential environment variable name is empty"
            );
        }
        Ok(())
    }
}

fn validate_endpoint(endpoint: &str, allow_http: bool) -> Result<()> {
    let url = reqwest::Url::parse(endpoint).context("invalid storage endpoint URL")?;
    if url.scheme() != "https" && !(url.scheme() == "http" && allow_http) {
        bail!("storage endpoint requires HTTPS unless allow_http is explicit");
    }
    ensure!(url.host_str().is_some(), "storage endpoint requires a host");
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "credentials must not appear in endpoint URLs"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none() && url.path() == "/",
        "storage endpoint must be an origin URL"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> S3Config {
        S3Config {
            endpoint: "https://storage.example".into(),
            bucket: "drift".into(),
            ..S3Config::default()
        }
    }

    #[test]
    fn legacy_sync_fails_with_an_explicit_migration_error() {
        let mut storage = crate::config::StorageConfig::default();
        assert!(storage.validate_sync().is_ok());
        storage.backend = "aspen".into();
        assert!(storage.validate_sync().is_err());
        storage.backend = "s3".into();
        assert!(storage.validate_sync().is_err());
        storage.user_id = Some("alice".into());
        storage.device_id = Some("phone".into());
        storage.s3 = Some(config());
        assert!(storage.validate_sync().is_ok());
        storage.cluster_ticket = Some("legacy-ticket".into());
        assert!(storage.validate_sync().is_err());
    }

    #[test]
    fn accepts_explicit_https_and_opt_in_http() {
        assert!(config().validate("alice").is_ok());
        let mut value = config();
        value.endpoint = "http://localhost".into();
        assert!(value.validate("alice").is_err());
        value.allow_http = true;
        assert!(value.validate("alice").is_ok());
    }

    #[test]
    fn rejects_credentials_paths_and_namespace_escape() {
        for endpoint in [
            "https://user:secret@example.com",
            "https://example.com/path",
            "https://example.com?q=secret",
            "file:///tmp/store",
        ] {
            let mut value = config();
            value.endpoint = endpoint.into();
            assert!(value.validate("alice").is_err());
        }
        for user in ["", "..", "alice/bob", "alice:admin"] {
            assert!(config().validate(user).is_err());
        }
        let mut value = config();
        value.prefix = "drift/../other".into();
        assert!(value.validate("alice").is_err());
    }
}
