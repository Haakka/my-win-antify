pub mod error;
pub mod ipc;
pub mod models;

pub use error::{IpcError, IpcErrorCode};
pub use ipc::{
    IpcEvent, IpcRequest, IpcResponse, PipeConnectionState, PongInfo, RpcRequest, RpcResult,
    ServerFrame, CONFIG_SCHEMA_VERSION, DEFAULT_PIPE_NAME, EVENT_ACTIVITY_LOG,
    EVENT_CONFIG_CHANGED, EVENT_CONNECTION_EVENT, EVENT_PIPE_CONNECTION, EVENT_STATS_TICK,
    EVENT_STATUS_CHANGED, IPC_PROTOCOL_VERSION,
};
pub use models::{
    ActivityLogEntry, AppConfig, ConnectionProtocol, ConnectionSnapshot, ConnectionState,
    GeneralSettings, LogCategory, LogLevel, NetworkInterfaceInfo, Profile, ProxyAuth, ProxyConfig,
    ProxyProtocol, ProxyTestResult, Rule, RuleAction, RuleMatchTick, RuleMatcher,
    RunningProcessInfo, ServiceStatus, ThemeMode, TrafficStats,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_request_roundtrip_unit_and_struct_variants() {
        let ping = IpcRequest {
            id: 1,
            request: RpcRequest::Ping,
        };
        let json = serde_json::to_string(&ping).unwrap();
        assert!(json.contains("\"method\":\"ping\""));
        let decoded: IpcRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(ping, decoded);

        let save = IpcRequest {
            id: 2,
            request: RpcRequest::SaveConfig {
                config: AppConfig::default(),
            },
        };
        let save_json = serde_json::to_string(&save).unwrap();
        assert!(save_json.contains("\"method\":\"saveConfig\""));
        assert!(save_json.contains("\"activeProfileId\":\"default\""));
        let decoded_save: IpcRequest = serde_json::from_str(&save_json).unwrap();
        assert_eq!(save, decoded_save);

        let conns_req = IpcRequest {
            id: 3,
            request: RpcRequest::GetConnections {
                include_closed: true,
                limit: Some(50),
            },
        };
        let conns_json = serde_json::to_string(&conns_req).unwrap();
        assert!(conns_json.contains("\"includeClosed\":true"));
        let decoded_conns: IpcRequest = serde_json::from_str(&conns_json).unwrap();
        assert_eq!(conns_req, decoded_conns);
    }

    #[test]
    fn server_frame_response_and_event_roundtrip() {
        let resp = ServerFrame::Response(IpcResponse::ok(
            7,
            RpcResult::Pong(PongInfo {
                core_version: "0.1.0".into(),
                protocol_version: IPC_PROTOCOL_VERSION,
                service_pid: 4242,
            }),
        ));
        let resp_json = serde_json::to_string(&resp).unwrap();
        assert!(resp_json.contains("\"frame\":\"response\""));
        let decoded_resp: ServerFrame = serde_json::from_str(&resp_json).unwrap();
        assert_eq!(resp, decoded_resp);

        let evt = ServerFrame::Event(IpcEvent::StatsTick(TrafficStats {
            timestamp_ms: 1_700_000_000_000,
            running: true,
            uptime_secs: 120,
            active_connections: 5,
            active_apps: 3,
            upload_bps: 1024,
            download_bps: 4096,
            total_proxied_up: 10_000,
            total_proxied_down: 50_000,
            total_direct_up: 500,
            total_direct_down: 1200,
            rule_last_matched: vec![RuleMatchTick {
                rule_id: "r1".into(),
                last_matched_at_ms: 1_700_000_000_000,
            }],
        }));
        let evt_json = serde_json::to_string(&evt).unwrap();
        assert!(evt_json.contains("\"frame\":\"event\""));
        assert!(evt_json.contains("\"event\":\"statsTick\""));
        let decoded_evt: ServerFrame = serde_json::from_str(&evt_json).unwrap();
        assert_eq!(evt, decoded_evt);
    }

    #[test]
    fn rule_matcher_and_error_roundtrip() {
        let rule = Rule {
            id: "rule-1".into(),
            profile_id: "default".into(),
            enabled: true,
            name: "Antigravity".into(),
            matcher: RuleMatcher::ExecutablePath {
                path: r"C:\Apps\Antigravity\antigravity.exe".into(),
                include_children: true,
            },
            action: RuleAction::Proxy,
            proxy_id: Some("proxy-1".into()),
            network_lock: false,
            priority: 10,
            target_hosts: vec!["*.google.com".into()],
            target_ports: vec!["443".into()],
        };
        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("\"kind\":\"executablePath\""));
        assert!(json.contains("\"includeChildren\":true"));
        assert!(json.contains("\"targetHosts\":[\"*.google.com\"]"));
        assert!(json.contains("\"targetPorts\":[\"443\"]"));
        let decoded: Rule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, decoded);

        let legacy_rule_json = r#"{
            "id":"rule-legacy",
            "profileId":"default",
            "enabled":true,
            "name":"Legacy",
            "matcher":{"kind":"processName","name":"curl.exe"},
            "action":"direct"
        }"#;
        let parsed_legacy: Rule = serde_json::from_str(legacy_rule_json).unwrap();
        assert!(parsed_legacy.target_hosts.is_empty());
        assert!(parsed_legacy.target_ports.is_empty());

        let err = ServerFrame::Response(IpcResponse::err(
            9,
            IpcError::new(IpcErrorCode::DriverUnavailable, "WinDivert64.sys missing"),
        ));
        let err_json = serde_json::to_string(&err).unwrap();
        assert!(err_json.contains("\"code\":\"driverUnavailable\""));
        let decoded_err: ServerFrame = serde_json::from_str(&err_json).unwrap();
        assert_eq!(err, decoded_err);
    }

    #[test]
    fn proxy_config_roundtrip_and_legacy_defaults() {
        let proxy = ProxyConfig {
            id: "proxy-1".into(),
            name: "FlClash".into(),
            protocol: ProxyProtocol::Socks5,
            host: "127.0.0.1".into(),
            port: 7890,
            auth: None,
            interface_name: None,
            allow_lan: true,
            udp_relay: false,
            fallback_direct: true,
            enabled: true,
        };
        let json = serde_json::to_string(&proxy).unwrap();
        assert!(json.contains("\"udpRelay\":false"));
        assert!(json.contains("\"fallbackDirect\":true"));
        let decoded: ProxyConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(proxy, decoded);

        let legacy = r#"{
            "id":"proxy-legacy",
            "name":"Legacy",
            "protocol":"socks5",
            "host":"127.0.0.1",
            "port":7890
        }"#;
        let parsed: ProxyConfig = serde_json::from_str(legacy).unwrap();
        assert!(parsed.udp_relay);
        assert!(!parsed.fallback_direct);
        assert!(!parsed.allow_lan);
        assert!(parsed.enabled);
    }

    #[test]
    fn network_interface_rpc_roundtrip() {
        let req = IpcRequest {
            id: 42,
            request: RpcRequest::ListNetworkInterfaces,
        };
        let req_json = serde_json::to_string(&req).unwrap();
        assert!(req_json.contains("\"method\":\"listNetworkInterfaces\""));
        let decoded_req: IpcRequest = serde_json::from_str(&req_json).unwrap();
        assert_eq!(req, decoded_req);

        let frame = ServerFrame::Response(IpcResponse::ok(
            42,
            RpcResult::NetworkInterfaces(vec![NetworkInterfaceInfo {
                name: "WLAN".into(),
                description: "Intel(R) Wi-Fi 6E AX211".into(),
                ipv4: "192.168.1.24".into(),
                is_up: true,
            }]),
        ));
        let frame_json = serde_json::to_string(&frame).unwrap();
        assert!(frame_json.contains("\"kind\":\"networkInterfaces\""));
        let decoded_frame: ServerFrame = serde_json::from_str(&frame_json).unwrap();
        assert_eq!(frame, decoded_frame);

        let clear_logs = IpcRequest {
            id: 43,
            request: RpcRequest::ClearActivityLogs,
        };
        let clear_json = serde_json::to_string(&clear_logs).unwrap();
        assert!(clear_json.contains("\"method\":\"clearActivityLogs\""));
        let decoded_clear: IpcRequest = serde_json::from_str(&clear_json).unwrap();
        assert_eq!(clear_logs, decoded_clear);
    }
}
