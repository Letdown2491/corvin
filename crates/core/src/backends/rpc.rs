use super::BackendError;
use crate::types::{BackendKind, NodeStatus};
use bitcoincore_rpc::{Auth, Client, RpcApi};

pub struct RpcConfig {
    pub url: String,
    pub user: String,
    pub pass: String,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:8332".to_string(),
            user: String::new(),
            pass: String::new(),
        }
    }
}

pub fn build_client(cfg: &RpcConfig) -> Result<Client, BackendError> {
    Client::new(&cfg.url, Auth::UserPass(cfg.user.clone(), cfg.pass.clone()))
        .map_err(|e| BackendError::Rpc(e.to_string()))
}

pub fn broadcast_tx(
    tx: &bdk_wallet::bitcoin::Transaction,
    cfg: &RpcConfig,
) -> Result<(), BackendError> {
    use bdk_wallet::bitcoin::consensus::serialize;
    use bitcoincore_rpc::RpcApi;
    let client = build_client(cfg)?;
    let tx_hex = hex::encode(serialize(tx));
    client
        .send_raw_transaction(tx_hex.as_str())
        .map_err(|e| BackendError::Rpc(e.to_string()))?;
    Ok(())
}

/// Ask Core whether `tx` would be accepted to the mempool (no broadcast).
/// Used by payjoin receive to validate the sender's original is broadcastable
/// before we contribute to it. Electrum has no equivalent, so payjoin receive
/// requires an RPC backend.
pub fn test_mempool_accept(
    tx: &bdk_wallet::bitcoin::Transaction,
    cfg: &RpcConfig,
) -> Result<bool, BackendError> {
    use bdk_wallet::bitcoin::consensus::serialize;
    let client = build_client(cfg)?;
    let tx_hex = hex::encode(serialize(tx));
    let res = client
        .test_mempool_accept(&[tx_hex.as_str()])
        .map_err(|e| BackendError::Rpc(e.to_string()))?;
    Ok(res.first().map(|r| r.allowed).unwrap_or(false))
}

pub fn probe_status(cfg: &RpcConfig) -> NodeStatus {
    match build_client(cfg) {
        Ok(client) => match client.get_blockchain_info() {
            Ok(info) => NodeStatus {
                backend: BackendKind::Rpc,
                connected: true,
                network: info.chain.to_string(),
                tip_height: Some(info.blocks as u32),
                error: None,
                offline: false,
            },
            Err(e) => {
                let msg = e.to_string();
                tracing::warn!("RPC getblockchaininfo failed: {msg}");
                NodeStatus {
                    backend: BackendKind::Rpc,
                    connected: false,
                    network: "unknown".to_string(),
                    tip_height: None,
                    error: Some(msg),
                    offline: false,
                }
            }
        },
        Err(e) => {
            let msg = e.to_string();
            tracing::warn!("RPC connect failed: {msg}");
            NodeStatus {
                backend: BackendKind::Rpc,
                connected: false,
                network: "unknown".to_string(),
                tip_height: None,
                error: Some(msg),
                offline: false,
            }
        }
    }
}
