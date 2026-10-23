//! Privileged metadata service boundary for Kite's file-search plugin.
//!
//! The crate intentionally contains no index or search implementation.  The
//! normal Kite process owns the index and search files; this crate only moves
//! metadata capture and USN-watch work into a demand-start LocalSystem service.
//! A service session is created for one authenticated Kite process and all
//! output paths are generated below `%ProgramData%\Kite\FileIndex\<SID>`.
#![cfg(windows)]

mod error;
mod identity;
mod install;
mod pipe;
mod protocol;
mod service;

pub use error::{ErrorKind, ServiceError, ServiceResult};
pub use install::{
    current_state, ensure_installed, install_entry, request_install, uninstall_entry,
    InstallOptions, ServiceState,
};
pub use protocol::{BackendCapture, CaptureOutcome, IndexProgress, SessionMode, WatchOutcome};
pub use service::{
    dispatch_entry, local_ntfs_volumes, register_backend, service_entry, CancelToken,
    MetadataBackend, ProgressSink,
};

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The stable SCM service name.  The service is demand-start and has no UI.
pub const SERVICE_NAME: &str = "KiteFileIndex";
/// Human-readable SCM display name.
pub const SERVICE_DISPLAY_NAME: &str = "Kite File Index Metadata Service";
/// Named-pipe protocol version.
/// Version 3 adds durable cursor recovery and acknowledged feed segments.
pub const PROTOCOL_VERSION: u32 = 3;
/// Fixed endpoint across protocol revisions, so an outdated service can answer
/// the handshake with a version error instead of a pipe connection timeout.
pub const PIPE_NAME: &str = r"\\.\pipe\Kite\FileIndex\v1";

/// A connected normal-user client.  The service is started on demand; the
/// client keeps the connection alive for one capture or watch session.
pub struct Client {
    reader: std::io::BufReader<std::fs::File>,
    writer: Arc<Mutex<std::io::BufWriter<std::fs::File>>>,
    pub(crate) identity: identity::ClientIdentity,
}

/// A cancellation handle that is safe to keep in a UI/controller task while a
/// capture or watch call blocks on the pipe.
#[derive(Clone)]
pub struct CancelHandle {
    writer: Arc<Mutex<std::io::BufWriter<std::fs::File>>>,
}

impl CancelHandle {
    /// Confirm that feed records through this sequence are durably consumed.
    pub fn acknowledge(&self, seq: u64) -> ServiceResult<()> {
        protocol::send_request(&self.writer, protocol::Request::Acknowledge { seq })
    }

    /// Ask the service to cancel the active metadata operation.  Closing the
    /// client still cancels it if the pipe is already broken.
    pub fn cancel(&self) -> ServiceResult<()> {
        protocol::send_request(&self.writer, protocol::Request::Cancel)
    }
}

/// Long-running USN watch session.  Run it on a worker thread; the UI can
/// cancel through [`WatchSession::cancel`] or its [`CancelHandle`].
pub struct WatchSession {
    reader: std::io::BufReader<std::fs::File>,
    writer: Arc<Mutex<std::io::BufWriter<std::fs::File>>>,
    feed_path: PathBuf,
    done: bool,
}

impl Client {
    /// Connect to a demand-start service. When `auto_install` is true and the
    /// fixed service is missing, the caller is explicitly asking for the
    /// one-time `runas` install flow; false leaves installation to the UI.
    pub fn connect(auto_install: bool, parent_pid: u32) -> ServiceResult<Self> {
        let mut state = current_state()?;
        if matches!(state, ServiceState::Missing | ServiceState::PathMismatch) {
            if auto_install {
                request_install()?;
                state = current_state()?;
                if matches!(state, ServiceState::Missing | ServiceState::PathMismatch) {
                    return Err(ServiceError::needs_install(
                        "管理员安装完成后仍未找到受保护的 Kite 文件索引服务",
                    ));
                }
            } else {
                return Err(ServiceError::not_installed());
            }
        }
        // Keep one deadline for the whole post-install connection attempt.
        // In particular, do not give the pipe open a fresh timeout after an
        // old service instance has transitioned from RUNNING to STOPPED.
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut last_transient = None;
        loop {
            if Instant::now() >= deadline {
                return Err(connection_timeout(last_transient));
            }

            // Repeat this state check for every attempt. A previous client
            // may still be draining its service session; when that instance
            // finally stops, start_service_until observes STOPPED and starts
            // the replacement within this same deadline.
            state = current_state()?;
            if matches!(state, ServiceState::Missing | ServiceState::PathMismatch) {
                return Err(ServiceError::needs_install(
                    "管理员安装完成后仍未找到受保护的 Kite 文件索引服务",
                ));
            }
            install::start_service_until(deadline)?;

            match Self::connect_once(parent_pid, deadline) {
                Ok(client) => return Ok(client),
                Err(error) if is_transient_connect_error(&error) => {
                    last_transient = Some(error);
                    if Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(40));
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn connect_once(parent_pid: u32, deadline: Instant) -> ServiceResult<Self> {
        let file = pipe::connect_client_once()?;
        let identity = identity::current_client_identity(parent_pid)?;
        let reader_file = file.try_clone().map_err(ServiceError::from)?;
        let writer_file = file.try_clone().map_err(ServiceError::from)?;
        let writer = Arc::new(Mutex::new(std::io::BufWriter::new(writer_file)));
        let mut client = Self {
            reader: std::io::BufReader::new(reader_file),
            writer,
            identity,
        };
        protocol::send_request(
            &client.writer,
            protocol::Request::Hello {
                protocol: PROTOCOL_VERSION,
                pid: client.identity.pid,
                parent_pid: client.identity.parent_pid,
                session_id: client.identity.session_id,
                sid: client.identity.sid.clone(),
            },
        )?;
        match protocol::read_response_until(&mut client.reader, Some(deadline))? {
            protocol::Response::HelloOk { protocol } if protocol == PROTOCOL_VERSION => Ok(client),
            protocol::Response::HelloOk { .. } => Err(ServiceError::needs_install(
                "文件索引读取组件版本不匹配，请授权更新后重试",
            )),
            protocol::Response::Error {
                error_kind: kind,
                message,
                code,
            } => {
                let error = ServiceError::remote(kind, message, code);
                if error.kind == ErrorKind::Protocol {
                    Err(ServiceError::needs_install(
                        "文件索引读取组件版本不匹配，请授权更新后重试",
                    ))
                } else {
                    Err(error)
                }
            }
            other => Err(ServiceError::protocol(format!(
                "unexpected handshake response: {other:?}"
            ))),
        }
    }

    /// Connect using the current process as the authenticated client. The
    /// service still derives and validates this process's actual parent PID;
    /// no caller-supplied PID is trusted for authorization.
    pub fn connect_current(auto_install: bool) -> ServiceResult<Self> {
        Self::connect(auto_install, 0)
    }

    /// Build the complete metadata capture.  The service owns the output
    /// directory and returns only generated paths after checking their ACL.
    pub fn capture(
        &mut self,
        volumes: &[String],
        progress: Option<&ProgressSink>,
    ) -> ServiceResult<CaptureOutcome> {
        protocol::send_request(
            &self.writer,
            protocol::Request::Capture {
                volumes: volumes.to_vec(),
            },
        )?;
        loop {
            match protocol::read_response(&mut self.reader)? {
                protocol::Response::Progress { event } => {
                    if let Some(callback) = progress {
                        callback(event);
                    }
                }
                protocol::Response::CaptureComplete {
                    capture_root,
                    manifest,
                    checkpoint,
                } => {
                    return Ok(CaptureOutcome {
                        capture_root: PathBuf::from(capture_root),
                        manifest_path: PathBuf::from(manifest),
                        checkpoint_path: PathBuf::from(checkpoint),
                    });
                }
                protocol::Response::Error {
                    error_kind: kind,
                    message,
                    code,
                } => {
                    return Err(ServiceError::remote(kind, message, code));
                }
                other => {
                    return Err(ServiceError::protocol(format!(
                        "unexpected capture response: {other:?}"
                    )))
                }
            }
        }
    }

    /// Start an indefinite USN watch.  The checkpoint is sent as bounded JSON
    /// bytes; the service copies it into its generated session directory and
    /// validates its volume/journal identities before reading.
    pub fn watch(
        mut self,
        checkpoint_json: &[u8],
        progress: Option<&ProgressSink>,
    ) -> ServiceResult<WatchSession> {
        if checkpoint_json.is_empty() || checkpoint_json.len() > protocol::MAX_CHECKPOINT_BYTES {
            return Err(ServiceError::invalid_request(
                "checkpoint JSON size is outside the supported bound",
            ));
        }
        protocol::send_request(
            &self.writer,
            protocol::Request::Watch {
                checkpoint_path: None,
                checkpoint_json: Some(checkpoint_json.to_vec()),
                resume_cursors: None,
            },
        )?;
        loop {
            match protocol::read_response(&mut self.reader)? {
                protocol::Response::Progress { event } => {
                    if let Some(callback) = progress {
                        callback(event);
                    }
                }
                protocol::Response::WatchReady { feed_path } => {
                    return Ok(WatchSession {
                        reader: self.reader,
                        writer: Arc::clone(&self.writer),
                        feed_path: PathBuf::from(feed_path),
                        done: false,
                    });
                }
                protocol::Response::Error {
                    error_kind: kind,
                    message,
                    code,
                } => {
                    return Err(ServiceError::remote(kind, message, code));
                }
                other => {
                    return Err(ServiceError::protocol(format!(
                        "unexpected watch response: {other:?}"
                    )))
                }
            }
        }
    }

    /// Start a watch from a checkpoint that was produced by a prior service
    /// capture.  The service verifies the path is inside the protected
    /// ProgramData\Kite\FileIndex\<SID> tree before copying it.
    pub fn watch_path(
        self,
        checkpoint_path: &std::path::Path,
        progress: Option<&ProgressSink>,
    ) -> ServiceResult<WatchSession> {
        self.watch_path_from(checkpoint_path, None, progress)
    }

    /// Resume only cursors bound to the original protected checkpoint identities.
    pub fn watch_path_from(
        mut self,
        checkpoint_path: &std::path::Path,
        resume_cursors: Option<serde_json::Value>,
        progress: Option<&ProgressSink>,
    ) -> ServiceResult<WatchSession> {
        let path = checkpoint_path.to_string_lossy().into_owned();
        protocol::send_request(
            &self.writer,
            protocol::Request::Watch {
                checkpoint_path: Some(path),
                checkpoint_json: None,
                resume_cursors,
            },
        )?;
        loop {
            match protocol::read_response(&mut self.reader)? {
                protocol::Response::Progress { event } => {
                    if let Some(callback) = progress {
                        callback(event);
                    }
                }
                protocol::Response::WatchReady { feed_path } => {
                    return Ok(WatchSession {
                        reader: self.reader,
                        writer: Arc::clone(&self.writer),
                        feed_path: PathBuf::from(feed_path),
                        done: false,
                    });
                }
                protocol::Response::Error {
                    error_kind: kind,
                    message,
                    code,
                } => {
                    return Err(ServiceError::remote(kind, message, code));
                }
                other => {
                    return Err(ServiceError::protocol(format!(
                        "unexpected watch response: {other:?}"
                    )))
                }
            }
        }
    }

    /// Obtain a cancellation handle without moving the client.
    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle {
            writer: Arc::clone(&self.writer),
        }
    }

    /// Cancel the active capture/watch without waiting for the worker to
    /// finish. Dropping the client remains the stronger lifetime boundary and
    /// also closes the authenticated pipe session.
    pub fn stop(&self) -> ServiceResult<()> {
        self.cancel_handle().cancel()
    }
}

fn is_transient_connect_error(error: &ServiceError) -> bool {
    matches!(error.kind, ErrorKind::Pipe)
        || matches!(
            error.code,
            Some(2 | 53 | 109 | 231 | 232 | 233 | 535 | 1460)
        )
}

fn connection_timeout(last_transient: Option<ServiceError>) -> ServiceError {
    let code = last_transient.and_then(|error| error.code).unwrap_or(2);
    ServiceError::with_code(ErrorKind::Internal, "连接 Kite 文件索引服务超时", code)
}

impl WatchSession {
    pub fn feed_path(&self) -> &std::path::Path {
        &self.feed_path
    }

    /// Read the next service event.  A `None` result means the watch ended
    /// normally after cancellation or a client disconnect.
    pub fn next(&mut self, progress: Option<&ProgressSink>) -> ServiceResult<bool> {
        if self.done {
            return Ok(false);
        }
        match protocol::read_response(&mut self.reader)? {
            protocol::Response::Progress { event } => {
                if let Some(callback) = progress {
                    callback(event);
                }
                Ok(true)
            }
            protocol::Response::WatchComplete => {
                self.done = true;
                Ok(false)
            }
            protocol::Response::Error {
                error_kind: kind,
                message,
                code,
            } => {
                self.done = true;
                Err(ServiceError::remote(kind, message, code))
            }
            other => Err(ServiceError::protocol(format!(
                "unexpected watch response: {other:?}"
            ))),
        }
    }

    pub fn cancel(&self) -> ServiceResult<()> {
        protocol::send_request(&self.writer, protocol::Request::Cancel)
    }

    pub fn stop(&self) -> ServiceResult<()> {
        self.cancel()
    }
}

impl Drop for WatchSession {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.cancel();
        }
    }
}
