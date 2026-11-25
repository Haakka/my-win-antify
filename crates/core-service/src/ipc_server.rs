use std::{
    ffi::c_void, io, mem::size_of, os::windows::io::AsRawHandle, path::PathBuf, ptr, sync::Arc,
    time::Duration,
};

use shared::{IpcError, IpcErrorCode, IpcRequest, IpcResponse, RpcRequest, RpcResult, ServerFrame};
use tokio::{
    io::{self as aio, AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    sync::{broadcast, mpsc},
    task::JoinSet,
};

use crate::{config, service::Service};

// Keep the standard authenticated-user, SYSTEM and administrator access, and
// explicitly grant the creator read/write access when run in a restricted console.
const PIPE_SDDL: &str = "D:(A;;GRGW;;;AU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;OW)";
const MAX_CLIENTS: usize = 32;
const FRAME_LIMIT: usize = 32 * 1024 * 1024;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit_handle: i32,
}

#[link(name = "advapi32")]
extern "system" {
    fn ImpersonateNamedPipeClient(pipe: isize) -> i32;
    fn RevertToSelf() -> i32;
    fn OpenThreadToken(thread: isize, access: u32, open_as_self: i32, token: *mut isize) -> i32;
    fn SetThreadToken(thread: *const isize, token: isize) -> i32;
    fn GetTokenInformation(
        token: isize,
        class: u32,
        data: *mut c_void,
        size: u32,
        returned: *mut u32,
    ) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        length: *mut u32,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
    fn GetCurrentThread() -> isize;
    fn CloseHandle(handle: isize) -> i32;
}

struct Descriptor(*mut c_void);

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

struct Impersonation;

impl Drop for Impersonation {
    fn drop(&mut self) {
        if unsafe { RevertToSelf() } == 0 {
            // This thread cannot safely serve more requests with a client token.
            std::process::abort();
        }
    }
}

struct ClientToken(isize);

impl ClientToken {
    fn from_pipe(pipe: isize) -> io::Result<Self> {
        const TOKEN_QUERY_AND_IMPERSONATE: u32 = 0x0008 | 0x0004;
        const TOKEN_IMPERSONATION_LEVEL: u32 = 9;
        const SECURITY_IMPERSONATION: u32 = 2;
        if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _guard = Impersonation;
        let mut token = 0;
        if unsafe {
            OpenThreadToken(
                GetCurrentThread(),
                TOKEN_QUERY_AND_IMPERSONATE,
                1,
                &mut token,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let token = Self(token);
        let mut level = 0u32;
        let mut size = 0;
        if unsafe {
            GetTokenInformation(
                token.0,
                TOKEN_IMPERSONATION_LEVEL,
                (&mut level as *mut u32).cast(),
                size_of::<u32>() as u32,
                &mut size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if level < SECURITY_IMPERSONATION {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Client permits identification only",
            ));
        }
        Ok(token)
    }

    fn with_identity<T>(&self, work: impl FnOnce() -> T) -> io::Result<T> {
        if unsafe { SetThreadToken(ptr::null(), self.0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _guard = Impersonation;
        Ok(work())
    }
}

impl Drop for ClientToken {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn client_access(error: io::Error) -> IpcError {
    IpcError::new(
        IpcErrorCode::AccessDenied,
        format!("Cannot use the diagnostic requester's identity: {error}"),
    )
}

async fn dispatch(
    service: &Arc<Service>,
    pipe: isize,
    request: RpcRequest,
) -> Result<RpcResult, IpcError> {
    if let RpcRequest::CollectDiagnosticBundle {
        output_path: Some(path),
    } = request
    {
        // Capture the last reader's token while the pipe is live. Only the scoped file
        // write uses it, and impersonation never spans an await.
        let token = ClientToken::from_pipe(pipe).map_err(client_access)?;
        let bundle = service
            .collect_diagnostics(Some(PathBuf::from(path)), move |path, bytes| {
                token
                    .with_identity(|| config::atomic_write(path, bytes))
                    .map_err(client_access)?
            })
            .await?;
        return Ok(RpcResult::DiagnosticBundle(bundle));
    }
    service.dispatch(request).await
}

pub(crate) fn create(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    let sddl: Vec<_> = PIPE_SDDL.encode_utf16().chain([0]).collect();
    let mut descriptor = ptr::null_mut();
    let success = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    let descriptor = Descriptor(descriptor);
    let mut attributes = SecurityAttributes {
        length: size_of::<SecurityAttributes>() as u32,
        descriptor: descriptor.0,
        inherit_handle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    // CreateNamedPipe copies the security descriptor during this synchronous call.
    unsafe {
        options.create_with_security_attributes_raw(
            name,
            (&mut attributes as *mut SecurityAttributes).cast(),
        )
    }
}

pub(crate) async fn run(
    name: &str,
    mut listener: NamedPipeServer,
    service: Arc<Service>,
) -> io::Result<()> {
    let mut clients = JoinSet::new();
    loop {
        if clients.len() >= MAX_CLIENTS {
            clients.join_next().await;
            continue;
        }
        tokio::select! {
            result = listener.connect() => {
                result?;
                let client = std::mem::replace(&mut listener, create(name, false)?);
                let service = Arc::clone(&service);
                clients.spawn(async move {
                    if let Err(error) = serve(client, service).await {
                        tracing::debug!(%error, "Named Pipe client disconnected");
                    }
                });
            }
            _ = clients.join_next(), if !clients.is_empty() => {}
        }
    }
}

async fn serve(pipe: NamedPipeServer, service: Arc<Service>) -> io::Result<()> {
    let mut events = service.telemetry.events.subscribe();
    let pipe_handle = pipe.as_raw_handle() as isize;
    let (read, mut write) = aio::split(pipe);
    let mut reader = BufReader::new(read);
    let (reply, mut replies) = mpsc::channel(32);
    let read = async {
        let mut frame = Vec::new();
        while read_frame(&mut reader, &mut frame).await? {
            let response = match serde_json::from_slice::<IpcRequest>(&frame) {
                Ok(request) => match dispatch(&service, pipe_handle, request.request).await {
                    Ok(result) => IpcResponse::ok(request.id, result),
                    Err(error) => IpcResponse::err(request.id, error),
                },
                Err(error) => {
                    let id = serde_json::from_slice::<serde_json::Value>(&frame)
                        .ok()
                        .and_then(|value| value.get("id")?.as_u64())
                        .unwrap_or(0);
                    IpcResponse::err(
                        id,
                        IpcError::new(IpcErrorCode::InvalidRequest, error.to_string()),
                    )
                }
            };
            if reply.send(ServerFrame::Response(response)).await.is_err() {
                break;
            }
        }
        Ok(())
    };
    let write = async {
        loop {
            let frame = tokio::select! {
                biased;
                response = replies.recv() => {
                    let Some(response) = response else { return Ok(()) };
                    response
                }
                event = events.recv() => match event {
                    Ok(event) => ServerFrame::Event(event),
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "Named Pipe client missed events");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            };
            let mut bytes = serde_json::to_vec(&frame)?;
            bytes.push(b'\n');
            tokio::time::timeout(Duration::from_secs(5), write.write_all(&bytes)).await??;
        }
    };
    tokio::select! { result = read => result, result = write => result }
}

async fn read_frame(
    reader: &mut (impl AsyncBufRead + Unpin),
    frame: &mut Vec<u8>,
) -> io::Result<bool> {
    frame.clear();
    let count = reader
        .take((FRAME_LIMIT + 1) as u64)
        .read_until(b'\n', frame)
        .await?;
    if count == 0 {
        return Ok(false);
    }
    if count > FRAME_LIMIT || frame.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Oversized or unterminated NDJSON frame",
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{AppConfig, IpcEvent, IPC_PROTOCOL_VERSION};
    use tokio::net::windows::named_pipe::ClientOptions;

    #[tokio::test]
    async fn pipe_dacl_multiple_clients_and_ndjson_round_trip() {
        let name = format!(
            r"\\.\pipe\my-win-antify-test-{}-{}",
            std::process::id(),
            crate::now_ms()
        );
        let server = create(&name, true).unwrap();
        assert_eq!(
            create(&name, true).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let config_path = std::env::temp_dir().join(format!(
            "my-win-antify-pipe-{}-{}.json",
            std::process::id(),
            crate::now_ms()
        ));
        let service = Service::new(config_path.clone()).await.unwrap();
        let listener_name = name.clone();
        let task_service = Arc::clone(&service);
        let listener = tokio::spawn(async move { run(&listener_name, server, task_service).await });
        let ticker = tokio::spawn(async move { service.telemetry_loop().await });
        let test = async {
            let (mut first, mut second) = (connect(&name).await, connect(&name).await);
            send(&mut first, 1, RpcRequest::Ping).await;
            let pong = response(&mut first, 1).await;
            assert!(
                matches!(pong, RpcResult::Pong(pong) if pong.protocol_version == IPC_PROTOCOL_VERSION)
            );
            // Confirm both peers are being served before broadcasting a config update.
            send(&mut second, 2, RpcRequest::GetConfig).await;
            assert_eq!(
                response(&mut second, 2).await,
                RpcResult::Config(AppConfig::default())
            );
            let mut config = AppConfig::default();
            config.settings.language = "en-US".into();
            send(
                &mut first,
                3,
                RpcRequest::SaveConfig {
                    config: config.clone(),
                },
            )
            .await;
            assert_eq!(
                response(&mut first, 3).await,
                RpcResult::Config(config.clone())
            );
            loop {
                if let ServerFrame::Event(IpcEvent::ConfigChanged(saved)) = recv(&mut second).await
                {
                    assert_eq!(saved, config);
                    break;
                }
            }
            send(&mut second, 4, RpcRequest::GetConfig).await;
            assert_eq!(response(&mut second, 4).await, RpcResult::Config(config));
            send(&mut second, 5, RpcRequest::ListNetworkInterfaces).await;
            assert!(
                matches!(response(&mut second, 5).await, RpcResult::NetworkInterfaces(interfaces)
                if interfaces.iter().all(|iface| iface.ipv4.parse::<std::net::Ipv4Addr>().is_ok()))
            );
            loop {
                if let ServerFrame::Event(IpcEvent::StatsTick(stats)) = recv(&mut second).await {
                    assert!(!stats.running);
                    break;
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(5), test)
            .await
            .unwrap();
        listener.abort();
        ticker.abort();
        std::fs::remove_file(config_path).unwrap();
    }

    #[tokio::test]
    async fn custom_diagnostic_path_requires_client_impersonation() {
        const SECURITY_IMPERSONATION: u32 = 0x0002_0000;
        let scope = format!(
            "antify-diagnostic-pipe-{}-{}",
            std::process::id(),
            crate::now_ms()
        );
        let name = format!(r"\\.\pipe\{scope}");
        let server = create(&name, true).unwrap();
        let dir = std::env::temp_dir().join(scope);
        let config_path = dir.join("config.json");
        let path = dir.join("custom.json");
        let service = Service::new(config_path.clone()).await.unwrap();
        let listener_name = name.clone();
        let listener = tokio::spawn(async move { run(&listener_name, server, service).await });
        let request = || RpcRequest::CollectDiagnosticBundle {
            output_path: Some(path.to_string_lossy().into_owned()),
        };

        tokio::time::timeout(Duration::from_secs(5), async {
            let mut identified = connect(&name).await;
            send(&mut identified, 1, request()).await;
            let ServerFrame::Response(denied) = recv(&mut identified).await else {
                panic!("Expected a diagnostic response");
            };
            assert_eq!(denied.error.unwrap().code, IpcErrorCode::AccessDenied);
            assert!(!path.exists());

            let mut client = BufReader::new(
                ClientOptions::new()
                    .security_qos_flags(SECURITY_IMPERSONATION)
                    .open(&name)
                    .unwrap(),
            );
            send(&mut client, 2, request()).await;
            let RpcResult::DiagnosticBundle(summary) = response(&mut client, 2).await else {
                panic!("Expected a diagnostic bundle");
            };
            assert_eq!(PathBuf::from(summary.path), path);
            assert!(
                serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).is_ok()
            );
        })
        .await
        .unwrap();
        listener.abort();
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(&config_path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    type Client = BufReader<tokio::net::windows::named_pipe::NamedPipeClient>;

    async fn connect(name: &str) -> Client {
        loop {
            match ClientOptions::new().open(name) {
                Ok(client) => return BufReader::new(client),
                Err(error) if error.raw_os_error() == Some(231) => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(error) => panic!("pipe open failed: {error}"),
            }
        }
    }

    async fn send(client: &mut Client, id: u64, request: RpcRequest) {
        let mut bytes = serde_json::to_vec(&IpcRequest { id, request }).unwrap();
        bytes.push(b'\n');
        client.get_mut().write_all(&bytes).await.unwrap();
    }

    async fn recv(client: &mut Client) -> ServerFrame {
        let mut frame = Vec::new();
        assert!(read_frame(client, &mut frame).await.unwrap());
        serde_json::from_slice(&frame).unwrap()
    }

    async fn response(client: &mut Client, id: u64) -> RpcResult {
        loop {
            if let ServerFrame::Response(response) = recv(client).await {
                assert_eq!(response.id, id);
                assert!(response.ok, "{:?}", response.error);
                return response.result.unwrap();
            }
        }
    }
}
