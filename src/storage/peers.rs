//! Peer cluster management for cross-user playlist sharing.
//!
//! Manages Aspen peer cluster subscriptions:
//! - Adds configured peers as peer clusters on startup
//! - Applies `drift:` include filter so only drift keys are synced
//! - Provides methods to add/remove/list peers at runtime
//!
//! Peer data flows: peer's cluster → our cluster (via Aspen replication)
//! → our local storage (via LocalFirstStorage poll). Peer data is read-only.

use std::collections::HashMap;

use anyhow::{Context, Result};
use tracing::{info, warn};

use aspen_client::AspenClient;
use aspen_client::ClientRpcRequest;
use aspen_client::ClientRpcResponse;

use crate::config::PeerConfig;

/// Key prefix filter applied to all peer subscriptions.
/// Only keys starting with "drift:" are replicated from peers.
const DRIFT_KEY_PREFIX: &str = "drift:";

/// Manages peer cluster subscriptions in the Aspen cluster.
pub struct PeerClusterManager {
    /// Maps peer name → cluster_id (once added).
    peers: HashMap<String, String>,
}

impl PeerClusterManager {
    /// Create a new peer cluster manager and register all configured peers.
    ///
    /// This is called once during `LocalFirstStorage::new()`. It iterates
    /// over enabled peers in the config, calls `AddPeerCluster` for each,
    /// and applies the `drift:` include filter.
    ///
    /// Failures are logged but don't prevent startup — peering is additive.
    pub async fn init(client: &AspenClient, peers: &[PeerConfig]) -> Self {
        let mut manager = Self {
            peers: HashMap::new(),
        };

        if peers.is_empty() {
            return manager;
        }

        info!("Initializing {} peer cluster subscriptions", peers.len());

        // First, check what peers are already registered
        let existing = Self::list_existing_peers(client).await;

        for peer in peers {
            if !peer.enabled {
                info!("Skipping disabled peer '{}'", peer.name);
                continue;
            }

            // Check if already registered (idempotent)
            if let Some(cluster_id) = existing.get(&peer.name) {
                info!("Peer '{}' already registered ({})", peer.name, cluster_id);
                manager.peers.insert(peer.name.clone(), cluster_id.clone());
                continue;
            }

            match Self::add_peer(client, peer).await {
                Ok(cluster_id) => {
                    info!("Added peer cluster '{}' → {}", peer.name, cluster_id);
                    manager.peers.insert(peer.name.clone(), cluster_id);
                }
                Err(e) => {
                    warn!("Failed to add peer '{}': {}", peer.name, e);
                }
            }
        }

        manager
    }

    /// Add a single peer cluster and apply the drift key filter.
    async fn add_peer(client: &AspenClient, peer: &PeerConfig) -> Result<String> {
        // Step 1: Add the peer cluster
        let resp = client
            .send(ClientRpcRequest::AddPeerCluster {
                ticket: peer.ticket.clone(),
            })
            .await
            .context("AddPeerCluster RPC failed")?;

        let cluster_id = match resp {
            ClientRpcResponse::AddPeerClusterResult(r) if r.is_success => r
                .cluster_id
                .ok_or_else(|| anyhow::anyhow!("AddPeerCluster succeeded but no cluster_id"))?,
            ClientRpcResponse::AddPeerClusterResult(r) => {
                anyhow::bail!(
                    "AddPeerCluster failed: {}",
                    r.error.unwrap_or_default()
                );
            }
            ClientRpcResponse::Error(e) => {
                anyhow::bail!("AddPeerCluster error: {}", e.message);
            }
            _ => anyhow::bail!("Unexpected response to AddPeerCluster"),
        };

        // Step 2: Apply include filter for drift: keys only
        let prefixes_json = serde_json::to_string(&[DRIFT_KEY_PREFIX])?;
        let filter_resp = client
            .send(ClientRpcRequest::UpdatePeerClusterFilter {
                cluster_id: cluster_id.clone(),
                filter_type: "include".to_string(),
                prefixes: Some(prefixes_json),
            })
            .await
            .context("UpdatePeerClusterFilter RPC failed")?;

        match filter_resp {
            ClientRpcResponse::UpdatePeerClusterFilterResult(r) if r.is_success => {
                info!(
                    "Applied drift: include filter to peer '{}'",
                    cluster_id
                );
            }
            ClientRpcResponse::UpdatePeerClusterFilterResult(r) => {
                warn!(
                    "Filter update for '{}' failed: {} (peer still added without filter)",
                    cluster_id,
                    r.error.unwrap_or_default()
                );
            }
            _ => {
                warn!("Unexpected response to UpdatePeerClusterFilter for '{}'", cluster_id);
            }
        }

        // Step 3: Set priority if non-default
        if peer.priority != 10 {
            let priority_resp = client
                .send(ClientRpcRequest::UpdatePeerClusterPriority {
                    cluster_id: cluster_id.clone(),
                    priority: peer.priority,
                })
                .await;

            if let Err(e) = priority_resp {
                warn!("Failed to set priority for '{}': {}", cluster_id, e);
            }
        }

        Ok(cluster_id)
    }

    /// List existing peer clusters from the Aspen cluster.
    async fn list_existing_peers(client: &AspenClient) -> HashMap<String, String> {
        let mut map = HashMap::new();

        let resp = match client.send(ClientRpcRequest::ListPeerClusters).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Failed to list existing peer clusters: {}", e);
                return map;
            }
        };

        match resp {
            ClientRpcResponse::ListPeerClustersResult(r) => {
                for peer in r.peers {
                    map.insert(peer.name.clone(), peer.cluster_id.clone());
                }
            }
            _ => {
                warn!("Unexpected response to ListPeerClusters");
            }
        }

        map
    }

    /// Remove a peer cluster subscription.
    pub async fn remove_peer(
        &mut self,
        client: &AspenClient,
        peer_name: &str,
    ) -> Result<()> {
        let cluster_id = self
            .peers
            .get(peer_name)
            .ok_or_else(|| anyhow::anyhow!("Unknown peer: {}", peer_name))?
            .clone();

        let resp = client
            .send(ClientRpcRequest::RemovePeerCluster {
                cluster_id: cluster_id.clone(),
            })
            .await
            .context("RemovePeerCluster RPC failed")?;

        match resp {
            ClientRpcResponse::RemovePeerClusterResult(r) if r.is_success => {
                info!("Removed peer cluster '{}' ({})", peer_name, cluster_id);
                self.peers.remove(peer_name);
                Ok(())
            }
            ClientRpcResponse::RemovePeerClusterResult(r) => {
                anyhow::bail!(
                    "RemovePeerCluster failed: {}",
                    r.error.unwrap_or_default()
                );
            }
            ClientRpcResponse::Error(e) => {
                anyhow::bail!("RemovePeerCluster error: {}", e.message);
            }
            _ => anyhow::bail!("Unexpected response to RemovePeerCluster"),
        }
    }

    /// Get the cluster ID for a peer by name.
    pub fn cluster_id(&self, peer_name: &str) -> Option<&str> {
        self.peers.get(peer_name).map(|s| s.as_str())
    }

    /// Get all registered peer names.
    pub fn peer_names(&self) -> Vec<&str> {
        self.peers.keys().map(|s| s.as_str()).collect()
    }
}
