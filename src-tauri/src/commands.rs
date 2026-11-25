use shared::{
    ActivityLogEntry, AppConfig, ConnectionSnapshot, IpcError, IpcErrorCode, NetworkInterfaceInfo,
    PipeConnectionState, PongInfo, ProxyTestResult, RpcRequest, RpcResult, RunningProcessInfo,
    ServiceStatus,
};
use tauri::State;

use crate::pipe_client::{unexpected, PipeClient};

macro_rules! command {
    ($name:ident $(($($arg:ident: $arg_type:ty),* $(,)?))?, $request:expr, $variant:ident => $output:ty) => {
        #[tauri::command]
        pub(crate) async fn $name(
            client: State<'_, PipeClient>,
            $($($arg: $arg_type,)*)?
        ) -> Result<$output, IpcError> {
            match client.request($request).await? {
                RpcResult::$variant(value) => Ok(value),
                _ => Err(unexpected("Unexpected core response type")),
            }
        }
    };
}

command!(core_ping, RpcRequest::Ping, Pong => PongInfo);
command!(core_get_status, RpcRequest::GetStatus, Status => ServiceStatus);
command!(core_start_engine, RpcRequest::StartEngine, Status => ServiceStatus);
command!(core_stop_engine, RpcRequest::StopEngine, Status => ServiceStatus);
command!(core_get_config, RpcRequest::GetConfig, Config => AppConfig);
command!(core_save_config(config: AppConfig), RpcRequest::SaveConfig { config }, Config => AppConfig);
command!(core_set_active_profile(profile_id: String), RpcRequest::SetActiveProfile { profile_id }, Config => AppConfig);
command!(
    core_get_connections(include_closed: bool, limit: Option<u32>),
    RpcRequest::GetConnections { include_closed, limit },
    Connections => Vec<ConnectionSnapshot>
);
command!(core_list_running_processes, RpcRequest::ListRunningProcesses, Processes => Vec<RunningProcessInfo>);
command!(core_test_proxy(proxy_id: String), RpcRequest::TestProxy { proxy_id }, ProxyTest => ProxyTestResult);
command!(core_get_activity_logs(limit: Option<u32>), RpcRequest::GetActivityLogs { limit }, ActivityLogs => Vec<ActivityLogEntry>);

#[tauri::command]
pub(crate) async fn core_list_network_interfaces(
    client: State<'_, PipeClient>,
) -> Result<Vec<NetworkInterfaceInfo>, IpcError> {
    match client.request(RpcRequest::ListNetworkInterfaces).await {
        Ok(RpcResult::NetworkInterfaces(interfaces)) => return Ok(interfaces),
        Ok(_) => tracing::debug!("Unexpected adapter-list response from core; enumerating locally"),
        Err(error) => tracing::debug!(%error, "Adapter-list RPC unavailable; enumerating locally"),
    }
    tokio::task::spawn_blocking(crate::network_interfaces::list)
        .await
        .map_err(|error| unexpected(&error.to_string()))?
        .map_err(|error| unexpected(&format!("Windows adapter enumeration failed: {error}")))
}

#[tauri::command]
pub(crate) async fn core_clear_connections(client: State<'_, PipeClient>) -> Result<(), IpcError> {
    match client.request(RpcRequest::ClearConnections).await? {
        RpcResult::Ack => Ok(()),
        _ => Err(unexpected("Expected core acknowledgement")),
    }
}

#[tauri::command]
pub(crate) async fn core_clear_activity_logs(
    client: State<'_, PipeClient>,
) -> Result<(), IpcError> {
    match client.request(RpcRequest::ClearActivityLogs).await? {
        RpcResult::Ack => Ok(()),
        _ => Err(unexpected("Expected core acknowledgement")),
    }
}

#[tauri::command]
pub(crate) async fn core_launch_service() -> Result<bool, IpcError> {
    tokio::task::spawn_blocking(crate::service_launcher::ensure_core_service_running)
        .await
        .map_err(|error| IpcError::new(IpcErrorCode::AccessDenied, error.to_string()))?
        .map_err(|message| IpcError::new(IpcErrorCode::AccessDenied, message))
}

#[tauri::command]
pub(crate) fn core_pipe_state(client: State<'_, PipeClient>) -> PipeConnectionState {
    client.state()
}
