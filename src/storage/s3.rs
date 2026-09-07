//! Replication shell over S3 blobs and a conditional metadata port.

use std::sync::Arc;

use anyhow::{bail, ensure, Context, Result};
use tokio::io::AsyncReadExt;

use super::object_port::{BlobPort, MetadataPort};
use super::replication::{prepare, BlobIndex, Snapshot, BLOB_PREFIX};
use super::settings::{S3Config, MAX_BLOB_BYTES, MAX_CAS_ATTEMPTS};
use super::wal::{ReplicationOp, WalEntry};
use super::BlobRef;

pub struct S3Storage {
    metadata: Arc<dyn MetadataPort>,
    blobs: Arc<dyn BlobPort>,
    user: String,
}

impl S3Storage {
    pub fn new(config: &S3Config, user: &str) -> Result<Self> {
        config.validate(user)?;
        let (metadata, blobs) = super::s3_adapter::connect(config, user)?;
        Ok(Self {
            metadata,
            blobs,
            user: user.into(),
        })
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        let current = self.metadata.load().await?;
        match current.bytes {
            Some(bytes) => Snapshot::decode(&bytes, &self.user),
            None => {
                ensure!(
                    current.revision.is_none(),
                    "missing snapshot has a revision"
                );
                Ok(Snapshot::empty(&self.user))
            }
        }
    }

    /// Publication is idempotent: stable operation stamps and exact blob bytes.
    /// An unknown outcome returns an error. The next attempt reads current state.
    pub async fn replicate(&self, device: &str, sequence: u64, entry: &WalEntry) -> Result<()> {
        let blob = match &entry.op {
            ReplicationOp::UploadBlob {
                file_path,
                expected_hash,
                ..
            } => {
                let expected = expected_hash
                    .as_deref()
                    .context("blob intent has no durable content identity")?;
                let bytes = read_blob(file_path).await?;
                let hash = blake3::hash(&bytes).to_hex().to_string();
                ensure!(hash == expected, "queued blob changed; WAL entry retained");
                let size = bytes.len() as u64;
                self.blobs.put(&hash, bytes).await?;
                let format = std::path::Path::new(file_path)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or("unknown")
                    .to_lowercase();
                Some(BlobIndex { hash, size, format })
            }
            _ => None,
        };
        let mutation = prepare(&self.user, device, sequence, entry, blob.as_ref())?;
        for _ in 0..MAX_CAS_ATTEMPTS {
            let current = self.metadata.load().await?;
            ensure!(
                current.bytes.is_some() == current.revision.is_some(),
                "metadata revision is missing or inconsistent"
            );
            let state = match &current.bytes {
                Some(bytes) => Snapshot::decode(bytes, &self.user)?,
                None => Snapshot::empty(&self.user),
            };
            let updated = state.apply(&mutation)?.encode()?;
            if current.bytes.as_ref() == Some(&updated) {
                // The current durable state already contains or supersedes this operation.
                return Ok(());
            }
            if self
                .metadata
                .compare_and_swap(current.revision.as_deref(), updated)
                .await?
            {
                return Ok(());
            }
        }
        bail!("metadata contention exceeded the retry limit; WAL entry retained")
    }

    pub async fn has_blob(&self, track_id: &str) -> Result<Option<BlobRef>> {
        let state = self.snapshot().await?;
        let index: Option<BlobIndex> = state.value(&format!("{BLOB_PREFIX}{track_id}"))?;
        index
            .map(|index| {
                validate_blob_index(&index)?;
                Ok(BlobRef {
                    hash: index.hash,
                    size: index.size,
                    format: index.format,
                })
            })
            .transpose()
    }

    pub async fn fetch_blob(&self, track_id: &str) -> Result<Option<Vec<u8>>> {
        let Some(index) = self.has_blob(track_id).await? else {
            return Ok(None);
        };
        let bytes = self
            .blobs
            .get(&index.hash)
            .await?
            .context("indexed S3 blob is missing")?;
        ensure!(bytes.len() as u64 == index.size, "S3 blob size mismatch");
        ensure!(
            blake3::hash(&bytes).to_hex().as_str() == index.hash,
            "S3 blob content identity mismatch"
        );
        Ok(Some(bytes))
    }
}

fn validate_blob_index(index: &BlobIndex) -> Result<()> {
    blake3::Hash::from_hex(&index.hash).context("invalid BLAKE3 blob identity")?;
    ensure!(
        index.size <= MAX_BLOB_BYTES as u64,
        "S3 blob exceeds the byte limit"
    );
    Ok(())
}

pub async fn bind_blob(entry: &mut WalEntry) -> Result<bool> {
    if let ReplicationOp::UploadBlob {
        file_path,
        expected_hash,
        ..
    } = &mut entry.op
    {
        if expected_hash.is_none() {
            *expected_hash = Some(
                blake3::hash(&read_blob(file_path).await?)
                    .to_hex()
                    .to_string(),
            );
            return Ok(true);
        }
    }
    Ok(false)
}

async fn read_blob(path: &str) -> Result<Vec<u8>> {
    let file = tokio::fs::File::open(path)
        .await
        .context("cannot open queued blob")?;
    let metadata = file.metadata().await?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_BLOB_BYTES as u64,
        "queued blob is not a bounded regular file"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BLOB_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(
        bytes.len() <= MAX_BLOB_BYTES,
        "queued blob grew beyond its byte limit"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests;
