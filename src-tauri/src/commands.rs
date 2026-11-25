use shared::{
    ActivityLogEntry, AppConfig, CoexistenceConflict, ConfigBackupExport, ConnectionSnapshot,
    CurrentWifiInfo, DiagnosticBundleSummary, IpcError, IpcErrorCode, NetworkInterfaceInfo,
    PipeConnectionState, PongInfo, ProxyTestResult, RpcRequest, RpcResult, RulesImportPreview,
    RunningProcessInfo, ServiceStatus, TrafficHistoryQuery, TrafficHistoryResponse,
};
use tauri::State;

use crate::{
    autostart,
    pipe_client::{unexpected, PipeClient},
};

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
command!(core_restore_config_backup, RpcRequest::RestoreConfigBackup, Config => AppConfig);

#[tauri::command]
pub(crate) async fn core_save_config(
    client: State<'_, PipeClient>,
    config: AppConfig,
) -> Result<AppConfig, IpcError> {
    save_config_and_autostart(&client, RpcRequest::SaveConfig { config }).await
}

#[tauri::command]
pub(crate) async fn core_import_config_backup(
    client: State<'_, PipeClient>,
    json: String,
) -> Result<AppConfig, IpcError> {
    save_config_and_autostart(&client, RpcRequest::ImportConfigBackup { json }).await
}

async fn save_config_and_autostart(
    client: &PipeClient,
    request: RpcRequest,
) -> Result<AppConfig, IpcError> {
    // Registry writes follow the same order as their corresponding config commits.
    static SAVE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = SAVE.lock().await;
    let RpcResult::Config(saved) = client.request(request).await? else {
        return Err(unexpected("Unexpected core response type"));
    };
    autostart::sync_launch_at_login(saved.settings.launch_at_login)
        .map_err(|message| IpcError::new(IpcErrorCode::AccessDenied, message))?;
    Ok(saved)
}
command!(core_set_active_profile(profile_id: String), RpcRequest::SetActiveProfile { profile_id }, Config => AppConfig);
command!(
    core_get_connections(include_closed: bool, limit: Option<u32>),
    RpcRequest::GetConnections { include_closed, limit },
    Connections => Vec<ConnectionSnapshot>
);
command!(core_list_running_processes, RpcRequest::ListRunningProcesses, Processes => Vec<RunningProcessInfo>);
command!(core_test_proxy(proxy_id: String), RpcRequest::TestProxy { proxy_id }, ProxyTest => ProxyTestResult);
command!(core_get_activity_logs(limit: Option<u32>), RpcRequest::GetActivityLogs { limit }, ActivityLogs => Vec<ActivityLogEntry>);

command!(core_export_config_backup(include_passwords: bool), RpcRequest::ExportConfigBackup { include_passwords }, ConfigBackup => ConfigBackupExport);
command!(core_export_rules_json(profile_id: Option<String>), RpcRequest::ExportRulesJson { profile_id }, RulesJson => String);
command!(core_import_rules_json(json: String, profile_id: String, dry_run: bool), RpcRequest::ImportRulesJson { json, profile_id, dry_run }, RulesImport => RulesImportPreview);
command!(core_get_traffic_history(query: TrafficHistoryQuery), RpcRequest::GetTrafficHistory { query }, TrafficHistory => TrafficHistoryResponse);
command!(core_get_current_wifi, RpcRequest::GetCurrentWifi, CurrentWifi => CurrentWifiInfo);
command!(core_get_coexistence_conflicts, RpcRequest::GetCoexistenceConflicts, CoexistenceConflicts => Vec<CoexistenceConflict>);

#[tauri::command]
pub(crate) async fn core_clear_traffic_history(
    client: State<'_, PipeClient>,
) -> Result<(), IpcError> {
    acknowledge(&client, RpcRequest::ClearTrafficHistory).await
}

#[tauri::command]
pub(crate) async fn core_dismiss_coexistence_conflict(
    client: State<'_, PipeClient>,
    conflict_id: String,
) -> Result<(), IpcError> {
    acknowledge(
        &client,
        RpcRequest::DismissCoexistenceConflict { conflict_id },
    )
    .await
}

#[tauri::command]
pub(crate) async fn core_collect_diagnostic_bundle(
    client: State<'_, PipeClient>,
    output_path: Option<String>,
) -> Result<DiagnosticBundleSummary, IpcError> {
    let RpcResult::DiagnosticBundle(mut summary) = client
        .request(RpcRequest::CollectDiagnosticBundle { output_path: None })
        .await?
    else {
        return Err(unexpected("Unexpected core response type"));
    };
    // Keep the desktop pipe at identification level. The desktop writes an
    // explicit destination as the signed-in user, using the sanitized core export.
    if let Some(output_path) = output_path {
        let source = summary.path.clone();
        let target = output_path.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let contents = std::fs::read(source)?;
            crate::atomic_file::write(std::path::Path::new(&target), &contents)
        })
        .await
        .map_err(|error| unexpected(&error.to_string()))?
        .map_err(|error| IpcError::new(IpcErrorCode::ConfigIoError, error.to_string()))?;
        summary.path = output_path;
    }
    Ok(summary)
}

async fn acknowledge(client: &PipeClient, request: RpcRequest) -> Result<(), IpcError> {
    match client.request(request).await? {
        RpcResult::Ack => Ok(()),
        _ => Err(unexpected("Expected core acknowledgement")),
    }
}

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
    acknowledge(&client, RpcRequest::ClearConnections).await
}

#[tauri::command]
pub(crate) async fn core_clear_activity_logs(
    client: State<'_, PipeClient>,
) -> Result<(), IpcError> {
    acknowledge(&client, RpcRequest::ClearActivityLogs).await
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
