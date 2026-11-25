mod atomic_file;
mod config;
mod diagnostics;
mod engine;
mod health;
mod ipc_server;
mod network_interfaces;
#[cfg(test)]
mod network_interfaces_tests;
mod packet;
mod power_net;
mod process;
mod relay;
mod service;
mod sniffer;
mod socks5;
mod targets;
mod telemetry;
mod traffic_history;
mod udp;
mod upstream;
mod wifi;
mod windivert;

use std::{
    ffi::OsString,
    io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use shared::DEFAULT_PIPE_NAME;
use tracing_subscriber::EnvFilter;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(args) = parse_args(std::env::args_os().skip(1))? else {
        println!("Usage: core-service [--console] [--config <PATH>] [--parent-pid <PID>]");
        return Ok(());
    };
    let attached = args.parent_pid.is_some();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run(args));
    if attached {
        // An infinite Win32 wait cannot be cancelled by Tokio. The engine has already
        // shut down, so let process exit release the waiter if the parent is still alive.
        runtime.shutdown_background();
    }
    result?;
    Ok(())
}

async fn run(args: CliArgs) -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let parent_exit = async {
        match args.parent_pid {
            Some(pid) => tokio::task::spawn_blocking(move || wait_for_parent(pid))
                .await
                .map_err(io::Error::other),
            None => std::future::pending().await,
        }
    };
    tokio::pin!(parent_exit);

    // Claim the endpoint before reading or initializing the single-writer config.
    let pipe = ipc_server::create(DEFAULT_PIPE_NAME, true)?;
    let service = tokio::select! {
        biased;
        parent = &mut parent_exit => return parent,
        service = service::Service::new(args.config) => service.map_err(io::Error::other)?,
    };
    tracing::info!(
        pipe = DEFAULT_PIPE_NAME,
        "MyWinAntify core service listening"
    );
    let result = tokio::select! {
        biased;
        parent = &mut parent_exit => parent,
        result = async {
            service.start_if_configured().await;
            ipc_server::run(DEFAULT_PIPE_NAME, pipe, service.clone()).await
        } => result,
        () = service.telemetry_loop() => Ok(()),
        () = service.monitor_network() => Ok(()),
        () = service.monitor_wifi() => Ok(()),
        signal = tokio::signal::ctrl_c() => signal,
    };
    service.shutdown().await;
    result
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> isize;
    fn WaitForSingleObject(handle: isize, timeout_ms: u32) -> u32;
    fn CloseHandle(handle: isize) -> i32;
}

struct ProcessHandle(isize);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn wait_for_parent(pid: u32) {
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const INFINITE: u32 = 0xffff_ffff;
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle != 0 {
        let handle = ProcessHandle(handle);
        unsafe { WaitForSingleObject(handle.0, INFINITE) };
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CliArgs {
    config: PathBuf,
    parent_pid: Option<u32>,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> io::Result<Option<CliArgs>> {
    let mut args = args.into_iter();
    let mut config = None;
    let mut parent_pid = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--console") => {} // Console is the only M1 execution mode.
            Some("--config") if config.is_none() => {
                config = Some(PathBuf::from(
                    args.next()
                        .filter(|arg| !arg.is_empty() && !arg.to_string_lossy().starts_with("--"))
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidInput, "--config requires a path")
                        })?,
                ));
            }
            Some("--parent-pid") if parent_pid.is_none() => {
                parent_pid = args
                    .next()
                    .and_then(|pid| pid.to_str()?.parse::<u32>().ok())
                    .filter(|&pid| pid != 0);
                if parent_pid.is_none() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--parent-pid requires a nonzero process ID",
                    ));
                }
            }
            Some("--help" | "-h") => return Ok(None),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unexpected argument {}", arg.to_string_lossy()),
                ))
            }
        }
    }
    Ok(Some(CliArgs {
        config: config.unwrap_or_else(config::default_path),
        parent_pid,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_parent_pid_alongside_console_and_config() {
        let args = [
            "--parent-pid",
            "4321",
            "--console",
            "--config",
            "C:/config.json",
        ];
        let parsed = parse_args(args.map(Into::into)).unwrap().unwrap();
        assert_eq!(parsed.parent_pid, Some(4321));
        assert_eq!(parsed.config, PathBuf::from("C:/config.json"));
    }

    #[test]
    fn rejects_missing_zero_and_invalid_parent_pids() {
        for args in [
            &["--parent-pid"][..],
            &["--parent-pid", "0"],
            &["--parent-pid", "abc"],
        ] {
            let error = parse_args(args.iter().copied().map(Into::into)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn parses_console_and_config_override() {
        let parsed =
            parse_args(["--console", "--config", "C:/Some Directory/配置.json"].map(Into::into))
                .unwrap();
        assert_eq!(
            parsed,
            Some(CliArgs {
                config: PathBuf::from("C:/Some Directory/配置.json"),
                parent_pid: None,
            })
        );
        assert!(parse_args(["--config"].map(Into::into)).is_err());
        assert!(parse_args(["--unknown"].map(Into::into)).is_err());
        assert!(parse_args(["--help"].map(Into::into)).unwrap().is_none());
    }
}
