use std::{
    collections::{HashMap, HashSet},
    fs, io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use shared::{
    AppConfig, ConfigBackupExport, IpcError, IpcErrorCode, ProxyProtocol, Rule, RuleAction,
    RuleMatcher, RulesImportPreview, CONFIG_SCHEMA_VERSION,
};

use crate::{
    process::{normalized_path, Rules},
    targets::{BypassHosts, Targets},
};

const BACKUP_COUNT: usize = 3;

pub(crate) fn default_path() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into()))
        .join("MyWinAntify")
        .join("config.json")
}

pub(crate) fn load_with_diagnostics(path: &Path) -> Result<(AppConfig, Vec<String>), IpcError> {
    let mut config = match read_optional(path)? {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|error| config_io(path, error))?,
        None => {
            let config = AppConfig::default();
            save(path, &config)?;
            config
        }
    };
    migrate(&mut config);
    validate_structure(&config).map_err(|error| config_io(path, error.message))?;
    let invalid_rule_ids = invalid_rule_ids(&config);
    Ok((config, invalid_rule_ids))
}

pub(crate) fn save(path: &Path, config: &AppConfig) -> Result<(), IpcError> {
    let bytes = serde_json::to_vec_pretty(config).map_err(|error| config_io(path, error))?;
    if let Some(previous) = read_optional(path)? {
        for index in (2..=BACKUP_COUNT).rev() {
            if let Some(bytes) = read_optional(&backup_path(path, index - 1))? {
                atomic_write(&backup_path(path, index), &bytes)?;
            }
        }
        atomic_write(&backup_path(path, 1), &previous)?;
    }
    atomic_write(path, &bytes)
}

pub(crate) fn restore_latest_backup(path: &Path) -> Result<AppConfig, IpcError> {
    for index in 1..=BACKUP_COUNT {
        let backup = backup_path(path, index);
        let bytes = match read_optional(&backup) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(%error, "Could not read config backup");
                continue;
            }
        };
        let Ok(mut config) = serde_json::from_slice(&bytes) else {
            continue;
        };
        migrate(&mut config);
        if validate(&config).is_ok() {
            // Preserve the backup chain, including the copy used for this recovery.
            let bytes =
                serde_json::to_vec_pretty(&config).map_err(|error| config_io(path, error))?;
            atomic_write(path, &bytes)?;
            return Ok(config);
        }
    }
    Err(config_io(
        path,
        "No valid config backup was found in .bak.1 through .bak.3",
    ))
}

fn backup_path(path: &Path, index: usize) -> PathBuf {
    sibling(path, &format!(".bak.{index}"))
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, IpcError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(config_io(path, error)),
    }
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), IpcError> {
    crate::atomic_file::write(path, bytes).map_err(|error| config_io(path, error))
}

fn config_io(path: &Path, error: impl std::fmt::Display) -> IpcError {
    IpcError::new(
        IpcErrorCode::ConfigIoError,
        format!("{}: {error}", path.display()),
    )
}

fn ids<'a>(
    values: impl Iterator<Item = &'a str>,
    kind: &str,
) -> Result<HashSet<&'a str>, IpcError> {
    let mut unique = HashSet::new();
    for id in values {
        if id.trim().is_empty() || !unique.insert(id) {
            return Err(validation(format!(
                "{kind} IDs must be nonempty and unique"
            )));
        }
    }
    Ok(unique)
}

pub(crate) fn validate(config: &AppConfig) -> Result<(), IpcError> {
    validate_structure(config)?;
    for rule in &config.rules {
        Targets::parse(rule).map_err(|error| validation(format!("Rule {}: {error}", rule.id)))?;
    }
    Ok(())
}

fn migrate(config: &mut AppConfig) {
    if config.schema_version == 1 {
        config.schema_version = CONFIG_SCHEMA_VERSION;
    }
}

pub(crate) fn export_config_backup(
    config: &AppConfig,
    include_passwords: bool,
) -> ConfigBackupExport {
    let mut backup = config.clone();
    if !include_passwords {
        for proxy in &mut backup.proxies {
            proxy.auth = None;
        }
    }
    ConfigBackupExport {
        json: serde_json::to_string_pretty(&backup).expect("AppConfig serializes to JSON"),
        includes_passwords: include_passwords,
        warning: include_passwords.then(|| {
            "Plaintext proxy credentials are included in this backup. Store it securely.".into()
        }),
    }
}

pub(crate) fn import_config_backup(json: &str) -> Result<AppConfig, IpcError> {
    let mut backup: serde_json::Value = serde_json::from_str(json).map_err(invalid_json)?;
    if let Some(fields) = backup.as_object_mut() {
        fields.entry("activeProfileId").or_insert_with(|| "".into());
    }
    let mut config: AppConfig = serde_json::from_value(backup).map_err(invalid_json)?;
    migrate(&mut config);
    if let Some(first) = config.profiles.first() {
        if !config
            .profiles
            .iter()
            .any(|profile| profile.id == config.active_profile_id)
        {
            config.active_profile_id.clone_from(&first.id);
        }
    }
    validate(&config)?;
    Ok(config)
}

fn invalid_json(error: serde_json::Error) -> IpcError {
    validation(format!("Invalid JSON: {error}"))
}

const PROFILE_TOKEN: &str = "%USERPROFILE%";

fn matcher_path(matcher: &mut RuleMatcher) -> Option<&mut String> {
    match matcher {
        RuleMatcher::ExecutablePath { path, .. } => Some(path),
        RuleMatcher::Directory { dir, .. } => Some(dir),
        RuleMatcher::ProcessName { .. } => None,
    }
}

fn path_suffix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let head = path.get(..prefix.len())?;
    let tail = &path[prefix.len()..];
    (head.to_lowercase() == prefix.to_lowercase() && (tail.is_empty() || tail.starts_with('\\')))
        .then_some(tail)
}

fn portable_path(path: &str, user_profile: Option<&str>) -> String {
    let normalized = path.trim().replace('/', "\\");
    let normalized = normalized.strip_prefix(r"\\?\").unwrap_or(&normalized);
    if let Some(tail) = user_profile.and_then(|profile| path_suffix(normalized, profile)) {
        return format!("{PROFILE_TOKEN}{tail}");
    }
    if normalized
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && normalized
            .get(1..9)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r":\Users\"))
    {
        let rest = &normalized[9..];
        let user_end = rest.find('\\').unwrap_or(rest.len());
        if user_end != 0 {
            return format!("{PROFILE_TOKEN}{}", &rest[user_end..]);
        }
    }
    path.into()
}

fn user_profile() -> Option<String> {
    std::env::var("USERPROFILE").ok().and_then(|profile| {
        let profile = profile
            .trim()
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\");
        (!profile.is_empty()).then_some(profile)
    })
}

pub(crate) fn export_rules_json(config: &AppConfig, profile_id: Option<&str>) -> String {
    let profile_id = profile_id.unwrap_or(&config.active_profile_id);
    let profile = user_profile();
    let mut rules: Vec<_> = config
        .rules
        .iter()
        .filter(|rule| rule.profile_id == profile_id)
        .cloned()
        .collect();
    for rule in &mut rules {
        if let Some(path) = matcher_path(&mut rule.matcher) {
            *path = portable_path(path, profile.as_deref());
        }
    }
    serde_json::to_string_pretty(&rules).expect("Rule serializes to JSON")
}

#[derive(PartialEq, Eq, Hash)]
enum MatcherTarget {
    Executable(String),
    Process(String),
    Directory(String),
}

#[derive(PartialEq, Eq, Hash)]
struct MatcherKey {
    target: MatcherTarget,
    hosts: Vec<String>,
    ports: Vec<String>,
}

impl From<&Rule> for MatcherKey {
    fn from(rule: &Rule) -> Self {
        let target = match &rule.matcher {
            RuleMatcher::ExecutablePath { path, .. } => {
                MatcherTarget::Executable(normalized_path(path))
            }
            RuleMatcher::ProcessName { name, .. } => {
                MatcherTarget::Process(name.trim().to_lowercase())
            }
            RuleMatcher::Directory { dir, .. } => MatcherTarget::Directory(normalized_path(dir)),
        };
        let normalize = |items: &[String]| {
            let mut values: Vec<_> = items
                .iter()
                .map(|item| item.trim().to_lowercase())
                .collect();
            values.sort_unstable();
            values.dedup();
            values
        };
        Self {
            target,
            hosts: normalize(&rule.target_hosts),
            ports: normalize(&rule.target_ports),
        }
    }
}

pub(crate) fn import_rules_json(
    config: &AppConfig,
    json: &str,
    profile_id: &str,
) -> Result<(AppConfig, RulesImportPreview), IpcError> {
    if !config
        .profiles
        .iter()
        .any(|profile| profile.id == profile_id)
    {
        return Err(validation("Target profile does not exist"));
    }
    let rules: Vec<Rule> = serde_json::from_str(json).map_err(invalid_json)?;
    let mut preview = RulesImportPreview {
        committed: false,
        total_in_file: rules.len(),
        added: Vec::new(),
        skipped_duplicates: Vec::new(),
        missing_proxy_rules: Vec::new(),
    };
    let mut known_ids: HashSet<_> = config.rules.iter().map(|rule| rule.id.clone()).collect();
    let mut matchers: HashSet<_> = config
        .rules
        .iter()
        .filter(|rule| rule.profile_id == profile_id)
        .map(MatcherKey::from)
        .collect();
    let profile = user_profile();
    let proxies: HashSet<_> = config
        .proxies
        .iter()
        .map(|proxy| proxy.id.as_str())
        .collect();
    let mut next_id = 1;
    let mut updated = config.clone();
    for mut rule in rules {
        rule.profile_id = profile_id.into();
        if let Some(path) = matcher_path(&mut rule.matcher) {
            let normalized = path.trim().replace('/', "\\");
            if let (Some(tail), Some(profile)) = (path_suffix(&normalized, PROFILE_TOKEN), &profile)
            {
                *path = format!("{profile}{tail}");
            }
        }
        Targets::parse(&rule).map_err(|error| validation(format!("Rule {}: {error}", rule.id)))?;
        if !matchers.insert(MatcherKey::from(&rule)) {
            preview.skipped_duplicates.push(rule.name);
            continue;
        }
        if rule.id.trim().is_empty() || !known_ids.insert(rule.id.clone()) {
            loop {
                let id = format!("imported-{next_id}");
                next_id += 1;
                if known_ids.insert(id.clone()) {
                    rule.id = id;
                    break;
                }
            }
        }
        if rule
            .proxy_id
            .as_deref()
            .is_some_and(|id| !proxies.contains(id))
        {
            rule.proxy_id = None;
        }
        if rule.action == RuleAction::Proxy && rule.proxy_id.is_none() {
            preview.missing_proxy_rules.push(rule.name.clone());
        }
        preview.added.push(rule.name.clone());
        updated.rules.push(rule);
    }
    validate(&updated)?;
    Ok((updated, preview))
}

pub(crate) fn invalid_rule_ids(config: &AppConfig) -> Vec<String> {
    config
        .rules
        .iter()
        .filter(|rule| Targets::parse(rule).is_err())
        .map(|rule| rule.id.clone())
        .collect()
}

fn validate_structure(config: &AppConfig) -> Result<(), IpcError> {
    if config.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(validation("Unsupported config schema version"));
    }
    let profiles = ids(
        config.profiles.iter().map(|item| item.id.as_str()),
        "Profile",
    )?;
    let proxies = ids(config.proxies.iter().map(|item| item.id.as_str()), "Proxy")?;
    ids(config.rules.iter().map(|item| item.id.as_str()), "Rule")?;
    if !profiles.contains(config.active_profile_id.as_str()) {
        return Err(validation("Active profile does not exist"));
    }
    ids(
        config
            .automation
            .wifi_rules
            .iter()
            .map(|item| item.id.as_str()),
        "Wi-Fi rule",
    )?;
    for rule in &config.automation.wifi_rules {
        if rule.ssid.trim().is_empty() || !profiles.contains(rule.profile_id.as_str()) {
            return Err(validation(format!(
                "Invalid SSID or profile for Wi-Fi rule {}",
                rule.id
            )));
        }
    }
    let validate_proxy_id = |id: Option<&str>| {
        if id.is_some_and(|id| !proxies.contains(id)) {
            return Err(validation("Profile or rule references an unknown proxy"));
        }
        Ok(())
    };
    for proxy in &config.proxies {
        if proxy.protocol == ProxyProtocol::DirectInterface {
            if proxy
                .interface_name
                .as_deref()
                .is_none_or(|name| name.trim().is_empty())
            {
                return Err(validation(format!(
                    "Proxy {} requires a network interface",
                    proxy.id
                )));
            }
            continue;
        }
        if proxy.host.trim().is_empty()
            || proxy
                .host
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
            || proxy.port == 0
        {
            return Err(validation(format!(
                "Invalid host or port for proxy {}",
                proxy.id
            )));
        }
        if proxy.auth.as_ref().is_some_and(|auth| {
            !(1..=255).contains(&auth.username.len()) || !(1..=255).contains(&auth.password.len())
        }) {
            return Err(validation(format!(
                "Proxy {} credentials must each be 1–255 UTF-8 bytes",
                proxy.id
            )));
        }
    }
    for profile in &config.profiles {
        validate_proxy_id(profile.default_proxy_id.as_deref())?;
    }
    for rule in &config.rules {
        if !profiles.contains(rule.profile_id.as_str()) {
            return Err(validation(format!("Unknown profile for rule {}", rule.id)));
        }
        let matcher = match &rule.matcher {
            RuleMatcher::ExecutablePath { path, .. } => path,
            RuleMatcher::ProcessName { name, .. } => name,
            RuleMatcher::Directory { dir, .. } => dir,
        };
        if matcher.trim().is_empty() || matcher.contains('\0') {
            return Err(validation(format!("Invalid matcher for rule {}", rule.id)));
        }
        validate_proxy_id(rule.proxy_id.as_deref())?;
    }
    BypassHosts::parse(&config.settings.bypass_hosts)
        .map_err(|error| validation(format!("bypass_hosts: {error}")))?;
    Ok(())
}

fn validation(message: impl Into<String>) -> IpcError {
    IpcError::new(IpcErrorCode::ValidationFailed, message)
}

pub(crate) struct Policy {
    pub config: AppConfig,
    pub rules: Rules,
    pub bypass_hosts: Option<Arc<BypassHosts>>,
    pub local_proxy_ports: HashSet<u16>,
    local_ipv4s: HashSet<Ipv4Addr>,
    upstream: HashSet<SocketAddrV4>,
    upstream_v6: HashSet<SocketAddrV6>,
}

impl Policy {
    pub async fn new(config: AppConfig) -> Self {
        let local_ipv4s: HashSet<Ipv4Addr> =
            tokio::task::spawn_blocking(crate::network_interfaces::list)
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default()
                .into_iter()
                .filter(|interface| interface.is_up)
                .filter_map(|interface| interface.ipv4.parse().ok())
                .collect();
        let mut policy = Self {
            rules: Rules::compile(&config),
            bypass_hosts: BypassHosts::parse(&config.settings.bypass_hosts)
                .expect("validated bypass hosts"),
            local_ipv4s,
            config,
            local_proxy_ports: HashSet::new(),
            upstream: HashSet::new(),
            upstream_v6: HashSet::new(),
        };
        let mut hosts: HashMap<String, HashSet<u16>> = HashMap::new();
        let mut endpoints = Vec::new();
        for proxy in policy
            .config
            .proxies
            .iter()
            .filter(|proxy| proxy.enabled && proxy.protocol != ProxyProtocol::DirectInterface)
        {
            let host = proxy
                .host
                .strip_prefix('[')
                .and_then(|host| host.strip_suffix(']'))
                .unwrap_or(&proxy.host);
            if host.eq_ignore_ascii_case("localhost") {
                policy.local_proxy_ports.insert(proxy.port);
            }
            if let Ok(ip) = host.parse::<IpAddr>() {
                endpoints.push((ip, proxy.port));
            } else {
                hosts.entry(host.to_owned()).or_default().insert(proxy.port);
            }
        }
        for (ip, port) in endpoints {
            policy.record_upstream(ip, port);
        }

        let mut lookups = tokio::task::JoinSet::new();
        for (host, ports) in hosts {
            lookups.spawn(async move {
                let result = tokio::time::timeout(
                    Duration::from_secs(2),
                    tokio::net::lookup_host((host, 0)),
                )
                .await;
                (ports, result)
            });
        }
        while let Some(result) = lookups.join_next().await {
            let Ok((ports, Ok(Ok(addresses)))) = result else {
                continue;
            };
            for address in addresses {
                for &port in &ports {
                    policy.record_upstream(address.ip(), port);
                }
            }
        }
        policy
    }

    fn record_upstream(&mut self, ip: IpAddr, port: u16) {
        let ipv4 = match ip {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(ip) => ip.to_ipv4_mapped(),
        };
        if ip.is_loopback()
            || ip.is_unspecified()
            || ipv4.is_some_and(|ip| {
                ip.is_loopback() || ip.is_unspecified() || self.local_ipv4s.contains(&ip)
            })
        {
            self.local_proxy_ports.insert(port);
        }
        if let IpAddr::V6(ip) = ip {
            self.upstream_v6.insert(SocketAddrV6::new(ip, port, 0, 0));
        }
        if let Some(ip) = ipv4 {
            self.upstream.insert(SocketAddrV4::new(ip, port));
        }
    }

    pub fn must_bypass_v4(&self, destination: SocketAddrV4) -> bool {
        let ip = destination.ip();
        ip.is_loopback()
            || ip.is_unspecified()
            || self.local_ipv4s.contains(ip)
            || self.upstream.contains(&destination)
    }

    pub fn must_bypass_v6(&self, destination: SocketAddrV6) -> bool {
        let ip = *destination.ip();
        ip.is_loopback()
            || ip.is_unspecified()
            || self
                .upstream_v6
                .contains(&SocketAddrV6::new(ip, destination.port(), 0, 0))
    }
}

pub(crate) fn is_private_or_lan_v4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local() || ip.is_multicast() || ip.is_broadcast()
}

pub(crate) fn is_private_or_lan_v6(ip: Ipv6Addr) -> bool {
    ip.is_multicast() || ip.is_unicast_link_local() || ip.is_unique_local()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::ProxyProtocol;

    #[test]
    fn loads_v1_with_default_automation_and_saves_v2() {
        let dir = std::env::temp_dir().join(format!(
            "antify-v1-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let path = dir.join("config.json");
        let mut legacy = serde_json::to_value(AppConfig::default()).unwrap();
        legacy["schemaVersion"] = 1.into();
        legacy.as_object_mut().unwrap().remove("automation");
        atomic_write(&path, &serde_json::to_vec(&legacy).unwrap()).unwrap();

        let (config, invalid) = load_with_diagnostics(&path).unwrap();
        assert_eq!(config.schema_version, 2);
        assert_eq!(config.automation, shared::AutomationConfig::default());
        assert!(invalid.is_empty());
        save(&path, &config).unwrap();
        assert_eq!(
            serde_json::from_slice::<AppConfig>(&fs::read(&path).unwrap()).unwrap(),
            config
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(backup_path(&path, 1)).unwrap())
                .unwrap(),
            legacy
        );
        assert_eq!(restore_latest_backup(&path).unwrap(), config);
        assert_eq!(
            serde_json::from_slice::<AppConfig>(&fs::read(&path).unwrap())
                .unwrap()
                .schema_version,
            2
        );
        fs::remove_file(backup_path(&path, 1)).unwrap();
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn backup_credential_modes_round_trip_and_warn_for_plaintext() {
        let mut config = AppConfig::default();
        config.proxies.push(shared::ProxyConfig {
            id: "proxy".into(),
            name: "Office proxy".into(),
            protocol: ProxyProtocol::Socks5,
            host: "127.0.0.1".into(),
            port: 1080,
            auth: Some(shared::ProxyAuth {
                username: "test-login".into(),
                password: "private-password".into(),
            }),
            interface_name: None,
            allow_lan: false,
            udp_relay: false,
            skip_tls_verify: false,
            fallback_direct: false,
            enabled: true,
        });
        let full = export_config_backup(&config, true);
        assert!(full.includes_passwords);
        assert_eq!(
            full.warning.as_deref(),
            Some("Plaintext proxy credentials are included in this backup. Store it securely.")
        );
        assert_eq!(import_config_backup(&full.json).unwrap(), config);
        let redacted = export_config_backup(&config, false);
        assert!(!redacted.includes_passwords);
        assert!(redacted.warning.is_none());
        assert!(!redacted.json.contains("private-password"));
        assert!(!redacted.json.contains("test-login"));
        config.proxies[0].auth = None;
        assert_eq!(import_config_backup(&redacted.json).unwrap(), config);
    }

    #[test]
    fn backup_import_falls_back_to_first_profile_and_validates_all_fields() {
        let mut config = AppConfig::default();
        assert_eq!(
            import_config_backup("{").unwrap_err().code,
            IpcErrorCode::ValidationFailed
        );

        let mut backup = serde_json::to_value(&config).unwrap();
        backup["schemaVersion"] = 1.into();
        backup["activeProfileId"] = "deleted".into();
        assert_eq!(import_config_backup(&backup.to_string()).unwrap(), config);
        backup.as_object_mut().unwrap().remove("activeProfileId");
        assert_eq!(import_config_backup(&backup.to_string()).unwrap(), config);
        backup["schemaVersion"] = 3.into();
        assert_eq!(
            import_config_backup(&backup.to_string()).unwrap_err().code,
            IpcErrorCode::ValidationFailed
        );

        config.automation.wifi_rules.push(shared::WifiProfileRule {
            id: "".into(),
            ssid: "Work".into(),
            profile_id: "default".into(),
            enabled: true,
        });
        assert!(validate(&config).is_err());
        config.automation.wifi_rules[0].id = "wifi-rule".into();
        config.automation.wifi_rules[0].ssid = " ".into();
        assert!(validate(&config).is_err());
        config.automation.wifi_rules[0].ssid = "Work".into();
        config.automation.wifi_rules[0].profile_id = "deleted".into();
        assert!(validate(&config).is_err());
        config.automation.wifi_rules[0].profile_id = "default".into();
        assert!(validate(&config).is_ok());

        config.settings.bypass_hosts.push("10.2.3.0/33".into());
        assert_eq!(
            import_config_backup(&serde_json::to_string(&config).unwrap())
                .unwrap_err()
                .code,
            IpcErrorCode::ValidationFailed,
        );
    }

    #[test]
    fn exports_rules_with_portable_executable_and_directory_paths() {
        let mut config = AppConfig::default();
        let base = Rule {
            id: "app".into(),
            name: "Portable app".into(),
            profile_id: "default".into(),
            enabled: true,
            matcher: RuleMatcher::ExecutablePath {
                path: r"c:\uSERS\Another Person\Apps\Client.exe".into(),
                include_children: false,
            },
            action: RuleAction::Direct,
            proxy_id: None,
            target_hosts: vec![],
            target_ports: vec![],
            priority: 0,
            network_lock: false,
        };
        config.rules = vec![
            base.clone(),
            Rule {
                id: "folder".into(),
                matcher: RuleMatcher::Directory {
                    dir: r"D:/Users/Another Person/Projects".into(),
                    include_children: true,
                },
                ..base.clone()
            },
            Rule {
                id: "other-app".into(),
                profile_id: "other".into(),
                ..base
            },
        ];
        let mut exported: Vec<Rule> =
            serde_json::from_str(&export_rules_json(&config, None)).unwrap();
        assert_eq!(exported.len(), 2);
        assert_eq!(
            matcher_path(&mut exported[0].matcher).unwrap(),
            r"%USERPROFILE%\Apps\Client.exe"
        );
        assert_eq!(
            matcher_path(&mut exported[1].matcher).unwrap(),
            r"%USERPROFILE%\Projects"
        );
        assert_eq!(
            portable_path(r"D:\Profiles\Current\Apps", Some(r"d:\profiles\current")),
            r"%USERPROFILE%\Apps"
        );
        assert_eq!(
            portable_path(r"D:\Profiles\CurrentOther", Some(r"d:\profiles\current")),
            r"D:\Profiles\CurrentOther"
        );
        assert_eq!(
            serde_json::from_str::<Vec<Rule>>(&export_rules_json(&config, Some("other")))
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn detects_enabled_local_proxies_and_bypasses_ipv6_upstreams() {
        let mut config = AppConfig::default();
        config.settings.bypass_lan = false;
        config.proxies = [
            ("LOCALHOST", 8000, ProxyProtocol::Http, true),
            ("0.0.0.0", 8001, ProxyProtocol::Socks5, true),
            ("[::]", 8002, ProxyProtocol::Http, true),
            ("[::ffff:127.0.0.1]", 8003, ProxyProtocol::Socks5, true),
            ("[::1]", 8004, ProxyProtocol::Socks5, false),
            ("[2001:db8::1]", 8005, ProxyProtocol::Socks5, true),
            ("127.0.0.9", 8006, ProxyProtocol::Socks5, true),
            ("[::1]", 8007, ProxyProtocol::Socks5, true),
            ("127.0.0.9", 8010, ProxyProtocol::Https, true),
            ("127.0.0.9", 8011, ProxyProtocol::DirectInterface, true),
        ]
        .into_iter()
        .map(|(host, port, protocol, enabled)| shared::ProxyConfig {
            id: port.to_string(),
            name: host.into(),
            host: host.into(),
            port,
            protocol,
            enabled,
            auth: None,
            interface_name: None,
            allow_lan: false,
            udp_relay: false,
            skip_tls_verify: false,
            fallback_direct: false,
        })
        .collect();
        let mut policy = Policy::new(config).await;
        assert_eq!(
            policy.local_proxy_ports,
            HashSet::from([8000, 8001, 8002, 8003, 8006, 8007, 8010])
        );
        let upstream = SocketAddrV6::new("2001:db8::1".parse().unwrap(), 8005, 10, 11);
        assert!(policy.must_bypass_v6(upstream));
        assert!(policy.must_bypass_v6("[::1]:8007".parse().unwrap()));
        assert!(policy.must_bypass_v6("[::1]:8004".parse().unwrap()));
        assert!(policy.must_bypass_v4("127.0.0.1:8003".parse().unwrap()));

        policy.local_ipv4s = HashSet::from(["192.0.2.1".parse().unwrap()]);
        policy.record_upstream("192.0.2.1".parse().unwrap(), 8008);
        policy.record_upstream("::ffff:192.0.2.1".parse().unwrap(), 8009);
        assert!(policy.local_proxy_ports.contains(&8008));
        assert!(policy.local_proxy_ports.contains(&8009));
        assert!(policy.must_bypass_v4("192.0.2.1:9999".parse().unwrap()));
        assert!(!policy.must_bypass_v4("192.168.1.10:443".parse().unwrap()));

        assert!(policy.must_bypass_v6("[::]:443".parse().unwrap()));
        for ip in ["ff02::1", "fe80::1", "fd00::1"] {
            let ip = ip.parse().unwrap();
            assert!(is_private_or_lan_v6(ip));
            assert!(!policy.must_bypass_v6(SocketAddrV6::new(ip, 443, 0, 0)));
        }
        assert!(!policy.must_bypass_v6("[2001:db8::2]:443".parse().unwrap()));
    }

    #[test]
    fn rolls_three_atomic_backups_and_restores_the_newest_valid_copy() {
        let dir = std::env::temp_dir().join(format!(
            "my-win-antify-config-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let path = dir.join("config.json");
        let (mut config, invalid) = load_with_diagnostics(&path).unwrap();
        assert_eq!(config, AppConfig::default());
        assert!(invalid.is_empty());

        for language in ["first", "second", "third", "fourth"] {
            config.settings.language = language.into();
            save(&path, &config).unwrap();
        }
        assert_eq!(load_with_diagnostics(&path).unwrap().0, config);
        for (index, language) in [(1, "third"), (2, "second"), (3, "first")] {
            let backup = backup_path(&path, index);
            assert_eq!(
                load_with_diagnostics(&backup).unwrap().0.settings.language,
                language
            );
            assert!(!sibling(&backup, ".tmp").exists());
        }
        assert!(!backup_path(&path, 4).exists());
        assert!(!sibling(&path, ".tmp").exists());

        let mut broken = config.clone();
        broken.active_profile_id = "missing".into();
        fs::write(backup_path(&path, 1), serde_json::to_vec(&broken).unwrap()).unwrap();
        fs::write(&path, b"invalid").unwrap();
        let restored = restore_latest_backup(&path).unwrap();
        assert_eq!(restored.settings.language, "second");
        assert_eq!(load_with_diagnostics(&path).unwrap().0, restored);

        for index in 1..=BACKUP_COUNT {
            fs::remove_file(backup_path(&path, index)).unwrap();
        }
        assert_eq!(
            restore_latest_backup(&path).unwrap_err().code,
            IpcErrorCode::ConfigIoError
        );
        assert_eq!(load_with_diagnostics(&path).unwrap().0, restored);
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn rejects_invalid_bypass_hosts() {
        let mut config = AppConfig::default();
        config.settings.bypass_hosts = vec!["10.0.0.0/99".into()];
        let error = validate(&config).unwrap_err();
        assert_eq!(error.code, IpcErrorCode::ValidationFailed);
        assert!(error.message.contains("bypass_hosts"));
    }
}
