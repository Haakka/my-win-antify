use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::Instant,
};

use shared::{
    ActivityLogEntry, ConnectionProtocol, ConnectionSnapshot, ConnectionState, IpcEvent,
    LogCategory, LogLevel, RuleAction, RuleMatchTick, ServiceStatus, TrafficStats,
};
use tokio::sync::broadcast;

use crate::{
    now_ms,
    packet::FlowKey,
    process::{normalized_path, Decision, Process},
};

const HISTORY_LIMIT: usize = 1000;

pub(crate) struct Telemetry {
    pub events: broadcast::Sender<IpcEvent>,
    state: Mutex<State>,
}

struct State {
    next_id: u64,
    active: HashMap<u64, Live>,
    history: VecDeque<ConnectionSnapshot>,
    logs: VecDeque<ActivityLogEntry>,
    last_matches: HashMap<String, i64>,
    totals: Totals,
    sample: Option<(Instant, Totals)>,
}

struct Live {
    snapshot: ConnectionSnapshot,
    app_key: String,
    visible: bool,
    managed: bool,
    dirty: bool,
}

#[derive(Clone, Copy, Default)]
struct Totals {
    proxy_up: u64,
    proxy_down: u64,
    direct_up: u64,
    direct_down: u64,
}

impl Telemetry {
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(512);
        Arc::new(Self {
            events,
            state: Mutex::new(State {
                next_id: 1,
                active: HashMap::new(),
                history: VecDeque::new(),
                logs: VecDeque::new(),
                last_matches: HashMap::new(),
                totals: Totals::default(),
                sample: None,
            }),
        })
    }

    pub fn open(&self, key: FlowKey, process: &Process, decision: &Decision, visible: bool) -> u64 {
        let mut s = self.state.lock().unwrap();
        let id = s.next_id;
        s.next_id += 1;
        let timestamp_ms = now_ms();
        if let Some(rule_id) = decision
            .rule_id
            .as_ref()
            .filter(|_| decision.pending_hosts.is_none())
        {
            s.last_matches.insert(rule_id.clone(), timestamp_ms);
        }
        let snapshot = ConnectionSnapshot {
            id,
            timestamp_ms,
            protocol: ConnectionProtocol::Tcp,
            pid: process.pid,
            process_name: process.name.clone(),
            executable_path: process.path.clone(),
            app_name: process.app_name(),
            src_addr: key.local.to_string(),
            dst_addr: key.remote.to_string(),
            domain: None,
            action: decision.action,
            rule_id: decision.rule_id.clone(),
            rule_name: decision.rule_name.clone(),
            proxy_id: decision.proxy.as_ref().map(|p| p.id.clone()),
            proxy_name: decision.proxy.as_ref().map(|p| p.name.clone()),
            state: if decision.action == RuleAction::Direct && decision.pending_hosts.is_none() {
                ConnectionState::Active
            } else {
                ConnectionState::Connecting
            },
            bytes_up: 0,
            bytes_down: 0,
            error: None,
        };
        // Block is emitted once as a terminal event by finish().
        if visible && decision.action != RuleAction::Block {
            let _ = self
                .events
                .send(IpcEvent::ConnectionEvent(snapshot.clone()));
        }
        let app_key = if process.path.is_empty() {
            process.name.clone()
        } else {
            normalized_path(&process.path)
        };
        s.active.insert(
            id,
            Live {
                snapshot,
                visible,
                app_key,
                managed: decision.rule_id.is_some() || decision.action != RuleAction::Direct,
                dirty: false,
            },
        );
        id
    }

    pub fn domain(&self, id: u64, domain: String) {
        let mut state = self.state.lock().unwrap();
        if let Some(live) = state.active.get_mut(&id) {
            live.snapshot.domain = Some(domain);
        }
    }

    pub fn resolve_route(&self, id: u64, action: RuleAction, rule_matched: bool) {
        let mut state = self.state.lock().unwrap();
        let Some(live) = state.active.get_mut(&id) else {
            return;
        };
        let snapshot = &mut live.snapshot;
        snapshot.action = action;
        if action != RuleAction::Proxy {
            snapshot.proxy_id = None;
            snapshot.proxy_name = None;
        }
        if !rule_matched {
            snapshot.rule_id = None;
            snapshot.rule_name = None;
        }
        live.managed = snapshot.rule_id.is_some() || action != RuleAction::Direct;
        if let Some(rule_id) = snapshot.rule_id.clone() {
            state.last_matches.insert(rule_id, crate::now_ms());
        }
    }

    pub fn activate(&self, id: u64) {
        let mut s = self.state.lock().unwrap();
        if let Some(live) = s.active.get_mut(&id) {
            live.snapshot.state = ConnectionState::Active;
            if live.visible {
                let _ = self
                    .events
                    .send(IpcEvent::ConnectionEvent(live.snapshot.clone()));
            }
        }
    }

    pub fn bytes(&self, id: u64, up: usize, down: usize) {
        if up == 0 && down == 0 {
            return;
        }
        let mut s = self.state.lock().unwrap();
        let Some(live) = s.active.get_mut(&id) else {
            return;
        };
        let (up, down) = (up as u64, down as u64);
        live.snapshot.bytes_up += up;
        live.snapshot.bytes_down += down;
        live.dirty = true;
        match live.snapshot.action {
            RuleAction::Proxy => {
                s.totals.proxy_up += up;
                s.totals.proxy_down += down;
            }
            RuleAction::Direct if live.managed => {
                s.totals.direct_up += up;
                s.totals.direct_down += down;
            }
            RuleAction::Direct | RuleAction::Block => {}
        }
    }

    pub fn finish(&self, id: u64, error: Option<String>) {
        let mut s = self.state.lock().unwrap();
        if let Some(mut live) = s.active.remove(&id) {
            live.snapshot.state = if error.is_some() {
                ConnectionState::Failed
            } else {
                ConnectionState::Closed
            };
            live.snapshot.error = error;
            if live.visible {
                let _ = self
                    .events
                    .send(IpcEvent::ConnectionEvent(live.snapshot.clone()));
                retain_recent(&mut s.history, live.snapshot);
            }
        }
    }

    pub fn finish_all(&self) {
        let ids: Vec<_> = self.state.lock().unwrap().active.keys().copied().collect();
        for id in ids {
            self.finish(id, None);
        }
    }

    pub fn connections(&self, include_closed: bool, limit: Option<u32>) -> Vec<ConnectionSnapshot> {
        let s = self.state.lock().unwrap();
        let mut snapshots: Vec<_> = s
            .active
            .values()
            .filter(|live| live.visible)
            .map(|live| &live.snapshot)
            .collect();
        if include_closed {
            snapshots.extend(s.history.iter());
        }
        snapshots.sort_unstable_by_key(|snapshot| std::cmp::Reverse(snapshot.id));
        snapshots
            .into_iter()
            .take(limit.unwrap_or(HISTORY_LIMIT as u32) as usize)
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        state.history.clear();
        for live in state.active.values_mut() {
            live.visible = false;
        }
    }

    pub fn log(&self, level: LogLevel, category: LogCategory, title: &str, detail: Option<String>) {
        let mut s = self.state.lock().unwrap();
        let entry = ActivityLogEntry {
            id: s.next_id,
            timestamp_ms: now_ms(),
            level,
            category,
            title: title.into(),
            detail,
        };
        s.next_id += 1;
        let _ = self.events.send(IpcEvent::ActivityLog(entry.clone()));
        retain_recent(&mut s.logs, entry);
    }

    pub fn logs(&self, limit: Option<u32>) -> Vec<ActivityLogEntry> {
        self.state
            .lock()
            .unwrap()
            .logs
            .iter()
            .rev()
            .take(limit.unwrap_or(HISTORY_LIMIT as u32) as usize)
            .cloned()
            .collect()
    }

    pub fn clear_logs(&self) {
        self.state.lock().unwrap().logs.clear();
    }

    pub fn retain_rules(&self, config: &shared::AppConfig) {
        let ids: HashSet<_> = config.rules.iter().map(|rule| rule.id.as_str()).collect();
        self.state
            .lock()
            .unwrap()
            .last_matches
            .retain(|id, _| ids.contains(id.as_str()));
    }

    pub fn tick(&self, status: &ServiceStatus) {
        let mut s = self.state.lock().unwrap();
        let now = Instant::now();
        let totals = s.totals;
        let (upload_bps, download_bps) =
            s.sample
                .replace((now, totals))
                .map_or((0, 0), |(prev, sample)| {
                    let seconds = now.duration_since(prev).as_secs_f64().max(0.001);
                    let rate = |delta: u64| (delta as f64 / seconds) as u64;
                    (
                        rate(
                            totals
                                .proxy_up
                                .saturating_sub(sample.proxy_up)
                                .saturating_add(totals.direct_up.saturating_sub(sample.direct_up)),
                        ),
                        rate(
                            totals
                                .proxy_down
                                .saturating_sub(sample.proxy_down)
                                .saturating_add(
                                    totals.direct_down.saturating_sub(sample.direct_down),
                                ),
                        ),
                    )
                });
        let mut apps = HashSet::new();
        let mut active_connections = 0;
        for live in s
            .active
            .values_mut()
            .filter(|live| live.snapshot.state == ConnectionState::Active)
        {
            if live.managed {
                apps.insert(&live.app_key);
                active_connections += 1;
            }
            if live.visible && live.dirty {
                live.dirty = false;
                let _ = self
                    .events
                    .send(IpcEvent::ConnectionEvent(live.snapshot.clone()));
            }
        }
        let active_apps = apps.len() as u32;
        let mut rule_last_matched: Vec<_> = s
            .last_matches
            .iter()
            .map(|(id, &time)| RuleMatchTick {
                rule_id: id.clone(),
                last_matched_at_ms: time,
            })
            .collect();
        rule_last_matched.sort_unstable_by(|a, b| a.rule_id.cmp(&b.rule_id));
        let _ = self.events.send(IpcEvent::StatsTick(TrafficStats {
            timestamp_ms: now_ms(),
            running: status.running,
            uptime_secs: status.uptime_secs,
            active_connections,
            active_apps,
            upload_bps,
            download_bps,
            total_proxied_up: totals.proxy_up,
            total_proxied_down: totals.proxy_down,
            total_direct_up: totals.direct_up,
            total_direct_down: totals.direct_down,
            rule_last_matched,
        }));
    }
}

fn retain_recent<T>(entries: &mut VecDeque<T>, entry: T) {
    if entries.len() == HISTORY_LIMIT {
        entries.pop_front();
    }
    entries.push_back(entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> ServiceStatus {
        ServiceStatus {
            running: true,
            started_at_ms: None,
            uptime_secs: 1,
            service_pid: 1,
            core_version: "test".into(),
            driver_ready: true,
            active_profile_id: "default".into(),
            last_error: None,
        }
    }

    fn open(telemetry: &Telemetry, pid: u32, decision: &Decision, visible: bool) -> u64 {
        telemetry.open(
            FlowKey {
                local: "192.0.2.1:50000".parse().unwrap(),
                remote: "203.0.113.1:443".parse().unwrap(),
            },
            &Process::unknown(pid),
            decision,
            visible,
        )
    }

    fn tick_events(
        telemetry: &Telemetry,
        events: &mut broadcast::Receiver<IpcEvent>,
    ) -> (TrafficStats, Vec<ConnectionSnapshot>) {
        telemetry.tick(&status());
        let mut updates = Vec::new();
        loop {
            match events.try_recv().unwrap() {
                IpcEvent::ConnectionEvent(connection) => updates.push(connection),
                IpcEvent::StatsTick(stats) => return (stats, updates),
                other => panic!("Unexpected event: {other:?}"),
            }
        }
    }

    #[test]
    fn ticks_count_managed_connections_and_publish_visible_dirty_bytes_once() {
        let telemetry = Telemetry::new();
        let direct_rule = Decision {
            rule_id: Some("direct".into()),
            ..Decision::direct()
        };
        let proxy = Decision {
            action: RuleAction::Proxy,
            ..Decision::direct()
        };
        let managed = open(&telemetry, 1, &direct_rule, true);
        let hidden = open(&telemetry, 1, &direct_rule, false);
        let background = open(&telemetry, 2, &Decision::direct(), true);
        let proxied = open(&telemetry, 3, &proxy, true);
        telemetry.activate(proxied);
        let connecting = open(&telemetry, 4, &proxy, true);
        let routed_direct = open(&telemetry, 5, &direct_rule, true);
        telemetry.resolve_route(routed_direct, RuleAction::Direct, false);

        let mut events = telemetry.events.subscribe();
        telemetry.bytes(managed, 10, 20);
        telemetry.bytes(managed, 5, 10);
        telemetry.bytes(hidden, 2, 3);
        telemetry.bytes(background, 100, 200);
        telemetry.bytes(proxied, 1, 4);
        telemetry.bytes(routed_direct, 16, 32);
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        let (stats, mut updates) = tick_events(&telemetry, &mut events);
        assert_eq!((stats.active_connections, stats.active_apps), (3, 2));
        assert_eq!((stats.total_direct_up, stats.total_direct_down), (17, 33));
        assert_eq!((stats.total_proxied_up, stats.total_proxied_down), (1, 4));
        updates.sort_by_key(|snapshot| snapshot.id);
        assert_eq!(
            updates
                .iter()
                .map(|snapshot| (snapshot.id, snapshot.bytes_up, snapshot.bytes_down))
                .collect::<Vec<_>>(),
            [
                (managed, 15, 30),
                (background, 100, 200),
                (proxied, 1, 4),
                (routed_direct, 16, 32)
            ],
        );
        assert!(updates
            .iter()
            .all(|snapshot| snapshot.state == ConnectionState::Active));
        assert!(tick_events(&telemetry, &mut events).1.is_empty());

        telemetry.bytes(connecting, 1, 1);
        let (stats, updates) = tick_events(&telemetry, &mut events);
        assert_eq!(stats.active_connections, 3);
        assert!(updates.is_empty());
    }

    #[test]
    fn tick_saturates_rate_deltas_if_counters_decrease() {
        let telemetry = Telemetry::new();
        let mut events = telemetry.events.subscribe();
        telemetry.state.lock().unwrap().sample = Some((
            Instant::now(),
            Totals {
                proxy_up: 1,
                proxy_down: 2,
                direct_up: 3,
                direct_down: 4,
            },
        ));
        let (stats, _) = tick_events(&telemetry, &mut events);
        assert_eq!((stats.upload_bps, stats.download_bps), (0, 0));
    }

    #[test]
    fn clear_logs_empties_history_and_keeps_subsequent_entries() {
        let telemetry = Telemetry::new();
        telemetry.log(LogLevel::Info, LogCategory::App, "Before clear", None);
        assert_eq!(telemetry.logs(None).len(), 1);
        telemetry.clear_logs();
        assert!(telemetry.logs(None).is_empty());
        telemetry.log(LogLevel::Info, LogCategory::App, "After clear", None);
        let logs = telemetry.logs(None);
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].title, "After clear");
    }
}
