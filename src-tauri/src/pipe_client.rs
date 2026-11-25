use std::{
    collections::HashMap,
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use shared::{
    IpcError, IpcErrorCode, IpcEvent, IpcRequest, IpcResponse, PipeConnectionState, RpcRequest,
    RpcResult, ServerFrame, DEFAULT_PIPE_NAME, EVENT_ACTIVITY_LOG, EVENT_CONFIG_CHANGED,
    EVENT_CONNECTION_EVENT, EVENT_PIPE_CONNECTION, EVENT_PROXY_TESTS_STALE, EVENT_STATS_TICK,
    EVENT_STATUS_CHANGED, EVENT_WIFI_CHANGED,
};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{self as aio, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::windows::named_pipe::{ClientOptions, NamedPipeClient},
    sync::{mpsc, oneshot},
};

const FRAME_LIMIT: usize = 32 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(8);
const MAX_PENDING: usize = 64;

pub(crate) struct PipeClient {
    connection: Arc<Mutex<Connection>>,
    next: AtomicU64,
}

struct Connection {
    phase: PipeConnectionState,
    calls: Option<mpsc::Sender<Call>>,
}

struct Call {
    request: IpcRequest,
    reply: oneshot::Sender<IpcResponse>,
}

impl PipeClient {
    pub fn start(app: AppHandle) -> Self {
        let connection = Arc::new(Mutex::new(Connection {
            phase: PipeConnectionState::Disconnected,
            calls: None,
        }));
        tauri::async_runtime::spawn(reconnect(app, Arc::clone(&connection)));
        Self {
            connection,
            next: AtomicU64::new(1),
        }
    }

    pub fn state(&self) -> PipeConnectionState {
        self.connection.lock().unwrap().phase
    }

    pub async fn request(&self, request: RpcRequest) -> Result<RpcResult, IpcError> {
        let calls = self
            .connection
            .lock()
            .unwrap()
            .calls
            .clone()
            .ok_or_else(disconnected)?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (reply, response) = oneshot::channel();
        calls
            .try_send(Call {
                request: IpcRequest { id, request },
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Closed(_) => disconnected(),
                mpsc::error::TrySendError::Full(_) => {
                    IpcError::new(IpcErrorCode::Internal, "Core service request queue is full")
                }
            })?;
        let response = tokio::time::timeout(REQUEST_TIMEOUT, response)
            .await
            .map_err(|_| IpcError::new(IpcErrorCode::Internal, "Core service request timed out"))?
            .map_err(|_| disconnected())?;
        if response.ok {
            response
                .result
                .ok_or_else(|| unexpected("Core response has no result"))
        } else {
            Err(response
                .error
                .unwrap_or_else(|| unexpected("Core response has no error")))
        }
    }
}

fn disconnected() -> IpcError {
    IpcError::new(
        IpcErrorCode::ServiceDisconnected,
        "Core service is disconnected",
    )
}

pub(crate) fn unexpected(message: &str) -> IpcError {
    IpcError::new(IpcErrorCode::Internal, message)
}

async fn reconnect(app: AppHandle, connection: Arc<Mutex<Connection>>) {
    let mut backoff = INITIAL_BACKOFF;
    loop {
        match ClientOptions::new().open(DEFAULT_PIPE_NAME) {
            Ok(pipe) => {
                let (calls, queue) = mpsc::channel(MAX_PENDING);
                transition(
                    &app,
                    &connection,
                    PipeConnectionState::Connected,
                    Some(calls),
                );
                if let Err(error) = session(&app, pipe, queue).await {
                    tracing::debug!(%error, "Core service pipe disconnected");
                }
                transition(&app, &connection, PipeConnectionState::Disconnected, None);
                backoff = INITIAL_BACKOFF;
            }
            Err(error) => tracing::trace!(%error, "Core service pipe open failed"),
        }
        transition(&app, &connection, PipeConnectionState::Reconnecting, None);
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn transition(
    app: &AppHandle,
    connection: &Mutex<Connection>,
    phase: PipeConnectionState,
    calls: Option<mpsc::Sender<Call>>,
) {
    let changed = {
        let mut connection = connection.lock().unwrap();
        let previous = std::mem::replace(&mut connection.phase, phase);
        connection.calls = calls;
        previous != phase
    };
    if changed {
        emit(app, EVENT_PIPE_CONNECTION, phase);
    }
}

async fn session(
    app: &AppHandle,
    pipe: NamedPipeClient,
    mut calls: mpsc::Receiver<Call>,
) -> io::Result<()> {
    let (read, mut write) = aio::split(pipe);
    let mut reader = BufReader::new(read);
    // Each transport owns its queue and pending map, so a reconnect never replays an RPC.
    let pending = Mutex::new(HashMap::<u64, oneshot::Sender<IpcResponse>>::new());
    let read = async {
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            let count = (&mut reader)
                .take((FRAME_LIMIT + 1) as u64)
                .read_until(b'\n', &mut bytes)
                .await?;
            if count == 0 {
                return Ok(());
            }
            if count > FRAME_LIMIT || bytes.last() != Some(&b'\n') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Oversized or unterminated core frame",
                ));
            }
            match serde_json::from_slice::<ServerFrame>(&bytes)? {
                ServerFrame::Response(response) => {
                    if let Some(reply) = pending.lock().unwrap().remove(&response.id) {
                        let _ = reply.send(response);
                    }
                }
                ServerFrame::Event(event) => match event {
                    IpcEvent::StatusChanged(payload) => emit(app, EVENT_STATUS_CHANGED, payload),
                    IpcEvent::StatsTick(payload) => emit(app, EVENT_STATS_TICK, payload),
                    IpcEvent::ConnectionEvent(payload) => {
                        emit(app, EVENT_CONNECTION_EVENT, payload)
                    }
                    IpcEvent::ActivityLog(payload) => emit(app, EVENT_ACTIVITY_LOG, payload),
                    IpcEvent::ConfigChanged(payload) => emit(app, EVENT_CONFIG_CHANGED, payload),
                    IpcEvent::WifiChanged(payload) => emit(app, EVENT_WIFI_CHANGED, payload),
                    IpcEvent::ProxyTestsStale => emit(app, EVENT_PROXY_TESTS_STALE, ()),
                },
            }
        }
    };
    let write = async {
        while let Some(call) = calls.recv().await {
            if call.reply.is_closed() {
                continue;
            }
            let mut bytes = serde_json::to_vec(&call.request)?;
            bytes.push(b'\n');
            {
                let mut pending = pending.lock().unwrap();
                pending.retain(|_, reply| !reply.is_closed());
                if pending.len() >= MAX_PENDING || bytes.len() > FRAME_LIMIT {
                    let response = IpcResponse::err(
                        call.request.id,
                        unexpected(
                            "Core service request exceeded the frame or pending request limit",
                        ),
                    );
                    let _ = call.reply.send(response);
                    continue;
                }
                pending.insert(call.request.id, call.reply);
            }
            tokio::time::timeout(REQUEST_TIMEOUT, write.write_all(&bytes)).await??;
        }
        Ok(())
    };
    tokio::select! { result = read => result, result = write => result }
}

fn emit(app: &AppHandle, name: &str, payload: impl serde::Serialize + Clone) {
    if let Err(error) = app.emit(name, payload) {
        tracing::warn!(event = name, %error, "Core event emission failed");
    }
}
