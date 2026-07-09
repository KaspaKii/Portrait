//! Kaspa wRPC node client: lazy-connecting WebSocket+Borsh client with
//! convenience accessors for server info and chain state.

use std::sync::Arc;

use kaspa_consensus_core::network::{NetworkId, NetworkType};
use kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_wrpc_client::{prelude::ConnectOptions, KaspaRpcClient, Resolver, WrpcEncoding};
use tokio::sync::OnceCell;

use crate::error::{Error, Result};

/// Connection parameters for a Kaspa node.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// WebSocket URL of the node's wRPC endpoint.
    pub url: String,
    /// Network the node is expected to be on.
    pub network_id: NetworkId,
}

impl NodeConfig {
    /// Config for a testnet node with the given numeric suffix.
    pub fn testnet(url: impl Into<String>, suffix: u32) -> Self {
        Self {
            url: url.into(),
            network_id: NetworkId::with_suffix(NetworkType::Testnet, suffix),
        }
    }

    /// Config for a testnet-10 node (the Toccata covenant testnet where the
    /// library's evidence was captured; adjust the suffix if it moves).
    pub fn tn10(url: impl Into<String>) -> Self {
        Self::testnet(url, 10)
    }

    /// Config for a mainnet node. Real funds — callers gate any submit path.
    pub fn mainnet(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            network_id: NetworkId::new(NetworkType::Mainnet),
        }
    }
}

/// Lazy-connecting wRPC client. The first call that needs the node connects
/// and caches the client; later calls reuse it.
pub struct NodeClient {
    config: NodeConfig,
    client: OnceCell<Arc<KaspaRpcClient>>,
}

impl NodeClient {
    /// Create a client for `config`. Does not connect yet.
    pub fn new(config: NodeConfig) -> Self {
        Self {
            config,
            client: OnceCell::new(),
        }
    }

    /// The underlying RPC client, connecting on first use.
    pub async fn rpc(&self) -> Result<Arc<KaspaRpcClient>> {
        let cfg = self.config.clone();
        self.client
            .get_or_try_init(|| async move {
                let client = KaspaRpcClient::new(
                    WrpcEncoding::Borsh,
                    Some(cfg.url.as_str()),
                    None::<Resolver>,
                    Some(cfg.network_id),
                    None,
                )
                .map_err(|e| Error::Rpc(format!("client init: {e}")))?;
                client
                    .connect(Some(ConnectOptions::default()))
                    .await
                    .map_err(|e| Error::Rpc(format!("connect: {e}")))?;
                Ok::<_, Error>(Arc::new(client))
            })
            .await
            .cloned()
    }

    /// Server version, network, sync status, and virtual DAA score.
    pub async fn server_info(&self) -> Result<ServerInfoSnapshot> {
        let client = self.rpc().await?;
        let info = client
            .get_server_info()
            .await
            .map_err(|e| Error::Rpc(format!("get_server_info: {e}")))?;
        Ok(ServerInfoSnapshot {
            server_version: info.server_version,
            network_id: format!("{}", info.network_id),
            is_synced: info.is_synced,
            virtual_daa_score: info.virtual_daa_score,
        })
    }

    /// The current `virtual_daa_score` — the best on-chain measure of "now"
    /// available without spending a UTXO.
    pub async fn virtual_daa_score(&self) -> Result<u64> {
        let client = self.rpc().await?;
        let dag = client
            .get_block_dag_info()
            .await
            .map_err(|e| Error::Rpc(format!("get_block_dag_info: {e}")))?;
        Ok(dag.virtual_daa_score)
    }
}

/// Snapshot of node identity and sync state.
#[derive(Debug, Clone)]
pub struct ServerInfoSnapshot {
    /// kaspad server version string.
    pub server_version: String,
    /// Network the node reports (e.g. `testnet-10`).
    pub network_id: String,
    /// Whether the node reports itself synced.
    pub is_synced: bool,
    /// Virtual DAA score at the time of the call.
    pub virtual_daa_score: u64,
}
