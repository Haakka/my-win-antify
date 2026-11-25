use crate::error::IpcError;
use crate::models::{
    ActivityLogEntry, AppConfig, ConnectionSnapshot, NetworkInterfaceInfo, ProxyTestResult,
    RunningProcessInfo, ServiceStatus, TrafficStats,
};
use serde::{Deserialize, Serialize};

/// Windows Named Pipe endpoint exposed by `core-service`.
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\my-win-antify-core-v1";

/// Wire protocol version for Named Pipe NDJSON messages.
pub const IPC_PROTOCOL_VERSION: u32 = 1;

/// Configuration schema version stored in `config.json`.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Tauri WebView event names emitted by `src-tauri` bridge.
pub const EVENT_STATUS_CHANGED: &str = "core://status-changed";
pub const EVENT_STATS_TICK: &str = "core://stats-tick";
pub const EVENT_CONNECTION_EVENT: &str = "core://connection-event";
pub const EVENT_ACTIVITY_LOG: &str = "core://activity-log";
pub const EVENT_CONFIG_CHANGED: &str = "core://config-changed";
pub const EVENT_PIPE_CONNECTION: &str = "core://pipe-connection";

/// Handshake response metadata returned by `RpcRequest::Ping`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PongInfo {
    pub core_version: String,
    pub protocol_version: u32,
    pub service_pid: u32,
}

/// Client-to-server RPC request envelope (one JSON line per request).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IpcRequest {
    pub id: u64,
    pub request: RpcRequest,
}

/// Supported Named Pipe RPC operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "method", content = "params", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RpcRequest {
    Ping,
    GetStatus,
    StartEngine,
    StopEngine,
    GetConfig,
    SaveConfig {
        config: AppConfig,
    },
    SetActiveProfile {
        profile_id: String,
    },
    GetConnections {
        #[serde(default)]
        include_closed: bool,
        #[serde(default)]
        limit: Option<u32>,
    },
    ClearConnections,
    ListRunningProcesses,
    ListNetworkInterfaces,
    TestProxy {
        proxy_id: String,
    },
    GetActivityLogs {
        #[serde(default)]
        limit: Option<u32>,
    },
    ClearActivityLogs,
}

/// Typed result payload inside `IpcResponse`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum RpcResult {
    Pong(PongInfo),
    Status(ServiceStatus),
    Config(AppConfig),
    Connections(Vec<ConnectionSnapshot>),
    Processes(Vec<RunningProcessInfo>),
    NetworkInterfaces(Vec<NetworkInterfaceInfo>),
    ProxyTest(ProxyTestResult),
    ActivityLogs(Vec<ActivityLogEntry>),
    Ack,
}

/// Server-to-client RPC response matching a prior `IpcRequest.id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IpcResponse {
    pub id: u64,
    pub ok: bool,
    #[serde(default)]
    pub result: Option<RpcResult>,
    #[serde(default)]
    pub error: Option<IpcError>,
}

impl IpcResponse {
    pub fn ok(id: u64, result: RpcResult) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, error: IpcError) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}

/// Unsolicited server-to-client push event over the Named Pipe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum IpcEvent {
    StatusChanged(ServiceStatus),
    StatsTick(TrafficStats),
    ConnectionEvent(ConnectionSnapshot),
    ActivityLog(ActivityLogEntry),
    ConfigChanged(AppConfig),
}

/// Top-level NDJSON frame sent from `core-service` to `src-tauri`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "frame", content = "payload", rename_all = "camelCase")]
pub enum ServerFrame {
    Response(IpcResponse),
    Event(IpcEvent),
}

/// Tauri-to-Frontend Named Pipe link health state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PipeConnectionState {
    Connected,
    Disconnected,
    Reconnecting,
}
