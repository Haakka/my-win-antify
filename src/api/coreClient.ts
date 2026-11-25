import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  TAURI_EVENTS,
  type ActivityLogEntry,
  type AppConfig,
  type CoexistenceConflict,
  type ConfigBackupExport,
  type ConnectionSnapshot,
  type CurrentWifiInfo,
  type DiagnosticBundleSummary,
  type NetworkInterfaceInfo,
  type PipeConnectionState,
  type PongInfo,
  type ProxyTestResult,
  type RulesImportPreview,
  type RunningProcessInfo,
  type ServiceStatus,
  type TauriCommandMap,
  type TrafficHistoryQuery,
  type TrafficHistoryResponse,
  type TrafficStats,
} from "@/types";
import { MockCoreClient } from "./mockClient";

export interface CoreEventCallbacks {
  onStatusChanged: (status: ServiceStatus) => void;
  onStatsTick: (stats: TrafficStats) => void;
  onConnectionEvent: (conn: ConnectionSnapshot) => void;
  onActivityLog: (entry: ActivityLogEntry) => void;
  onConfigChanged: (config: AppConfig) => void;
  onPipeState: (state: PipeConnectionState) => void;
  onProxyTestsStale: () => void;
  onWifiChanged: (wifi: CurrentWifiInfo) => void;
}

export interface CoreClient {
  subscribe(callbacks: CoreEventCallbacks): Promise<() => void>;
  ping(): Promise<PongInfo>;
  getPipeState(): Promise<PipeConnectionState>;
  getStatus(): Promise<ServiceStatus>;
  startEngine(): Promise<ServiceStatus>;
  stopEngine(): Promise<ServiceStatus>;
  getConfig(): Promise<AppConfig>;
  saveConfig(config: AppConfig): Promise<AppConfig>;
  restoreConfigBackup(): Promise<AppConfig>;
  setActiveProfile(profileId: string): Promise<AppConfig>;
  getConnections(
    includeClosed: boolean,
    limit: number | null,
  ): Promise<ConnectionSnapshot[]>;
  clearConnections(): Promise<void>;
  listRunningProcesses(): Promise<RunningProcessInfo[]>;
  listNetworkInterfaces(): Promise<NetworkInterfaceInfo[]>;
  testProxy(proxyId: string): Promise<ProxyTestResult>;
  getActivityLogs(limit: number | null): Promise<ActivityLogEntry[]>;
  clearActivityLogs(): Promise<void>;
  launchService(): Promise<boolean>;
  exportConfigBackup(includePasswords: boolean): Promise<ConfigBackupExport>;
  importConfigBackup(json: string): Promise<AppConfig>;
  exportRulesJson(profileId: string | null): Promise<string>;
  importRulesJson(
    json: string,
    profileId: string,
    dryRun: boolean,
  ): Promise<RulesImportPreview>;
  getTrafficHistory(query: TrafficHistoryQuery): Promise<TrafficHistoryResponse>;
  clearTrafficHistory(): Promise<void>;
  getCurrentWifi(): Promise<CurrentWifiInfo>;
  getCoexistenceConflicts(): Promise<CoexistenceConflict[]>;
  dismissCoexistenceConflict(conflictId: string): Promise<void>;
  collectDiagnosticBundle(outputPath: string | null): Promise<DiagnosticBundleSummary>;
}

export function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    "__TAURI_INTERNALS__" in window
  );
}

function tauriInvoke<K extends keyof TauriCommandMap>(
  cmd: K,
  args?: TauriCommandMap[K]["args"],
): Promise<TauriCommandMap[K]["returns"]> {
  return invoke<TauriCommandMap[K]["returns"]>(
    cmd,
    args as Record<string, unknown> | undefined,
  );
}

export class TauriCoreClient implements CoreClient {
  async subscribe(callbacks: CoreEventCallbacks): Promise<() => void> {
    const unlisteners: UnlistenFn[] = [];
    unlisteners.push(
      await listen<ServiceStatus>(TAURI_EVENTS.STATUS_CHANGED, (e) =>
        callbacks.onStatusChanged(e.payload),
      ),
    );
    unlisteners.push(
      await listen<TrafficStats>(TAURI_EVENTS.STATS_TICK, (e) =>
        callbacks.onStatsTick(e.payload),
      ),
    );
    unlisteners.push(
      await listen<ConnectionSnapshot>(TAURI_EVENTS.CONNECTION_EVENT, (e) =>
        callbacks.onConnectionEvent(e.payload),
      ),
    );
    unlisteners.push(
      await listen<ActivityLogEntry>(TAURI_EVENTS.ACTIVITY_LOG, (e) =>
        callbacks.onActivityLog(e.payload),
      ),
    );
    unlisteners.push(
      await listen<AppConfig>(TAURI_EVENTS.CONFIG_CHANGED, (e) =>
        callbacks.onConfigChanged(e.payload),
      ),
    );
    unlisteners.push(
      await listen<PipeConnectionState>(TAURI_EVENTS.PIPE_CONNECTION, (e) =>
        callbacks.onPipeState(e.payload),
      ),
    );
    unlisteners.push(
      await listen<void>(TAURI_EVENTS.PROXY_TESTS_STALE, () =>
        callbacks.onProxyTestsStale(),
      ),
    );
    unlisteners.push(
      await listen<CurrentWifiInfo>(TAURI_EVENTS.WIFI_CHANGED, (e) =>
        callbacks.onWifiChanged(e.payload),
      ),
    );
    return () => {
      for (const fn of unlisteners) {
        fn();
      }
    };
  }

  ping() {
    return tauriInvoke("core_ping");
  }
  getPipeState() {
    return tauriInvoke("core_pipe_state");
  }
  getStatus() {
    return tauriInvoke("core_get_status");
  }
  startEngine() {
    return tauriInvoke("core_start_engine");
  }
  stopEngine() {
    return tauriInvoke("core_stop_engine");
  }
  getConfig() {
    return tauriInvoke("core_get_config");
  }
  saveConfig(config: AppConfig) {
    return tauriInvoke("core_save_config", { config });
  }
  restoreConfigBackup() {
    return tauriInvoke("core_restore_config_backup");
  }
  setActiveProfile(profileId: string) {
    return tauriInvoke("core_set_active_profile", { profileId });
  }
  getConnections(includeClosed: boolean, limit: number | null) {
    return tauriInvoke("core_get_connections", { includeClosed, limit });
  }
  clearConnections() {
    return tauriInvoke("core_clear_connections");
  }
  listRunningProcesses() {
    return tauriInvoke("core_list_running_processes");
  }
  listNetworkInterfaces() {
    return tauriInvoke("core_list_network_interfaces");
  }
  testProxy(proxyId: string) {
    return tauriInvoke("core_test_proxy", { proxyId });
  }
  getActivityLogs(limit: number | null) {
    return tauriInvoke("core_get_activity_logs", { limit });
  }
  clearActivityLogs() {
    return tauriInvoke("core_clear_activity_logs");
  }
  launchService() {
    return tauriInvoke("core_launch_service");
  }
  exportConfigBackup(includePasswords: boolean) {
    return tauriInvoke("core_export_config_backup", { includePasswords });
  }
  importConfigBackup(json: string) {
    return tauriInvoke("core_import_config_backup", { json });
  }
  exportRulesJson(profileId: string | null) {
    return tauriInvoke("core_export_rules_json", { profileId });
  }
  importRulesJson(json: string, profileId: string, dryRun: boolean) {
    return tauriInvoke("core_import_rules_json", { json, profileId, dryRun });
  }
  getTrafficHistory(query: TrafficHistoryQuery) {
    return tauriInvoke("core_get_traffic_history", { query });
  }
  clearTrafficHistory() {
    return tauriInvoke("core_clear_traffic_history");
  }
  getCurrentWifi() {
    return tauriInvoke("core_get_current_wifi");
  }
  getCoexistenceConflicts() {
    return tauriInvoke("core_get_coexistence_conflicts");
  }
  dismissCoexistenceConflict(conflictId: string) {
    return tauriInvoke("core_dismiss_coexistence_conflict", { conflictId });
  }
  collectDiagnosticBundle(outputPath: string | null) {
    return tauriInvoke("core_collect_diagnostic_bundle", { outputPath });
  }
}

export function createCoreClient(useMock: boolean): CoreClient {
  if (useMock || !isTauriRuntime()) {
    return new MockCoreClient();
  }
  return new TauriCoreClient();
}
