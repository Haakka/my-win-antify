use serde::{Deserialize, Serialize};
use std::fmt;

/// Machine-readable IPC and core service error codes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum IpcErrorCode {
    InvalidRequest,
    NotFound,
    ValidationFailed,
    DriverUnavailable,
    AccessDenied,
    ProxyError,
    ConfigIoError,
    EngineAlreadyRunning,
    EngineNotRunning,
    ServiceDisconnected,
    Internal,
}

/// Structured error returned over Named Pipe IPC and Tauri commands.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: IpcErrorCode,
    pub message: String,
}

impl IpcError {
    pub fn new(code: IpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?}] {}", self.code, self.message)
    }
}
