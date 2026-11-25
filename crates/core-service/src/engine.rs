use std::{
    collections::{HashMap, HashSet},
    io,
    net::{Ipv4Addr, SocketAddrV4},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use shared::{IpcError, IpcErrorCode, LogCategory, LogLevel, RuleAction};
use tokio::{net::TcpListener, task::JoinHandle as Task};

use crate::{
    config::Policy,
    packet::{FlowKey, Packet, PacketV6},
    process::{self, Decision, Process, ProcessCache},
    relay::{self, Mapping, NatKey, NatTable},
    telemetry::Telemetry,
    windivert::{self, Address, Divert, Driver},
};

pub(crate) type CurrentPolicy = Arc<RwLock<Arc<Policy>>>;

pub(crate) struct Engine {
    flow: Arc<Divert>,
    network: Arc<Divert>,
    threads: Vec<JoinHandle<()>>,
    relay: Task<io::Result<()>>,
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
        let shared = Arc::new(Runtime {
            pid: std::process::id(),
            policy,
            telemetry,
            flows: Mutex::new(HashMap::new()),
            processes: Mutex::new(ProcessCache::default()),
            logged_proxy_pids: Mutex::new(HashSet::new()),
            nat: Arc::new(NatTable::default()),
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
        let relay = tokio::spawn(relay::run(
            listener,
            Arc::clone(&shared.nat),
            Arc::clone(&shared.telemetry),
        ));
        let mut engine = Self {
            flow,
            network,
            threads: Vec::new(),
            relay,
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

    pub fn maintain(&self) -> Option<String> {
        if let Some(error) = self.shared.failure.lock().unwrap().clone() {
            return Some(error);
        }
        if self.relay.is_finished() {
            return Some("Transparent relay listener exited".into());
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
        self.network.stop_receiving();
        self.flow.stop_receiving();
        self.relay.abort();
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                tracing::error!("WinDivert worker panicked");
            }
        }
        self.shared.telemetry.finish_all();
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
    flows: Mutex<HashMap<FlowKey, Flow>>,
    processes: Mutex<ProcessCache>,
    logged_proxy_pids: Mutex<HashSet<u32>>,
    nat: Arc<NatTable>,
    relay_port: u16,
    stopping: AtomicBool,
    failure: Mutex<Option<String>>,
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
        (owner, policy.rules.decide(&lineage, remote), false)
    }

    fn receive_flows(&self, handle: &Divert) -> io::Result<()> {
        while !self.stopping.load(Ordering::Acquire) {
            let (_, address) = handle.recv(&mut [])?;
            let event = address.flow();
            let key = FlowKey {
                local: SocketAddrV4::new(Ipv4Addr::from(event.local_addr[0]), event.local_port),
                remote: SocketAddrV4::new(Ipv4Addr::from(event.remote_addr[0]), event.remote_port),
            };
            match address.event() {
                windivert::FLOW_ESTABLISHED => {
                    // Preserve the route already selected on SYN, including direct connections.
                    let (owner, decision, bypass) =
                        self.classify(event.process_id, key.remote, &self.policy());
                    let mut flows = self.flows.lock().unwrap();
                    if let Some(existing) = flows.get_mut(&key).filter(|flow| {
                        flow.owner.pid == event.process_id
                            && flow.endpoint.is_none_or(|id| id == event.endpoint_id)
                    }) {
                        existing.endpoint = Some(event.endpoint_id);
                    } else {
                        let flow = Flow::new(owner, decision, bypass, Some(event.endpoint_id));
                        if let Some(old) = flows.insert(key, flow) {
                            self.finish(key, old.route);
                        }
                    }
                }
                windivert::FLOW_DELETED => {
                    let mut flows = self.flows.lock().unwrap();
                    if flows
                        .get(&key)
                        .is_some_and(|flow| flow.endpoint == Some(event.endpoint_id))
                    {
                        let old = flows.remove(&key).unwrap();
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
            match flows.get(&key.reversed()).and_then(|flow| flow.route) {
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

    fn route_ipv6(&self, handle: &Divert, address: Address, bytes: &[u8]) -> io::Result<()> {
        if !address.outbound() {
            return handle.send(bytes, &address);
        }
        let Some(packet) = PacketV6::parse(bytes).filter(PacketV6::initial_syn) else {
            return handle.send(bytes, &address);
        };
        let policy = self.policy();
        if policy.bypass_v6(packet.key.remote) {
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
        if policy
            .rules
            .decide_ipv6(&lineage, packet.key.remote.port())
            .action
            == RuleAction::Direct
        {
            return handle.send(bytes, &address);
        }
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
        let policy = self.policy();
        let initial_seq = packet.initial_syn().then_some(packet.seq);
        {
            let mut flows = self.flows.lock().unwrap();
            if let Some(flow) = flows.get_mut(&key) {
                flow.last_packet = Instant::now();
                if let Some(route) = flow.route {
                    if initial_seq.is_none() || flow.initial_seq == initial_seq {
                        return route;
                    }
                    let old = flows.remove(&key).unwrap();
                    self.finish(key, old.route);
                }
            } else if policy.bypass(key.remote) {
                return Route::Pass;
            }
        }

        // Query ownership outside the flow lock on misses: FLOW may arrive during this
        // synchronous lookup. Reuse that event's entry if it arrived first.
        let missing = !self.flows.lock().unwrap().contains_key(&key);
        let fallback = missing
            .then(|| {
                let pid = process::find_tcp_owner(key)?;
                let (owner, decision, bypass) = self.classify(pid, key.remote, &policy);
                Some(Flow::new(owner, decision, bypass, None))
            })
            .flatten();

        let mut flows = self.flows.lock().unwrap();
        let flow = match flows.entry(key) {
            std::collections::hash_map::Entry::Occupied(slot) => slot.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => {
                let Some(fallback) = fallback else {
                    return Route::Pass;
                };
                slot.insert(fallback)
            }
        };
        if let Some(route) = flow.route {
            return route;
        }
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
        if flow.bypass || flow.owner.pid == self.pid || policy.bypass(key.remote) {
            return Route::Pass;
        }
        // Only new SYNs can establish a transparent relay. A connection already
        // established before StartEngine continues on its original direct socket.
        let needs_relay =
            flow.decision.pending_hosts.is_some() || flow.decision.action == RuleAction::Proxy;
        let decision = if flow.initial_seq.is_none() && needs_relay {
            Decision::direct()
        } else {
            flow.decision.clone()
        };
        let settings = &policy.config.settings;
        let visible = settings.record_connections
            && (decision.rule_id.is_some()
                || settings.discover_unmatched
                || decision.action != RuleAction::Direct);
        let id = self.telemetry.open(key, &flow.owner, &decision, visible);
        if decision.pending_hosts.is_some() || decision.action == RuleAction::Proxy {
            let mapping = Mapping {
                flow: key,
                id,
                decision,
                interface,
            };
            return if self.nat.insert(NatKey::from_flow(key), mapping) {
                Route::Relay(id)
            } else {
                self.telemetry.finish(
                    id,
                    Some("The reflected TCP endpoint is already in use".into()),
                );
                Route::Block
            };
        }
        if decision.action == RuleAction::Block {
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
            self.telemetry.finish(
                mapping.id,
                Some("Transparent relay handshake timed out".into()),
            );
        }
        self.flows.lock().unwrap().retain(|&key, flow| {
            // Flows observed only in the synchronous table may lack a delete event.
            let idle = flow.endpoint.is_none() && flow.last_packet.elapsed() > Duration::from_secs(300);
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
