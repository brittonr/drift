//! Deterministic replication documents. The shell owns files, clocks, and I/O.

use std::collections::BTreeMap;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::settings::MAX_METADATA_BYTES;
use super::wal::{ReplicationOp, WalEntry};
use crate::queue_persistence::PersistedQueue;
use crate::service::CoverArt;

pub const SCHEMA_VERSION: u32 = 1;
pub const HISTORY_PREFIX: &str = "history/";
pub const PLAYLIST_PREFIX: &str = "playlists/";
pub const BLOB_PREFIX: &str = "blobs/";
pub const QUEUE_KEY: &str = "queue";
const MAX_HISTORY: usize = drift_plugin::DEFAULT_MAX_HISTORY_ENTRIES;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub clock: u64,
    pub time_ms: u64,
    pub device: String,
    pub operation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub stamp: Stamp,
    /// None is a durable tombstone, not an absent document.
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub user: String,
    pub documents: BTreeMap<String, Document>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobIndex {
    pub hash: String,
    pub size: u64,
    pub format: String,
}

pub struct Mutation {
    pub key: String,
    pub document: Document,
}

impl Snapshot {
    pub fn empty(user: &str) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            user: user.into(),
            documents: BTreeMap::new(),
        }
    }

    pub fn decode(bytes: &[u8], user: &str) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_METADATA_BYTES,
            "metadata exceeds its byte limit"
        );
        let state: Self = serde_json::from_slice(bytes).context("invalid replication snapshot")?;
        ensure!(
            state.schema == SCHEMA_VERSION,
            "unsupported replication schema"
        );
        ensure!(state.user == user, "replication account mismatch");
        Ok(state)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() <= MAX_METADATA_BYTES,
            "metadata exceeds its byte limit; WAL entry retained"
        );
        Ok(bytes)
    }

    pub fn value<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        self.documents
            .get(key)
            .and_then(|document| document.value.clone())
            .map(serde_json::from_value)
            .transpose()
            .context("invalid replication document")
    }

    pub fn apply(&self, mutation: &Mutation) -> Result<Self> {
        let mut next = self.clone();
        if self
            .documents
            .get(&mutation.key)
            .is_none_or(|old| mutation.document.stamp > old.stamp)
        {
            next.documents
                .insert(mutation.key.clone(), mutation.document.clone());
        }
        let mut history: Vec<_> = next
            .documents
            .iter()
            .filter(|(key, _)| key.starts_with(HISTORY_PREFIX))
            .map(|(key, document)| (document.stamp.clone(), key.clone()))
            .collect();
        history.sort();
        let excess = history.len().saturating_sub(MAX_HISTORY);
        for (_, key) in history.into_iter().take(excess) {
            next.documents.remove(&key);
        }
        next.encode()?;
        Ok(next)
    }
}

pub fn prepare(
    user: &str,
    device: &str,
    sequence: u64,
    entry: &WalEntry,
    blob: Option<&BlobIndex>,
) -> Result<Mutation> {
    // Attempts are intentionally excluded. A restart or retry keeps this identity.
    let identity = serde_json::to_vec(&(user, device, sequence, entry.created_at_ms, &entry.op))?;
    let operation = blake3::hash(&identity).to_hex().to_string();
    let mut stamp = Stamp {
        clock: 0,
        time_ms: entry.created_at_ms,
        device: device.into(),
        operation: operation.clone(),
    };
    let (key, value) = match &entry.op {
        ReplicationOp::RecordPlay(track) => {
            let cover_art_id = match &track.cover_art {
                CoverArt::ServiceId { id, .. } => Some(id.clone()),
                CoverArt::Url(url) => Some(url.clone()),
                CoverArt::None => None,
            };
            let record = drift_plugin::HistoryRecord {
                track_id: track.id.clone(),
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                duration_seconds: track.duration_seconds,
                cover_art_id,
                service: track.service.to_string(),
                played_at_ms: entry.created_at_ms,
            };
            (
                format!("{HISTORY_PREFIX}{operation}"),
                Some(serde_json::to_value(record)?),
            )
        }
        ReplicationOp::SaveQueue(queue) => {
            stamp.clock = queue.lamport_clock;
            stamp.time_ms = queue.updated_at_ms;
            stamp.device = queue.device_id.clone();
            (QUEUE_KEY.into(), Some(serde_json::to_value(queue)?))
        }
        ReplicationOp::CacheSearch {
            query,
            service_filter,
            results,
        } => {
            let key = blake3::hash(&serde_json::to_vec(&(
                query.trim().to_lowercase(),
                service_filter,
            ))?)
            .to_hex();
            (
                format!("search/{key}"),
                Some(serde_json::to_value(results)?),
            )
        }
        ReplicationOp::SaveSearchHistory(history) => (
            "search_history".into(),
            Some(serde_json::to_value(history)?),
        ),
        ReplicationOp::SavePlaylist(playlist) => (
            format!("{PLAYLIST_PREFIX}{}", playlist.id),
            Some(serde_json::to_value(playlist)?),
        ),
        ReplicationOp::DeletePlaylist { playlist_id } => {
            (format!("{PLAYLIST_PREFIX}{playlist_id}"), None)
        }
        ReplicationOp::UploadBlob { track_id, .. } => {
            let index =
                blob.context("blob publication must complete before its metadata mutation")?;
            (
                format!("{BLOB_PREFIX}{track_id}"),
                Some(serde_json::to_value(index)?),
            )
        }
    };
    Ok(Mutation {
        key,
        document: Document { stamp, value },
    })
}

pub fn queue_order(queue: &PersistedQueue) -> (u64, u64, &str) {
    (queue.lamport_clock, queue.updated_at_ms, &queue.device_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL_TIME: u64 = 10;
    const NEWER_TIME: u64 = 20;

    fn deletion(time: u64) -> WalEntry {
        WalEntry {
            op: ReplicationOp::DeletePlaylist {
                playlist_id: "mix".into(),
            },
            created_at_ms: time,
            attempts: 0,
        }
    }

    #[test]
    fn retry_identity_survives_attempt_changes_and_restart() {
        let mut entry = deletion(ORIGINAL_TIME);
        let first = prepare("alice", "laptop", 1, &entry, None).unwrap();
        entry.attempts += 1;
        let retry = prepare("alice", "laptop", 1, &entry, None).unwrap();
        assert_eq!(first.document.stamp, retry.document.stamp);
        let state = Snapshot::empty("alice").apply(&first).unwrap();
        let restored = Snapshot::decode(&state.encode().unwrap(), "alice").unwrap();
        assert_eq!(
            restored.apply(&retry).unwrap().encode().unwrap(),
            state.encode().unwrap()
        );
    }

    #[test]
    fn stale_write_cannot_replace_a_newer_tombstone() {
        let deletion = prepare("alice", "phone", 1, &deletion(NEWER_TIME), None).unwrap();
        let mut stale = prepare("alice", "laptop", 1, &deletion_entry(), None).unwrap();
        stale.document.value = Some(serde_json::json!({"id": "mix"}));
        let state = Snapshot::empty("alice").apply(&deletion).unwrap();
        assert!(state.apply(&stale).unwrap().documents["playlists/mix"]
            .value
            .is_none());
    }

    fn deletion_entry() -> WalEntry {
        deletion(ORIGINAL_TIME)
    }

    #[test]
    fn rejects_wrong_account_unknown_schema_and_malformed_state() {
        let mut state = Snapshot::empty("alice");
        assert!(Snapshot::decode(&state.encode().unwrap(), "bob").is_err());
        state.schema += 1;
        assert!(Snapshot::decode(&state.encode().unwrap(), "alice").is_err());
        assert!(Snapshot::decode(b"{}", "alice").is_err());
        assert!(Snapshot::decode(b"not json", "alice").is_err());
    }

    #[test]
    fn independent_updates_converge_in_both_orders() {
        let first = prepare("alice", "phone", 0, &deletion(ORIGINAL_TIME), None).unwrap();
        let mut second = prepare("alice", "laptop", 1, &deletion(NEWER_TIME), None).unwrap();
        second.key = "playlists/other".into();
        let state = Snapshot::empty("alice");
        let left = state.apply(&first).unwrap().apply(&second).unwrap();
        let right = state.apply(&second).unwrap().apply(&first).unwrap();
        assert_eq!(left.encode().unwrap(), right.encode().unwrap());
    }
}
