use anyhow::{Context, Result};
use chrono::Utc;
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::service::{SearchResults, ServiceType, Track};
use crate::queue_persistence::PersistedQueue;
use crate::search::SearchHistory;

const WAL_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("wal_entries");

/// A replication operation to be sent to the remote cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReplicationOp {
    RecordPlay(Track),
    SaveQueue(PersistedQueue),
    CacheSearch {
        query: String,
        service_filter: Option<ServiceType>,
        results: SearchResults,
    },
    SaveSearchHistory(SearchHistory),
    UploadBlob {
        track_id: String,
        file_path: String,
    },
}

/// A timestamped WAL entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    pub op: ReplicationOp,
    pub created_at_ms: u64,
    pub attempts: u32,
}

/// Persistent write-ahead log backed by redb.
pub struct WalManager {
    db: Database,
    next_seq: Mutex<u64>,
}

impl WalManager {
    /// Create or open the WAL database at `~/.local/share/drift/wal.redb`.
    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path()?;
        let db = Database::create(&db_path)
            .context("Failed to open WAL database")?;
        Self::init_table(&db)?;
        
        // Read the max key to initialize the sequence counter
        let next_seq = {
            let rtxn = db.begin_read()?;
            let table = rtxn.open_table(WAL_TABLE)?;
            // Get the last (highest) key
            let max_key = table.iter()?
                .rev()
                .next()
                .transpose()?
                .map(|(k, _)| k.value())
                .unwrap_or(0);
            max_key + 1
        };
        
        Ok(Self {
            db,
            next_seq: Mutex::new(next_seq),
        })
    }

    /// Create an in-memory WAL for testing.
    pub fn new_in_memory() -> Result<Self> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("drift-wal-test-{}-{}.redb", std::process::id(), n));
        let db = Database::create(&path)
            .context("Failed to create test WAL database")?;
        Self::init_table(&db)?;
        
        Ok(Self {
            db,
            next_seq: Mutex::new(1),
        })
    }

    fn get_db_path() -> Result<PathBuf> {
        let data_dir = dirs::data_dir()
            .context("Failed to get data directory")?
            .join("drift");
        std::fs::create_dir_all(&data_dir)
            .context("Failed to create data directory")?;
        Ok(data_dir.join("wal.redb"))
    }

    fn init_table(db: &Database) -> Result<()> {
        let txn = db.begin_write()?;
        { let _ = txn.open_table(WAL_TABLE)?; }
        txn.commit()?;
        Ok(())
    }

    fn now_ms() -> u64 {
        Utc::now().timestamp_millis() as u64
    }

    /// Append a new replication operation. Returns the WAL sequence number.
    pub fn append(&self, op: &ReplicationOp) -> Result<u64> {
        let seq = {
            let mut next_seq = self.next_seq.lock().unwrap();
            let seq = *next_seq;
            *next_seq += 1;
            seq
        };

        let entry = WalEntry {
            op: op.clone(),
            created_at_ms: Self::now_ms(),
            attempts: 0,
        };

        let json = serde_json::to_vec(&entry)?;
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(WAL_TABLE)?;
            table.insert(seq, json.as_slice())?;
        }
        txn.commit()?;

        Ok(seq)
    }

    /// Remove a successfully replicated entry by sequence number.
    pub fn remove(&self, seq: u64) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(WAL_TABLE)?;
            table.remove(seq)?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Get all pending entries in insertion order (oldest first).
    pub fn drain_pending(&self) -> Result<Vec<(u64, WalEntry)>> {
        let rtxn = self.db.begin_read()?;
        let table = rtxn.open_table(WAL_TABLE)?;
        let mut entries = Vec::new();

        for item in table.iter()? {
            let (key, val) = item?;
            if let Ok(entry) = serde_json::from_slice::<WalEntry>(val.value()) {
                entries.push((key.value(), entry));
            }
        }

        Ok(entries)
    }

    /// Remove entries older than `max_age`.
    pub fn prune_expired(&self, max_age: std::time::Duration) -> Result<usize> {
        let now_ms = Self::now_ms();
        let cutoff_ms = now_ms.saturating_sub(max_age.as_millis() as u64);
        
        let to_remove = {
            let rtxn = self.db.begin_read()?;
            let table = rtxn.open_table(WAL_TABLE)?;
            let mut keys = Vec::new();
            
            for item in table.iter()? {
                let (key, val) = item?;
                if let Ok(entry) = serde_json::from_slice::<WalEntry>(val.value()) {
                    if entry.created_at_ms < cutoff_ms {
                        keys.push(key.value());
                    }
                }
            }
            keys
        };

        let count = to_remove.len();
        if count > 0 {
            let txn = self.db.begin_write()?;
            {
                let mut table = txn.open_table(WAL_TABLE)?;
                for key in to_remove {
                    table.remove(key)?;
                }
            }
            txn.commit()?;
        }

        Ok(count)
    }

    /// Enforce maximum entry count, dropping oldest entries.
    pub fn enforce_max_entries(&self, max_entries: usize) -> Result<usize> {
        let to_remove = {
            let rtxn = self.db.begin_read()?;
            let table = rtxn.open_table(WAL_TABLE)?;
            let count = table.len()? as usize;
            
            if count <= max_entries {
                return Ok(0);
            }
            
            let to_delete = count - max_entries;
            // Collect oldest keys (forward iteration = ascending order)
            let keys: Vec<u64> = table.iter()?
                .take(to_delete)
                .map(|r| r.map(|(k, _)| k.value()))
                .collect::<std::result::Result<_, _>>()?;
            keys
        };

        let count = to_remove.len();
        if count > 0 {
            let txn = self.db.begin_write()?;
            {
                let mut table = txn.open_table(WAL_TABLE)?;
                for key in to_remove {
                    table.remove(key)?;
                }
            }
            txn.commit()?;
        }

        Ok(count)
    }

    /// Number of pending entries.
    pub fn len(&self) -> Result<usize> {
        let rtxn = self.db.begin_read()?;
        let table = rtxn.open_table(WAL_TABLE)?;
        Ok(table.len()? as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{CoverArt, ServiceType};
    use std::time::Duration;

    fn create_test_track(id: &str, title: &str, artist: &str) -> Track {
        Track {
            id: id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            album: "Test Album".to_string(),
            duration_seconds: 180,
            cover_art: CoverArt::None,
            service: ServiceType::Tidal,
        }
    }

    #[test]
    fn test_append_and_drain() {
        let wal = WalManager::new_in_memory().unwrap();
        
        let track1 = create_test_track("1", "Song 1", "Artist 1");
        let track2 = create_test_track("2", "Song 2", "Artist 2");
        let track3 = create_test_track("3", "Song 3", "Artist 3");
        
        let seq1 = wal.append(&ReplicationOp::RecordPlay(track1)).unwrap();
        let seq2 = wal.append(&ReplicationOp::RecordPlay(track2)).unwrap();
        let seq3 = wal.append(&ReplicationOp::RecordPlay(track3)).unwrap();
        
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        assert_eq!(seq3, 3);
        
        let entries = wal.drain_pending().unwrap();
        assert_eq!(entries.len(), 3);
        
        // Verify order (oldest first)
        assert_eq!(entries[0].0, 1);
        assert_eq!(entries[1].0, 2);
        assert_eq!(entries[2].0, 3);
        
        // Verify content
        match &entries[0].1.op {
            ReplicationOp::RecordPlay(track) => assert_eq!(track.id, "1"),
            _ => panic!("Wrong op type"),
        }
    }

    #[test]
    fn test_remove() {
        let wal = WalManager::new_in_memory().unwrap();
        
        let track = create_test_track("1", "Song", "Artist");
        let seq = wal.append(&ReplicationOp::RecordPlay(track)).unwrap();
        
        assert_eq!(wal.len().unwrap(), 1);
        
        wal.remove(seq).unwrap();
        
        let entries = wal.drain_pending().unwrap();
        assert!(entries.is_empty());
        assert_eq!(wal.len().unwrap(), 0);
    }

    #[test]
    fn test_prune_expired() {
        let wal = WalManager::new_in_memory().unwrap();
        
        // Create entries with old timestamps by directly manipulating the DB
        let txn = wal.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(WAL_TABLE).unwrap();
            
            let now_ms = WalManager::now_ms();
            let old_ms = now_ms - (2 * 60 * 60 * 1000); // 2 hours ago
            
            // Old entry
            let old_entry = WalEntry {
                op: ReplicationOp::RecordPlay(create_test_track("old", "Old", "Artist")),
                created_at_ms: old_ms,
                attempts: 0,
            };
            let json = serde_json::to_vec(&old_entry).unwrap();
            table.insert(1, json.as_slice()).unwrap();
            
            // Recent entry
            let recent_entry = WalEntry {
                op: ReplicationOp::RecordPlay(create_test_track("recent", "Recent", "Artist")),
                created_at_ms: now_ms,
                attempts: 0,
            };
            let json = serde_json::to_vec(&recent_entry).unwrap();
            table.insert(2, json.as_slice()).unwrap();
        }
        txn.commit().unwrap();
        
        // Update the sequence counter
        *wal.next_seq.lock().unwrap() = 3;
        
        assert_eq!(wal.len().unwrap(), 2);
        
        // Prune entries older than 1 hour
        let removed = wal.prune_expired(Duration::from_secs(60 * 60)).unwrap();
        assert_eq!(removed, 1);
        
        let entries = wal.drain_pending().unwrap();
        assert_eq!(entries.len(), 1);
        
        // Verify the recent entry remains
        match &entries[0].1.op {
            ReplicationOp::RecordPlay(track) => assert_eq!(track.id, "recent"),
            _ => panic!("Wrong op type"),
        }
    }

    #[test]
    fn test_enforce_max_entries() {
        let wal = WalManager::new_in_memory().unwrap();
        
        // Append 10 entries
        for i in 1..=10 {
            let track = create_test_track(&format!("{}", i), &format!("Song {}", i), "Artist");
            wal.append(&ReplicationOp::RecordPlay(track)).unwrap();
        }
        
        assert_eq!(wal.len().unwrap(), 10);
        
        // Enforce max of 5 entries
        let removed = wal.enforce_max_entries(5).unwrap();
        assert_eq!(removed, 5);
        assert_eq!(wal.len().unwrap(), 5);
        
        // Verify the 5 most recent entries remain
        let entries = wal.drain_pending().unwrap();
        assert_eq!(entries.len(), 5);
        
        // Entries 6-10 should remain
        match &entries[0].1.op {
            ReplicationOp::RecordPlay(track) => assert_eq!(track.id, "6"),
            _ => panic!("Wrong op type"),
        }
        match &entries[4].1.op {
            ReplicationOp::RecordPlay(track) => assert_eq!(track.id, "10"),
            _ => panic!("Wrong op type"),
        }
    }

    #[test]
    fn test_persistence() {
        use std::sync::atomic::{AtomicU64, Ordering};
        
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("drift-wal-persist-test-{}.redb", n));
        
        // Create WAL and add entries
        {
            let db = Database::create(&path).unwrap();
            WalManager::init_table(&db).unwrap();
            let wal = WalManager {
                db,
                next_seq: Mutex::new(1),
            };
            
            let track1 = create_test_track("1", "Song 1", "Artist");
            let track2 = create_test_track("2", "Song 2", "Artist");
            
            wal.append(&ReplicationOp::RecordPlay(track1)).unwrap();
            wal.append(&ReplicationOp::RecordPlay(track2)).unwrap();
            
            assert_eq!(wal.len().unwrap(), 2);
        }
        
        // Reopen and verify entries survive
        {
            let db = Database::create(&path).unwrap();
            let next_seq = {
                let rtxn = db.begin_read().unwrap();
                let table = rtxn.open_table(WAL_TABLE).unwrap();
                let max_key = table.iter().unwrap()
                    .rev()
                    .next()
                    .transpose().unwrap()
                    .map(|(k, _)| k.value())
                    .unwrap_or(0);
                max_key + 1
            };
            
            let wal = WalManager {
                db,
                next_seq: Mutex::new(next_seq),
            };
            
            assert_eq!(wal.len().unwrap(), 2);
            
            let entries = wal.drain_pending().unwrap();
            assert_eq!(entries.len(), 2);
            
            match &entries[0].1.op {
                ReplicationOp::RecordPlay(track) => assert_eq!(track.id, "1"),
                _ => panic!("Wrong op type"),
            }
            match &entries[1].1.op {
                ReplicationOp::RecordPlay(track) => assert_eq!(track.id, "2"),
                _ => panic!("Wrong op type"),
            }
        }
        
        // Cleanup
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_len() {
        let wal = WalManager::new_in_memory().unwrap();
        
        assert_eq!(wal.len().unwrap(), 0);
        
        let track = create_test_track("1", "Song", "Artist");
        wal.append(&ReplicationOp::RecordPlay(track.clone())).unwrap();
        assert_eq!(wal.len().unwrap(), 1);
        
        wal.append(&ReplicationOp::RecordPlay(track.clone())).unwrap();
        assert_eq!(wal.len().unwrap(), 2);
        
        wal.remove(1).unwrap();
        assert_eq!(wal.len().unwrap(), 1);
        
        wal.remove(2).unwrap();
        assert_eq!(wal.len().unwrap(), 0);
    }

    #[test]
    fn test_multiple_op_types() {
        let wal = WalManager::new_in_memory().unwrap();
        
        // RecordPlay
        let track = create_test_track("1", "Song", "Artist");
        wal.append(&ReplicationOp::RecordPlay(track)).unwrap();
        
        // SaveQueue
        let queue = PersistedQueue::new();
        wal.append(&ReplicationOp::SaveQueue(queue)).unwrap();
        
        // CacheSearch
        wal.append(&ReplicationOp::CacheSearch {
            query: "test query".to_string(),
            service_filter: Some(ServiceType::Tidal),
            results: SearchResults::default(),
        }).unwrap();
        
        // SaveSearchHistory
        let history = SearchHistory::new(10);
        wal.append(&ReplicationOp::SaveSearchHistory(history)).unwrap();
        
        // UploadBlob
        wal.append(&ReplicationOp::UploadBlob {
            track_id: "track123".to_string(),
            file_path: "/path/to/file.flac".to_string(),
        }).unwrap();
        
        assert_eq!(wal.len().unwrap(), 5);
        
        let entries = wal.drain_pending().unwrap();
        assert_eq!(entries.len(), 5);
        
        // Verify each op type
        match &entries[0].1.op {
            ReplicationOp::RecordPlay(_) => {},
            _ => panic!("Expected RecordPlay"),
        }
        match &entries[1].1.op {
            ReplicationOp::SaveQueue(_) => {},
            _ => panic!("Expected SaveQueue"),
        }
        match &entries[2].1.op {
            ReplicationOp::CacheSearch { .. } => {},
            _ => panic!("Expected CacheSearch"),
        }
        match &entries[3].1.op {
            ReplicationOp::SaveSearchHistory(_) => {},
            _ => panic!("Expected SaveSearchHistory"),
        }
        match &entries[4].1.op {
            ReplicationOp::UploadBlob { .. } => {},
            _ => panic!("Expected UploadBlob"),
        }
    }
}
