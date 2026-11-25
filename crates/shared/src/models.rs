use serde::{Deserialize, Serialize};

/// Supported upstream proxy protocols.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyProtocol {
    Socks5,
    Http,
    Https,
    DirectInterface,
}

/// Optional username/password credentials for an upstream proxy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyAuth {
    pub username: String,
    pub password: String,
}

/// Upstream proxy node definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    pub id: String,
    pub name: String,
    pub protocol: ProxyProtocol,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub auth: Option<ProxyAuth>,
    #[serde(default)]
    pub interface_name: Option<String>,
    #[serde(default)]
    pub allow_lan: bool,
    #[serde(default = "default_true")]
    pub udp_relay: bool,
    #[serde(default)]
    pub fallback_direct: bool,
    #[serde(default)]
    pub skip_tls_verify: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Health classification for a configured proxy node.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyHealthStatus {
    Unknown,
    Healthy,
    Degraded,
    Unreachable,
    Stale,
}

/// Sliding-window health telemetry for a proxy node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyHealthSnapshot {
    pub proxy_id: String,
    pub score: u8,
    pub status: ProxyHealthStatus,
    #[serde(default)]
    pub last_error_code: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// Result of testing reachability & handshake latency against an upstream proxy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTestResult {
    pub proxy_id: String,
    pub reachable: bool,
    #[serde(default)]
    pub latency_ms: Option<u32>,
    #[serde(default)]
    pub error: Option<String>,
    pub checked_at_ms: i64,
    #[serde(default)]
    pub stale: bool,
}

/// Process/application matcher for a routing rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RuleMatcher {
    /// Exact normalized executable path (case-insensitive on Windows).
    ExecutablePath {
        path: String,
        #[serde(default = "default_true")]
        include_children: bool,
    },
    /// Process executable file name, e.g. `curl.exe` or `Code.exe`.
    ProcessName {
        name: String,
        #[serde(default = "default_true")]
        include_children: bool,
    },
    /// All executables residing under the given directory prefix.
    Directory {
        dir: String,
        #[serde(default = "default_true")]
        include_children: bool,
    },
}

/// Traffic routing action when a rule (or profile default) matches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuleAction {
    Proxy,
    Direct,
    Block,
}

/// Per-application routing rule belonging to a profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub profile_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub name: String,
    pub matcher: RuleMatcher,
    pub action: RuleAction,
    #[serde(default)]
    pub proxy_id: Option<String>,
    /// When true, proxy failures fail closed instead of falling back to direct.
    #[serde(default)]
    pub network_lock: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub target_hosts: Vec<String>,
    #[serde(default)]
    pub target_ports: Vec<String>,
}

/// Configuration profile (e.g., "General", "Work", "Gaming").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub default_action: RuleAction,
    #[serde(default)]
    pub default_proxy_id: Option<String>,
}

/// UI color theme preference.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

/// Global application settings persisted inside `AppConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSettings {
    pub theme: ThemeMode,
    pub language: String,
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default)]
    pub auto_start_proxy: bool,
    #[serde(default = "default_true")]
    pub bypass_lan: bool,
    #[serde(default)]
    pub bypass_hosts: Vec<String>,
    #[serde(default = "default_true")]
    pub record_connections: bool,
    #[serde(default)]
    pub discover_unmatched: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Light,
            language: "zh-CN".to_string(),
            launch_at_login: false,
            auto_start_proxy: false,
            bypass_lan: true,
            bypass_hosts: Vec::new(),
            record_connections: true,
            discover_unmatched: false,
        }
    }
}

/// Wi-Fi SSID to profile auto-switching rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WifiProfileRule {
    pub id: String,
    pub ssid: String,
    pub profile_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Automation rules configuration persisted inside `AppConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub wifi_rules: Vec<WifiProfileRule>,
}

/// Live Windows Wi-Fi connection status returned by `GetCurrentWifi`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentWifiInfo {
    pub supported: bool,
    pub connected: bool,
    #[serde(default)]
    pub ssid: Option<String>,
    #[serde(default)]
    pub interface_name: Option<String>,
}

/// Time range selector for persisted traffic history queries.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrafficHistoryRange {
    #[serde(rename = "24h")]
    Last24h,
    #[serde(rename = "month")]
    Month,
}

/// Query parameters for `RpcRequest::GetTrafficHistory`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrafficHistoryQuery {
    pub range: TrafficHistoryRange,
    #[serde(default)]
    pub profile_id: Option<String>,
}

/// Aggregated traffic bytes for a single rule, proxy, or application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrafficDimensionItem {
    pub key: String,
    pub label: String,
    pub up_bytes: u64,
    pub down_bytes: u64,
}

/// Single time bucket in a persisted traffic history response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrafficHistoryBucket {
    pub bucket_start_ms: i64,
    pub label: String,
    pub proxied_up: u64,
    pub proxied_down: u64,
    pub direct_up: u64,
    pub direct_down: u64,
}

/// Historical traffic summary and breakdown response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrafficHistoryResponse {
    pub range: TrafficHistoryRange,
    #[serde(default)]
    pub profile_id: Option<String>,
    pub total_proxied_up: u64,
    pub total_proxied_down: u64,
    pub total_direct_up: u64,
    pub total_direct_down: u64,
    pub buckets: Vec<TrafficHistoryBucket>,
    #[serde(default)]
    pub top_rules: Vec<TrafficDimensionItem>,
    #[serde(default)]
    pub top_proxies: Vec<TrafficDimensionItem>,
    #[serde(default)]
    pub top_apps: Vec<TrafficDimensionItem>,
}

/// Detected system proxy or TUN virtual NIC coexistence warning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoexistenceConflict {
    pub id: String,
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub detail: String,
    #[serde(default)]
    pub dismissed: bool,
}

/// Result of exporting a full `AppConfig` backup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigBackupExport {
    pub json: String,
    pub includes_passwords: bool,
    #[serde(default)]
    pub warning: Option<String>,
}

/// Result of previewing or committing a rules JSON import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RulesImportPreview {
    pub committed: bool,
    pub total_in_file: usize,
    pub added: Vec<String>,
    pub skipped_duplicates: Vec<String>,
    pub missing_proxy_rules: Vec<String>,
}

/// Summary returned after collecting and writing a sanitized diagnostic bundle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticBundleSummary {
    pub path: String,
    pub generated_at_ms: i64,
    pub log_count: usize,
    pub interface_count: usize,
    pub sanitized_fields: Vec<String>,
}

/// Full persisted configuration shared between `core-service` and the UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub schema_version: u32,
    pub active_profile_id: String,
    pub profiles: Vec<Profile>,
    pub proxies: Vec<ProxyConfig>,
    pub rules: Vec<Rule>,
    pub settings: GeneralSettings,
    #[serde(default)]
    pub automation: AutomationConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: crate::ipc::CONFIG_SCHEMA_VERSION,
            active_profile_id: "default".to_string(),
            profiles: vec![Profile {
                id: "default".to_string(),
                name: "General".to_string(),
                default_action: RuleAction::Direct,
                default_proxy_id: None,
            }],
            proxies: Vec::new(),
            rules: Vec::new(),
            settings: GeneralSettings::default(),
            automation: AutomationConfig::default(),
        }
    }
}

/// Runtime status of `core-service` and its WinDivert traffic engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub running: bool,
    #[serde(default)]
    pub started_at_ms: Option<i64>,
    pub uptime_secs: u64,
    pub service_pid: u32,
    pub core_version: String,
    pub driver_ready: bool,
    pub active_profile_id: String,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub config_error: Option<String>,
    #[serde(default)]
    pub invalid_rule_ids: Vec<String>,
}

/// Transport protocol of an observed connection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionProtocol {
    Tcp,
    Udp,
}

/// Lifecycle state of an observed connection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Connecting,
    Active,
    Closed,
    Failed,
}

/// Snapshot of a single network connection handled or observed by the core service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSnapshot {
    pub id: u64,
    pub timestamp_ms: i64,
    pub protocol: ConnectionProtocol,
    pub pid: u32,
    pub process_name: String,
    pub executable_path: String,
    pub app_name: String,
    pub src_addr: String,
    pub dst_addr: String,
    #[serde(default)]
    pub domain: Option<String>,
    pub action: RuleAction,
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default)]
    pub rule_name: Option<String>,
    #[serde(default)]
    pub proxy_id: Option<String>,
    #[serde(default)]
    pub proxy_name: Option<String>,
    pub state: ConnectionState,
    pub bytes_up: u64,
    pub bytes_down: u64,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub match_reason: Option<String>,
    #[serde(default)]
    pub failure_code: Option<String>,
}

/// Last-matched timestamp update for a rule, emitted inside `TrafficStats`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuleMatchTick {
    pub rule_id: String,
    pub last_matched_at_ms: i64,
}

/// 1Hz telemetry snapshot pushed from `core-service` to connected clients.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrafficStats {
    pub timestamp_ms: i64,
    pub running: bool,
    pub uptime_secs: u64,
    pub active_connections: u32,
    pub active_apps: u32,
    pub upload_bps: u64,
    pub download_bps: u64,
    pub total_proxied_up: u64,
    pub total_proxied_down: u64,
    pub total_direct_up: u64,
    pub total_direct_down: u64,
    #[serde(default)]
    pub rule_last_matched: Vec<RuleMatchTick>,
    #[serde(default)]
    pub proxy_health: Vec<ProxyHealthSnapshot>,
}

/// Running Windows process metadata returned for the rule picker modal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunningProcessInfo {
    pub pid: u32,
    #[serde(default)]
    pub parent_pid: Option<u32>,
    pub name: String,
    pub executable_path: String,
    pub app_name: String,
    pub has_active_connections: bool,
}

/// Active Windows IPv4 network adapter metadata for proxy outbound interface selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterfaceInfo {
    pub name: String,
    pub description: String,
    pub ipv4: String,
    pub is_up: bool,
}

/// Severity level for activity log entries.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Source category for activity log entries.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LogCategory {
    Service,
    Proxy,
    Rule,
    App,
}

/// Activity / audit event emitted by `core-service`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityLogEntry {
    pub id: u64,
    pub timestamp_ms: i64,
    pub level: LogLevel,
    pub category: LogCategory,
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
}

const fn default_true() -> bool {
    true
}
