use std::{
    collections::{BTreeMap, HashMap},
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use shared::{
    ConnectionSnapshot, IpcError, IpcErrorCode, RuleAction, TrafficDimensionItem,
    TrafficHistoryBucket, TrafficHistoryQuery, TrafficHistoryRange, TrafficHistoryResponse,
};

use crate::{config::atomic_write, now_ms};

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;
const HOURLY_RETENTION: i64 = 48;
const DAILY_RETENTION: i64 = 35;

pub(crate) struct TrafficHistory {
    path: PathBuf,
    state: Mutex<State>,
    writer: Mutex<()>,
}

#[derive(Default)]
struct State {
    data: History,
    dirty: bool,
}

type Buckets = BTreeMap<i64, HashMap<String, Aggregate>>;

#[derive(Clone, Default, Serialize, Deserialize)]
struct History {
    hourly: Buckets,
    daily: Buckets,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Aggregate {
    totals: Totals,
    rules: Dimensions,
    proxies: Dimensions,
    apps: Dimensions,
}

type Dimensions = HashMap<String, Dimension>;

#[derive(Clone, Serialize, Deserialize)]
struct Dimension {
    label: String,
    bytes: Bytes,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct Totals {
    proxied: Bytes,
    direct: Bytes,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct Bytes {
    up: u64,
    down: u64,
}

impl Bytes {
    fn add(&mut self, bytes: Self) {
        self.up = self.up.saturating_add(bytes.up);
        self.down = self.down.saturating_add(bytes.down);
    }
}

impl TrafficHistory {
    pub fn load(config_path: &Path) -> Self {
        let path = config_path.with_file_name("traffic-history.json");
        let mut data = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!(%error, "Could not parse traffic history");
                History::default()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => History::default(),
            Err(error) => {
                tracing::warn!(%error, "Could not read traffic history");
                History::default()
            }
        };
        data.prune(now_ms());
        Self {
            path,
            state: Mutex::new(State { data, dirty: false }),
            writer: Mutex::new(()),
        }
    }

    pub fn record(
        &self,
        at_ms: i64,
        profile_id: Option<&str>,
        connection: &ConnectionSnapshot,
        up: u64,
        down: u64,
    ) {
        let mut state = self.state.lock().unwrap();
        let data = &mut state.data;
        // An empty key represents the absence of a profile. Config requires nonempty IDs.
        let profile_id = profile_id.unwrap_or_default();
        let bytes = Bytes { up, down };
        record(
            &mut data.hourly,
            bucket_start(at_ms, HOUR),
            profile_id,
            connection,
            bytes,
        );
        record(
            &mut data.daily,
            bucket_start(at_ms, DAY),
            profile_id,
            connection,
            bytes,
        );
        data.prune(at_ms);
        state.dirty = true;
    }

    pub fn query(&self, query: TrafficHistoryQuery, now: i64) -> TrafficHistoryResponse {
        let state = self.state.lock().unwrap();
        let (source, step, count) = match query.range {
            TrafficHistoryRange::Last24h => (&state.data.hourly, HOUR, 24),
            TrafficHistoryRange::Month => (&state.data.daily, DAY, 30),
        };
        let last = bucket_start(now, step);
        let first = last - (count - 1) * step;
        let mut response = TrafficHistoryResponse {
            range: query.range,
            profile_id: query.profile_id,
            total_proxied_up: 0,
            total_proxied_down: 0,
            total_direct_up: 0,
            total_direct_down: 0,
            buckets: Vec::with_capacity(count as usize),
            top_rules: Vec::new(),
            top_proxies: Vec::new(),
            top_apps: Vec::new(),
        };
        let mut summary = Aggregate::default();
        for index in 0..count {
            let start = first + index * step;
            let mut totals = Totals::default();
            if let Some(profiles) = source.get(&start) {
                for (_, bucket) in profiles.iter().filter(|(id, _)| {
                    response
                        .profile_id
                        .as_ref()
                        .is_none_or(|filter| filter == *id)
                }) {
                    totals.proxied.add(bucket.totals.proxied);
                    totals.direct.add(bucket.totals.direct);
                    merge(&mut summary.rules, &bucket.rules);
                    merge(&mut summary.proxies, &bucket.proxies);
                    merge(&mut summary.apps, &bucket.apps);
                }
            }
            summary.totals.proxied.add(totals.proxied);
            summary.totals.direct.add(totals.direct);
            response.buckets.push(TrafficHistoryBucket {
                bucket_start_ms: start,
                label: bucket_label(start, query.range),
                proxied_up: totals.proxied.up,
                proxied_down: totals.proxied.down,
                direct_up: totals.direct.up,
                direct_down: totals.direct.down,
            });
        }
        response.total_proxied_up = summary.totals.proxied.up;
        response.total_proxied_down = summary.totals.proxied.down;
        response.total_direct_up = summary.totals.direct.up;
        response.total_direct_down = summary.totals.direct.down;
        response.top_rules = top(summary.rules);
        response.top_proxies = top(summary.proxies);
        response.top_apps = top(summary.apps);
        response
    }

    pub fn flush(&self) -> Result<(), IpcError> {
        let _writer = self.writer.lock().unwrap();
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            if !state.dirty {
                return Ok(());
            }
            state.data.prune(now_ms());
            state.dirty = false;
            state.data.clone()
        };
        let result = self.write(&snapshot);
        if result.is_err() {
            self.state.lock().unwrap().dirty = true;
        }
        result
    }

    pub fn clear(&self) -> Result<(), IpcError> {
        let _writer = self.writer.lock().unwrap();
        let mut state = self.state.lock().unwrap();
        let empty = State::default();
        self.write(&empty.data)?;
        *state = empty;
        Ok(())
    }

    fn write(&self, data: &History) -> Result<(), IpcError> {
        let bytes = serde_json::to_vec(data)
            .map_err(|error| IpcError::new(IpcErrorCode::ConfigIoError, error.to_string()))?;
        atomic_write(&self.path, &bytes)
    }
}

impl History {
    fn prune(&mut self, now: i64) {
        for (buckets, step, count) in [
            (&mut self.hourly, HOUR, HOURLY_RETENTION),
            (&mut self.daily, DAY, DAILY_RETENTION),
        ] {
            let oldest = bucket_start(now, step) - (count - 1) * step;
            while buckets
                .first_key_value()
                .is_some_and(|(&start, _)| start < oldest)
            {
                buckets.pop_first();
            }
        }
    }
}

fn record(
    buckets: &mut Buckets,
    start: i64,
    profile_id: &str,
    conn: &ConnectionSnapshot,
    bytes: Bytes,
) {
    let profiles = buckets.entry(start).or_default();
    if !profiles.contains_key(profile_id) {
        profiles.insert(profile_id.into(), Aggregate::default());
    }
    let bucket = profiles.get_mut(profile_id).unwrap();
    match conn.action {
        RuleAction::Proxy => bucket.totals.proxied.add(bytes),
        RuleAction::Direct => bucket.totals.direct.add(bytes),
        RuleAction::Block => return,
    }
    if let Some(id) = &conn.rule_id {
        add_dimension(
            &mut bucket.rules,
            id,
            conn.rule_name.as_deref().unwrap_or(id),
            bytes,
        );
    }
    if let Some(id) = &conn.proxy_id {
        add_dimension(
            &mut bucket.proxies,
            id,
            conn.proxy_name.as_deref().unwrap_or(id),
            bytes,
        );
    }
    add_dimension(&mut bucket.apps, &conn.app_name, &conn.app_name, bytes);
}

fn add_dimension(dimensions: &mut Dimensions, key: &str, label: &str, bytes: Bytes) {
    if let Some(item) = dimensions.get_mut(key) {
        item.bytes.add(bytes);
        if item.label != label {
            item.label = label.into();
        }
    } else {
        dimensions.insert(
            key.into(),
            Dimension {
                label: label.into(),
                bytes,
            },
        );
    }
}

fn merge(target: &mut Dimensions, source: &Dimensions) {
    for (key, item) in source {
        add_dimension(target, key, &item.label, item.bytes);
    }
}

fn top(dimensions: Dimensions) -> Vec<TrafficDimensionItem> {
    let mut items: Vec<_> = dimensions
        .into_iter()
        .map(|(key, item)| TrafficDimensionItem {
            key,
            label: item.label,
            up_bytes: item.bytes.up,
            down_bytes: item.bytes.down,
        })
        .collect();
    items.sort_unstable_by(|a, b| {
        b.up_bytes
            .saturating_add(b.down_bytes)
            .cmp(&a.up_bytes.saturating_add(a.down_bytes))
            .then_with(|| a.key.cmp(&b.key))
    });
    items.truncate(10);
    items
}

fn bucket_start(now: i64, step: i64) -> i64 {
    now.div_euclid(step) * step
}

#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
#[derive(Default)]
struct SystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    millisecond: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn FileTimeToSystemTime(file_time: *const FileTime, system_time: *mut SystemTime) -> i32;
}

fn bucket_label(start: i64, range: TrafficHistoryRange) -> String {
    // FILETIME is a UTC count of 100 ns ticks since 1601-01-01.
    let ticks = ((start + 11_644_473_600_000) as u64) * 10_000;
    let file_time = FileTime {
        low: ticks as u32,
        high: (ticks >> 32) as u32,
    };
    let mut time = SystemTime::default();
    unsafe { FileTimeToSystemTime(&file_time, &mut time) };
    match range {
        TrafficHistoryRange::Last24h => format!("{:02}:00 UTC", time.hour),
        TrafficHistoryRange::Month => format!("{}/{}", time.month, time.day),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{ConnectionProtocol, ConnectionState};

    fn connection(app: &str, action: RuleAction) -> ConnectionSnapshot {
        let proxied = action == RuleAction::Proxy;
        ConnectionSnapshot {
            id: 1,
            timestamp_ms: 0,
            protocol: ConnectionProtocol::Tcp,
            pid: 1,
            process_name: app.into(),
            executable_path: String::new(),
            app_name: app.into(),
            src_addr: String::new(),
            dst_addr: String::new(),
            domain: None,
            action,
            rule_id: Some(format!("rule-{app}")),
            rule_name: Some(format!("Rule for {app}")),
            proxy_id: proxied.then(|| "proxy".into()),
            proxy_name: proxied.then(|| "Proxy".into()),
            state: ConnectionState::Active,
            bytes_up: 0,
            bytes_down: 0,
            error: None,
            match_reason: None,
            failure_code: None,
        }
    }

    fn test_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "antify-history-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ))
    }

    fn query(range: TrafficHistoryRange, profile_id: Option<&str>) -> TrafficHistoryQuery {
        TrafficHistoryQuery {
            range,
            profile_id: profile_id.map(String::from),
        }
    }

    #[test]
    fn buckets_are_contiguous_and_dimensions_follow_profile_and_time_filters() {
        let history = TrafficHistory::load(&test_dir("buckets").join("config.json"));
        let day = 20_000 * DAY;
        let now = day + 3 * HOUR + 100;
        let proxy = connection("Client", RuleAction::Proxy);
        let direct = connection("Browser", RuleAction::Direct);
        history.record(day - DAY, Some("work"), &proxy, 100, 200);
        history.record(day - 50 * HOUR, Some("work"), &proxy, 10, 20);
        history.record(day - 35 * DAY, Some("work"), &proxy, 1, 2);
        history.record(now - HOUR, Some("work"), &proxy, 3, 7);
        history.record(now - HOUR, Some("home"), &proxy, 13, 17);
        history.record(now, Some("work"), &direct, 30, 20);
        history.record(now, None, &direct, 100, 200);

        let hours = history.query(query(TrafficHistoryRange::Last24h, Some("work")), now);
        assert_eq!(hours.buckets.len(), 24);
        assert_eq!(hours.buckets[22].bucket_start_ms, day + 2 * HOUR);
        assert_eq!(hours.buckets[22].label, "02:00 UTC");
        assert_eq!(hours.buckets[22].proxied_down, 7);
        assert_eq!(hours.buckets[23].direct_up, 30);
        assert!(hours
            .buckets
            .windows(2)
            .all(|b| b[1].bucket_start_ms - b[0].bucket_start_ms == HOUR));
        assert_eq!((hours.total_proxied_up, hours.total_proxied_down), (3, 7));
        assert_eq!((hours.total_direct_up, hours.total_direct_down), (30, 20));
        assert_eq!(
            hours
                .top_apps
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            ["Browser", "Client"]
        );
        assert_eq!(hours.top_rules[0].label, "Rule for Browser");
        assert_eq!(hours.top_proxies[0].up_bytes, 3);

        let days = history.query(query(TrafficHistoryRange::Month, None), now);
        assert_eq!(days.buckets.len(), 30);
        assert!(days
            .buckets
            .windows(2)
            .all(|b| b[1].bucket_start_ms - b[0].bucket_start_ms == DAY));
        assert_eq!(days.buckets[28].proxied_up, 100);
        assert_eq!(days.buckets[29].proxied_up, 16);
        assert_eq!(days.total_proxied_up, 126);
        assert_eq!(days.total_direct_down, 220);
        assert_eq!(days.top_proxies[0].up_bytes, 126);
        let state = history.state.lock().unwrap();
        assert!(!state.data.hourly.contains_key(&(day - 50 * HOUR)));
        assert!(!state.data.daily.contains_key(&(day - 35 * DAY)));
    }

    #[test]
    fn atomically_saves_reloads_and_clears_only_when_requested_or_dirty() {
        let dir = test_dir("flush");
        let config_path = dir.join("config.json");
        let path = config_path.with_file_name("traffic-history.json");
        let history = TrafficHistory::load(&config_path);
        history.flush().unwrap();
        assert!(!path.exists());
        let now = now_ms();
        let conn = connection("Client", RuleAction::Proxy);
        history.record(now, Some("work"), &conn, 10, 20);
        history.flush().unwrap();
        history.record(now, Some("work"), &conn, 5, 10);
        history.flush().unwrap();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let loaded = TrafficHistory::load(&config_path);
        let query = query(TrafficHistoryRange::Last24h, Some("work"));
        let response = loaded.query(query.clone(), now);
        assert_eq!(
            (response.total_proxied_up, response.total_proxied_down),
            (15, 30)
        );
        assert_eq!(response, history.query(query.clone(), now));
        loaded.clear().unwrap();
        let response = loaded.query(query.clone(), now);
        assert_eq!(response.total_proxied_up, 0);
        assert!(response.top_rules.is_empty());
        assert_eq!(
            response,
            TrafficHistory::load(&config_path).query(query, now)
        );
        fs::remove_file(&path).unwrap();
        loaded.flush().unwrap();
        assert!(!path.exists());
        fs::remove_dir(&dir).unwrap();
    }
}
