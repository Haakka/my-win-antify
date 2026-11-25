use serde::{Deserialize, Serialize};

/// Supported upstream proxy protocols.
/// Milestone 1 implements `Socks5`; `Http` is reserved for Milestone 2+.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyProtocol {
    Socks5,
    Http,
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
    #[serde(default = "default_true")]
    pub enabled: bool,
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
