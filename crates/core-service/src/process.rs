use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    io,
    mem::{size_of, zeroed},
    net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6},
    path::{Component, Path, PathBuf},
    ptr,
    sync::Arc,
    time::{Duration, Instant},
};

use shared::{AppConfig, ProxyConfig, RuleAction, RuleMatcher, RunningProcessInfo};

use crate::{
    packet::{FlowKey, FlowKeyV6},
    targets::{HostPatterns, Targets},
};

const CACHE_TTL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub(crate) struct Process {
    pub pid: u32,
    parent: Option<u32>,
    pub path: String,
    pub name: String,
}

impl Process {
    fn from_path(pid: u32, parent: Option<u32>, path: String) -> Self {
        let name = path
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or_default()
            .to_owned();
        Self {
            pid,
            parent,
            path,
            name,
        }
    }

    pub fn unknown(pid: u32) -> Self {
        Self {
            pid,
            parent: None,
            path: String::new(),
            name: format!("PID {pid}"),
        }
    }

    pub fn app_name(&self) -> String {
        let name = self
            .name
            .rsplit_once('.')
            .filter(|(_, extension)| extension.eq_ignore_ascii_case("exe"))
            .map(|(name, _)| name);
        name.unwrap_or(&self.name).to_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Decision {
    pub action: RuleAction,
    pub rule_id: Option<String>,
    pub rule_name: Option<String>,
    pub proxy: Option<ProxyConfig>,
    pub network_lock: bool,
    pub pending_hosts: Option<Arc<HostPatterns>>,
}

impl Decision {
    pub fn match_reason(&self) -> String {
        match (self.action, &self.rule_name) {
            (RuleAction::Block, Some(name)) => format!("Blocked by rule: {name}"),
            (RuleAction::Block, None) => "Blocked by profile default".into(),
            (_, Some(name)) => format!("Matched rule: {name}"),
            (_, None) => "Profile default action".into(),
        }
    }

    pub fn use_direct(&mut self) {
        self.action = RuleAction::Direct;
        self.proxy = None;
        self.pending_hosts = None;
    }

    pub fn direct() -> Self {
        Self {
            action: RuleAction::Direct,
            rule_id: None,
            rule_name: None,
            proxy: None,
            network_lock: false,
            pending_hosts: None,
        }
    }
}

pub(crate) struct Rules {
    rules: Vec<(Matcher, Option<Targets>, Decision)>,
    default: Decision,
}

enum Matcher {
    Path(String, bool),
    Name(String, bool),
    Directory(String, bool),
}

impl Matcher {
    fn compile(matcher: &RuleMatcher) -> Self {
        match matcher {
            RuleMatcher::ExecutablePath {
                path,
                include_children,
            } => Self::Path(normalized_path(path), *include_children),
            RuleMatcher::ProcessName {
                name,
                include_children,
            } => Self::Name(name.trim().to_lowercase(), *include_children),
            RuleMatcher::Directory {
                dir,
                include_children,
            } => Self::Directory(
                format!("{}\\", normalized_path(dir).trim_end_matches('\\')),
                *include_children,
            ),
        }
    }

    fn matches(&self, process: &Process, is_child: bool) -> bool {
        match self {
            Self::Path(path, children) => {
                (!is_child || *children)
                    && !process.path.is_empty()
                    && normalized_path(&process.path) == *path
            }
            Self::Name(name, children) => {
                (!is_child || *children) && process.name.to_lowercase() == *name
            }
            Self::Directory(dir, children) => {
                (!is_child || *children) && normalized_path(&process.path).starts_with(dir)
            }
        }
    }
}

impl Rules {
    pub fn compile(config: &AppConfig) -> Self {
        let profile = config
            .profiles
            .iter()
            .find(|p| p.id == config.active_profile_id)
            .expect("validated active profile");
        let proxy = |id: Option<&str>| {
            id.and_then(|id| config.proxies.iter().find(|p| p.id == id && p.enabled))
                .cloned()
        };
        let mut rules: Vec<_> = config
            .rules
            .iter()
            .filter(|rule| rule.enabled && rule.profile_id == profile.id)
            .map(|rule| (rule, Targets::parse(rule).ok()))
            .collect();
        // A malformed destination filter must block the matching process even if an
        // earlier valid rule would otherwise route it.
        rules.sort_by_key(|(rule, targets)| (targets.is_some(), rule.priority));
        let rules = rules
            .into_iter()
            .map(|(rule, targets)| {
                let action = if targets.is_some() {
                    rule.action
                } else {
                    RuleAction::Block
                };
                let decision = Decision {
                    action,
                    rule_id: Some(rule.id.clone()),
                    rule_name: Some(rule.name.clone()),
                    proxy: if action == RuleAction::Proxy {
                        proxy(rule.proxy_id.as_deref())
                    } else {
                        None
                    },
                    network_lock: rule.network_lock,
                    pending_hosts: None,
                };
                (Matcher::compile(&rule.matcher), targets, decision)
            })
            .collect();
        Self {
            rules,
            default: Decision {
                action: profile.default_action,
                proxy: if profile.default_action == RuleAction::Proxy {
                    proxy(profile.default_proxy_id.as_deref())
                } else {
                    None
                },
                ..Decision::direct()
            },
        }
    }

    pub fn decide_ipv6(&self, lineage: &[Process], remote_port: u16) -> Decision {
        self.rules
            .iter()
            .find(|(matcher, targets, _)| {
                targets
                    .as_ref()
                    .is_none_or(|targets| targets.check_ipv6(remote_port))
                    && lineage
                        .iter()
                        .enumerate()
                        .any(|(i, process)| matcher.matches(process, i != 0))
            })
            .map_or_else(|| self.default.clone(), |(_, _, decision)| decision.clone())
    }

    pub fn decide(&self, lineage: &[Process], remote: SocketAddrV4) -> Decision {
        self.decide_destination(lineage, remote, None)
    }

    pub fn decide_datagram(
        &self,
        lineage: &[Process],
        remote: SocketAddrV4,
        domain: Option<&str>,
    ) -> Decision {
        self.decide_destination(lineage, remote, Some(domain))
    }

    fn decide_destination(
        &self,
        lineage: &[Process],
        remote: SocketAddrV4,
        inspected_domain: Option<Option<&str>>,
    ) -> Decision {
        self.rules
            .iter()
            .find_map(|(matcher, targets, decision)| {
                let pending_hosts = match targets {
                    Some(targets) => targets.check(*remote.ip(), remote.port())?,
                    None => None,
                };
                let pending_hosts = if let Some(domain) = inspected_domain {
                    if pending_hosts
                        .as_ref()
                        .is_some_and(|hosts| !hosts.matches(*remote.ip(), domain))
                    {
                        return None;
                    }
                    None
                } else {
                    pending_hosts
                };
                lineage
                    .iter()
                    .enumerate()
                    .any(|(i, process)| matcher.matches(process, i != 0))
                    .then(|| Decision {
                        pending_hosts,
                        ..decision.clone()
                    })
            })
            .unwrap_or_else(|| self.default.clone())
    }
}

pub(crate) fn normalized_path(path: &str) -> String {
    let path = path.trim().replace('/', "\\").to_lowercase();
    let path = if let Some(unc) = path.strip_prefix(r"\\?\unc\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    };
    let mut normalized = PathBuf::new();
    for part in Path::new(&path).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component),
        }
    }
    normalized.to_string_lossy().into_owned()
}

#[derive(Default)]
pub(crate) struct ProcessCache {
    entries: HashMap<u32, Entry>,
    paths: HashMap<u32, Process>,
    refreshed: Option<Instant>,
    proxy_ports: HashSet<u16>,
    proxy_pids: HashSet<u32>,
    proxy_paths: HashSet<String>,
    proxy_refreshed: Option<Instant>,
}

impl ProcessCache {
    fn refresh(&mut self, pid: u32) {
        if self
            .refreshed
            .is_some_and(|when| when.elapsed() < CACHE_TTL)
            && (self.entries.contains_key(&pid) || self.paths.contains_key(&pid))
        {
            return;
        }
        if let Ok(entries) = snapshot() {
            self.entries = entries;
        }
        self.paths.clear();
        self.refreshed = Some(Instant::now());
    }

    fn refresh_local_proxies(&mut self, ports: &HashSet<u16>) {
        if self.proxy_ports != *ports
            || self
                .proxy_refreshed
                .is_none_or(|last| last.elapsed() >= CACHE_TTL)
        {
            self.proxy_ports.clone_from(ports);
            self.proxy_pids = find_tcp_listener_pids(ports);
            self.proxy_paths = self
                .proxy_pids
                .iter()
                .filter_map(|&pid| exe_path(pid).ok())
                .map(|path| normalized_path(&path))
                .collect();
            self.proxy_refreshed = Some(Instant::now());
        }
    }

    pub fn local_proxy_match(
        &mut self,
        lineage: &[Process],
        ports: &HashSet<u16>,
    ) -> Option<Process> {
        self.refresh_local_proxies(ports);
        lineage
            .iter()
            .find(|process| {
                self.proxy_pids.contains(&process.pid)
                    || (!process.path.is_empty()
                        && self.proxy_paths.contains(&normalized_path(&process.path)))
            })
            .cloned()
    }

    pub fn active_local_proxy_processes(&mut self, ports: &HashSet<u16>) -> Vec<Process> {
        self.refresh_local_proxies(ports);
        let mut pids: Vec<_> = self.proxy_pids.iter().copied().collect();
        pids.sort_unstable();
        pids.into_iter()
            .filter_map(|pid| self.lineage(pid).into_iter().next())
            .collect()
    }

    pub fn lineage(&mut self, pid: u32) -> Vec<Process> {
        self.refresh(pid);
        let cached = &mut self.paths;
        let entries = &self.entries;
        walk_lineage(pid, |pid| {
            if let Some(process) = cached.get(&pid) {
                return Some(process.clone());
            }
            let entry = entries.get(&pid);
            let parent = entry.and_then(|e| e.parent);
            let process = match exe_path(pid) {
                Ok(path) => Process::from_path(pid, parent, path),
                Err(_) => Process {
                    parent,
                    name: entry
                        .map(|e| e.name.clone())
                        .unwrap_or_else(|| format!("PID {pid}")),
                    ..Process::unknown(pid)
                },
            };
            cached.insert(pid, process.clone());
            Some(process)
        })
    }
}

fn walk_lineage(pid: u32, mut lookup: impl FnMut(u32) -> Option<Process>) -> Vec<Process> {
    let mut lineage = Vec::new();
    let mut current = Some(pid);
    let mut seen = HashSet::new();
    while let Some(pid) = current.filter(|&pid| pid != 0 && seen.insert(pid)) {
        let Some(process) = lookup(pid) else { break };
        current = process.parent;
        lineage.push(process);
    }
    lineage
}

pub(crate) fn list_running() -> io::Result<Vec<RunningProcessInfo>> {
    let active: HashSet<_> = tcp_rows(AddressFamily::Ipv4)?
        .into_iter()
        .chain(tcp_rows(AddressFamily::Ipv6).unwrap_or_default())
        .filter(|row| row.state == 5) // MIB_TCP_STATE_ESTAB
        .map(|row| row.pid)
        .collect();
    let mut by_path: HashMap<String, RunningProcessInfo> = HashMap::new();
    for (pid, entry) in snapshot()? {
        let Ok(path) = exe_path(pid) else { continue };
        let process = Process::from_path(pid, entry.parent, path);
        let key = normalized_path(&process.path);
        let mut info = RunningProcessInfo {
            pid,
            parent_pid: process.parent,
            app_name: process.app_name(),
            name: process.name,
            executable_path: process.path,
            has_active_connections: active.contains(&pid),
        };
        by_path
            .entry(key)
            .and_modify(|existing| {
                let has_active = existing.has_active_connections || info.has_active_connections;
                if (info.has_active_connections, std::cmp::Reverse(info.pid))
                    > (
                        existing.has_active_connections,
                        std::cmp::Reverse(existing.pid),
                    )
                {
                    std::mem::swap(existing, &mut info);
                }
                existing.has_active_connections = has_active;
            })
            .or_insert(info);
    }
    let mut result: Vec<_> = by_path.into_values().collect();
    result.sort_by_cached_key(|process| {
        (
            process.name.to_lowercase(),
            normalized_path(&process.executable_path),
        )
    });
    Ok(result)
}

#[repr(C)]
struct ProcessEntry {
    size: u32,
    usage: u32,
    pid: u32,
    default_heap: usize,
    module_id: u32,
    threads: u32,
    parent_pid: u32,
    priority: i32,
    flags: u32,
    executable: [u16; 260],
}

struct Entry {
    parent: Option<u32>,
    name: String,
}

struct Handle(isize);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn CloseHandle(handle: isize) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
    fn Process32FirstW(snapshot: isize, entry: *mut ProcessEntry) -> i32;
    fn Process32NextW(snapshot: isize, entry: *mut ProcessEntry) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn QueryFullProcessImageNameW(handle: isize, flags: u32, path: *mut u16, len: *mut u32) -> i32;
}

fn snapshot() -> io::Result<HashMap<u32, Entry>> {
    let handle = unsafe { CreateToolhelp32Snapshot(0x0000_0002, 0) };
    if handle == -1 {
        return Err(io::Error::last_os_error());
    }
    let handle = Handle(handle);
    let mut entry: ProcessEntry = unsafe { zeroed() };
    entry.size = size_of::<ProcessEntry>() as u32;
    if unsafe { Process32FirstW(handle.0, &mut entry) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut entries = HashMap::new();
    loop {
        let len = entry
            .executable
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.executable.len());
        entries.insert(
            entry.pid,
            Entry {
                parent: (entry.parent_pid != 0).then_some(entry.parent_pid),
                name: String::from_utf16_lossy(&entry.executable[..len]),
            },
        );
        if unsafe { Process32NextW(handle.0, &mut entry) } == 0 {
            break;
        }
    }
    Ok(entries)
}

fn exe_path(pid: u32) -> io::Result<String> {
    let handle = unsafe { OpenProcess(0x1000, 0, pid) };
    if handle == 0 {
        return Err(io::Error::last_os_error());
    }
    let handle = Handle(handle);
    let mut buffer = vec![0; 512];
    loop {
        let mut len = buffer.len() as u32;
        if unsafe { QueryFullProcessImageNameW(handle.0, 0, buffer.as_mut_ptr(), &mut len) } != 0 {
            return Ok(String::from_utf16_lossy(&buffer[..len as usize]));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(122) || buffer.len() >= 32768 {
            return Err(error);
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

struct TcpRow {
    state: u32,
    pid: u32,
    local_port: u16,
    key: Option<FlowKey>,
    key_v6: Option<FlowKeyV6>,
}

#[derive(Clone, Copy)]
enum AddressFamily {
    Ipv4,
    Ipv6,
}

impl AddressFamily {
    fn number(self) -> u32 {
        match self {
            Self::Ipv4 => 2,  // AF_INET
            Self::Ipv6 => 23, // AF_INET6
        }
    }

    fn words_per_row(self) -> usize {
        match self {
            Self::Ipv4 => 6,  // MIB_TCPROW_OWNER_PID
            Self::Ipv6 => 14, // MIB_TCP6ROW_OWNER_PID
        }
    }

    fn decode(self, row: &[u32]) -> TcpRow {
        let socket = |ip, port: u32| SocketAddrV4::new(ip, u16::from_be(port as u16));
        match self {
            Self::Ipv4 => TcpRow {
                state: row[0],
                pid: row[5],
                local_port: u16::from_be(row[2] as u16),
                key_v6: None,
                key: Some(FlowKey {
                    local: socket(Ipv4Addr::from(row[1].to_ne_bytes()), row[2]),
                    remote: socket(Ipv4Addr::from(row[3].to_ne_bytes()), row[4]),
                }),
            },
            Self::Ipv6 => {
                let ipv6 = |words: &[u32]| {
                    let mut bytes = [0; 16];
                    for (chunk, word) in bytes.chunks_exact_mut(4).zip(words) {
                        chunk.copy_from_slice(&word.to_ne_bytes());
                    }
                    Ipv6Addr::from(bytes)
                };
                let local = ipv6(&row[..4]);
                let remote = ipv6(&row[6..10]);
                let local_port = u16::from_be(row[5] as u16);
                let remote_port = u16::from_be(row[11] as u16);
                let key =
                    local
                        .to_ipv4_mapped()
                        .zip(remote.to_ipv4_mapped())
                        .map(|(local, remote)| FlowKey {
                            local: SocketAddrV4::new(local, local_port),
                            remote: SocketAddrV4::new(remote, remote_port),
                        });
                TcpRow {
                    state: row[12],
                    pid: row[13],
                    local_port,
                    key,
                    key_v6: Some(FlowKeyV6 {
                        local: SocketAddrV6::new(local, local_port, 0, 0),
                        remote: SocketAddrV6::new(remote, remote_port, 0, 0),
                    }),
                }
            }
        }
    }
}

#[link(name = "iphlpapi")]
extern "system" {
    fn GetExtendedTcpTable(
        table: *mut c_void,
        size: *mut u32,
        sort: i32,
        family: u32,
        class: i32,
        reserved: u32,
    ) -> u32;
    fn GetExtendedUdpTable(
        table: *mut c_void,
        size: *mut u32,
        sort: i32,
        family: u32,
        class: i32,
        reserved: u32,
    ) -> u32;

}

fn tcp_rows(family: AddressFamily) -> io::Result<Vec<TcpRow>> {
    let mut storage = Vec::<u32>::new();
    for _ in 0..5 {
        let mut bytes = (storage.len() * size_of::<u32>()) as u32;
        let buffer = if storage.is_empty() {
            ptr::null_mut()
        } else {
            storage.as_mut_ptr().cast()
        };
        match unsafe { GetExtendedTcpTable(buffer, &mut bytes, 0, family.number(), 5, 0) } {
            122 => storage.resize((bytes as usize).div_ceil(size_of::<u32>()), 0),
            0 if !storage.is_empty() => {
                // MIB_TCPTABLE_OWNER_PID contains the row count followed by aligned rows.
                let count = storage[0] as usize;
                let width = family.words_per_row();
                if count > (storage.len() - 1) / width {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid TCP table row count",
                    ));
                }
                return Ok(storage[1..1 + count * width]
                    .chunks_exact(width)
                    .map(|row| family.decode(row))
                    .collect());
            }
            code => return Err(io::Error::from_raw_os_error(code as i32)),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "TCP table changed during lookup",
    ))
}

pub(crate) fn find_udp_owner(local: SocketAddrV4) -> Option<u32> {
    let (exact_v4, wildcard_v4) = udp_owner_rows(local, AddressFamily::Ipv4);
    if exact_v4.is_some() {
        return exact_v4;
    }
    let (exact_v6, wildcard_v6) = udp_owner_rows(local, AddressFamily::Ipv6);
    exact_v6.or(wildcard_v4).or(wildcard_v6)
}

fn udp_owner_rows(local: SocketAddrV4, family: AddressFamily) -> (Option<u32>, Option<u32>) {
    const UDP_TABLE_OWNER_PID: i32 = 1;
    let mut storage = Vec::<u32>::new();
    for _ in 0..5 {
        let mut bytes = (storage.len() * size_of::<u32>()) as u32;
        let buffer = if storage.is_empty() {
            ptr::null_mut()
        } else {
            storage.as_mut_ptr().cast()
        };
        match unsafe {
            GetExtendedUdpTable(
                buffer,
                &mut bytes,
                0,
                family.number(),
                UDP_TABLE_OWNER_PID,
                0,
            )
        } {
            122 => storage.resize((bytes as usize).div_ceil(size_of::<u32>()), 0),
            0 if !storage.is_empty() => {
                let width = match family {
                    AddressFamily::Ipv4 => 3,
                    AddressFamily::Ipv6 => 7,
                };
                let count = storage[0] as usize;
                if count > (storage.len() - 1) / width {
                    break;
                }
                let mut wildcard = None;
                for row in storage[1..1 + count * width].chunks_exact(width) {
                    let pid = row[width - 1];
                    let port = u16::from_be(row[width - 2] as u16);
                    if pid == 0 || port != local.port() {
                        continue;
                    }
                    let ip = match family {
                        AddressFamily::Ipv4 => Ipv4Addr::from(row[0].to_ne_bytes()),
                        AddressFamily::Ipv6 => {
                            let mut bytes = [0; 16];
                            for (chunk, word) in bytes.chunks_exact_mut(4).zip(row) {
                                chunk.copy_from_slice(&word.to_ne_bytes());
                            }
                            let ip = Ipv6Addr::from(bytes);
                            if ip.is_unspecified() {
                                Ipv4Addr::UNSPECIFIED
                            } else if let Some(mapped) = ip.to_ipv4_mapped() {
                                mapped
                            } else {
                                continue;
                            }
                        }
                    };
                    if &ip == local.ip() {
                        return (Some(pid), wildcard);
                    }
                    if ip.is_unspecified() {
                        wildcard = Some(pid);
                    }
                }
                return (None, wildcard);
            }
            _ => break,
        }
    }
    (None, None)
}

pub(crate) fn find_tcp_owner(key: FlowKey) -> Option<u32> {
    [AddressFamily::Ipv4, AddressFamily::Ipv6]
        .into_iter()
        .find_map(|family| {
            tcp_rows(family)
                .ok()?
                .into_iter()
                .find(|row| {
                    row.pid != 0 && row.state != 11 && row.state != 12 && row.key == Some(key)
                })
                .map(|row| row.pid)
        })
}

pub(crate) fn find_tcp_listener_pids(ports: &HashSet<u16>) -> HashSet<u32> {
    if ports.is_empty() {
        return HashSet::new();
    }
    [AddressFamily::Ipv4, AddressFamily::Ipv6]
        .into_iter()
        .flat_map(|family| tcp_rows(family).unwrap_or_default())
        .filter(|row| row.pid != 0 && row.state == 2 && ports.contains(&row.local_port))
        .map(|row| row.pid)
        .collect()
}

pub(crate) fn find_tcp_owner_v6(key: FlowKeyV6) -> Option<u32> {
    tcp_rows(AddressFamily::Ipv6)
        .ok()?
        .into_iter()
        .find(|row| row.pid != 0 && row.state != 11 && row.state != 12 && row.key_v6 == Some(key))
        .map(|row| row.pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::Rule;

    fn rule(id: &str, matcher: RuleMatcher, priority: i32, action: RuleAction) -> Rule {
        Rule {
            id: id.into(),
            profile_id: "default".into(),
            enabled: true,
            name: id.into(),
            matcher,
            action,
            proxy_id: None,
            network_lock: false,
            priority,
            target_hosts: Vec::new(),
            target_ports: Vec::new(),
        }
    }

    #[test]
    fn active_profile_priority_and_parent_inheritance_win() {
        let mut config = AppConfig::default();
        config.rules = vec![
            rule(
                "name",
                RuleMatcher::ProcessName {
                    name: "NODE.EXE".into(),
                    include_children: false,
                },
                10,
                RuleAction::Direct,
            ),
            rule(
                "parent",
                RuleMatcher::ExecutablePath {
                    path: r"\\?\C:\APPS\.\Code.exe".into(),
                    include_children: true,
                },
                -5,
                RuleAction::Proxy,
            ),
        ];
        let parent = Process::from_path(10, Some(11), r"C:\Apps\Code.EXE".into());
        let child = Process::from_path(11, Some(10), r"C:\Runtime\node.exe".into());
        let lineage = walk_lineage(child.pid, |pid| {
            [child.clone(), parent.clone()]
                .into_iter()
                .find(|p| p.pid == pid)
        });
        assert_eq!(lineage.len(), 2);
        assert_eq!(
            Rules::compile(&config)
                .decide(&lineage, "203.0.113.1:443".parse().unwrap())
                .rule_id
                .as_deref(),
            Some("parent")
        );
        config.rules[1].matcher = RuleMatcher::ExecutablePath {
            path: parent.path.clone(),
            include_children: false,
        };
        assert_eq!(
            Rules::compile(&config)
                .decide(&lineage, "203.0.113.1:443".parse().unwrap())
                .rule_id
                .as_deref(),
            Some("name")
        );
        config.rules[0].enabled = false;
        config.profiles[0].default_action = RuleAction::Block;
        assert_eq!(
            Rules::compile(&config)
                .decide(&lineage, "203.0.113.1:443".parse().unwrap())
                .action,
            RuleAction::Block
        );
    }

    #[test]
    fn directory_match_requires_component_boundary() {
        let mut config = AppConfig::default();
        config.rules.push(rule(
            "dir",
            RuleMatcher::Directory {
                dir: "C:/Apps/".into(),
                include_children: false,
            },
            0,
            RuleAction::Block,
        ));
        let rules = Rules::compile(&config);
        assert_eq!(
            rules
                .decide(
                    &[Process::from_path(
                        1,
                        None,
                        r"c:\apps\tools\tool.exe".into()
                    )],
                    "203.0.113.1:443".parse().unwrap()
                )
                .action,
            RuleAction::Block
        );
        assert_eq!(
            rules
                .decide(
                    &[Process::from_path(2, None, r"c:\appstore\tool.exe".into())],
                    "203.0.113.1:443".parse().unwrap()
                )
                .action,
            RuleAction::Direct
        );
    }

    #[test]
    fn destination_filters_participate_in_rule_priority() {
        let mut config = AppConfig::default();
        let matcher = RuleMatcher::ProcessName {
            name: "curl.exe".into(),
            include_children: false,
        };
        let mut primary = rule("primary", matcher.clone(), 10, RuleAction::Proxy);
        primary.target_ports = vec!["8000-9000".into(), "-8123".into()];
        primary.target_hosts = vec!["*.example.org".into(), "-203.0.113.2".into()];
        config.rules = vec![rule("next", matcher, 20, RuleAction::Block), primary];
        let processes = [Process::from_path(1, None, r"C:\Apps\curl.exe".into())];
        let rules = Rules::compile(&config);
        let decide = |remote: &str| rules.decide(&processes, remote.parse().unwrap());
        let pending = decide("203.0.113.1:8000");
        assert_eq!(pending.rule_id.as_deref(), Some("primary"));
        assert!(pending.pending_hosts.is_some());
        for destination in ["203.0.113.1:443", "203.0.113.1:8123", "203.0.113.2:8000"] {
            let decision = decide(destination);
            assert_eq!(decision.rule_id.as_deref(), Some("next"));
            assert!(decision.pending_hosts.is_none());
        }
    }

    #[test]
    fn finds_local_listener_pids_in_each_address_family() {
        let ipv4 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ipv6 = std::net::TcpListener::bind("[::1]:0").unwrap();
        for listener in [&ipv4, &ipv6] {
            let ports = HashSet::from([listener.local_addr().unwrap().port()]);
            assert!(find_tcp_listener_pids(&ports).contains(&std::process::id()));
        }
        assert!(find_tcp_listener_pids(&HashSet::new()).is_empty());
    }

    #[test]
    fn matches_proxy_listener_pid_its_children_and_normalized_executable_path() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ports = HashSet::from([listener.local_addr().unwrap().port()]);
        let pid = std::process::id();
        let mut cache = ProcessCache::default();
        let proxy = cache
            .active_local_proxy_processes(&ports)
            .into_iter()
            .find(|process| process.pid == pid)
            .unwrap();
        assert!(!proxy.path.is_empty());
        assert_eq!(
            cache
                .local_proxy_match(&[Process::unknown(pid)], &ports)
                .unwrap()
                .pid,
            pid
        );

        let child = Process::from_path(0, Some(pid), r"C:\other\child.exe".into());
        assert_eq!(
            cache
                .local_proxy_match(&[child, proxy.clone()], &ports)
                .unwrap()
                .pid,
            pid
        );

        let same_path = proxy.path.to_uppercase().replace('\\', "/");
        let other_instance = Process::from_path(0, None, same_path);
        assert_eq!(
            cache
                .local_proxy_match(&[other_instance], &ports)
                .unwrap()
                .pid,
            0
        );
        assert!(cache.local_proxy_match(&[proxy], &HashSet::new()).is_none());
    }

    #[test]
    fn ipv6_decision_respects_priority_and_ignores_ipv4_only_allow_rules() {
        let mut config = AppConfig::default();
        let matcher = RuleMatcher::ProcessName {
            name: "curl.exe".into(),
            include_children: false,
        };
        let mut ipv4_only = rule("ipv4-only", matcher.clone(), 1, RuleAction::Direct);
        ipv4_only.target_hosts = vec!["192.0.2.0/24".into()];
        let mut domain = rule("domain", matcher.clone(), 2, RuleAction::Proxy);
        domain.target_hosts = vec!["*.example.org".into()];
        domain.target_ports = vec!["443".into()];
        let fallback = rule("block", matcher, 3, RuleAction::Block);
        config.rules = vec![fallback, domain, ipv4_only];

        let rules = Rules::compile(&config);
        let lineage = [Process::from_path(1, None, r"C:\Apps\curl.exe".into())];
        assert_eq!(rules.decide_ipv6(&lineage, 443).action, RuleAction::Proxy);
        assert_eq!(rules.decide_ipv6(&lineage, 80).action, RuleAction::Block);
        assert_eq!(rules.decide_ipv6(&[], 443).action, RuleAction::Direct);
    }

    #[test]
    fn decodes_ipv4_mapped_tcp_owner_rows() {
        let mut row = [0; 14];
        row[2] = u32::from_ne_bytes([0, 0, 255, 255]);
        row[3] = u32::from_ne_bytes([192, 0, 2, 1]);
        row[5] = u32::from(55000u16.to_be());
        row[8] = row[2];
        row[9] = u32::from_ne_bytes([203, 0, 113, 2]);
        row[11] = u32::from(443u16.to_be());
        row[12] = 5;
        row[13] = 123;
        let row = AddressFamily::Ipv6.decode(&row);
        assert_eq!((row.state, row.pid), (5, 123));
        assert_eq!(
            row.key.unwrap(),
            FlowKey {
                local: "192.0.2.1:55000".parse().unwrap(),
                remote: "203.0.113.2:443".parse().unwrap(),
            }
        );
    }

    #[test]
    fn finds_pid_for_wildcard_and_exact_ipv4_udp_sockets() {
        for address in ["127.0.0.1:0", "0.0.0.0:0"] {
            let socket = std::net::UdpSocket::bind(address).unwrap();
            let local = SocketAddrV4::new(Ipv4Addr::LOCALHOST, socket.local_addr().unwrap().port());
            assert_eq!(find_udp_owner(local), Some(std::process::id()));
        }
    }

    #[tokio::test]
    async fn tcp_owner_uses_the_exact_ipv4_tuple() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (_server, _) = listener.accept().await.unwrap();
        let key = FlowKey {
            local: client.local_addr().unwrap().to_string().parse().unwrap(),
            remote: client.peer_addr().unwrap().to_string().parse().unwrap(),
        };
        assert_eq!(find_tcp_owner(key), Some(std::process::id()));
        assert!(list_running()
            .unwrap()
            .iter()
            .any(|p| p.executable_path.ends_with(".exe") && p.has_active_connections));
    }
}
