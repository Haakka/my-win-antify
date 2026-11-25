use std::{
    collections::{HashMap, HashSet},
    io,
    net::{Ipv4Addr, SocketAddrV4, SocketAddrV6},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use shared::{IpcError, IpcErrorCode, LogCategory, LogLevel, ProxyProtocol, RuleAction};
use tokio::{net::TcpListener, sync::watch, task::JoinHandle as Task};

use crate::{
    config::{is_private_or_lan_v4, is_private_or_lan_v6, Policy},
    packet::{parse_dns_query_name, FlowKey, Packet, PacketV6, UdpPacket},
    process::{self, Decision, Process, ProcessCache},
    relay::{self, Mapping, NatKey, NatTable},
    telemetry::Telemetry,
    udp,
    windivert::{self, Address, Divert, Driver},
};

pub(crate) type CurrentPolicy = Arc<RwLock<Arc<Policy>>>;

pub(crate) struct Engine {
    flow: Arc<Divert>,
    network: Arc<Divert>,
    threads: Vec<JoinHandle<()>>,
    relay: Task<io::Result<()>>,
    udp_task: Task<()>,
    shutdown: watch::Sender<bool>,
    shared: Arc<Runtime>,
}

impl Engine {
    pub async fn start(policy: CurrentPolicy, telemetry: Arc<Telemetry>) -> Result<Self, IpcError> {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))
            .await
            .map_err(internal)?;
        let relay_port = listener.local_addr().map_err(internal)?.port();
        let driver = Driver::load().map_err(driver_error)?;
        let flow = Arc::new(
            driver
                .open(
                    windivert::FLOW_FILTER,
                    windivert::FLOW,
                    windivert::FLOW_FLAGS,
                )
                .map_err(driver_error)?,
        );
        let network = Arc::new(
            driver
                .open(windivert::NETWORK_FILTER, windivert::NETWORK, 0)
                .map_err(driver_error)?,
        );
        let (udp, udp_task) = udp::Relay::start(Arc::downgrade(&network), Arc::clone(&telemetry));
        let shared = Arc::new(Runtime {
            pid: std::process::id(),
            policy,
            telemetry,
            flows: Mutex::new(FlowTable::default()),
            processes: Mutex::new(ProcessCache::default()),
            logged_proxy_pids: Mutex::new(HashSet::new()),
            nat: Arc::new(NatTable::default()),
            udp,
            udp_owners: Mutex::new(HashMap::new()),
            relay_port,
            stopping: AtomicBool::new(false),
            failure: Mutex::new(None),
        });
        let proxies = shared
            .processes
            .lock()
            .unwrap()
            .active_local_proxy_processes(&shared.policy().local_proxy_ports);
        for process in &proxies {
            shared.log_bypassed_proxy(process);
        }
        let (shutdown, cancel) = watch::channel(false);
        let relay = tokio::spawn(relay::run(
            listener,
            Arc::clone(&shared.nat),
            Arc::clone(&shared.telemetry),
            cancel,
        ));
        let mut engine = Self {
            flow,
            network,
            threads: Vec::new(),
            relay,
            udp_task,
            shutdown,
            shared,
        };
        engine.worker(
            "windivert-flow",
            Arc::clone(&engine.flow),
            Runtime::receive_flows,
        )?;
        engine.worker(
            "windivert-network",
            Arc::clone(&engine.network),
            Runtime::receive_packets,
        )?;
        Ok(engine)
    }

    fn worker(
        &mut self,
        name: &'static str,
        handle: Arc<Divert>,
        run: fn(&Runtime, &Divert) -> io::Result<()>,
    ) -> Result<(), IpcError> {
        let runtime = Arc::clone(&self.shared);
        let flow = Arc::clone(&self.flow);
        let network = Arc::clone(&self.network);
        let task = thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                if let Err(error) = run(&runtime, &handle) {
                    if !runtime.stopping.swap(true, Ordering::AcqRel) {
                        *runtime.failure.lock().unwrap() = Some(format!("{name}: {error}"));
                        network.stop_receiving();
                        flow.stop_receiving();
                    }
                }
            })
            .map_err(internal)?;
        self.threads.push(task);
        Ok(())
    }

    pub fn on_policy_updated(&self, new_policy: Arc<Policy>) {
        self.shared.on_policy_updated(new_policy);
    }

    pub fn maintain(&self) -> Option<String> {
        if let Some(error) = self.shared.failure.lock().unwrap().clone() {
            return Some(error);
        }
        if self.relay.is_finished() {
            return Some("Transparent relay listener exited".into());
        }
        if self.udp_task.is_finished() {
            return Some("Transparent UDP dispatcher exited".into());
        }
        if self.threads.iter().any(JoinHandle::is_finished) {
            return Some("WinDivert worker exited".into());
        }
        self.shared.expire();
        None
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        self.shutdown.send_replace(true);
        self.shared.udp.reset();
        self.udp_task.abort();
        self.network.stop_receiving();
        self.flow.stop_receiving();
        {
            // Wait for any in-progress route selection to finish before clearing NAT.
            let _flows = self.shared.flows.lock().unwrap();
            self.shared.nat.clear();
        }
        self.relay.abort();
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                tracing::error!("WinDivert worker panicked");
            }
        }
        self.shared.telemetry.finish_all();
    }
}

fn proxy_allows_lan(decision: &Decision) -> bool {
    decision.proxy.as_ref().is_some_and(|proxy| proxy.allow_lan)
}

fn ipv6_action(policy: &Policy, lineage: &[Process], remote: SocketAddrV6) -> RuleAction {
    let decision = policy.rules.decide_ipv6(lineage, remote.port());
    if decision.action == RuleAction::Proxy
        && is_private_or_lan_v6(*remote.ip())
        && !proxy_allows_lan(&decision)
    {
        RuleAction::Direct
    } else {
        decision.action
    }
}

fn internal(error: io::Error) -> IpcError {
    IpcError::new(IpcErrorCode::Internal, error.to_string())
}

fn driver_error(error: io::Error) -> IpcError {
    let code = if error.kind() == io::ErrorKind::PermissionDenied {
        IpcErrorCode::AccessDenied
    } else {
        IpcErrorCode::DriverUnavailable
    };
    IpcError::new(code, format!("WinDivert: {error}"))
}

struct Runtime {
    pid: u32,
    policy: CurrentPolicy,
    telemetry: Arc<Telemetry>,
    flows: Mutex<FlowTable>,
    processes: Mutex<ProcessCache>,
    logged_proxy_pids: Mutex<HashSet<u32>>,
    nat: Arc<NatTable>,
    udp: udp::Relay,
    udp_owners: Mutex<HashMap<FlowKey, (u32, u64)>>,
    relay_port: u16,
    stopping: AtomicBool,
    failure: Mutex<Option<String>>,
}

#[derive(Default)]
struct FlowTable {
    active: HashMap<FlowKey, Flow>,
    retired_relays: HashMap<FlowKey, RetiredRelay>,
}

struct RetiredRelay {
    initial_seq: Option<u32>,
    endpoint: Option<u64>,
    since: Instant,
}

struct Flow {
    decision: Decision,
    owner: Process,
    bypass: bool,
    endpoint: Option<u64>,
    route: Option<Route>,
    initial_seq: Option<u32>,
    last_packet: Instant,
}

#[derive(Clone, Copy)]
enum Route {
    Pass,
    Direct(u64),
    Relay(u64),
    Block,
}

impl Flow {
    fn new(owner: Process, decision: Decision, bypass: bool, endpoint: Option<u64>) -> Self {
        Self {
            decision,
            owner,
            bypass,
            endpoint,
            route: None,
            initial_seq: None,
            last_packet: Instant::now(),
        }
    }
}

impl Runtime {
    fn policy(&self) -> Arc<Policy> {
        Arc::clone(&self.policy.read().unwrap())
    }

    fn on_policy_updated(&self, new_policy: Arc<Policy>) {
        let mut flows = self.flows.lock().unwrap();
        let previous = self.policy();
        let FlowTable {
            active,
            retired_relays,
        } = &mut *flows;
        active.retain(|&key, flow| {
            if matches!(flow.route, Some(Route::Relay(_))) {
                // Retire the reflected socket. Only a fresh SYN can reuse its tuple.
                retired_relays.insert(
                    key,
                    RetiredRelay {
                        initial_seq: flow.initial_seq,
                        endpoint: flow.endpoint,
                        since: Instant::now(),
                    },
                );
                return false;
            }
            if flow.decision.action != RuleAction::Proxy {
                let (_, decision, bypass) = self.classify(flow.owner.pid, key.remote, &new_policy);
                if flow.decision == decision
                    && flow.bypass == bypass
                    && previous.must_bypass_v4(key.remote) == new_policy.must_bypass_v4(key.remote)
                {
                    return true;
                }
            }
            self.finish(key, flow.route);
            false
        });
        for id in self.nat.clear() {
            self.telemetry.finish(id, None);
        }
        let mut policy = self.policy.write().unwrap();
        self.udp.reset();
        *policy = new_policy;
    }

    fn log_bypassed_proxy(&self, process: &Process) {
        let first_match = self.logged_proxy_pids.lock().unwrap().insert(process.pid);
        if first_match {
            self.telemetry.log(
                LogLevel::Info,
                LogCategory::Service,
                "Auto-bypassed local proxy process",
                Some(format!("{} (PID {})", process.name, process.pid)),
            );
        }
    }

    fn lineage_with_proxy(&self, pid: u32, policy: &Policy) -> (Vec<Process>, Option<Process>) {
        let mut processes = self.processes.lock().unwrap();
        let lineage = processes.lineage(pid);
        let proxy = processes.local_proxy_match(&lineage, &policy.local_proxy_ports);
        (lineage, proxy)
    }

    fn classify(
        &self,
        pid: u32,
        remote: SocketAddrV4,
        policy: &Policy,
    ) -> (Process, Decision, bool) {
        self.classify_matching(pid, policy, |lineage| policy.rules.decide(lineage, remote))
    }

    fn classify_matching(
        &self,
        pid: u32,
        policy: &Policy,
        decide: impl FnOnce(&[Process]) -> Decision,
    ) -> (Process, Decision, bool) {
        if pid == 0 || pid == self.pid {
            return (Process::unknown(pid), Decision::direct(), true);
        }
        let (lineage, proxy) = self.lineage_with_proxy(pid, policy);
        let owner = lineage
            .first()
            .cloned()
            .unwrap_or_else(|| Process::unknown(pid));
        if let Some(proxy) = proxy {
            self.log_bypassed_proxy(&proxy);
            return (owner, Decision::direct(), true);
        }
        (owner, decide(&lineage), false)
    }

    fn receive_flows(&self, handle: &Divert) -> io::Result<()> {
        while !self.stopping.load(Ordering::Acquire) {
            let (_, address) = handle.recv(&mut [])?;
            let event = address.flow();
            let key = FlowKey {
                local: SocketAddrV4::new(Ipv4Addr::from(event.local_addr[0]), event.local_port),
                remote: SocketAddrV4::new(Ipv4Addr::from(event.remote_addr[0]), event.remote_port),
            };
            if event.protocol == 17 {
                const OWNER_CACHE_LIMIT: usize = 16_384;
                let mut owners = self.udp_owners.lock().unwrap();
                match address.event() {
                    windivert::FLOW_ESTABLISHED => {
                        if owners.len() >= OWNER_CACHE_LIMIT {
                            owners.clear(); // Misses use GetExtendedUdpTable.
                        }
                        owners.insert(key, (event.process_id, event.endpoint_id));
                    }
                    windivert::FLOW_DELETED
                        if owners
                            .get(&key)
                            .is_some_and(|&(_, id)| id == event.endpoint_id) =>
                    {
                        owners.remove(&key);
                    }
                    _ => {}
                }
                continue;
            }
            match address.event() {
                windivert::FLOW_ESTABLISHED => {
                    // Preserve the route already selected on SYN, including direct connections.
                    let mut flows = self.flows.lock().unwrap();
                    if flows.retired_relays.contains_key(&key) {
                        continue;
                    }
                    if let Some(existing) = flows.active.get_mut(&key).filter(|flow| {
                        flow.owner.pid == event.process_id
                            && flow.endpoint.is_none_or(|id| id == event.endpoint_id)
                    }) {
                        existing.endpoint = Some(event.endpoint_id);
                    } else {
                        let (owner, decision, bypass) =
                            self.classify(event.process_id, key.remote, &self.policy());
                        let flow = Flow::new(owner, decision, bypass, Some(event.endpoint_id));
                        if let Some(old) = flows.active.insert(key, flow) {
                            self.finish(key, old.route);
                        }
                    }
                }
                windivert::FLOW_DELETED => {
                    let mut flows = self.flows.lock().unwrap();
                    if flows
                        .retired_relays
                        .get(&key)
                        .is_some_and(|relay| relay.endpoint == Some(event.endpoint_id))
                    {
                        flows.retired_relays.remove(&key);
                    }
                    if flows
                        .active
                        .get(&key)
                        .is_some_and(|flow| flow.endpoint == Some(event.endpoint_id))
                    {
                        let old = flows.active.remove(&key).unwrap();
                        self.finish(key, old.route);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn receive_packets(&self, handle: &Divert) -> io::Result<()> {
        let mut buffer = vec![0; 40 + usize::from(u16::MAX)];
        let mut last_warning: Option<Instant> = None;
        loop {
            let (len, address) = handle.recv(&mut buffer)?;
            let bytes = &mut buffer[..len];
            let result = if self.stopping.load(Ordering::Acquire) {
                // Queued writes from the retiring relay carry reflected endpoints.
                if Packet::parse(bytes).is_some_and(|p| {
                    if address.outbound() {
                        p.key.local.port() == self.relay_port
                    } else {
                        p.key.remote.port() == self.relay_port
                    }
                }) {
                    Ok(())
                } else {
                    handle.send(bytes, &address)
                }
            } else {
                self.route_packet(handle, address, bytes)
            };
            if let Err(error) = result {
                if last_warning.is_none_or(|when| when.elapsed() >= Duration::from_secs(1)) {
                    tracing::warn!(%error, "WinDivert packet injection failed");
                    last_warning = Some(Instant::now());
                }
            }
        }
    }

    fn route_packet(&self, handle: &Divert, address: Address, bytes: &mut [u8]) -> io::Result<()> {
        if bytes.first().is_some_and(|b| b >> 4 == 6) {
            return self.route_ipv6(handle, address, bytes);
        }
        if let Some(packet) = UdpPacket::parse(bytes) {
            if address.outbound() && !self.route_udp(&packet, address, bytes) {
                return Ok(());
            }
            return handle.send(bytes, &address);
        }
        let Some(packet) = Packet::parse(bytes) else {
            return handle.send(bytes, &address);
        };
        let key = packet.key;
        if !address.outbound() {
            // Only reflected packets may reach the wildcard relay listener. WinDivert does
            // not recapture packets injected by this handle at its own priority.
            if key.remote.port() == self.relay_port {
                return Ok(());
            }
            let flows = self.flows.lock().unwrap();
            let key = key.reversed();
            if flows.retired_relays.contains_key(&key) {
                return Ok(());
            }
            match flows.active.get(&key).and_then(|flow| flow.route) {
                Some(Route::Direct(id)) => self.telemetry.bytes(id, 0, packet.payload_len),
                Some(Route::Block | Route::Relay(_)) => return Ok(()),
                _ => {}
            }
            drop(flows);
            return handle.send(bytes, &address);
        }
        if key.local.port() == self.relay_port {
            if let Some(mapping) = self.nat.find(NatKey::from_relay_reply(key)) {
                packet.reverse(bytes, mapping.flow);
                return handle.reflect(bytes, address, mapping.interface);
            }
            // Do not leak stale relay replies to the original public address.
            return Ok(());
        }
        let route = self.route(&packet, address.interface());
        match route {
            Route::Pass => handle.send(bytes, &address),
            Route::Direct(id) => {
                self.telemetry.bytes(id, packet.payload_len, 0);
                handle.send(bytes, &address)
            }
            Route::Block => self.send_reset(handle, address, &packet),
            Route::Relay(id) => {
                if self
                    .nat
                    .find(NatKey::from_flow(key))
                    .is_some_and(|mapping| mapping.id == id)
                {
                    packet.forward(bytes, self.relay_port);
                    handle.reflect(bytes, address, address.interface())
                } else {
                    self.send_reset(handle, address, &packet)
                }
            }
        }
    }

    // True asks the receive worker to reinject the original outbound datagram.
    fn route_udp(&self, packet: &UdpPacket<'_>, address: Address, bytes: &[u8]) -> bool {
        let key = packet.key;
        let (policy, revision) = {
            let policy = self.policy.read().unwrap();
            (Arc::clone(&policy), self.udp.revision())
        };
        if policy.must_bypass_v4(key.remote) || self.stopping.load(Ordering::Acquire) {
            return true;
        }
        let cached = self
            .udp_owners
            .lock()
            .unwrap()
            .get(&key)
            .map(|&(pid, _)| pid);
        let Some(pid) = cached.or_else(|| process::find_udp_owner(key.local)) else {
            return true;
        };
        let dns = key.remote.port() == 53;
        let domain = dns.then(|| parse_dns_query_name(packet.payload)).flatten();
        let (owner, mut decision, bypass) = self.classify_matching(pid, &policy, |lineage| {
            policy
                .rules
                .decide_datagram(lineage, key.remote, domain.as_deref())
        });
        if bypass {
            return true;
        }

        let mut reason = None;
        let mut code = None;
        let mut path = None;
        if decision.action == RuleAction::Proxy {
            let ip = *key.remote.ip();
            if policy
                .bypass_hosts
                .as_ref()
                .is_some_and(|hosts| hosts.matches(ip, domain.as_deref()))
            {
                reason = Some("Bypassed via bypassHosts");
            } else if decision
                .proxy
                .as_ref()
                .is_some_and(|proxy| proxy.protocol == ProxyProtocol::DirectInterface)
            {
                reason = Some("DirectInterface uses the app's UDP socket");
            } else if !dns && is_private_or_lan_v4(ip) && !proxy_allows_lan(&decision) {
                reason = Some("Private LAN fallback (allowLan=false)");
            } else if let Some(proxy) = &decision.proxy {
                let supports_udp = proxy.protocol == ProxyProtocol::Socks5 && proxy.udp_relay;
                if supports_udp {
                    path = Some(udp::Path::Associate);
                } else if dns {
                    path = Some(udp::Path::DnsTcp);
                    reason = Some("DNS-over-TCP via proxy");
                } else if proxy.fallback_direct && !decision.network_lock {
                    reason = Some("Fallback direct after proxy failure");
                    code = Some("protocol_error");
                }
            }
            if reason.is_some() && path.is_none() {
                decision.use_direct();
            }
        }

        let settings = &policy.config.settings;
        let visible = settings.record_connections
            && (decision.rule_id.is_some()
                || settings.discover_unmatched
                || decision.action != RuleAction::Direct);
        if !visible && decision.action == RuleAction::Direct && decision.rule_id.is_none() {
            return true;
        }
        let id = self.telemetry.open_udp(
            key,
            &owner,
            &decision,
            visible,
            domain,
            Some(&policy.config.active_profile_id),
        );
        self.telemetry.route_detail(id, reason, code);
        match decision.action {
            RuleAction::Direct => {
                self.telemetry.bytes(id, packet.payload.len(), 0);
                self.telemetry.finish(id, None);
                true
            }
            RuleAction::Block => {
                self.telemetry.route_detail(id, None, Some("blocked"));
                self.telemetry
                    .finish(id, Some("Blocked by routing policy".into()));
                false
            }
            RuleAction::Proxy => {
                if let (Some(path), Some(proxy)) = (path, decision.proxy) {
                    let allow_fallback = proxy.fallback_direct && !decision.network_lock;
                    self.udp.submit(
                        udp::Request {
                            id,
                            key,
                            owner: owner.pid,
                            address,
                            bytes: bytes.to_vec(),
                            proxy,
                            allow_fallback,
                            path,
                        },
                        revision,
                    );
                } else {
                    self.telemetry
                        .route_detail(id, None, Some("protocol_error"));
                    self.telemetry.finish(
                        id,
                        Some("No configured upstream supports this UDP flow".into()),
                    );
                }
                false
            }
        }
    }

    fn route_ipv6(&self, handle: &Divert, address: Address, bytes: &[u8]) -> io::Result<()> {
        if !address.outbound() {
            return handle.send(bytes, &address);
        }
        let Some(packet) = PacketV6::parse(bytes).filter(PacketV6::initial_syn) else {
            return handle.send(bytes, &address);
        };
        let policy = self.policy();
        if policy.must_bypass_v6(packet.key.remote) {
            return handle.send(bytes, &address);
        }
        let Some(pid) = process::find_tcp_owner_v6(packet.key) else {
            return handle.send(bytes, &address);
        };
        if pid == 0 || pid == self.pid {
            return handle.send(bytes, &address);
        }
        let (lineage, proxy) = self.lineage_with_proxy(pid, &policy);
        if let Some(proxy) = proxy {
            self.log_bypassed_proxy(&proxy);
            return handle.send(bytes, &address);
        }
        if ipv6_action(&policy, &lineage, packet.key.remote) == RuleAction::Direct {
            return handle.send(bytes, &address);
        }
        // IPv6 is filtered but still has no transparent relay.
        handle.reflect(&mut packet.block_reset(), address, address.interface())
    }

    fn send_reset(&self, handle: &Divert, address: Address, packet: &Packet) -> io::Result<()> {
        if packet.reset() {
            return Ok(()); // Never respond to a reset with another reset.
        }
        handle.reflect(&mut packet.block_reset(), address, address.interface())
    }

    fn route(&self, packet: &Packet, interface: [u32; 2]) -> Route {
        let key = packet.key;
        let initial_seq = packet.initial_syn().then_some(packet.seq);
        let mut flows = self.flows.lock().unwrap();
        if self.stopping.load(Ordering::Acquire) || self.policy().must_bypass_v4(key.remote) {
            return Route::Pass;
        }
        if let Some(retired) = flows.retired_relays.get(&key) {
            if initial_seq.is_none() || retired.initial_seq == initial_seq {
                return Route::Block;
            }
            flows.retired_relays.remove(&key);
        }
        if let Some(flow) = flows.active.get_mut(&key) {
            flow.last_packet = Instant::now();
            if let Some(route) = flow.route {
                if initial_seq.is_none() || flow.initial_seq == initial_seq {
                    return route;
                }
                let old = flows.active.remove(&key).unwrap();
                self.finish(key, old.route);
            }
        }

        // Query ownership outside the flow lock on misses: FLOW may arrive during this
        // synchronous lookup. Reuse that event's entry if it arrived first.
        let fallback_pid = if flows.active.contains_key(&key) {
            None
        } else {
            drop(flows);
            let pid = process::find_tcp_owner(key);
            flows = self.flows.lock().unwrap();
            pid
        };

        // Policy publication and flow invalidation use this same flow lock.
        let policy = self.policy();
        if self.stopping.load(Ordering::Acquire) || policy.must_bypass_v4(key.remote) {
            return Route::Pass;
        }
        let flow = match flows.active.entry(key) {
            std::collections::hash_map::Entry::Occupied(slot) => slot.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => {
                let Some(pid) = fallback_pid else {
                    return Route::Pass;
                };
                let (owner, decision, bypass) = self.classify(pid, key.remote, &policy);
                slot.insert(Flow::new(owner, decision, bypass, None))
            }
        };
        flow.initial_seq = initial_seq;
        let route = self.select_route(key, flow, &policy, interface);
        flow.route = Some(route);
        route
    }

    fn select_route(
        &self,
        key: FlowKey,
        flow: &Flow,
        policy: &Policy,
        interface: [u32; 2],
    ) -> Route {
        if flow.bypass || flow.owner.pid == self.pid || policy.must_bypass_v4(key.remote) {
            return Route::Pass;
        }
        let mut decision = flow.decision.clone();
        let remote_ip = *key.remote.ip();
        let mut bypass_hosts = None;
        let mut reason = None;
        if decision.action == RuleAction::Proxy {
            let hosts = policy.bypass_hosts.as_ref();
            let bypass_ip = hosts.is_some_and(|hosts| hosts.matches_ip(remote_ip));
            let bypass_lan = is_private_or_lan_v4(remote_ip) && !proxy_allows_lan(&decision);
            if bypass_ip || bypass_lan {
                decision.use_direct();
                reason = Some(if bypass_ip {
                    "Bypassed via bypassHosts"
                } else {
                    "Private LAN fallback (allowLan=false)"
                });
            } else if let Some(hosts) = hosts.filter(|hosts| hosts.needs_domain(remote_ip)) {
                bypass_hosts = Some(Arc::clone(hosts));
            }
        }
        // Only new SYNs can establish a transparent relay. A connection already
        // established before StartEngine continues on its original direct socket.
        let needs_relay = decision.pending_hosts.is_some() || decision.action == RuleAction::Proxy;
        if flow.initial_seq.is_none() && needs_relay {
            decision = Decision::direct();
            reason = Some("TCP connection established before interception");
        }
        let settings = &policy.config.settings;
        let visible = settings.record_connections
            && (decision.rule_id.is_some()
                || settings.discover_unmatched
                || decision.action != RuleAction::Direct);
        let id = self.telemetry.open(
            key,
            &flow.owner,
            &decision,
            visible,
            Some(&policy.config.active_profile_id),
        );
        if reason.is_some() {
            self.telemetry.route_detail(id, reason, None);
        }
        if decision.pending_hosts.is_some() || decision.action == RuleAction::Proxy {
            let mapping = Mapping {
                flow: key,
                id,
                decision,
                bypass_hosts,
                interface,
            };
            return if self.nat.insert(NatKey::from_flow(key), mapping) {
                Route::Relay(id)
            } else {
                self.telemetry
                    .route_detail(id, None, Some("protocol_error"));
                self.telemetry.finish(
                    id,
                    Some("The reflected TCP endpoint is already in use".into()),
                );
                Route::Block
            };
        }
        if decision.action == RuleAction::Block {
            self.telemetry.route_detail(id, None, Some("blocked"));
            self.telemetry
                .finish(id, Some("Blocked by routing policy".into()));
            Route::Block
        } else {
            Route::Direct(id)
        }
    }

    fn finish(&self, key: FlowKey, route: Option<Route>) {
        match route {
            Some(Route::Direct(id)) => self.telemetry.finish(id, None),
            Some(Route::Relay(id)) => {
                if self.nat.remove_unclaimed(key, id) {
                    self.telemetry.finish(id, None);
                }
            }
            _ => {}
        }
    }

    fn expire(&self) {
        for mapping in self.nat.expire() {
            self.telemetry
                .route_detail(mapping.id, None, Some("timeout"));
            self.telemetry.finish(
                mapping.id,
                Some("Transparent relay handshake timed out".into()),
            );
        }
        let mut flows = self.flows.lock().unwrap();
        let idle_timeout = Duration::from_secs(300);
        flows
            .retired_relays
            .retain(|_, relay| relay.since.elapsed() <= idle_timeout);
        flows.active.retain(|&key, flow| {
            // Flows observed only in the synchronous table may lack a delete event.
            let idle = flow.endpoint.is_none() && flow.last_packet.elapsed() > idle_timeout;
            if !idle {
                return true;
            }
            let proxied = matches!(flow.route, Some(Route::Relay(id)) if self.nat.find(NatKey::from_flow(key)).is_some_and(|mapping| mapping.id == id));
            if !proxied {
                self.finish(key, flow.route);
                false
            } else {
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{AppConfig, ConnectionState, ProxyConfig, ProxyProtocol, Rule, RuleMatcher};

    const OWNER: u32 = u32::MAX - 1;

    fn config(action: RuleAction, allow_lan: bool) -> AppConfig {
        let mut config = AppConfig::default();
        config.settings.bypass_lan = true;
        config.proxies.push(ProxyConfig {
            id: "test-proxy".into(),
            name: "Test proxy".into(),
            protocol: ProxyProtocol::Socks5,
            host: "127.0.0.1".into(),
            port: 1080,
            auth: None,
            interface_name: None,
            allow_lan,
            udp_relay: false,
            skip_tls_verify: false,
            fallback_direct: false,
            enabled: true,
        });
        config.rules.push(Rule {
            id: "test-rule".into(),
            name: "Test rule".into(),
            profile_id: config.active_profile_id.clone(),
            enabled: true,
            matcher: RuleMatcher::ProcessName {
                name: Process::unknown(OWNER).name,
                include_children: false,
            },
            action,
            proxy_id: Some("test-proxy".into()),
            network_lock: false,
            priority: 0,
            target_hosts: Vec::new(),
            target_ports: Vec::new(),
        });
        config
    }

    async fn runtime(config: AppConfig) -> Runtime {
        let telemetry = Telemetry::new(None);
        let (udp, _) = udp::Relay::start(std::sync::Weak::new(), Arc::clone(&telemetry));
        Runtime {
            pid: std::process::id(),
            policy: Arc::new(RwLock::new(Arc::new(Policy::new(config).await))),
            telemetry,
            flows: Mutex::new(FlowTable::default()),
            processes: Mutex::new(ProcessCache::default()),
            logged_proxy_pids: Mutex::new(HashSet::new()),
            nat: Arc::new(NatTable::default()),
            udp,
            udp_owners: Mutex::new(HashMap::new()),
            relay_port: 61000,
            stopping: AtomicBool::new(false),
            failure: Mutex::new(None),
        }
    }

    fn syn(key: FlowKey, sequence: u32) -> Packet {
        let mut bytes = [0; 40];
        bytes[0] = 0x45;
        bytes[2..4].copy_from_slice(&40u16.to_be_bytes());
        bytes[9] = 6;
        bytes[12..16].copy_from_slice(&key.local.ip().octets());
        bytes[16..20].copy_from_slice(&key.remote.ip().octets());
        bytes[20..22].copy_from_slice(&key.local.port().to_be_bytes());
        bytes[22..24].copy_from_slice(&key.remote.port().to_be_bytes());
        bytes[24..28].copy_from_slice(&sequence.to_be_bytes());
        bytes[32] = 0x50;
        bytes[33] = 0x02;
        Packet::parse(&bytes).unwrap()
    }

    fn seed(runtime: &Runtime, key: FlowKey) {
        let owner = Process::unknown(OWNER);
        let decision = runtime
            .policy()
            .rules
            .decide(std::slice::from_ref(&owner), key.remote);
        let flow = Flow::new(owner, decision, false, None);
        runtime.flows.lock().unwrap().active.insert(key, flow);
    }

    #[tokio::test]
    async fn lan_block_wins_and_only_allow_lan_proxies_relay() {
        let key = FlowKey {
            local: "192.0.2.1:55000".parse().unwrap(),
            remote: "192.168.1.10:443".parse().unwrap(),
        };
        for (action, allow_lan) in [
            (RuleAction::Block, false),
            (RuleAction::Block, true),
            (RuleAction::Proxy, false),
            (RuleAction::Proxy, true),
        ] {
            let runtime = runtime(config(action, allow_lan)).await;
            seed(&runtime, key);
            let route = runtime.route(&syn(key, 3), [0, 0]);
            let snapshot = runtime.telemetry.connections(true, Some(1)).remove(0);
            assert_eq!(snapshot.rule_id.as_deref(), Some("test-rule"));
            let policy = runtime.policy();
            for ip in ["fd00::10", "fe80::10"] {
                let remote = SocketAddrV6::new(ip.parse().unwrap(), 443, 0, 0);
                let expected = if action == RuleAction::Proxy && !allow_lan {
                    RuleAction::Direct
                } else {
                    action
                };
                assert_eq!(
                    ipv6_action(&policy, &[Process::unknown(OWNER)], remote),
                    expected
                );
            }
            match (action, allow_lan) {
                (RuleAction::Block, _) => {
                    assert!(matches!(route, Route::Block));
                    assert_eq!(snapshot.action, RuleAction::Block);
                    assert_eq!(snapshot.state, ConnectionState::Failed);
                }
                (RuleAction::Proxy, false) => {
                    assert!(matches!(route, Route::Direct(_)));
                    assert_eq!(snapshot.action, RuleAction::Direct);
                    assert!(snapshot.proxy_id.is_none());
                }
                (RuleAction::Proxy, true) => {
                    assert!(matches!(route, Route::Relay(_)));
                    assert_eq!(snapshot.action, RuleAction::Proxy);
                }
                _ => unreachable!(),
            }
        }
    }

    #[tokio::test]
    async fn ip_bypass_resolves_proxy_direct_and_domain_bypass_is_carried_to_the_relay() {
        let mut config = config(RuleAction::Proxy, true);
        config.settings.bypass_hosts = vec!["203.0.113.0/24".into(), "*.example.org".into()];
        config.rules[0].target_hosts = vec!["*.example.org".into()];
        let runtime = runtime(config).await;
        let mut key = FlowKey {
            local: "192.0.2.1:55000".parse().unwrap(),
            remote: "203.0.113.2:443".parse().unwrap(),
        };
        seed(&runtime, key);
        let Route::Direct(id) = runtime.route(&syn(key, 3), [0, 0]) else {
            panic!("The IP bypass must resolve to Direct");
        };
        assert!(runtime.nat.find(NatKey::from_flow(key)).is_none());
        let snapshot = runtime.telemetry.connections(false, Some(1)).remove(0);
        assert_eq!(snapshot.id, id);
        assert_eq!(snapshot.action, RuleAction::Direct);
        assert_eq!(snapshot.rule_id.as_deref(), Some("test-rule"));
        assert!(snapshot.proxy_id.is_none());

        let owner = Process::unknown(OWNER);
        let policy = runtime.policy();
        let midstream = Flow::new(
            owner.clone(),
            policy.rules.decide(&[owner], key.remote),
            false,
            None,
        );
        assert!(matches!(
            runtime.select_route(key, &midstream, &policy, [0, 0]),
            Route::Direct(_)
        ));
        let snapshot = runtime.telemetry.connections(false, Some(1)).remove(0);
        assert_eq!(snapshot.rule_id.as_deref(), Some("test-rule"));

        key.remote = "198.51.100.2:443".parse().unwrap();
        seed(&runtime, key);
        assert!(matches!(
            runtime.route(&syn(key, 4), [0, 0]),
            Route::Relay(_)
        ));
        let mapping = runtime.nat.find(NatKey::from_flow(key)).unwrap();
        assert!(mapping.decision.pending_hosts.is_some());
        assert!(mapping
            .bypass_hosts
            .unwrap()
            .matches(*key.remote.ip(), Some("www.example.org")));
    }

    #[tokio::test]
    async fn hot_reload_retires_proxy_routes_and_applies_a_new_rule_to_reconnections() {
        let runtime = runtime(config(RuleAction::Proxy, false)).await;
        let key = FlowKey {
            local: "192.0.2.1:55000".parse().unwrap(),
            remote: "203.0.113.2:443".parse().unwrap(),
        };
        seed(&runtime, key);
        let Route::Relay(old_id) = runtime.route(&syn(key, 7), [0, 0]) else {
            panic!("Expected the original flow to start a relay");
        };
        let orphan = FlowKey {
            local: "192.0.2.1:55001".parse().unwrap(),
            ..key
        };
        seed(&runtime, orphan);
        assert!(matches!(
            runtime.route(&syn(orphan, 8), [0, 0]),
            Route::Relay(_)
        ));
        runtime.flows.lock().unwrap().active.remove(&orphan);

        runtime.on_policy_updated(Arc::new(
            Policy::new(config(RuleAction::Direct, false)).await,
        ));
        assert!(runtime.flows.lock().unwrap().active.is_empty());
        assert!(runtime.nat.find(NatKey::from_flow(key)).is_none());
        assert!(runtime.nat.find(NatKey::from_flow(orphan)).is_none());
        assert!(matches!(runtime.route(&syn(key, 7), [0, 0]), Route::Block));

        // Model ownership rediscovery for a new SYN using the same tuple.
        seed(&runtime, key);
        let Route::Direct(new_id) = runtime.route(&syn(key, 9), [0, 0]) else {
            panic!("Expected the reconnected flow to go direct");
        };
        assert_ne!(old_id, new_id);
        runtime.on_policy_updated(Arc::new(
            Policy::new(config(RuleAction::Block, false)).await,
        ));
        assert!(runtime.flows.lock().unwrap().active.is_empty());
        seed(&runtime, key);
        assert!(matches!(runtime.route(&syn(key, 10), [0, 0]), Route::Block));
    }
}
