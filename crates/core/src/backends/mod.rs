pub mod electrum;
pub mod rpc;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("electrum error: {0}")]
    Electrum(String),
    #[error("rpc error: {0}")]
    Rpc(String),
    #[error("not connected")]
    NotConnected,
}
