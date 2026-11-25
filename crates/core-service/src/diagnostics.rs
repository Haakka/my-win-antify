use std::{
    collections::{BTreeMap, HashSet},
    ffi::c_void,
    path::{Path, PathBuf},
    ptr,
    sync::Mutex,
};

use serde_json::{json, Value};
use shared::{
    ActivityLogEntry, AppConfig, CoexistenceConflict, CurrentWifiInfo, DiagnosticBundleSummary,
    IpcError, IpcErrorCode, ProxyProtocol, ServiceStatus,
};

use crate::{
    network_interfaces::{self, DiagnosticAdapter},
    now_ms,
};

const LOG_WINDOW_MS: i64 = 30 * 60 * 1000;
const TUN_KEYWORDS: &[&str] = &[
    "wintun",
    "clash",
    "flclash",
    "mihomo",
    "tailscale",
    "wireguard",
    "sing-box",
    "tap-windows",
    "nekoray",
    "v2rayn",
];

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenCurrentUser(access: u32, key: *mut isize) -> i32;
    fn RegGetValueW(
        key: isize,
        subkey: *const u16,
        name: *const u16,
        flags: u32,
        value_type: *mut u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

#[derive(Clone, Copy)]
struct SystemProxy {
    enabled: Option<bool>,
    server_configured: Option<bool>,
}

fn system_proxy() -> SystemProxy {
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const RRF_RT_REG_SZ: u32 = 0x02;
    const RRF_RT_REG_DWORD: u32 = 0x10;
    let mut key = 0;
    let mut result = SystemProxy {
        enabled: None,
        server_configured: None,
    };
    if unsafe { RegOpenCurrentUser(KEY_QUERY_VALUE, &mut key) } != 0 {
        return result;
    }
    let settings: Vec<_> = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings"
        .encode_utf16()
        .chain([0])
        .collect();
    let get = |name: &str, flags, data, size: &mut u32| {
        let name: Vec<_> = name.encode_utf16().chain([0]).collect();
        unsafe {
            RegGetValueW(
                key,
                settings.as_ptr(),
                name.as_ptr(),
                flags,
                ptr::null_mut(),
                data,
                size,
            )
        }
    };
    let mut enabled = 0u32;
    let mut size = size_of::<u32>() as u32;
    match get(
        "ProxyEnable",
        RRF_RT_REG_DWORD,
        (&mut enabled as *mut u32).cast(),
        &mut size,
    ) {
        0 => result.enabled = Some(enabled == 1),
        2 => result.enabled = Some(false),
        _ => {}
    }
    let mut size = 0;
    match get("ProxyServer", RRF_RT_REG_SZ, ptr::null_mut(), &mut size) {
        0 if size <= 64 * 1024 => {
            let mut value = vec![0u16; (size as usize).div_ceil(2) + 1];
            let mut capacity = (value.len() * 2) as u32;
            if get(
                "ProxyServer",
                RRF_RT_REG_SZ,
                value.as_mut_ptr().cast(),
                &mut capacity,
            ) == 0
            {
                let length = value.iter().position(|&ch| ch == 0).unwrap_or(value.len());
                result.server_configured =
                    Some(!String::from_utf16_lossy(&value[..length]).trim().is_empty());
            }
        }
        2 => result.server_configured = Some(false),
        _ => {}
    }
    unsafe { RegCloseKey(key) };
    result
}

pub(crate) fn detect_conflicts(dismissed: &HashSet<String>) -> Vec<CoexistenceConflict> {
    let proxy = system_proxy();
    let interfaces = network_interfaces::diagnostic_adapters().unwrap_or_default();
    conflicts(proxy, &interfaces, dismissed)
}

fn conflicts(
    proxy: SystemProxy,
    interfaces: &[DiagnosticAdapter],
    dismissed: &HashSet<String>,
) -> Vec<CoexistenceConflict> {
    let mut conflicts = Vec::new();
    if proxy.enabled == Some(true) && proxy.server_configured == Some(true) {
        conflicts.push(CoexistenceConflict {
            id: "system-proxy".into(),
            kind: "systemProxy".into(),
            severity: "warn".into(),
            title: "Windows system proxy is enabled".into(),
            detail: "Apps that honor the Windows proxy may route through both proxies.".into(),
            dismissed: false,
        });
    }
    for adapter in interfaces.iter().filter(|adapter| adapter.is_up) {
        let names = format!("{}\n{}", adapter.name, adapter.description).to_lowercase();
        if TUN_KEYWORDS.iter().any(|keyword| names.contains(keyword)) {
            let name = if adapter.name.is_empty() {
                &adapter.description
            } else {
                &adapter.name
            };
            conflicts.push(CoexistenceConflict {
                id: format!("tun-{}", name.to_lowercase()),
                kind: "tunAdapter".into(),
                severity: "high".into(),
                title: format!("Active virtual network adapter: {name}"),
                detail: adapter.description.clone(),
                dismissed: false,
            });
        }
    }
    for conflict in &mut conflicts {
        conflict.dismissed = dismissed.contains(&conflict.id);
    }
    conflicts.sort_unstable_by(|a, b| {
        (a.severity != "high")
            .cmp(&(b.severity != "high"))
            .then_with(|| a.id.cmp(&b.id))
    });
    conflicts
}

pub(crate) struct DiagnosticData {
    pub status: ServiceStatus,
    pub config: AppConfig,
    pub wifi: CurrentWifiInfo,
    pub logs: Vec<ActivityLogEntry>,
}

pub(crate) fn collect_diagnostic_bundle(
    config_path: &Path,
    output_path: Option<PathBuf>,
    data: DiagnosticData,
    write: impl FnOnce(&Path, &[u8]) -> Result<(), IpcError>,
) -> Result<DiagnosticBundleSummary, IpcError> {
    let timestamp = now_ms();
    let interfaces = network_interfaces::diagnostic_adapters();
    let proxy = system_proxy();
    let (bundle, log_count) = build_bundle(&data, interfaces.as_deref().ok(), proxy, timestamp);
    let path = output_path.unwrap_or_else(|| {
        static LAST_NAME_MS: Mutex<i64> = Mutex::new(0);
        let mut last = LAST_NAME_MS.lock().unwrap();
        *last = timestamp.max(last.saturating_add(1));
        config_path
            .with_file_name("diagnostics")
            .join(format!("diagnostic-{last}.json"))
    });
    let bytes = serde_json::to_vec_pretty(&bundle)
        .map_err(|error| IpcError::new(IpcErrorCode::Internal, error.to_string()))?;
    write(&path, &bytes)?;
    Ok(DiagnosticBundleSummary {
        path: path.to_string_lossy().into_owned(),
        generated_at_ms: timestamp,
        log_count,
        interface_count: interfaces.map_or(0, |interfaces| interfaces.len()),
        sanitized_fields: [
            "proxy.host",
            "proxy.port",
            "proxy.auth.username",
            "proxy.auth.password",
            "wifi.ssid",
            "localUserPaths",
            "networkAddresses",
            "systemDnsServers",
            "systemProxyServer",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    })
}

fn build_bundle(
    data: &DiagnosticData,
    interfaces: Option<&[DiagnosticAdapter]>,
    proxy: SystemProxy,
    timestamp: i64,
) -> (Value, usize) {
    let mut counts = BTreeMap::<&str, usize>::new();
    for proxy in &data.config.proxies {
        let protocol = match proxy.protocol {
            ProxyProtocol::Socks5 => "socks5",
            ProxyProtocol::Http => "http",
            ProxyProtocol::Https => "https",
            ProxyProtocol::DirectInterface => "directInterface",
        };
        *counts.entry(protocol).or_default() += 1;
    }
    let logs: Vec<_> = data
        .logs
        .iter()
        .filter(|log| (timestamp - LOG_WINDOW_MS..=timestamp).contains(&log.timestamp_ms))
        .collect();
    let names: Vec<_> = interfaces
        .unwrap_or_default()
        .iter()
        .map(|interface| {
            json!({
                "name": interface.name,
                "isUp": interface.is_up,
                "ipv4Subnet": interface.ipv4.map(|ip| {
                    let [first, second, _, _] = ip.octets();
                    format!("{first}.{second}.0.0/16")
                }),
            })
        })
        .collect();
    let mut bundle = json!({
        "generatedAtMs": timestamp,
        "serviceStatus": data.status,
        "configSummary": {
            "schemaVersion": data.config.schema_version,
            "profileCount": data.config.profiles.len(),
            "ruleCount": data.config.rules.len(),
            "proxyProtocolCounts": counts,
        },
        "activityLogs": logs,
        "network": {
            "interfaces": names,
            "adapterQueryAvailable": interfaces.is_some(),
            "activeDnsServerCount": interfaces.map(|interfaces| interfaces.iter()
                .filter(|interface| interface.is_up)
                .map(|interface| interface.dns_servers).sum::<usize>()),
            "systemProxyEnabled": proxy.enabled,
            "systemProxyServerConfigured": proxy.server_configured,
        },
    });
    let redactions = redactions(data);
    sanitize(&mut bundle, &redactions);
    (bundle, logs.len())
}

fn redactions(data: &DiagnosticData) -> HashSet<String> {
    let mut values = HashSet::new();
    let mut add = |value: &str| {
        if !value.is_empty() {
            values.insert(value.to_lowercase());
            let quoted =
                serde_json::to_string(value).expect("diagnostic secret serializes to JSON");
            values.insert(quoted[1..quoted.len() - 1].to_lowercase());
        }
    };
    for proxy in &data.config.proxies {
        add(&proxy.host);
        if let Some(host) = proxy
            .host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
        {
            add(host);
        }
        if let Some(auth) = &proxy.auth {
            add(&auth.username);
            add(&auth.password);
        }
    }
    for rule in &data.config.automation.wifi_rules {
        add(&rule.ssid);
        add(rule.ssid.trim());
    }
    if let Some(ssid) = &data.wifi.ssid {
        add(ssid);
        add(ssid.trim());
    }
    values
}

fn sanitize(value: &mut Value, secrets: &HashSet<String>) {
    match value {
        Value::String(text) => {
            let folded = text.to_lowercase();
            if secrets.iter().any(|secret| folded.contains(secret)) || contains_user_path(&folded) {
                // Drop the entire field so overlapping secrets cannot leave fragments behind.
                text.clear();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|value| sanitize(value, secrets)),
        // All keys are fixed schema keys, including the protocol count keys.
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|value| sanitize(value, secrets)),
        _ => {}
    }
}

fn contains_user_path(folded: &str) -> bool {
    let path = folded.replace('/', "\\");
    path.match_indices(r":\users\").any(|(index, prefix)| {
        path.as_bytes()
            .get(index.wrapping_sub(1))
            .is_some_and(u8::is_ascii_alphabetic)
            && path
                .as_bytes()
                .get(index + prefix.len())
                .is_some_and(|ch| *ch != b'\\')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{LogCategory, LogLevel, ProxyAuth, ProxyConfig, WifiProfileRule};

    fn adapter(name: &str, description: &str, is_up: bool) -> DiagnosticAdapter {
        DiagnosticAdapter {
            name: name.into(),
            description: description.into(),
            is_up,
            ipv4: Some("192.168.1.46".parse().unwrap()),
            dns_servers: 2,
        }
    }

    #[test]
    fn sorts_high_severity_first_and_marks_dismissed_conflicts() {
        let proxy = SystemProxy {
            enabled: Some(true),
            server_configured: Some(true),
        };
        let interfaces = [
            adapter("zvpn", "WireGuard Tunnel", true),
            adapter("aclash", "Clash TUN", true),
            adapter("closed", "Wintun", false),
            adapter("Ethernet", "Ethernet Controller", true),
        ];
        let conflicts = conflicts(proxy, &interfaces, &HashSet::from(["tun-aclash".into()]));
        assert_eq!(
            conflicts
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["tun-aclash", "tun-zvpn", "system-proxy"]
        );
        assert!(conflicts[0].dismissed);
        assert!(!conflicts[1].dismissed);
        assert_eq!(conflicts[0].severity, "high");
        assert_eq!(
            (conflicts[2].kind.as_str(), conflicts[2].severity.as_str()),
            ("systemProxy", "warn")
        );
    }

    #[test]
    fn diagnostic_json_scrubs_secrets_in_every_text_field_and_limits_log_age() {
        let mut config = AppConfig::default();
        config.proxies.push(ProxyConfig {
            id: "upstream".into(),
            name: "NAME_IGNORED".into(),
            protocol: ProxyProtocol::Socks5,
            host: "private.proxy.example".into(),
            port: 31867,
            auth: Some(ProxyAuth {
                username: "confidential-user".into(),
                password: "secret\"with\\escapes".into(),
            }),
            interface_name: None,
            allow_lan: false,
            udp_relay: false,
            skip_tls_verify: false,
            fallback_direct: false,
            enabled: true,
        });
        config.automation.wifi_rules.push(WifiProfileRule {
            id: "private".into(),
            ssid: "SavedNetworkName".into(),
            profile_id: "default".into(),
            enabled: true,
        });
        let status = ServiceStatus {
            running: true,
            started_at_ms: Some(0),
            uptime_secs: 1,
            service_pid: 10,
            core_version: "0.4.0".into(),
            driver_ready: true,
            active_profile_id: "private.proxy.example".into(),
            last_error: Some(r"cannot open C:\Users\Private Person\file.log".into()),
            config_error: Some("SAVEDNETWORKNAME".into()),
            invalid_rule_ids: vec!["confidential-user".into()],
        };
        let timestamp = now_ms();
        let recent = ActivityLogEntry {
            id: 1, timestamp_ms: timestamp, level: LogLevel::Info, category: LogCategory::App,
            title: "CURRENTNETWORKNAME mentioned in a log title".into(),
            detail: Some(r#"PRIVATE.PROXY.EXAMPLE confidential-user secret"with\escapes D:/Users/Another Person/file SavedNetworkName"#.into()),
        };
        let old = ActivityLogEntry {
            id: 2,
            timestamp_ms: timestamp - LOG_WINDOW_MS - 1,
            title: "OLD_MARKER".into(),
            detail: None,
            ..recent.clone()
        };
        let safe = ActivityLogEntry {
            id: 3,
            title: "Safe activity".into(),
            detail: Some("Adapter enumeration complete".into()),
            ..recent.clone()
        };
        let data = DiagnosticData {
            config,
            status,
            logs: vec![recent, old, safe],
            wifi: CurrentWifiInfo {
                supported: true,
                connected: true,
                interface_name: None,
                ssid: Some("CurrentNetworkName".into()),
            },
        };
        let adapters = [
            adapter(
                "Network containing CurrentNetworkName",
                "Unused description",
                true,
            ),
            adapter("Ethernet", "Unused description", false),
        ];
        let proxy = SystemProxy {
            enabled: Some(true),
            server_configured: Some(true),
        };
        let (bundle, count) = build_bundle(&data, Some(&adapters), proxy, timestamp);
        assert_eq!(count, 2);
        let json = bundle.to_string().to_lowercase();
        for forbidden in [
            "private.proxy.example",
            "confidential-user",
            r#"secret\"with\\escapes"#,
            "savednetworkname",
            "currentnetworkname",
            "private person",
            "another person",
            "31867",
            "name_ignored",
            "old_marker",
            "192.168.1.46",
        ] {
            assert!(!json.contains(forbidden), "Leaked {forbidden}");
        }
        assert_eq!(bundle["network"]["activeDnsServerCount"], 2);
        assert_eq!(
            bundle["network"]["interfaces"][0]["ipv4Subnet"],
            "192.168.0.0/16"
        );
        assert_eq!(bundle["activityLogs"][1]["title"], "Safe activity");
        assert_eq!(bundle["configSummary"]["proxyProtocolCounts"]["socks5"], 1);

        let dir = std::env::temp_dir().join(format!(
            "antify-diagnostic-{}-{timestamp}",
            std::process::id()
        ));
        let summary = collect_diagnostic_bundle(
            &dir.join("config.json"),
            None,
            data,
            crate::config::atomic_write,
        )
        .unwrap();
        assert_eq!(summary.log_count, 2);
        let path = PathBuf::from(summary.path);
        assert_eq!(path.parent().unwrap(), dir.join("diagnostics"));
        assert!(summary
            .sanitized_fields
            .iter()
            .any(|field| field == "wifi.ssid"));
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(!contents.to_lowercase().contains("private.proxy.example"));
        assert!(serde_json::from_str::<Value>(&contents).is_ok());
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
