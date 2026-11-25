use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{self, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6},
    path::{Path, PathBuf},
    time::Duration,
};

use shared::{
    AppConfig, IpcError, IpcErrorCode, ProxyProtocol, RuleMatcher, CONFIG_SCHEMA_VERSION,
};

use crate::process::Rules;

pub(crate) fn default_path() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into()))
        .join("MyWinAntify")
        .join("config.json")
}

pub(crate) fn load(path: &Path) -> Result<AppConfig, IpcError> {
    let config = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| config_io(path, error))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let config = AppConfig::default();
            save(path, &config)?;
            config
        }
        Err(error) => return Err(config_io(path, error)),
    };
    validate(&config)?;
    Ok(config)
}

pub(crate) fn save(path: &Path, config: &AppConfig) -> Result<(), IpcError> {
    let bytes = serde_json::to_vec_pretty(config).map_err(|error| config_io(path, error))?;
    let write = || -> io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let mut name = path.as_os_str().to_owned();
        name.push(".tmp");
        let temporary = PathBuf::from(name);
        // The pipe singleton and serialized config operations protect this fixed sibling path.
        match fs::remove_file(&temporary) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let flushed = file.write_all(&bytes).and_then(|()| file.sync_all());
        drop(file);
        let result = flushed.and_then(|()| fs::rename(&temporary, path));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    };
    write().map_err(|error| config_io(path, error))
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
    let validate_proxy_id = |id: Option<&str>| {
        if id.is_some_and(|id| !proxies.contains(id)) {
            return Err(validation("Profile or rule references an unknown proxy"));
        }
        Ok(())
    };
    for proxy in &config.proxies {
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
        crate::targets::Targets::parse(rule)
            .map_err(|error| validation(format!("Rule {}: {error}", rule.id)))?;
    }
    Ok(())
}

fn validation(message: impl Into<String>) -> IpcError {
    IpcError::new(IpcErrorCode::ValidationFailed, message)
}

pub(crate) struct Policy {
    pub config: AppConfig,
    pub rules: Rules,
    pub local_proxy_ports: HashSet<u16>,
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
                .filter_map(|interface| interface.ipv4.parse().ok())
                .collect();
        let mut policy = Self {
            rules: Rules::compile(&config),
            config,
            local_proxy_ports: HashSet::new(),
            upstream: HashSet::new(),
            upstream_v6: HashSet::new(),
        };
        let mut hosts: HashMap<String, HashSet<u16>> = HashMap::new();
        let mut endpoints = Vec::new();
        for proxy in policy.config.proxies.iter().filter(|proxy| proxy.enabled) {
            let host = proxy
                .host
                .strip_prefix('[')
                .and_then(|host| host.strip_suffix(']'))
                .unwrap_or(&proxy.host);
            let socks5 = proxy.protocol == ProxyProtocol::Socks5;
            if host.eq_ignore_ascii_case("localhost") {
                policy.local_proxy_ports.insert(proxy.port);
            }
            if let Ok(ip) = host.parse::<IpAddr>() {
                endpoints.push((ip, proxy.port, socks5));
            } else if socks5 {
                hosts.entry(host.to_owned()).or_default().insert(proxy.port);
            }
        }
        for (ip, port, socks5) in endpoints {
            policy.record_upstream(ip, port, socks5, &local_ipv4s);
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
                    policy.record_upstream(address.ip(), port, true, &local_ipv4s);
                }
            }
        }
        policy
    }

    fn record_upstream(
        &mut self,
        ip: IpAddr,
        port: u16,
        socks5: bool,
        local_ipv4s: &HashSet<Ipv4Addr>,
    ) {
        let ipv4 = match ip {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(ip) => ip.to_ipv4_mapped(),
        };
        if ip.is_loopback()
            || ip.is_unspecified()
            || ipv4.is_some_and(|ip| {
                ip.is_loopback() || ip.is_unspecified() || local_ipv4s.contains(&ip)
            })
        {
            self.local_proxy_ports.insert(port);
        }
        if socks5 {
            if let IpAddr::V6(ip) = ip {
                self.upstream_v6.insert(SocketAddrV6::new(ip, port, 0, 0));
            }
            if let Some(ip) = ipv4 {
                self.upstream.insert(SocketAddrV4::new(ip, port));
            }
        }
    }

    pub fn bypass(&self, destination: SocketAddrV4) -> bool {
        self.upstream.contains(&destination)
            || (self.config.settings.bypass_lan && lan(*destination.ip()))
    }

    pub fn bypass_v6(&self, destination: SocketAddrV6) -> bool {
        let ip = *destination.ip();
        self.upstream_v6
            .contains(&SocketAddrV6::new(ip, destination.port(), 0, 0))
            || (self.config.settings.bypass_lan && lan_v6(ip))
    }
}

fn lan(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local() || ip.is_multicast() || ip.is_broadcast()
}

fn lan_v6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unicast_link_local()
        || ip.is_unique_local()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            fallback_direct: false,
        })
        .collect();
        let mut policy = Policy::new(config).await;
        assert_eq!(
            policy.local_proxy_ports,
            HashSet::from([8000, 8001, 8002, 8003, 8006, 8007])
        );
        let upstream = SocketAddrV6::new("2001:db8::1".parse().unwrap(), 8005, 10, 11);
        assert!(policy.bypass_v6(upstream));
        assert!(policy.bypass_v6("[::1]:8007".parse().unwrap()));
        assert!(!policy.bypass_v6("[::1]:8004".parse().unwrap()));
        assert!(policy.bypass("127.0.0.1:8003".parse().unwrap()));

        let local_ipv4s = HashSet::from(["192.0.2.1".parse().unwrap()]);
        policy.record_upstream("192.0.2.1".parse().unwrap(), 8008, true, &local_ipv4s);
        policy.record_upstream(
            "::ffff:192.0.2.1".parse().unwrap(),
            8009,
            true,
            &local_ipv4s,
        );
        assert!(policy.local_proxy_ports.contains(&8008));
        assert!(policy.local_proxy_ports.contains(&8009));

        policy.config.settings.bypass_lan = true;
        for ip in ["::", "::1", "ff02::1", "fe80::1", "fd00::1"] {
            assert!(policy.bypass_v6(SocketAddrV6::new(ip.parse().unwrap(), 443, 0, 0)));
        }
        assert!(!policy.bypass_v6("[2001:db8::2]:443".parse().unwrap()));
    }

    #[test]
    fn writes_defaults_and_replaces_existing_config_atomically() {
        let dir = std::env::temp_dir().join(format!(
            "my-win-antify-config-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let path = dir.join("nested").join("config.json");
        let mut config = load(&path).unwrap();
        assert_eq!(config, AppConfig::default());
        config.settings.bypass_lan = false;
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        assert!(!path.with_file_name("config.json.tmp").exists());
        fs::remove_file(&path).unwrap();
        fs::remove_dir(path.parent().unwrap()).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
