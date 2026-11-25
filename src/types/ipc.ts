/**
 * 1:1 TypeScript mirror of `crates/shared/src/` (error.rs, models.rs, ipc.rs).
 * Keep strictly synchronized with Rust `#[serde(rename_all = "camelCase")]` contracts.
 */

export const DEFAULT_PIPE_NAME = "\\\\.\\pipe\\my-win-antify-core-v1";
export const IPC_PROTOCOL_VERSION = 1;
export const CONFIG_SCHEMA_VERSION = 2;

export const TAURI_EVENTS = {
  STATUS_CHANGED: "core://status-changed",
  STATS_TICK: "core://stats-tick",
  CONNECTION_EVENT: "core://connection-event",
  ACTIVITY_LOG: "core://activity-log",
  CONFIG_CHANGED: "core://config-changed",
  PIPE_CONNECTION: "core://pipe-connection",
  PROXY_TESTS_STALE: "core://proxy-tests-stale",
  WIFI_CHANGED: "core://wifi-changed",
} as const;

export type IpcErrorCode =
  | "invalidRequest"
  | "notFound"
  | "validationFailed"
  | "driverUnavailable"
  | "accessDenied"
  | "proxyError"
  | "configIoError"
  | "engineAlreadyRunning"
  | "engineNotRunning"
  | "serviceDisconnected"
  | "internal";

export interface IpcError {
  code: IpcErrorCode;
  message: string;
}

export type ProxyProtocol = "socks5" | "http" | "https" | "directInterface";

export interface ProxyAuth {
  username: string;
  password: string;
}

export interface ProxyConfig {
  id: string;
  name: string;
  protocol: ProxyProtocol;
  host: string;
  port: number;
  auth: ProxyAuth | null;
  interfaceName: string | null;
  allowLan: boolean;
  udpRelay: boolean;
  fallbackDirect: boolean;
  skipTlsVerify: boolean;
  enabled: boolean;
}

export type ProxyHealthStatus =
  | "unknown"
  | "healthy"
  | "degraded"
  | "unreachable"
  | "stale";

export interface ProxyHealthSnapshot {
  proxyId: string;
  score: number;
  status: ProxyHealthStatus;
  lastErrorCode: string | null;
  lastError: string | null;
}

export interface ProxyTestResult {
  proxyId: string;
  reachable: boolean;
  latencyMs: number | null;
  error: string | null;
  checkedAtMs: number;
  stale: boolean;
}

export type RuleMatcher =
  | {
      kind: "executablePath";
      path: string;
      includeChildren: boolean;
    }
  | {
      kind: "processName";
      name: string;
      includeChildren: boolean;
    }
  | {
      kind: "directory";
      dir: string;
      includeChildren: boolean;
    };

export type RuleAction = "proxy" | "direct" | "block";

export interface Rule {
  id: string;
  profileId: string;
  enabled: boolean;
  name: string;
  matcher: RuleMatcher;
  action: RuleAction;
  proxyId: string | null;
  networkLock: boolean;
  priority: number;
  targetHosts: string[];
  targetPorts: string[];
}

export interface Profile {
  id: string;
  name: string;
  defaultAction: RuleAction;
  defaultProxyId: string | null;
}

export type ThemeMode = "system" | "light" | "dark";

export interface GeneralSettings {
  theme: ThemeMode;
  language: string;
  launchAtLogin: boolean;
  autoStartProxy: boolean;
  bypassLan: boolean;
  bypassHosts: string[];
  recordConnections: boolean;
  discoverUnmatched: boolean;
}

export interface WifiProfileRule {
  id: string;
  ssid: string;
  profileId: string;
  enabled: boolean;
}

export interface AutomationConfig {
  enabled: boolean;
  wifiRules: WifiProfileRule[];
}

export interface CurrentWifiInfo {
  supported: boolean;
  connected: boolean;
  ssid: string | null;
  interfaceName: string | null;
}

export type TrafficHistoryRange = "24h" | "month";

export interface TrafficHistoryQuery {
  range: TrafficHistoryRange;
  profileId: string | null;
}

export interface TrafficDimensionItem {
  key: string;
  label: string;
  upBytes: number;
  downBytes: number;
}

export interface TrafficHistoryBucket {
  bucketStartMs: number;
  label: string;
  proxiedUp: number;
  proxiedDown: number;
  directUp: number;
  directDown: number;
}

export interface TrafficHistoryResponse {
  range: TrafficHistoryRange;
  profileId: string | null;
  totalProxiedUp: number;
  totalProxiedDown: number;
  totalDirectUp: number;
  totalDirectDown: number;
  buckets: TrafficHistoryBucket[];
  topRules: TrafficDimensionItem[];
  topProxies: TrafficDimensionItem[];
  topApps: TrafficDimensionItem[];
}

export interface CoexistenceConflict {
  id: string;
  kind: string;
  severity: string;
  title: string;
  detail: string;
  dismissed: boolean;
}

export interface ConfigBackupExport {
  json: string;
  includesPasswords: boolean;
  warning: string | null;
}

export interface RulesImportPreview {
  committed: boolean;
  totalInFile: number;
  added: string[];
  skippedDuplicates: string[];
  missingProxyRules: string[];
}

export interface DiagnosticBundleSummary {
  path: string;
  generatedAtMs: number;
  logCount: number;
  interfaceCount: number;
  sanitizedFields: string[];
}

export interface AppConfig {
  schemaVersion: number;
  activeProfileId: string;
  profiles: Profile[];
  proxies: ProxyConfig[];
  rules: Rule[];
  settings: GeneralSettings;
  automation: AutomationConfig;
}

export interface ServiceStatus {
  running: boolean;
  startedAtMs: number | null;
  uptimeSecs: number;
  servicePid: number;
  coreVersion: string;
  driverReady: boolean;
  activeProfileId: string;
  lastError: string | null;
  configError: string | null;
  invalidRuleIds: string[];
}

export type ConnectionProtocol = "tcp" | "udp";
export type ConnectionState = "connecting" | "active" | "closed" | "failed";

export interface ConnectionSnapshot {
  id: number;
  timestampMs: number;
  protocol: ConnectionProtocol;
  pid: number;
  processName: string;
  executablePath: string;
  appName: string;
  srcAddr: string;
  dstAddr: string;
  domain: string | null;
  action: RuleAction;
  ruleId: string | null;
  ruleName: string | null;
  proxyId: string | null;
  proxyName: string | null;
  state: ConnectionState;
  bytesUp: number;
  bytesDown: number;
  error: string | null;
  matchReason: string | null;
  failureCode: string | null;
}

export interface RuleMatchTick {
  ruleId: string;
  lastMatchedAtMs: number;
}

export interface TrafficStats {
  timestampMs: number;
  running: boolean;
  uptimeSecs: number;
  activeConnections: number;
  activeApps: number;
  uploadBps: number;
  downloadBps: number;
  totalProxiedUp: number;
  totalProxiedDown: number;
  totalDirectUp: number;
  totalDirectDown: number;
  ruleLastMatched: RuleMatchTick[];
  proxyHealth: ProxyHealthSnapshot[];
}

export interface RunningProcessInfo {
  pid: number;
  parentPid: number | null;
  name: string;
  executablePath: string;
  appName: string;
  hasActiveConnections: boolean;
}

export interface NetworkInterfaceInfo {
  name: string;
  description: string;
  ipv4: string;
  isUp: boolean;
}

export type LogLevel = "info" | "warn" | "error";
export type LogCategory = "service" | "proxy" | "rule" | "app";

export interface ActivityLogEntry {
  id: number;
  timestampMs: number;
  level: LogLevel;
  category: LogCategory;
  title: string;
  detail: string | null;
}

export interface PongInfo {
  coreVersion: string;
  protocolVersion: number;
  servicePid: number;
}

export type RpcRequest =
  | { method: "ping" }
  | { method: "getStatus" }
  | { method: "startEngine" }
  | { method: "stopEngine" }
  | { method: "getConfig" }
  | { method: "saveConfig"; params: { config: AppConfig } }
  | { method: "restoreConfigBackup" }
  | { method: "setActiveProfile"; params: { profileId: string } }
  | {
      method: "getConnections";
      params: { includeClosed: boolean; limit: number | null };
    }
  | { method: "clearConnections" }
  | { method: "listRunningProcesses" }
  | { method: "listNetworkInterfaces" }
  | { method: "testProxy"; params: { proxyId: string } }
  | { method: "getActivityLogs"; params: { limit: number | null } }
  | { method: "clearActivityLogs" }
  | { method: "exportConfigBackup"; params: { includePasswords: boolean } }
  | { method: "importConfigBackup"; params: { json: string } }
  | { method: "exportRulesJson"; params: { profileId: string | null } }
  | {
      method: "importRulesJson";
      params: { json: string; profileId: string; dryRun: boolean };
    }
  | { method: "getTrafficHistory"; params: { query: TrafficHistoryQuery } }
  | { method: "clearTrafficHistory" }
  | { method: "getCurrentWifi" }
  | { method: "getCoexistenceConflicts" }
  | { method: "dismissCoexistenceConflict"; params: { conflictId: string } }
  | { method: "collectDiagnosticBundle"; params: { outputPath: string | null } };

export interface IpcRequest {
  id: number;
  request: RpcRequest;
}

export type RpcResult =
  | { kind: "pong"; data: PongInfo }
  | { kind: "status"; data: ServiceStatus }
  | { kind: "config"; data: AppConfig }
  | { kind: "connections"; data: ConnectionSnapshot[] }
  | { kind: "processes"; data: RunningProcessInfo[] }
  | { kind: "networkInterfaces"; data: NetworkInterfaceInfo[] }
  | { kind: "proxyTest"; data: ProxyTestResult }
  | { kind: "activityLogs"; data: ActivityLogEntry[] }
  | { kind: "configBackup"; data: ConfigBackupExport }
  | { kind: "rulesJson"; data: string }
  | { kind: "rulesImport"; data: RulesImportPreview }
  | { kind: "trafficHistory"; data: TrafficHistoryResponse }
  | { kind: "currentWifi"; data: CurrentWifiInfo }
  | { kind: "coexistenceConflicts"; data: CoexistenceConflict[] }
  | { kind: "diagnosticBundle"; data: DiagnosticBundleSummary }
  | { kind: "ack" };

export interface IpcResponse {
  id: number;
  ok: boolean;
  result: RpcResult | null;
  error: IpcError | null;
}

export type IpcEvent =
  | { event: "statusChanged"; data: ServiceStatus }
  | { event: "statsTick"; data: TrafficStats }
  | { event: "connectionEvent"; data: ConnectionSnapshot }
  | { event: "activityLog"; data: ActivityLogEntry }
  | { event: "configChanged"; data: AppConfig }
  | { event: "proxyTestsStale" }
  | { event: "wifiChanged"; data: CurrentWifiInfo };

export type ServerFrame =
  | { frame: "response"; payload: IpcResponse }
  | { frame: "event"; payload: IpcEvent };

export type PipeConnectionState = "connected" | "disconnected" | "reconnecting";

/**
 * Expected Tauri v2 invoke commands exposed by `src-tauri`.
 */
export interface TauriCommandMap {
  core_ping: { args: undefined; returns: PongInfo };
  core_get_status: { args: undefined; returns: ServiceStatus };
  core_start_engine: { args: undefined; returns: ServiceStatus };
  core_stop_engine: { args: undefined; returns: ServiceStatus };
  core_get_config: { args: undefined; returns: AppConfig };
  core_save_config: { args: { config: AppConfig }; returns: AppConfig };
  core_restore_config_backup: { args: undefined; returns: AppConfig };
  core_set_active_profile: { args: { profileId: string }; returns: AppConfig };
  core_get_connections: {
    args: { includeClosed: boolean; limit: number | null };
    returns: ConnectionSnapshot[];
  };
  core_clear_connections: { args: undefined; returns: void };
  core_list_running_processes: {
    args: undefined;
    returns: RunningProcessInfo[];
  };
  core_list_network_interfaces: {
    args: undefined;
    returns: NetworkInterfaceInfo[];
  };
  core_test_proxy: { args: { proxyId: string }; returns: ProxyTestResult };
  core_get_activity_logs: {
    args: { limit: number | null };
    returns: ActivityLogEntry[];
  };
  core_clear_activity_logs: { args: undefined; returns: void };
  core_pipe_state: { args: undefined; returns: PipeConnectionState };
  core_launch_service: { args: undefined; returns: boolean };
  core_export_config_backup: {
    args: { includePasswords: boolean };
    returns: ConfigBackupExport;
  };
  core_import_config_backup: { args: { json: string }; returns: AppConfig };
  core_export_rules_json: {
    args: { profileId: string | null };
    returns: string;
  };
  core_import_rules_json: {
    args: { json: string; profileId: string; dryRun: boolean };
    returns: RulesImportPreview;
  };
  core_get_traffic_history: {
    args: { query: TrafficHistoryQuery };
    returns: TrafficHistoryResponse;
  };
  core_clear_traffic_history: { args: undefined; returns: void };
  core_get_current_wifi: { args: undefined; returns: CurrentWifiInfo };
  core_get_coexistence_conflicts: {
    args: undefined;
    returns: CoexistenceConflict[];
  };
  core_dismiss_coexistence_conflict: {
    args: { conflictId: string };
    returns: void;
  };
  core_collect_diagnostic_bundle: {
    args: { outputPath: string | null };
    returns: DiagnosticBundleSummary;
  };
}
