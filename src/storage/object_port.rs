//! Application-owned capabilities for durable conditional metadata and blobs.

use anyhow::Result;
use async_trait::async_trait;

pub struct MetadataRead {
    pub bytes: Option<Vec<u8>>,
    pub revision: Option<String>,
}

#[async_trait]
pub trait MetadataPort: Send + Sync {
    async fn load(&self) -> Result<MetadataRead>;
    /// False means a definite revision conflict. Errors retain uncertainty.
    async fn compare_and_swap(&self, expected: Option<&str>, bytes: Vec<u8>) -> Result<bool>;
}

#[async_trait]
pub trait BlobPort: Send + Sync {
    /// The key is a BLAKE3 content identity. The caller verifies all reads.
    async fn put(&self, hash: &str, bytes: Vec<u8>) -> Result<()>;
    async fn get(&self, hash: &str) -> Result<Option<Vec<u8>>>;
}
