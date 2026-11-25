use std::collections::{HashMap, VecDeque};

use shared::{ProxyConfig, ProxyHealthSnapshot, ProxyHealthStatus};

use crate::upstream::UpstreamError;

const WINDOW: usize = 20;

#[derive(Default)]
pub(crate) struct Health {
    entries: HashMap<String, Entry>,
    generation: u64,
}

struct Entry {
    config: ProxyConfig,
    samples: VecDeque<Option<Failure>>,
    stale: bool,
}

struct Failure {
    code: &'static str,
    message: String,
}

impl Health {
    pub fn sync_proxies(&mut self, proxies: &[ProxyConfig]) {
        let previous = std::mem::take(&mut self.entries);
        let mut previous = previous;
        for proxy in proxies {
            let entry = previous
                .remove(&proxy.id)
                .filter(|entry| entry.config == *proxy)
                .unwrap_or_else(|| Entry {
                    config: proxy.clone(),
                    samples: VecDeque::with_capacity(WINDOW),
                    stale: false,
                });
            self.entries.insert(proxy.id.clone(), entry);
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn mark_stale(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        for entry in self.entries.values_mut() {
            entry.samples.clear();
            entry.stale = true;
        }
    }

    pub fn record(
        &mut self,
        proxy: &ProxyConfig,
        generation: u64,
        result: Result<(), &UpstreamError>,
    ) {
        if generation != self.generation {
            return;
        }
        let Some(entry) = self
            .entries
            .get_mut(&proxy.id)
            .filter(|entry| entry.config == *proxy)
        else {
            return;
        };
        let failure = match result {
            Ok(()) => None,
            Err(error) if error.is_proxy_fault => Some(Failure {
                code: error.code,
                message: error.to_string(),
            }),
            Err(_) => return,
        };
        entry.stale = false;
        if entry.samples.len() == WINDOW {
            entry.samples.pop_front();
        }
        entry.samples.push_back(failure);
    }

    pub fn snapshots(&self) -> Vec<ProxyHealthSnapshot> {
        let mut snapshots: Vec<_> = self
            .entries
            .iter()
            .map(|(id, entry)| {
                let successes = entry
                    .samples
                    .iter()
                    .filter(|sample| sample.is_none())
                    .count();
                let score = (successes * 100)
                    .checked_div(entry.samples.len())
                    .unwrap_or(0) as u8;
                let status = if entry.stale {
                    ProxyHealthStatus::Stale
                } else if entry.samples.is_empty() {
                    ProxyHealthStatus::Unknown
                } else if score >= 80 {
                    ProxyHealthStatus::Healthy
                } else if score == 0 {
                    ProxyHealthStatus::Unreachable
                } else {
                    ProxyHealthStatus::Degraded
                };
                let last_failure = entry.samples.iter().rev().find_map(Option::as_ref);
                ProxyHealthSnapshot {
                    proxy_id: id.clone(),
                    score,
                    status,
                    last_error_code: last_failure.map(|failure| failure.code.into()),
                    last_error: last_failure.map(|failure| failure.message.clone()),
                }
            })
            .collect();
        snapshots.sort_unstable_by(|a, b| a.proxy_id.cmp(&b.proxy_id));
        snapshots
    }
}
