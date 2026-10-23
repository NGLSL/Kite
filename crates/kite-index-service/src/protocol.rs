use crate::error::{ServiceError, ServiceResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::PeekNamedPipe;

pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CHECKPOINT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_VOLUMES: usize = 32;

/// Kernel cancellation token shared by capture/watch and the service's
/// disconnect/stop handlers.
pub type CancelToken = Arc<std::sync::atomic::AtomicBool>;

/// JSON progress payload passed through unchanged from the metadata kernel.
pub type ProgressSink = dyn Fn(Value) + Send + Sync;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    Rebuild,
    Checkpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexProgress {
    pub phase: String,
    #[serde(default)]
    pub volume: Option<String>,
    #[serde(default)]
    pub completed: Option<u64>,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct BackendCapture {
    /// The backend must return paths under the service-provided root.
    pub capture_root: PathBuf,
    pub manifest_path: PathBuf,
    pub checkpoint_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CaptureOutcome {
    pub capture_root: PathBuf,
    pub manifest_path: PathBuf,
    pub checkpoint_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct WatchOutcome {
    pub feed_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    Hello {
        protocol: u32,
        pid: u32,
        parent_pid: u32,
        session_id: u32,
        sid: String,
    },
    Capture {
        volumes: Vec<String>,
    },
    Watch {
        #[serde(default)]
        checkpoint_path: Option<String>,
        #[serde(default)]
        checkpoint_json: Option<Vec<u8>>,
        #[serde(default)]
        resume_cursors: Option<Value>,
    },
    Acknowledge {
        seq: u64,
    },
    Cancel,
    Ping,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    HelloOk {
        protocol: u32,
    },
    Accepted {
        operation: String,
    },
    Progress {
        event: Value,
    },
    CaptureComplete {
        capture_root: String,
        manifest: String,
        checkpoint: String,
    },
    WatchReady {
        feed_path: String,
    },
    WatchComplete,
    Pong,
    Error {
        error_kind: String,
        message: String,
        code: Option<u32>,
    },
}

pub fn send_request(
    writer: &Arc<Mutex<std::io::BufWriter<std::fs::File>>>,
    request: Request,
) -> ServiceResult<()> {
    let bytes = serde_json::to_vec(&request)
        .map_err(|e| ServiceError::protocol(format!("encode request: {e}")))?;
    if bytes.len() > MAX_LINE_BYTES {
        return Err(ServiceError::invalid_request(
            "request exceeds protocol size limit",
        ));
    }
    let mut guard = writer
        .lock()
        .map_err(|_| ServiceError::internal("pipe writer lock poisoned"))?;
    // `BufWriter::flush` on a synchronous Windows named pipe calls
    // `FlushFileBuffers`, which waits until the peer drains the pipe.  That
    // can deadlock a capture progress callback when the peer is itself
    // waiting for the next protocol response.  Bypass the user-space buffer
    // and submit each complete line directly with WriteFile instead.
    guard
        .get_mut()
        .write_all(&bytes)
        .map_err(ServiceError::from)?;
    guard.get_mut().write_all(b"\n").map_err(ServiceError::from)
}

pub fn read_request(reader: &mut BufReader<File>) -> ServiceResult<Request> {
    let line = read_bounded_line(
        reader,
        "client disconnected",
        "request exceeds protocol size limit",
        None,
    )?;
    serde_json::from_slice(&line)
        .map_err(|e| ServiceError::protocol(format!("decode request: {e}")))
}

pub fn send_response(
    writer: &Arc<Mutex<std::io::BufWriter<std::fs::File>>>,
    response: Response,
) -> ServiceResult<()> {
    let bytes = serde_json::to_vec(&response)
        .map_err(|e| ServiceError::protocol(format!("encode response: {e}")))?;
    if bytes.len() > MAX_LINE_BYTES {
        return Err(ServiceError::protocol(
            "response exceeds protocol size limit",
        ));
    }
    let mut guard = writer
        .lock()
        .map_err(|_| ServiceError::internal("pipe writer lock poisoned"))?;
    // See `send_request`: never use FlushFileBuffers for a synchronous pipe.
    guard
        .get_mut()
        .write_all(&bytes)
        .map_err(ServiceError::from)?;
    guard.get_mut().write_all(b"\n").map_err(ServiceError::from)
}

pub fn read_response(reader: &mut BufReader<File>) -> ServiceResult<Response> {
    read_response_until(reader, None)
}

/// Read one response with an optional deadline. The service uses the
/// unbounded form above for normal capture/watch traffic; the normal client
/// uses this bounded form only for the initial Hello handshake so a dead or
/// half-connected old service cannot hold a rebuild connection forever.
pub(crate) fn read_response_until(
    reader: &mut BufReader<File>,
    deadline: Option<Instant>,
) -> ServiceResult<Response> {
    let line = read_bounded_line(
        reader,
        "service disconnected",
        "response exceeds protocol size limit",
        deadline,
    )?;
    serde_json::from_slice(&line)
        .map_err(|e| ServiceError::protocol(format!("decode response: {e}")))
}

fn read_bounded_line(
    reader: &mut BufReader<File>,
    eof_message: &str,
    limit_message: &str,
    deadline: Option<Instant>,
) -> ServiceResult<Vec<u8>> {
    // `read_until` grows its destination until it sees a newline. A client
    // connects before authentication, so an unbounded call would let a local
    // peer make the SYSTEM service allocate indefinitely. Consume the
    // BufRead window in bounded chunks and reject before extending the line
    // beyond the protocol cap.
    let mut line = Vec::with_capacity(MAX_LINE_BYTES.min(64 * 1024));
    loop {
        // A synchronous ReadFile on an empty named pipe can hold the pipe's
        // file object in a way that blocks a peer WriteFile on a duplicated
        // handle. This is visible while the service waits for the next
        // request and a capture worker sends progress, or when the UI sends
        // Cancel during that wait. Peek first and issue a real read only
        // after bytes are available. Existing BufReader bytes are consumed
        // without another peek.
        if reader.buffer().is_empty() {
            wait_for_pipe_data(reader, deadline)?;
        }
        let available = reader.fill_buf().map_err(ServiceError::from)?;
        if available.is_empty() {
            return if line.is_empty() {
                Err(ServiceError::pipe(eof_message))
            } else {
                Err(ServiceError::protocol(
                    "protocol message is not newline terminated",
                ))
            };
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        if line.len().saturating_add(take) > MAX_LINE_BYTES {
            return Err(ServiceError::protocol(limit_message));
        }
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if line.ends_with(b"\n") {
            return Ok(line);
        }
    }
}

fn wait_for_pipe_data(reader: &BufReader<File>, deadline: Option<Instant>) -> ServiceResult<()> {
    let handle = HANDLE(reader.get_ref().as_raw_handle() as _);
    loop {
        if let Some(deadline) = deadline {
            if Instant::now() >= deadline {
                return Err(ServiceError::with_code(
                    crate::ErrorKind::Internal,
                    "连接 Kite 文件索引服务握手超时",
                    1460,
                ));
            }
        }
        let mut available = 0u32;
        unsafe { PeekNamedPipe(handle, None, 0, None, Some(&mut available), None) }
            .map_err(ServiceError::from)?;
        if available != 0 {
            return Ok(());
        }
        let interval = Duration::from_millis(5);
        let sleep = deadline.map_or(interval, |deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .min(interval)
        });
        thread::sleep(sleep);
    }
}

impl ServiceError {
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        ServiceError::new(crate::ErrorKind::Internal, message)
    }
}

pub(crate) fn error_response(error: &ServiceError) -> Response {
    Response::Error {
        error_kind: match error.kind {
            crate::ErrorKind::AccessDenied => "access_denied",
            crate::ErrorKind::NeedsInstall => "needs_install",
            crate::ErrorKind::NotInstalled => "not_installed",
            crate::ErrorKind::NotRunning => "not_running",
            crate::ErrorKind::InvalidRequest => "invalid_request",
            crate::ErrorKind::Authentication => "authentication",
            crate::ErrorKind::Protocol => "protocol",
            crate::ErrorKind::Pipe => "pipe",
            crate::ErrorKind::Service => "service",
            crate::ErrorKind::Backend => "backend",
            crate::ErrorKind::Cancelled => "cancelled",
            crate::ErrorKind::PathMismatch => "path_mismatch",
            crate::ErrorKind::Internal => "internal",
        }
        .to_string(),
        message: error.message.clone(),
        code: error.code,
    }
}
