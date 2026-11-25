use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use shared::{
    AppConfig, IpcError, IpcErrorCode, IpcEvent, LogCategory, LogLevel, PongInfo, RpcRequest,
    RpcResult, ServiceStatus, IPC_PROTOCOL_VERSION,
};
use tokio::sync::Mutex;

use crate::{
    config::{self, Policy},
    engine::{CurrentPolicy, Engine},
    network_interfaces, now_ms, process, socks5,
    telemetry::Telemetry,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) struct Service {
    path: PathBuf,
    config_write: Mutex<()>,
    policy: CurrentPolicy,
    lifecycle: Mutex<Lifecycle>,
    pub telemetry: Arc<Telemetry>,
}

#[derive(Default)]
struct Lifecycle {
    engine: Option<Engine>,
    started: Option<(i64, Instant)>,
    driver_ready: bool,
    last_error: Option<String>,
}

impl Service {
    pub async fn new(path: PathBuf) -> Result<Arc<Self>, IpcError> {
        let config = config::load(&path)?;
        let policy = Policy::new(config).await;
        let telemetry = Telemetry::new();
        let _ = telemetry
            .events
            .send(IpcEvent::ConfigChanged(policy.config.clone()));
        Ok(Arc::new(Self {
            path,
            config_write: Mutex::new(()),
            policy: Arc::new(RwLock::new(Arc::new(policy))),
            lifecycle: Mutex::new(Lifecycle::default()),
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

    async fn update_config(
        &self,
        change: impl FnOnce(&AppConfig) -> Result<AppConfig, IpcError>,
    ) -> Result<AppConfig, IpcError> {
        let _guard = self.config_write.lock().await;
        let previous = self.policy();
        let config = change(&previous.config)?;
        config::validate(&config)?;
        let policy = Policy::new(config.clone()).await;
        config::save(&self.path, &config)?;
        *self.policy.write().unwrap() = Arc::new(policy);
        self.telemetry.retain_rules(&config);
        self.log_config_changes(&previous.config, &config);
        let _ = self
            .telemetry
            .events
            .send(IpcEvent::ConfigChanged(config.clone()));
        let lifecycle = self.lifecycle.lock().await;
        self.status_changed(&lifecycle);
        Ok(config)
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
        if state.engine.is_some() {
            return Err(IpcError::new(
                IpcErrorCode::EngineAlreadyRunning,
                "Proxy engine is already running",
            ));
        }
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
                Ok(self.status_changed(&state))
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
                self.status_changed(&state);
                Err(error)
            }
        }
    }

    fn stop_locked(&self, state: &mut Lifecycle, error: Option<String>) -> ServiceStatus {
        drop(state.engine.take());
        state.started = None;
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
    }

    pub async fn telemetry_loop(&self) {
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
    }

    pub async fn dispatch(&self, request: RpcRequest) -> Result<RpcResult, IpcError> {
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
                if state.engine.is_none() {
                    return Err(IpcError::new(
                        IpcErrorCode::EngineNotRunning,
                        "Proxy engine is stopped",
                    ));
                }
                Ok(RpcResult::Status(self.stop_locked(&mut state, None)))
            }
            RpcRequest::GetConfig => Ok(RpcResult::Config(self.policy().config.clone())),
            RpcRequest::SaveConfig { config } => {
                Ok(RpcResult::Config(self.update_config(|_| Ok(config)).await?))
            }
            RpcRequest::SetActiveProfile { profile_id } => {
                let config = self
                    .update_config(|previous| {
                        if !previous
                            .profiles
                            .iter()
                            .any(|profile| profile.id == profile_id)
                        {
                            return Err(IpcError::new(
                                IpcErrorCode::NotFound,
                                "Profile does not exist",
                            ));
                        }
                        let mut config = previous.clone();
                        config.active_profile_id = profile_id;
                        Ok(config)
                    })
                    .await?;
                Ok(RpcResult::Config(config))
            }
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
                let policy = self.policy();
                let proxy = policy
                    .config
                    .proxies
                    .iter()
                    .find(|proxy| proxy.id == proxy_id)
                    .ok_or_else(|| IpcError::new(IpcErrorCode::NotFound, "Proxy does not exist"))?;
                let result = socks5::test_proxy(proxy).await;
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

fn internal(error: impl std::fmt::Display) -> IpcError {
    IpcError::new(IpcErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{Profile, ProxyAuth, ProxyConfig, ProxyProtocol, Rule, RuleAction, RuleMatcher};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn new_service(test: &str) -> Arc<Service> {
        let scope = format!(
            "my-win-antify-{test}-{}-{}.json",
            std::process::id(),
            now_ms()
        );
        Service::new(std::env::temp_dir().join(scope))
            .await
            .unwrap()
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
            fallback_direct: false,
            enabled: true,
        }
    }

    async fn save(service: &Service, config: AppConfig) {
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
        std::fs::remove_file(&service.path).unwrap();
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
        std::fs::remove_file(&service.path).unwrap();
    }
}
