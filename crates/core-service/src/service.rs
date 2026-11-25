use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use shared::{
    AppConfig, CurrentWifiInfo, DiagnosticBundleSummary, IpcError, IpcErrorCode, IpcEvent,
    LogCategory, LogLevel, PongInfo, RpcRequest, RpcResult, RulesImportPreview, ServiceStatus,
    IPC_PROTOCOL_VERSION,
};
use tokio::sync::Mutex;

use crate::{
    config::{self, Policy},
    diagnostics::{self, DiagnosticData},
    engine::{CurrentPolicy, Engine},
    network_interfaces, now_ms, power_net, process,
    telemetry::Telemetry,
    traffic_history::TrafficHistory,
    upstream, wifi,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) struct Service {
    path: PathBuf,
    config_write: Mutex<()>,
    policy: CurrentPolicy,
    lifecycle: Mutex<Lifecycle>,
    current_wifi: RwLock<CurrentWifiInfo>,
    dismissed_conflicts: Mutex<HashSet<String>>,
    pub telemetry: Arc<Telemetry>,
}

#[derive(Default)]
struct Lifecycle {
    engine: Option<Engine>,
    started: Option<(i64, Instant)>,
    driver_ready: bool,
    last_error: Option<String>,
    config_error: Option<String>,
    invalid_rule_ids: Vec<String>,
    restart_on_network_resume: bool,
}

enum ConfigUpdate {
    Save(AppConfig),
    Activate(String),
}

impl Service {
    pub async fn new(path: PathBuf) -> Result<Arc<Self>, IpcError> {
        let (config, invalid_rule_ids, config_error) = match config::load_with_diagnostics(&path) {
            Ok((config, invalid_rule_ids)) => (config, invalid_rule_ids, None),
            Err(error) => {
                tracing::warn!(%error, "Config unavailable; proxy engine disabled");
                (AppConfig::default(), Vec::new(), Some(error.message))
            }
        };
        let policy = Policy::new(config).await;
        let history_path = path.clone();
        let (history, current_wifi) = tokio::task::spawn_blocking(move || {
            (
                TrafficHistory::load(&history_path),
                wifi::Session::open(false)
                    .and_then(|session| session.current())
                    .unwrap_or_else(wifi::unavailable),
            )
        })
        .await
        .map_err(internal)?;
        let telemetry = Telemetry::new(Some(history));
        telemetry.update_config(&policy.config);
        let _ = telemetry
            .events
            .send(IpcEvent::ConfigChanged(policy.config.clone()));
        Ok(Arc::new(Self {
            path,
            config_write: Mutex::new(()),
            policy: Arc::new(RwLock::new(Arc::new(policy))),
            lifecycle: Mutex::new(Lifecycle {
                config_error,
                invalid_rule_ids,
                ..Lifecycle::default()
            }),
            current_wifi: RwLock::new(current_wifi),
            dismissed_conflicts: Mutex::new(HashSet::new()),
            telemetry,
        }))
    }

    fn policy(&self) -> Arc<Policy> {
        Arc::clone(&self.policy.read().unwrap())
    }

    fn make_status(&self, state: &Lifecycle) -> ServiceStatus {
        ServiceStatus {
            running: state.engine.is_some(),
            started_at_ms: state.started.map(|(at, _)| at),
            uptime_secs: state.started.map_or(0, |(_, at)| at.elapsed().as_secs()),
            service_pid: std::process::id(),
            core_version: VERSION.into(),
            driver_ready: state.driver_ready,
            active_profile_id: self.policy().config.active_profile_id.clone(),
            last_error: state.last_error.clone(),
            config_error: state.config_error.clone(),
            invalid_rule_ids: state.invalid_rule_ids.clone(),
        }
    }

    fn status_changed(&self, state: &Lifecycle) -> ServiceStatus {
        let status = self.make_status(state);
        let _ = self
            .telemetry
            .events
            .send(IpcEvent::StatusChanged(status.clone()));
        status
    }

    async fn update_config(&self, change: ConfigUpdate) -> Result<AppConfig, IpcError> {
        let _guard = self.config_write.lock().await;
        let previous = self.policy();
        let (config, invalid_rule_ids) = match change {
            ConfigUpdate::Save(config) => {
                config::validate(&config)?;
                (config, Vec::new())
            }
            ConfigUpdate::Activate(profile_id) => {
                let state = self.lifecycle.lock().await;
                ensure_config_available(&state)?;
                if !previous
                    .config
                    .profiles
                    .iter()
                    .any(|profile| profile.id == profile_id)
                {
                    return Err(IpcError::new(
                        IpcErrorCode::NotFound,
                        "Profile does not exist",
                    ));
                }
                let mut config = previous.config.clone();
                config.active_profile_id = profile_id;
                (config, state.invalid_rule_ids.clone())
            }
        };
        self.commit_config(&previous.config, &config, invalid_rule_ids)
            .await?;
        Ok(config)
    }

    // Callers hold config_write through persistence and publication.
    async fn commit_config(
        &self,
        previous: &AppConfig,
        config: &AppConfig,
        invalid_rule_ids: Vec<String>,
    ) -> Result<(), IpcError> {
        let policy = Arc::new(Policy::new(config.clone()).await);
        let mut state = self.lifecycle.lock().await;
        config::save(&self.path, config)?;
        self.apply_policy(&mut state, policy, invalid_rule_ids);
        self.log_config_changes(previous, config);
        self.config_changed(&state, config);
        Ok(())
    }

    async fn import_rules(
        &self,
        json: &str,
        profile_id: &str,
        dry_run: bool,
    ) -> Result<RulesImportPreview, IpcError> {
        let _guard = self.config_write.lock().await;
        let previous = self.policy();
        let (updated, mut preview) = config::import_rules_json(&previous.config, json, profile_id)?;
        if !dry_run {
            if !preview.added.is_empty() {
                self.commit_config(&previous.config, &updated, Vec::new())
                    .await?;
            }
            preview.committed = true;
        }
        Ok(preview)
    }

    async fn restore_config_backup(&self) -> Result<AppConfig, IpcError> {
        let _guard = self.config_write.lock().await;
        let config = config::restore_latest_backup(&self.path)?;
        let policy = Arc::new(Policy::new(config.clone()).await);
        let mut state = self.lifecycle.lock().await;
        self.apply_policy(&mut state, policy, Vec::new());
        self.telemetry.log(
            LogLevel::Info,
            LogCategory::App,
            "Config backup restored",
            None,
        );
        self.config_changed(&state, &config);
        Ok(config)
    }

    fn apply_policy(
        &self,
        state: &mut Lifecycle,
        policy: Arc<Policy>,
        invalid_rule_ids: Vec<String>,
    ) {
        self.telemetry.update_config(&policy.config);
        self.publish_policy(state, policy);
        state.config_error = None;
        state.invalid_rule_ids = invalid_rule_ids;
    }

    fn publish_policy(&self, state: &Lifecycle, policy: Arc<Policy>) {
        if let Some(engine) = &state.engine {
            engine.on_policy_updated(policy);
        } else {
            *self.policy.write().unwrap() = policy;
        }
    }

    fn config_changed(&self, state: &Lifecycle, config: &AppConfig) {
        let _ = self
            .telemetry
            .events
            .send(IpcEvent::ConfigChanged(config.clone()));
        self.status_changed(state);
    }

    fn log_config_changes(&self, previous: &AppConfig, current: &AppConfig) {
        let info = |category, title, detail| {
            self.telemetry.log(LogLevel::Info, category, title, detail);
        };
        for proxy in &current.proxies {
            let title = match previous.proxies.iter().find(|old| old.id == proxy.id) {
                None => "Proxy added",
                Some(old) if old != proxy => "Proxy updated",
                _ => continue,
            };
            info(LogCategory::Proxy, title, Some(proxy.name.clone()));
        }
        for proxy in &previous.proxies {
            if !current.proxies.iter().any(|new| new.id == proxy.id) {
                info(
                    LogCategory::Proxy,
                    "Proxy deleted",
                    Some(proxy.name.clone()),
                );
            }
        }
        for rule in &current.rules {
            let Some(old) = previous.rules.iter().find(|old| old.id == rule.id) else {
                info(LogCategory::Rule, "Rule added", Some(rule.name.clone()));
                continue;
            };
            if old.enabled != rule.enabled {
                let title = if rule.enabled {
                    "Rule enabled"
                } else {
                    "Rule disabled"
                };
                info(LogCategory::Rule, title, Some(rule.name.clone()));
            }
            if old != rule {
                let mut old = old.clone();
                old.enabled = rule.enabled;
                if &old != rule {
                    info(LogCategory::Rule, "Rule updated", Some(rule.name.clone()));
                }
            }
        }
        for rule in &previous.rules {
            if !current.rules.iter().any(|new| new.id == rule.id) {
                info(LogCategory::Rule, "Rule deleted", Some(rule.name.clone()));
            }
        }
        if previous.active_profile_id != current.active_profile_id {
            let name = current
                .profiles
                .iter()
                .find(|profile| profile.id == current.active_profile_id)
                .map_or(&current.active_profile_id, |profile| &profile.name);
            info(
                LogCategory::App,
                "Active profile switched",
                Some(name.clone()),
            );
        }
        if previous.settings != current.settings {
            info(LogCategory::App, "General settings updated", None);
        }
    }

    pub async fn start_if_configured(&self) {
        if self.policy().config.settings.auto_start_proxy {
            if let Err(error) = self.start().await {
                tracing::warn!(%error, "automatic engine start failed");
            }
        }
    }

    async fn start(&self) -> Result<ServiceStatus, IpcError> {
        let mut state = self.lifecycle.lock().await;
        self.start_locked(&mut state).await
    }

    async fn start_locked(&self, state: &mut Lifecycle) -> Result<ServiceStatus, IpcError> {
        ensure_config_available(state)?;
        if state.engine.is_some() {
            return Err(IpcError::new(
                IpcErrorCode::EngineAlreadyRunning,
                "Proxy engine is already running",
            ));
        }
        state.restart_on_network_resume = false;
        match Engine::start(Arc::clone(&self.policy), Arc::clone(&self.telemetry)).await {
            Ok(engine) => {
                state.engine = Some(engine);
                state.started = Some((now_ms(), Instant::now()));
                state.driver_ready = true;
                state.last_error = None;
                self.telemetry.log(
                    LogLevel::Info,
                    LogCategory::Service,
                    "Proxy engine started",
                    None,
                );
                Ok(self.status_changed(state))
            }
            Err(error) => {
                state.driver_ready = false;
                state.last_error = Some(error.message.clone());
                self.telemetry.log(
                    LogLevel::Error,
                    LogCategory::Service,
                    "Proxy engine failed to start",
                    Some(error.message.clone()),
                );
                self.status_changed(state);
                Err(error)
            }
        }
    }

    fn stop_locked(&self, state: &mut Lifecycle, error: Option<String>) -> ServiceStatus {
        drop(state.engine.take());
        state.started = None;
        state.restart_on_network_resume = error.is_some();
        if let Some(error) = error {
            state.driver_ready = false;
            state.last_error = Some(error.clone());
            self.telemetry.log(
                LogLevel::Error,
                LogCategory::Service,
                "Proxy engine stopped unexpectedly",
                Some(error),
            );
        } else {
            self.telemetry.log(
                LogLevel::Info,
                LogCategory::Service,
                "Proxy engine stopped",
                None,
            );
        }
        self.status_changed(state)
    }

    pub async fn shutdown(&self) {
        let mut state = self.lifecycle.lock().await;
        if state.engine.is_some() {
            self.stop_locked(&mut state, None);
        }
        drop(state);
        self.flush_traffic_history().await;
    }

    pub async fn monitor_network(&self) {
        let monitor = power_net::Monitor::new();
        let mut waiting_for_ipv4 = false;
        loop {
            let notified = if waiting_for_ipv4 || !monitor.has_network_notifications() {
                // A link can become ready before its IPv4 address is assigned.
                tokio::select! {
                    () = monitor.changed(), if monitor.is_registered() => true,
                    () = tokio::time::sleep(Duration::from_secs(2)) => false,
                }
            } else {
                monitor.changed().await;
                true
            };
            if notified {
                monitor.debounce().await;
            }
            let was_offline = waiting_for_ipv4;
            waiting_for_ipv4 = !tokio::task::spawn_blocking(network_interfaces::list)
                .await
                .is_ok_and(|interfaces| {
                    interfaces
                        .is_ok_and(|interfaces| interfaces.iter().any(|interface| interface.is_up))
                });
            if waiting_for_ipv4 {
                continue;
            }
            // When native NIC registration fails, refresh on the transition to
            // online or a native resume event, rather than on every fallback poll.
            if was_offline || notified {
                self.refresh_network_policy().await;
            }
        }
    }

    async fn refresh_network_policy(&self) {
        let _write = self.config_write.lock().await;
        let policy = Arc::new(Policy::new(self.policy().config.clone()).await);
        let mut state = self.lifecycle.lock().await;
        self.telemetry.mark_proxy_tests_stale();
        self.publish_policy(&state, policy);
        self.telemetry.log(
            LogLevel::Info,
            LogCategory::Service,
            "Network or power state resumed; refreshed policy",
            None,
        );
        if state.restart_on_network_resume && state.config_error.is_none() {
            let _ = self.start_locked(&mut state).await;
        }
    }

    pub async fn monitor_wifi(&self) {
        let mut session = None;
        let mut last_automation = None;
        let mut poll = tokio::time::interval(Duration::from_secs(10));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = poll.tick() => {}
                () = wifi::notification() => {}
            }
            let result = tokio::task::spawn_blocking(move || {
                let handle = session.or_else(|| wifi::Session::open(true));
                let current = handle.as_ref().and_then(wifi::Session::current);
                // Reopen next poll if the WLAN service restarted or became unavailable.
                if let Some(info) = current {
                    (handle, info)
                } else {
                    (None, wifi::unavailable())
                }
            })
            .await;
            let (handle, info) = result.unwrap_or_else(|_| (None, wifi::unavailable()));
            session = handle;
            let changed = {
                let mut current = self.current_wifi.write().unwrap();
                let changed = *current != info;
                if changed {
                    *current = info.clone();
                    let _ = self
                        .telemetry
                        .events
                        .send(IpcEvent::WifiChanged(info.clone()));
                }
                changed
            };
            let _guard = self.config_write.lock().await;
            let previous = self.policy();
            let automation = &previous.config.automation;
            if !changed && last_automation.as_ref() == Some(automation) {
                continue;
            }
            last_automation = Some(automation.clone());
            if let Err(error) = self.apply_wifi_rule(&previous.config, &info).await {
                tracing::warn!(%error, "Wi-Fi profile switch failed");
                last_automation = None;
            }
        }
    }

    async fn apply_wifi_rule(
        &self,
        previous: &AppConfig,
        info: &CurrentWifiInfo,
    ) -> Result<(), IpcError> {
        let Some(profile_id) = wifi::profile_to_activate(previous, info) else {
            return Ok(());
        };
        let invalid_rule_ids = {
            let state = self.lifecycle.lock().await;
            ensure_config_available(&state)?;
            state.invalid_rule_ids.clone()
        };
        let mut config = previous.clone();
        config.active_profile_id = profile_id.into();
        self.commit_config(previous, &config, invalid_rule_ids)
            .await?;
        self.telemetry.log(
            LogLevel::Info,
            LogCategory::App,
            "Wi-Fi automation switched profile",
            None,
        );
        Ok(())
    }

    fn traffic_history(&self) -> &TrafficHistory {
        self.telemetry
            .traffic_history
            .as_ref()
            .expect("service telemetry owns traffic history")
    }

    async fn flush_traffic_history(&self) {
        let telemetry = Arc::clone(&self.telemetry);
        let result = tokio::task::spawn_blocking(move || {
            telemetry
                .traffic_history
                .as_ref()
                .expect("service history")
                .flush()
        })
        .await
        .map_err(internal)
        .and_then(|result| result);
        if let Err(error) = result {
            tracing::warn!(%error, "Could not persist traffic history");
        }
    }

    pub async fn telemetry_loop(&self) {
        let ticks = async {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let mut state = self.lifecycle.lock().await;
                if let Some(error) = state.engine.as_ref().and_then(Engine::maintain) {
                    self.stop_locked(&mut state, Some(error));
                }
                self.telemetry.tick(&self.make_status(&state));
            }
        };
        let flushes = async {
            let period = Duration::from_secs(60);
            let mut interval =
                tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                self.flush_traffic_history().await;
            }
        };
        tokio::join!(ticks, flushes);
    }

    pub async fn collect_diagnostics(
        &self,
        output_path: Option<PathBuf>,
        write: impl FnOnce(&Path, &[u8]) -> Result<(), IpcError> + Send + 'static,
    ) -> Result<DiagnosticBundleSummary, IpcError> {
        let data = {
            let state = self.lifecycle.lock().await;
            DiagnosticData {
                status: self.make_status(&state),
                config: self.policy().config.clone(),
                wifi: self.current_wifi.read().unwrap().clone(),
                logs: self.telemetry.logs(None),
            }
        };
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            diagnostics::collect_diagnostic_bundle(&path, output_path, data, write)
        })
        .await
        .map_err(internal)?
    }

    pub async fn dispatch(self: &Arc<Self>, request: RpcRequest) -> Result<RpcResult, IpcError> {
        match request {
            RpcRequest::Ping => Ok(RpcResult::Pong(PongInfo {
                core_version: VERSION.into(),
                protocol_version: IPC_PROTOCOL_VERSION,
                service_pid: std::process::id(),
            })),
            RpcRequest::GetStatus => {
                let state = self.lifecycle.lock().await;
                Ok(RpcResult::Status(self.make_status(&state)))
            }
            RpcRequest::StartEngine => Ok(RpcResult::Status(self.start().await?)),
            RpcRequest::StopEngine => {
                let mut state = self.lifecycle.lock().await;
                state.restart_on_network_resume = false;
                let status = state
                    .engine
                    .is_some()
                    .then(|| self.stop_locked(&mut state, None));
                drop(state);
                self.flush_traffic_history().await;
                status.map(RpcResult::Status).ok_or_else(|| {
                    IpcError::new(IpcErrorCode::EngineNotRunning, "Proxy engine is stopped")
                })
            }
            RpcRequest::GetConfig => Ok(RpcResult::Config(self.policy().config.clone())),
            RpcRequest::SaveConfig { config } => Ok(RpcResult::Config(
                self.update_config(ConfigUpdate::Save(config)).await?,
            )),
            RpcRequest::RestoreConfigBackup => {
                // Complete publication even if the requesting pipe disconnects after the rename.
                let service = Arc::clone(self);
                let config = tokio::spawn(async move { service.restore_config_backup().await })
                    .await
                    .map_err(internal)??;
                Ok(RpcResult::Config(config))
            }
            RpcRequest::SetActiveProfile { profile_id } => Ok(RpcResult::Config(
                self.update_config(ConfigUpdate::Activate(profile_id))
                    .await?,
            )),
            RpcRequest::ExportConfigBackup { include_passwords } => Ok(RpcResult::ConfigBackup(
                config::export_config_backup(&self.policy().config, include_passwords),
            )),
            RpcRequest::ImportConfigBackup { json } => {
                let config = config::import_config_backup(&json)?;
                Ok(RpcResult::Config(
                    self.update_config(ConfigUpdate::Save(config)).await?,
                ))
            }
            RpcRequest::ExportRulesJson { profile_id } => Ok(RpcResult::RulesJson(
                config::export_rules_json(&self.policy().config, profile_id.as_deref()),
            )),
            RpcRequest::ImportRulesJson {
                json,
                profile_id,
                dry_run,
            } => Ok(RpcResult::RulesImport(
                self.import_rules(&json, &profile_id, dry_run).await?,
            )),
            RpcRequest::GetTrafficHistory { query } => {
                let service = Arc::clone(self);
                let response = tokio::task::spawn_blocking(move || {
                    service.traffic_history().query(query, now_ms())
                })
                .await
                .map_err(internal)?;
                Ok(RpcResult::TrafficHistory(response))
            }
            RpcRequest::ClearTrafficHistory => {
                let service = Arc::clone(self);
                tokio::task::spawn_blocking(move || service.traffic_history().clear())
                    .await
                    .map_err(internal)??;
                Ok(RpcResult::Ack)
            }
            RpcRequest::GetCurrentWifi => Ok(RpcResult::CurrentWifi(
                self.current_wifi.read().unwrap().clone(),
            )),
            RpcRequest::GetCoexistenceConflicts => {
                let dismissed = self.dismissed_conflicts.lock().await.clone();
                let conflicts =
                    tokio::task::spawn_blocking(move || diagnostics::detect_conflicts(&dismissed))
                        .await
                        .map_err(internal)?;
                Ok(RpcResult::CoexistenceConflicts(conflicts))
            }
            RpcRequest::DismissCoexistenceConflict { conflict_id } => {
                self.dismissed_conflicts.lock().await.insert(conflict_id);
                Ok(RpcResult::Ack)
            }
            RpcRequest::CollectDiagnosticBundle { output_path: None } => {
                Ok(RpcResult::DiagnosticBundle(
                    self.collect_diagnostics(None, config::atomic_write).await?,
                ))
            }
            RpcRequest::CollectDiagnosticBundle {
                output_path: Some(_),
            } => Err(IpcError::new(
                IpcErrorCode::AccessDenied,
                "A custom diagnostic path requires a local pipe client identity",
            )),
            RpcRequest::GetConnections {
                include_closed,
                limit,
            } => Ok(RpcResult::Connections(
                self.telemetry.connections(include_closed, limit),
            )),
            RpcRequest::ClearConnections => {
                self.telemetry.clear();
                Ok(RpcResult::Ack)
            }
            RpcRequest::ListRunningProcesses => {
                let processes = tokio::task::spawn_blocking(process::list_running)
                    .await
                    .map_err(internal)?
                    .map_err(internal)?;
                Ok(RpcResult::Processes(processes))
            }
            RpcRequest::ListNetworkInterfaces => {
                let interfaces = tokio::task::spawn_blocking(network_interfaces::list)
                    .await
                    .map_err(internal)?
                    .map_err(internal)?;
                Ok(RpcResult::NetworkInterfaces(interfaces))
            }
            RpcRequest::TestProxy { proxy_id } => {
                let generation = self.telemetry.health_generation();
                let policy = self.policy();
                let proxy = policy
                    .config
                    .proxies
                    .iter()
                    .find(|proxy| proxy.id == proxy_id)
                    .ok_or_else(|| IpcError::new(IpcErrorCode::NotFound, "Proxy does not exist"))?;
                let mut result = upstream::test_proxy(proxy).await;
                let current = self.policy();
                result.stale = generation != self.telemetry.health_generation()
                    || !current
                        .config
                        .proxies
                        .iter()
                        .any(|configured| configured == proxy);
                if !result.reachable {
                    let reason = result.error.as_deref().unwrap_or("Unknown error");
                    self.telemetry.log(
                        LogLevel::Warn,
                        LogCategory::Proxy,
                        "Proxy test failed",
                        Some(format!("{}: {reason}", proxy.name)),
                    );
                }
                Ok(RpcResult::ProxyTest(result))
            }
            RpcRequest::GetActivityLogs { limit } => {
                Ok(RpcResult::ActivityLogs(self.telemetry.logs(limit)))
            }
            RpcRequest::ClearActivityLogs => {
                self.telemetry.clear_logs();
                Ok(RpcResult::Ack)
            }
        }
    }
}

fn ensure_config_available(state: &Lifecycle) -> Result<(), IpcError> {
    match &state.config_error {
        Some(error) => Err(IpcError::new(IpcErrorCode::ConfigIoError, error.clone())),
        None => Ok(()),
    }
}

fn internal(error: impl std::fmt::Display) -> IpcError {
    IpcError::new(IpcErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{Profile, ProxyAuth, ProxyConfig, ProxyProtocol, Rule, RuleAction, RuleMatcher};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn test_path(test: &str) -> PathBuf {
        let scope = format!("my-win-antify-{test}-{}-{}", std::process::id(), now_ms());
        std::env::temp_dir().join(scope).join("config.json")
    }

    async fn new_service(test: &str) -> Arc<Service> {
        Service::new(test_path(test)).await.unwrap()
    }

    fn cleanup(path: &std::path::Path) {
        let dir = path.parent().unwrap();
        for entry in std::fs::read_dir(dir).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    async fn status(service: &Arc<Service>) -> ServiceStatus {
        let RpcResult::Status(status) = service.dispatch(RpcRequest::GetStatus).await.unwrap()
        else {
            panic!("Expected service status");
        };
        status
    }

    #[tokio::test]
    async fn imports_backup_and_rules_transactionally() {
        let service = new_service("portable-rules").await;
        let mut config = AppConfig::default();
        let mut work = config.profiles[0].clone();
        work.id = "work".into();
        config.profiles.push(work);

        let existing = Rule {
            id: "existing".into(),
            name: "Existing".into(),
            profile_id: "default".into(),
            enabled: true,
            matcher: RuleMatcher::ProcessName {
                name: "client.exe".into(),
                include_children: false,
            },
            action: RuleAction::Direct,
            proxy_id: None,
            network_lock: false,
            priority: 0,
            target_hosts: vec!["example.com".into(), "test.example".into()],
            target_ports: vec!["443".into(), "80".into()],
        };
        config.rules = vec![
            existing.clone(),
            Rule {
                profile_id: "work".into(),
                id: "collision".into(),
                ..existing.clone()
            },
        ];
        save(&service, config.clone()).await;
        let original = std::fs::read(&service.path).unwrap();
        let backups = std::fs::read_dir(service.path.parent().unwrap())
            .unwrap()
            .count();
        let duplicate = Rule {
            id: "different".into(),
            name: "Duplicate".into(),
            matcher: RuleMatcher::ProcessName {
                name: "CLIENT.EXE".into(),
                include_children: true,
            },
            target_hosts: vec!["TEST.EXAMPLE".into(), "example.com".into()],
            target_ports: vec!["80".into(), "443".into()],
            ..existing.clone()
        };
        let imported = Rule {
            id: "collision".into(),
            name: "Portable".into(),
            profile_id: "previous-profile".into(),
            matcher: RuleMatcher::ExecutablePath {
                path: r"%userprofile%\Desktop\portable.exe".into(),
                include_children: true,
            },
            action: RuleAction::Proxy,
            proxy_id: Some("missing".into()),
            ..existing
        };
        let request = |dry_run| RpcRequest::ImportRulesJson {
            json: serde_json::to_string(&[duplicate.clone(), imported.clone()]).unwrap(),
            profile_id: "default".into(),
            dry_run,
        };
        let RpcResult::RulesImport(preview) = service.dispatch(request(true)).await.unwrap() else {
            panic!("Expected preview");
        };
        assert_eq!(
            preview,
            RulesImportPreview {
                committed: false,
                total_in_file: 2,
                added: vec!["Portable".into()],
                skipped_duplicates: vec!["Duplicate".into()],
                missing_proxy_rules: vec!["Portable".into()],
            }
        );
        assert_eq!(service.policy().config, config);
        assert_eq!(std::fs::read(&service.path).unwrap(), original);
        assert_eq!(
            std::fs::read_dir(service.path.parent().unwrap())
                .unwrap()
                .count(),
            backups
        );

        let mut invalid = duplicate.clone();
        invalid.target_ports.push("65536".into());
        let mut invalid_backup = config.clone();
        invalid_backup.rules[0] = invalid.clone();
        for request in [
            RpcRequest::ImportConfigBackup { json: "{".into() },
            RpcRequest::ImportConfigBackup {
                json: serde_json::to_string(&invalid_backup).unwrap(),
            },
            RpcRequest::ImportRulesJson {
                json: serde_json::to_string(&[imported.clone(), invalid]).unwrap(),
                profile_id: "default".into(),
                dry_run: false,
            },
        ] {
            assert_eq!(
                service.dispatch(request).await.unwrap_err().code,
                IpcErrorCode::ValidationFailed
            );
            assert_eq!(std::fs::read(&service.path).unwrap(), original);
            assert_eq!(service.policy().config, config);
        }
        assert_eq!(
            service.dispatch(request(false)).await.unwrap(),
            RpcResult::RulesImport(RulesImportPreview {
                committed: true,
                ..preview
            })
        );
        let updated = config::load_with_diagnostics(&service.path).unwrap().0;
        let new_rule = &updated.rules[2];
        assert_ne!(new_rule.id, "collision");
        assert_eq!(new_rule.profile_id, "default");
        assert_eq!(new_rule.proxy_id, None);
        if let Ok(profile) = std::env::var("USERPROFILE") {
            assert!(
                matches!(&new_rule.matcher, RuleMatcher::ExecutablePath { path, .. }
                if path == &format!(r"{}\Desktop\portable.exe", profile.trim_end_matches('\\')))
            );
        }

        let mut backup = serde_json::to_value(&updated).unwrap();
        backup["schemaVersion"] = 1.into();
        backup["activeProfileId"] = "removed".into();
        assert_eq!(
            service
                .dispatch(RpcRequest::ImportConfigBackup {
                    json: backup.to_string()
                })
                .await
                .unwrap(),
            RpcResult::Config(updated)
        );
        cleanup(&service.path);
    }

    #[tokio::test]
    async fn wifi_rule_commits_active_profile_and_emits_app_log() {
        let service = new_service("wifi-rule").await;
        let mut config = AppConfig::default();
        let mut profile = config.profiles[0].clone();
        profile.id = "work".into();
        config.profiles.push(profile);
        config.automation.enabled = true;
        config.automation.wifi_rules.push(shared::WifiProfileRule {
            id: "office-rule".into(),
            ssid: "Office".into(),
            profile_id: "work".into(),
            enabled: true,
        });
        save(&service, config.clone()).await;
        let info = CurrentWifiInfo {
            connected: true,
            supported: true,
            ssid: Some("Office".into()),
            interface_name: None,
        };
        let _guard = service.config_write.lock().await;
        service.apply_wifi_rule(&config, &info).await.unwrap();
        assert_eq!(status(&service).await.active_profile_id, "work");
        assert_eq!(
            config::load_with_diagnostics(&service.path)
                .unwrap()
                .0
                .active_profile_id,
            "work"
        );
        assert!(service
            .telemetry
            .logs(Some(1))
            .iter()
            .any(|log| log.title == "Wi-Fi automation switched profile"
                && log.level == LogLevel::Info
                && log.category == LogCategory::App));
        cleanup(&service.path);
    }

    #[tokio::test]
    async fn corrupt_config_survives_startup_and_backup_restore_clears_the_error() {
        let original = new_service("recover-config").await;
        let path = original.path.clone();
        let mut config = AppConfig::default();
        config.settings.language = "saved-in-backup".into();
        save(&original, config.clone()).await;
        save(&original, AppConfig::default()).await;
        drop(original);

        let corrupt = b"{this is not valid JSON";
        std::fs::write(&path, corrupt).unwrap();
        let service = Service::new(path.clone()).await.unwrap();
        let mut events = service.telemetry.events.subscribe();
        let error = status(&service).await.config_error.unwrap();
        assert!(!status(&service).await.running);
        assert!(status(&service).await.invalid_rule_ids.is_empty());
        let rejected = service.dispatch(RpcRequest::StartEngine).await.unwrap_err();
        assert_eq!(rejected.code, IpcErrorCode::ConfigIoError);
        assert_eq!(rejected.message, error);
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
        assert_eq!(
            service
                .dispatch(RpcRequest::SetActiveProfile {
                    profile_id: AppConfig::default().active_profile_id,
                })
                .await
                .unwrap_err()
                .code,
            IpcErrorCode::ConfigIoError
        );

        assert_eq!(
            service
                .dispatch(RpcRequest::RestoreConfigBackup)
                .await
                .unwrap(),
            RpcResult::Config(config.clone())
        );
        let new_status = status(&service).await;
        assert!(new_status.config_error.is_none());
        assert_eq!(config::load_with_diagnostics(&path).unwrap().0, config);
        assert!(matches!(
            events.try_recv().unwrap(),
            IpcEvent::ActivityLog(log) if log.level == LogLevel::Info && log.title == "Config backup restored"
        ));
        assert_eq!(events.try_recv().unwrap(), IpcEvent::ConfigChanged(config));
        assert_eq!(
            events.try_recv().unwrap(),
            IpcEvent::StatusChanged(new_status)
        );
        cleanup(&path);
    }

    #[tokio::test]
    async fn structurally_invalid_config_is_preserved_and_save_recovers_it() {
        let service = new_service("invalid-structure").await;
        let path = service.path.clone();
        drop(service);
        let mut broken = AppConfig::default();
        broken.active_profile_id = "missing".into();
        let bytes = serde_json::to_vec(&broken).unwrap();
        std::fs::write(&path, &bytes).unwrap();

        let service = Service::new(path.clone()).await.unwrap();
        assert!(status(&service)
            .await
            .config_error
            .unwrap()
            .contains("Active profile"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        save(&service, AppConfig::default()).await;
        assert!(status(&service).await.config_error.is_none());
        assert_eq!(
            config::load_with_diagnostics(&path).unwrap().0,
            AppConfig::default()
        );
        cleanup(&path);
    }

    #[tokio::test]
    async fn malformed_rule_destinations_are_reported_and_fail_closed() {
        let service = new_service("invalid-rule").await;
        let path = service.path.clone();
        drop(service);
        let mut config = AppConfig::default();
        let owner = process::Process::unknown(123);
        let valid_rule = Rule {
            id: "valid".into(),
            name: "Valid".into(),
            profile_id: config.active_profile_id.clone(),
            enabled: true,
            matcher: RuleMatcher::ProcessName {
                name: owner.name.clone(),
                include_children: false,
            },
            action: RuleAction::Direct,
            proxy_id: None,
            network_lock: false,
            priority: 0,
            target_hosts: Vec::new(),
            target_ports: Vec::new(),
        };
        config.rules = vec![
            valid_rule.clone(),
            Rule {
                id: "bad-hosts".into(),
                priority: 1,
                target_hosts: vec!["10.0.0.0/33".into()],
                target_ports: vec!["81".into()],
                ..valid_rule.clone()
            },
            Rule {
                id: "bad-ports".into(),
                priority: 2,
                target_ports: vec!["65536".into()],
                ..valid_rule
            },
        ];
        let bytes = serde_json::to_vec(&config).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let service = Service::new(path.clone()).await.unwrap();
        let new_status = status(&service).await;
        assert!(new_status.config_error.is_none());
        assert_eq!(new_status.invalid_rule_ids, ["bad-hosts", "bad-ports"]);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        let lineage = [owner];
        let policy = service.policy();
        let ipv4 = policy
            .rules
            .decide(&lineage, "203.0.113.1:443".parse().unwrap());
        let ipv6 = policy.rules.decide_ipv6(&lineage, 443);
        for decision in [ipv4, ipv6] {
            assert_eq!(decision.action, RuleAction::Block);
            assert_eq!(decision.rule_id.as_deref(), Some("bad-hosts"));
            assert!(decision.pending_hosts.is_none());
        }
        assert_eq!(
            service
                .dispatch(RpcRequest::SaveConfig {
                    config: config.clone(),
                })
                .await
                .unwrap_err()
                .code,
            IpcErrorCode::ValidationFailed
        );

        let mut events = service.telemetry.events.subscribe();
        service
            .dispatch(RpcRequest::SetActiveProfile {
                profile_id: config.active_profile_id.clone(),
            })
            .await
            .unwrap();
        assert!(matches!(
            events.try_recv().unwrap(),
            IpcEvent::ConfigChanged(_)
        ));
        assert_eq!(
            events.try_recv().unwrap(),
            IpcEvent::StatusChanged(new_status)
        );

        config.rules.truncate(1);
        save(&service, config).await;
        assert!(status(&service).await.invalid_rule_ids.is_empty());
        cleanup(&path);
    }

    fn proxy(port: u16) -> ProxyConfig {
        ProxyConfig {
            id: "proxy".into(),
            name: "Test proxy".into(),
            protocol: ProxyProtocol::Socks5,
            host: "127.0.0.1".into(),
            port,
            auth: None,
            interface_name: None,
            allow_lan: false,
            udp_relay: false,
            skip_tls_verify: false,
            fallback_direct: false,
            enabled: true,
        }
    }

    async fn save(service: &Arc<Service>, config: AppConfig) {
        assert_eq!(
            service
                .dispatch(RpcRequest::SaveConfig {
                    config: config.clone()
                })
                .await
                .unwrap(),
            RpcResult::Config(config),
        );
    }

    #[tokio::test]
    async fn logs_committed_config_changes_and_clears_them_over_rpc() {
        let service = new_service("config-log").await;
        let mut events = service.telemetry.events.subscribe();
        let mut config = AppConfig::default();
        config.proxies.push(proxy(1080));
        config.rules.push(Rule {
            id: "rule".into(),
            name: "Test rule".into(),
            profile_id: config.active_profile_id.clone(),
            enabled: true,
            matcher: RuleMatcher::ProcessName {
                name: "test.exe".into(),
                include_children: false,
            },
            action: RuleAction::Proxy,
            proxy_id: Some("proxy".into()),
            network_lock: false,
            priority: 1,
            target_hosts: vec![],
            target_ports: vec![],
        });
        config.profiles.push(Profile {
            id: "test".into(),
            name: "Test profile".into(),
            default_action: RuleAction::Direct,
            default_proxy_id: None,
        });
        save(&service, config.clone()).await;
        config.proxies[0].auth = Some(ProxyAuth {
            username: "user".into(),
            password: "private-password".into(),
        });
        config.rules[0].priority = 2;
        config.rules[0].enabled = false;
        config.settings.language = "en-US".into();
        save(&service, config.clone()).await;
        assert!(matches!(
            service.dispatch(RpcRequest::SetActiveProfile { profile_id: "test".into() }).await.unwrap(),
            RpcResult::Config(saved) if saved.active_profile_id == "test"
        ));
        config.active_profile_id = "test".into();
        config.rules[0].enabled = true;
        save(&service, config.clone()).await;
        config.proxies.clear();
        config.rules.clear();
        save(&service, config.clone()).await;
        save(&service, config.clone()).await;
        config.active_profile_id = "invalid".into();
        assert!(service
            .dispatch(RpcRequest::SaveConfig { config })
            .await
            .is_err());

        let RpcResult::ActivityLogs(logs) = service
            .dispatch(RpcRequest::GetActivityLogs { limit: None })
            .await
            .unwrap()
        else {
            panic!("Expected logs");
        };
        let expected = [
            (LogCategory::Proxy, "Proxy added"),
            (LogCategory::Rule, "Rule added"),
            (LogCategory::Proxy, "Proxy updated"),
            (LogCategory::Rule, "Rule disabled"),
            (LogCategory::Rule, "Rule updated"),
            (LogCategory::App, "General settings updated"),
            (LogCategory::App, "Active profile switched"),
            (LogCategory::Rule, "Rule enabled"),
            (LogCategory::Proxy, "Proxy deleted"),
            (LogCategory::Rule, "Rule deleted"),
        ];
        assert_eq!(
            logs.iter()
                .rev()
                .map(|entry| (entry.category, entry.title.as_str()))
                .collect::<Vec<_>>(),
            expected,
        );
        assert!(logs.iter().all(|entry| entry.level == LogLevel::Info
            && !entry
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("private-password")));
        let config_events: Vec<_> = std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|event| match event {
                IpcEvent::ActivityLog(log) => Some(log),
                _ => None,
            })
            .collect();
        assert_eq!(config_events, logs.into_iter().rev().collect::<Vec<_>>());

        assert_eq!(
            service
                .dispatch(RpcRequest::ClearActivityLogs)
                .await
                .unwrap(),
            RpcResult::Ack
        );
        assert_eq!(
            service
                .dispatch(RpcRequest::GetActivityLogs { limit: None })
                .await
                .unwrap(),
            RpcResult::ActivityLogs(Vec::new()),
        );
        cleanup(&service.path);
    }

    #[tokio::test]
    async fn unsuccessful_proxy_test_logs_a_warning_with_the_proxy_name_and_error() {
        let service = new_service("proxy-log").await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = AppConfig::default();
        config
            .proxies
            .push(proxy(listener.local_addr().unwrap().port()));
        save(&service, config).await;
        let reject = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            socket.write_all(&[5, 0xff]).await.unwrap();
        };
        let request = service.dispatch(RpcRequest::TestProxy {
            proxy_id: "proxy".into(),
        });
        let ((), response) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(reject, request)
        })
        .await
        .unwrap();
        let RpcResult::ProxyTest(result) = response.unwrap() else {
            panic!("Expected proxy test result");
        };
        assert!(!result.reachable);
        let logs = service.telemetry.logs(Some(1));
        assert_eq!(
            (logs[0].category, logs[0].level),
            (LogCategory::Proxy, LogLevel::Warn)
        );
        assert_eq!(
            logs[0].detail.as_deref(),
            Some(format!("Test proxy: {}", result.error.unwrap()).as_str())
        );
        cleanup(&service.path);
    }
}
