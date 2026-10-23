use crate::error::{ServiceError, ServiceResult};
use crate::identity::{validate_client, HandleGuard};
use crate::install::{self};
use crate::pipe;
use crate::protocol::{self, BackendCapture, Request, Response};
use crate::{PROTOCOL_VERSION, SERVICE_NAME};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{GetLogicalDrives, GetVolumeInformationW};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Services::{
    RegisterServiceCtrlHandlerExW, SetServiceStatus, StartServiceCtrlDispatcherW,
    SERVICE_ACCEPT_STOP, SERVICE_CONTROL_INTERROGATE, SERVICE_CONTROL_STOP, SERVICE_RUNNING,
    SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOPPED,
    SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

pub use crate::protocol::{CancelToken, ProgressSink};

const MAX_DIAGNOSTIC_BYTES: u64 = 4 * 1024 * 1024;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

/// Adapter implemented by `kite-file-index`.  It receives only a service-owned
/// output root/checkpoint path and may never choose an arbitrary destination.
pub trait MetadataBackend: Send + Sync + 'static {
    fn capture(
        &self,
        root: &Path,
        volumes: &[String],
        cancel: CancelToken,
        progress: &ProgressSink,
    ) -> ServiceResult<BackendCapture>;
    fn watch(
        &self,
        checkpoint: &Path,
        feed: &Path,
        cancel: CancelToken,
        progress: &ProgressSink,
    ) -> ServiceResult<()>;
}

/// The production adapter is intentionally kept here so the fixed service
/// image can dispatch without the normal Kite process registering a callback
/// before its early command-line handling.  A caller may still register a
/// wrapper when it needs additional progress decoration.
struct KernelBackend;

impl MetadataBackend for KernelBackend {
    fn capture(
        &self,
        root: &Path,
        volumes: &[String],
        cancel: CancelToken,
        progress: &ProgressSink,
    ) -> ServiceResult<BackendCapture> {
        let manifest = kite_file_index::capture(root, volumes, cancel, progress)
            .map_err(|error| ServiceError::backend(error.to_string()))?;
        Ok(BackendCapture {
            capture_root: manifest.root.clone(),
            manifest_path: manifest.root.join("capture.json"),
            checkpoint_path: manifest.checkpoint.clone(),
        })
    }

    fn watch(
        &self,
        checkpoint: &Path,
        feed: &Path,
        cancel: CancelToken,
        progress: &ProgressSink,
    ) -> ServiceResult<()> {
        kite_file_index::watch(checkpoint, feed, cancel, progress)
            .map_err(|error| ServiceError::backend(error.to_string()))
    }
}

static BACKEND: OnceLock<Arc<dyn MetadataBackend>> = OnceLock::new();
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static STATUS_HANDLE: AtomicIsize = AtomicIsize::new(0);
/// Raw handle is held only while the service owns the corresponding `File`.
/// The SCM callback uses it to disconnect a blocking pipe read immediately;
/// the owner clears it before dropping the file.
static ACTIVE_PIPE: AtomicIsize = AtomicIsize::new(0);

/// Register the kernel adapter before calling [`dispatch_entry`].
pub fn register_backend(backend: Arc<dyn MetadataBackend>) -> ServiceResult<()> {
    BACKEND
        .set(backend)
        .map_err(|_| ServiceError::internal("索引服务后端已注册"))
}

/// Enumerate mounted NTFS drive roots at the point a session starts.  The
/// service does not keep a hard-coded C/D/E list, and removable/non-NTFS
/// volumes are left to the caller's normal Everything fallback.
pub fn local_ntfs_volumes() -> ServiceResult<Vec<String>> {
    let mask = unsafe { GetLogicalDrives() };
    if mask == 0 {
        return Err(ServiceError::service("无法枚举本地卷"));
    }
    let mut volumes = Vec::new();
    for index in 0..26u32 {
        if mask & (1 << index) == 0 {
            continue;
        }
        let letter = (b'A' + index as u8) as char;
        let root = format!("{letter}:\\");
        let wide = wide_null(&root);
        let mut fs_name = [0u16; 32];
        let result = unsafe {
            GetVolumeInformationW(
                PCWSTR(wide.as_ptr()),
                None,
                None,
                None,
                None,
                Some(&mut fs_name),
            )
        };
        if result.is_err() {
            continue;
        }
        let end = fs_name
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(fs_name.len());
        let name = String::from_utf16_lossy(&fs_name[..end]);
        if name.eq_ignore_ascii_case("NTFS") {
            volumes.push(format!("{letter}:"));
        }
    }
    if volumes.is_empty() {
        return Err(ServiceError::service("未找到可读取的 NTFS 卷"));
    }
    Ok(volumes)
}

/// Handle explicit internal process flags.  Returns true when the caller must
/// exit immediately (service/install/uninstall).  No implicit elevation is
/// performed; install is expected to be launched by the UI through `runas`.
pub fn dispatch_entry() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--kite-index-install") {
        let result = std::env::current_exe()
            .map(|executable| install::install_entry(&install::InstallOptions { executable }));
        if let Err(error) = result.unwrap_or_else(|error| Err(ServiceError::from(error))) {
            eprintln!("Kite file-index install failed: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if args.iter().any(|arg| arg == "--kite-index-uninstall") {
        if let Err(error) = install::uninstall_entry() {
            eprintln!("Kite file-index uninstall failed: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if args.iter().any(|arg| arg == "--kite-index-stop-for-update") {
        if let Err(error) = install::stop_for_update_entry() {
            eprintln!("Kite file-index service stop failed: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if args.iter().any(|arg| arg == "--kite-index-service") {
        let backend = BACKEND
            .get()
            .cloned()
            .unwrap_or_else(|| Arc::new(KernelBackend));
        if let Err(error) = service_entry(backend) {
            eprintln!("Kite file-index service failed: {error}");
            std::process::exit(1);
        }
        return true;
    }
    false
}

/// Enter the SCM dispatcher.  This is called only by the fixed Program Files
/// service image, never by ordinary Kite UI code.
pub fn service_entry(backend: Arc<dyn MetadataBackend>) -> ServiceResult<()> {
    let _ = BACKEND.set(backend);
    STOP_REQUESTED.store(false, Ordering::SeqCst);
    ACTIVE_PIPE.store(0, Ordering::SeqCst);
    let name = wide_null(SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(name.as_ptr() as _),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW::default(),
    ];
    unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) }.map_err(ServiceError::from)
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    let name = wide_null(SERVICE_NAME);
    let status = RegisterServiceCtrlHandlerExW(PCWSTR(name.as_ptr()), Some(service_control), None);
    let status = match status {
        Ok(value) => value,
        Err(_) => return,
    };
    STATUS_HANDLE.store(status.0 as isize, Ordering::SeqCst);
    report_status(status, SERVICE_START_PENDING, 0, 3000);
    report_status(status, SERVICE_RUNNING, SERVICE_ACCEPT_STOP, 0);
    if let Some(backend) = BACKEND.get() {
        let _ = run_pipe_server(Arc::clone(backend));
    }
    report_status(status, SERVICE_STOPPED, 0, 0);
    STATUS_HANDLE.store(0, Ordering::SeqCst);
}

unsafe extern "system" fn service_control(
    control: u32,
    _event_type: u32,
    _event_data: *mut std::ffi::c_void,
    _context: *mut std::ffi::c_void,
) -> u32 {
    match control {
        SERVICE_CONTROL_STOP => {
            STOP_REQUESTED.store(true, Ordering::SeqCst);
            let raw = ACTIVE_PIPE.load(Ordering::SeqCst);
            if raw != 0 {
                pipe::disconnect_from_handle(HANDLE(raw as *mut std::ffi::c_void));
            }
            0
        }
        SERVICE_CONTROL_INTERROGATE => 0,
        _ => 0,
    }
}

fn report_status(
    handle: SERVICE_STATUS_HANDLE,
    state: windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE,
    accepted: u32,
    wait_hint: u32,
) {
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: accepted,
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: wait_hint,
    };
    let _ = unsafe { SetServiceStatus(handle, &status) };
}

fn run_pipe_server(backend: Arc<dyn MetadataBackend>) -> ServiceResult<()> {
    loop {
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            return Ok(());
        }
        let (file, _descriptor) = pipe::create_server_pipe()?;
        let raw = file.as_raw_handle() as isize;
        ACTIVE_PIPE.store(raw, Ordering::SeqCst);
        let connected = match pipe::wait_for_client(&file, Duration::from_secs(30), || {
            STOP_REQUESTED.load(Ordering::SeqCst)
        }) {
            Ok(value) => value,
            Err(error) => {
                let _ = ACTIVE_PIPE.compare_exchange(raw, 0, Ordering::SeqCst, Ordering::SeqCst);
                pipe::disconnect(&file);
                return Err(error);
            }
        };
        if !connected {
            let _ = ACTIVE_PIPE.compare_exchange(raw, 0, Ordering::SeqCst, Ordering::SeqCst);
            pipe::disconnect(&file);
            drop(file);
            return Ok(());
        }
        let result = handle_client(file, Arc::clone(&backend));
        let _ = ACTIVE_PIPE.compare_exchange(raw, 0, Ordering::SeqCst, Ordering::SeqCst);
        if let Err(error) = result {
            if !matches!(error.kind, crate::ErrorKind::Pipe) {
                // The caller receives protocol errors where possible; only
                // unexpected server failures are kept in the service log.
                eprintln!("Kite file-index client ended: {error}");
            }
        }
        return Ok(());
    }
}

fn handle_client(file: File, backend: Arc<dyn MetadataBackend>) -> ServiceResult<()> {
    let pipe_handle = HANDLE(file.as_raw_handle() as _);
    let reader_file = file.try_clone().map_err(ServiceError::from)?;
    let writer_file = file.try_clone().map_err(ServiceError::from)?;
    let mut reader = BufReader::new(reader_file);
    let writer = Arc::new(Mutex::new(BufWriter::new(writer_file)));
    let hello = protocol::read_request(&mut reader)?;
    let (pid, parent_pid, session_id, sid) = match hello {
        Request::Hello {
            protocol,
            pid,
            parent_pid,
            session_id,
            sid,
        } if protocol == PROTOCOL_VERSION => (pid, parent_pid, session_id, sid),
        Request::Hello { protocol, .. } => {
            let _ = protocol::send_response(
                &writer,
                Response::Error {
                    error_kind: "protocol".into(),
                    message: format!("不支持的索引服务协议版本 {protocol}"),
                    code: None,
                },
            );
            return Err(ServiceError::protocol("unsupported protocol version"));
        }
        _ => {
            return Err(ServiceError::authentication(
                "首条命名管道消息必须为身份握手",
            ))
        }
    };
    validate_client(pipe_handle, pid, parent_pid, session_id, &sid)?;
    protocol::send_response(
        &writer,
        Response::HelloOk {
            protocol: PROTOCOL_VERSION,
        },
    )?;
    let session = SessionPaths::create(&sid)?;
    let monitor_stop = Arc::new(AtomicBool::new(false));
    let monitor_stop2 = Arc::clone(&monitor_stop);
    let monitor_pipe = pipe_handle.0 as isize;
    thread::spawn(move || {
        let monitor_pipe = HANDLE(monitor_pipe as *mut std::ffi::c_void);
        // The authenticated pipe client is the lifetime owner. Watching its
        // launcher/parent would let the Kite process disappear while the
        // service kept reading as SYSTEM because the launcher remained alive.
        let client = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
            Ok(handle) => handle,
            Err(_) => {
                // A parent that cannot be opened is not a valid lifetime
                // anchor for a privileged session; closing the pipe lets the
                // main loop cancel the backend and clean up immediately.
                if !monitor_stop2.load(Ordering::SeqCst) {
                    pipe::disconnect_from_handle(monitor_pipe);
                }
                return;
            }
        };
        let _client_guard = HandleGuard(client);
        let exited = unsafe { WaitForSingleObject(client, u32::MAX) }.0 == 0;
        if exited && !monitor_stop2.load(Ordering::SeqCst) {
            pipe::disconnect_from_handle(monitor_pipe);
        }
    });
    // A raw JSON checkpoint is accepted only when it was produced by the
    // capture running on this authenticated connection.  Restarted clients
    // must use `watch_path`, which is checked against the protected
    // ProgramData tree below.  This prevents an ordinary user from asking a
    // SYSTEM service to consume an arbitrary AppData checkpoint.
    let checkpoint_provenance: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let mut active_cancel: Option<CancelToken> = None;
    let mut active_done: Option<Arc<AtomicBool>> = None;
    loop {
        if active_done
            .as_ref()
            .is_some_and(|done| done.load(Ordering::SeqCst))
        {
            active_done = None;
            active_cancel = None;
        }
        let request = match protocol::read_request(&mut reader) {
            Ok(value) => value,
            Err(error) if matches!(error.kind, crate::ErrorKind::Pipe) => {
                if let Some(cancel) = active_cancel.take() {
                    cancel.store(true, Ordering::SeqCst);
                }
                break;
            }
            Err(error) => {
                let _ = protocol::send_response(&writer, protocol::error_response(&error));
                break;
            }
        };
        // The worker can finish while this thread is blocked waiting for the
        // next request. Reap that state after the read as well as before it;
        // otherwise the next legal Watch/Capture is rejected as "busy".
        if active_done
            .as_ref()
            .is_some_and(|done| done.load(Ordering::SeqCst))
        {
            active_done = None;
            active_cancel = None;
        }
        match request {
            Request::Capture {
                volumes: requested_volumes,
            } => {
                if active_cancel.is_some() {
                    let _ = protocol::send_response(
                        &writer,
                        protocol::error_response(&ServiceError::invalid_request(
                            "已有索引操作正在运行",
                        )),
                    );
                    continue;
                }
                let volumes = if requested_volumes.is_empty() {
                    match local_ntfs_volumes() {
                        Ok(value) => value,
                        Err(error) => {
                            let _ =
                                protocol::send_response(&writer, protocol::error_response(&error));
                            continue;
                        }
                    }
                } else {
                    requested_volumes
                };
                if let Err(error) = validate_volumes(&volumes) {
                    let _ = protocol::send_response(&writer, protocol::error_response(&error));
                    continue;
                }
                let cancel = Arc::new(AtomicBool::new(false));
                let done = Arc::new(AtomicBool::new(false));
                active_cancel = Some(Arc::clone(&cancel));
                active_done = Some(Arc::clone(&done));
                let root = session.capture_root.clone();
                let writer2 = Arc::clone(&writer);
                let backend2 = Arc::clone(&backend);
                let session2 = session.clone();
                let provenance = Arc::clone(&checkpoint_provenance);
                let sid2 = sid.clone();
                thread::spawn(move || {
                    let progress_writer = Arc::clone(&writer2);
                    let progress = move |event: Value| {
                        let _ =
                            protocol::send_response(&progress_writer, Response::Progress { event });
                    };
                    let result = backend2.capture(&root, &volumes, Arc::clone(&cancel), &progress);
                    let response = match result {
                        Ok(value) => {
                            if !session2.contains(&value.capture_root)
                                || !session2.contains(&value.manifest_path)
                                || !session2.contains(&value.checkpoint_path)
                            {
                                protocol::error_response(&ServiceError::new(
                                    crate::ErrorKind::PathMismatch,
                                    "索引后端返回了受保护目录之外的路径",
                                ))
                            } else {
                                match pipe::harden_metadata_directory(&value.capture_root, &sid2)
                                    .and_then(|()| read_checkpoint_bytes(&value.checkpoint_path))
                                {
                                    Ok(bytes) => {
                                        if let Ok(mut guard) = provenance.lock() {
                                            *guard = Some(bytes);
                                        }
                                        Response::CaptureComplete {
                                            capture_root: value
                                                .capture_root
                                                .to_string_lossy()
                                                .into_owned(),
                                            manifest: value
                                                .manifest_path
                                                .to_string_lossy()
                                                .into_owned(),
                                            checkpoint: value
                                                .checkpoint_path
                                                .to_string_lossy()
                                                .into_owned(),
                                        }
                                    }
                                    Err(error) => protocol::error_response(&error),
                                }
                            }
                        }
                        Err(error) => protocol::error_response(&error),
                    };
                    done.store(true, Ordering::SeqCst);
                    let _ = protocol::send_response(&writer2, response);
                });
            }
            Request::Watch {
                checkpoint_path: source_path,
                checkpoint_json,
            } => {
                if active_cancel.is_some() {
                    let _ = protocol::send_response(
                        &writer,
                        protocol::error_response(&ServiceError::invalid_request(
                            "已有索引操作正在运行",
                        )),
                    );
                    continue;
                }
                let checkpoint_bytes = match (source_path, checkpoint_json) {
                    (Some(path), None) => read_protected_checkpoint(&sid, Path::new(&path)),
                    (None, Some(bytes)) => {
                        let matches_capture = checkpoint_provenance
                            .lock()
                            .ok()
                            .and_then(|guard| guard.as_ref().map(|value| value == &bytes))
                            .unwrap_or(false);
                        if !matches_capture {
                            Err(ServiceError::new(
                                crate::ErrorKind::PathMismatch,
                                "checkpoint 必须来自受保护的索引采集",
                            ))
                        } else {
                            validate_checkpoint_bytes(bytes)
                        }
                    }
                    (Some(_), Some(_)) => Err(ServiceError::invalid_request(
                        "checkpoint 只能指定路径或 JSON",
                    )),
                    (None, None) => Err(ServiceError::invalid_request("缺少 checkpoint")),
                };
                let checkpoint_bytes = match checkpoint_bytes {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let _ = protocol::send_response(&writer, protocol::error_response(&error));
                        continue;
                    }
                };
                let checkpoint = session.checkpoint_path.clone();
                if let Err(error) =
                    fs::write(&checkpoint, checkpoint_bytes).map_err(ServiceError::from)
                {
                    let _ = protocol::send_response(&writer, protocol::error_response(&error));
                    continue;
                }
                let feed = session.feed_path.clone();
                let cancel = Arc::new(AtomicBool::new(false));
                let done = Arc::new(AtomicBool::new(false));
                active_cancel = Some(Arc::clone(&cancel));
                active_done = Some(Arc::clone(&done));
                let writer2 = Arc::clone(&writer);
                let backend2 = Arc::clone(&backend);
                let session2 = session.clone();
                let ready_announced = Arc::new(AtomicBool::new(false));
                let ready_announced2 = Arc::clone(&ready_announced);
                let feed2 = feed.clone();
                let cancel2 = Arc::clone(&cancel);
                let done2 = Arc::clone(&done);
                thread::spawn(move || {
                    let progress_writer = Arc::clone(&writer2);
                    let progress = move |event: Value| {
                        let _ =
                            protocol::send_response(&progress_writer, Response::Progress { event });
                    };
                    let result =
                        backend2.watch(&checkpoint, &feed2, Arc::clone(&cancel2), &progress);
                    let response = match result {
                        Ok(()) if session2.contains(&feed2) => {
                            // Keep terminal success behind WatchReady. This
                            // prevents a very short/cancelled watch from
                            // putting WatchComplete ahead of readiness on the
                            // same pipe.
                            while !ready_announced2.load(Ordering::SeqCst)
                                && !cancel2.load(Ordering::SeqCst)
                                && !STOP_REQUESTED.load(Ordering::SeqCst)
                            {
                                thread::sleep(Duration::from_millis(5));
                            }
                            if ready_announced2.load(Ordering::SeqCst) {
                                Response::WatchComplete
                            } else {
                                protocol::error_response(&ServiceError::cancelled(
                                    "watch 在 feed 就绪前被取消",
                                ))
                            }
                        }
                        Ok(()) => protocol::error_response(&ServiceError::new(
                            crate::ErrorKind::PathMismatch,
                            "索引后端返回了受保护目录之外的路径",
                        )),
                        Err(error) => protocol::error_response(&error),
                    };
                    done2.store(true, Ordering::SeqCst);
                    let _ = protocol::send_response(&writer2, response);
                });
                // The kernel creates and flushes the immutable begin line
                // before a live consumer may open the feed. Do not advertise
                // WatchReady while the path is absent or only partially
                // written; otherwise the normal LiveSearch opener can race
                // the producer and report a false readiness failure.
                let feed_ready = match wait_for_feed_ready(&feed, &cancel, &done) {
                    Ok(value) => value,
                    Err(error) => {
                        cancel.store(true, Ordering::SeqCst);
                        ready_announced.store(true, Ordering::SeqCst);
                        let worker_done = wait_for_done(&done);
                        monitor_stop.store(true, Ordering::SeqCst);
                        pipe::disconnect(&file);
                        if worker_done {
                            log_cleanup_failure(cleanup_session_artifacts(&session));
                        }
                        return Err(error);
                    }
                };
                if feed_ready {
                    if let Err(error) = protocol::send_response(
                        &writer,
                        Response::WatchReady {
                            feed_path: feed.to_string_lossy().into_owned(),
                        },
                    ) {
                        cancel.store(true, Ordering::SeqCst);
                        ready_announced.store(true, Ordering::SeqCst);
                        let worker_done = wait_for_done(&done);
                        monitor_stop.store(true, Ordering::SeqCst);
                        pipe::disconnect(&file);
                        if worker_done {
                            log_cleanup_failure(cleanup_session_artifacts(&session));
                        }
                        return Err(error);
                    }
                }
                ready_announced.store(true, Ordering::SeqCst);
            }
            Request::Cancel => {
                if let Some(cancel) = active_cancel.as_ref() {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
            Request::Ping => {
                if protocol::send_response(&writer, Response::Pong).is_err() {
                    break;
                }
            }
            Request::Hello { .. } => {
                if protocol::send_response(
                    &writer,
                    protocol::error_response(&ServiceError::protocol("重复身份握手")),
                )
                .is_err()
                {
                    break;
                }
            }
        }
        if active_done
            .as_ref()
            .is_some_and(|done| done.load(Ordering::SeqCst))
        {
            active_done = None;
            active_cancel = None;
        }
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            if let Some(cancel) = active_cancel.take() {
                cancel.store(true, Ordering::SeqCst);
            }
            break;
        }
    }
    if let Some(cancel) = active_cancel.take() {
        cancel.store(true, Ordering::SeqCst);
    }
    let worker_done = active_done.as_ref().map(wait_for_done).unwrap_or(true);
    monitor_stop.store(true, Ordering::SeqCst);
    pipe::disconnect(&file);
    if worker_done {
        log_cleanup_failure(cleanup_session_artifacts(&session));
    }
    Ok(())
}

fn wait_for_done(done: &Arc<AtomicBool>) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !done.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    done.load(Ordering::SeqCst)
}

fn log_cleanup_failure(result: ServiceResult<()>) {
    if let Err(error) = result {
        eprintln!("Kite file-index session cleanup skipped: {error}");
    }
}

/// Remove transient artifacts from exactly the session created for this pipe
/// client.  Checkpoints are the sole cross-process input retained by the
/// service; capture frames and stopped feeds must not accumulate after every
/// rebuild or restart.  A preflight rejects reparse points before any delete,
/// so an unexpected junction cannot redirect cleanup outside this session.
fn cleanup_session_artifacts(session: &SessionPaths) -> ServiceResult<()> {
    let metadata = fs::symlink_metadata(&session.root).map_err(ServiceError::from)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !metadata.is_dir() {
        return Err(ServiceError::new(
            crate::ErrorKind::PathMismatch,
            "索引服务 session 不是普通目录，已跳过清理",
        ));
    }
    let name = session
        .root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !name.starts_with("session-") {
        return Err(ServiceError::new(
            crate::ErrorKind::PathMismatch,
            "索引服务 session 路径不符合受保护命名约定",
        ));
    }
    let user_metadata = fs::symlink_metadata(&session.user_root).map_err(ServiceError::from)?;
    if user_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || !user_metadata.is_dir()
    {
        return Err(ServiceError::new(
            crate::ErrorKind::PathMismatch,
            "索引服务用户根不是受保护的普通目录，已跳过清理",
        ));
    }
    let canonical_user_root = session
        .user_root
        .canonicalize()
        .map_err(ServiceError::from)?;
    let canonical_root = session.root.canonicalize().map_err(ServiceError::from)?;
    if canonical_root.parent() != Some(canonical_user_root.as_path()) {
        return Err(ServiceError::new(
            crate::ErrorKind::PathMismatch,
            "索引服务 session 不在当前用户受保护目录的直接子目录内",
        ));
    }
    validate_cleanup_tree(&canonical_root, &canonical_root)?;
    prune_cleanup_children(&canonical_root)
}

fn validate_cleanup_tree(path: &Path, root: &Path) -> ServiceResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(ServiceError::from)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(ServiceError::new(
            crate::ErrorKind::PathMismatch,
            format!("session 内含 reparse 路径，已跳过清理: {}", path.display()),
        ));
    }
    let canonical = path.canonicalize().map_err(ServiceError::from)?;
    if !canonical.starts_with(root) {
        return Err(ServiceError::new(
            crate::ErrorKind::PathMismatch,
            format!("session 清理路径越界: {}", path.display()),
        ));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(ServiceError::from)? {
            validate_cleanup_tree(&entry.map_err(ServiceError::from)?.path(), root)?;
        }
    }
    Ok(())
}

fn prune_cleanup_children(root: &Path) -> ServiceResult<()> {
    for entry in fs::read_dir(root).map_err(ServiceError::from)? {
        let path = entry.map_err(ServiceError::from)?.path();
        let _ = prune_cleanup_entry(&path)?;
    }
    Ok(())
}

/// Return whether an entry was retained.  Checkpoints are retained wherever
/// they occur under this service-created session.  Small capture manifests
/// and capture errors are useful diagnostics; all frame/layout/feed/report
/// files are removed.
fn prune_cleanup_entry(path: &Path) -> ServiceResult<bool> {
    let metadata = fs::symlink_metadata(path).map_err(ServiceError::from)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(ServiceError::new(
            crate::ErrorKind::PathMismatch,
            format!("session 内含 reparse 路径，已停止清理: {}", path.display()),
        ));
    }
    if metadata.is_dir() {
        let mut retain_directory = false;
        for entry in fs::read_dir(path).map_err(ServiceError::from)? {
            if prune_cleanup_entry(&entry.map_err(ServiceError::from)?.path())? {
                retain_directory = true;
            }
        }
        if retain_directory {
            return Ok(true);
        }
        fs::remove_dir(path).map_err(ServiceError::from)?;
        return Ok(false);
    }

    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let keep_checkpoint = name.eq_ignore_ascii_case("journal-start.json");
    let keep_diagnostic = metadata.len() <= MAX_DIAGNOSTIC_BYTES
        && (name.eq_ignore_ascii_case("capture.json")
            || name.eq_ignore_ascii_case("capture-error.json"));
    if keep_checkpoint || keep_diagnostic {
        Ok(true)
    } else {
        fs::remove_file(path).map_err(ServiceError::from)?;
        Ok(false)
    }
}

fn wait_for_feed_ready(
    feed: &Path,
    cancel: &CancelToken,
    done: &Arc<AtomicBool>,
) -> ServiceResult<bool> {
    loop {
        if STOP_REQUESTED.load(Ordering::SeqCst) || cancel.load(Ordering::SeqCst) {
            cancel.store(true, Ordering::SeqCst);
            return Ok(false);
        }
        if done.load(Ordering::SeqCst) {
            return Ok(false);
        }
        if let Ok(file) = File::open(feed) {
            let mut reader = BufReader::new(file);
            let mut line = Vec::new();
            let bytes = reader
                .read_until(b'\n', &mut line)
                .map_err(ServiceError::from)?;
            if line.len() > protocol::MAX_LINE_BYTES {
                return Err(ServiceError::protocol(
                    "live feed begin exceeds protocol size limit",
                ));
            }
            if bytes != 0 && line.ends_with(b"\n") {
                let value: Value = serde_json::from_slice(&line).map_err(|error| {
                    ServiceError::protocol(format!("live feed begin JSON 无效: {error}"))
                })?;
                let complete = value.is_object()
                    && value.get("kind").and_then(Value::as_str) == Some("begin")
                    && value.get("scope").and_then(Value::as_str).is_some()
                    && value.get("generation").and_then(Value::as_str).is_some()
                    && value.get("volumes").and_then(Value::as_array).is_some();
                if complete {
                    return Ok(true);
                }
                return Err(ServiceError::protocol("live feed begin 缺少完整身份字段"));
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn validate_checkpoint_bytes(bytes: Vec<u8>) -> ServiceResult<Vec<u8>> {
    if bytes.is_empty() || bytes.len() > protocol::MAX_CHECKPOINT_BYTES {
        return Err(ServiceError::invalid_request(
            "checkpoint JSON 超出大小限制",
        ));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| ServiceError::invalid_request(format!("checkpoint JSON 无效: {error}")))?;
    if !value.is_object() {
        return Err(ServiceError::invalid_request("checkpoint 必须是 JSON 对象"));
    }
    Ok(bytes)
}

fn read_checkpoint_bytes(path: &Path) -> ServiceResult<Vec<u8>> {
    let metadata = fs::metadata(path).map_err(ServiceError::from)?;
    if !metadata.is_file() || metadata.len() > protocol::MAX_CHECKPOINT_BYTES as u64 {
        return Err(ServiceError::invalid_request(
            "checkpoint 文件类型或大小无效",
        ));
    }
    validate_checkpoint_bytes(fs::read(path).map_err(ServiceError::from)?)
}

fn read_protected_checkpoint(sid: &str, path: &Path) -> ServiceResult<Vec<u8>> {
    let requested_root = program_data_root(sid)?;
    let program_data = requested_root
        .ancestors()
        .nth(3)
        .ok_or_else(|| ServiceError::internal("无法确定 ProgramData 根目录"))?;
    let allowed = pipe::ensure_metadata_tree(program_data, sid)?
        .canonicalize()
        .map_err(ServiceError::from)?;
    let actual = path.canonicalize().map_err(ServiceError::from)?;
    let filename_ok = actual
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("journal-start.json"));
    if !actual.starts_with(&allowed) || !filename_ok {
        return Err(ServiceError::new(
            crate::ErrorKind::PathMismatch,
            "checkpoint 不在 Kite 受保护目录内",
        ));
    }
    read_checkpoint_bytes(&actual)
}

#[derive(Clone)]
struct SessionPaths {
    user_root: PathBuf,
    root: PathBuf,
    capture_root: PathBuf,
    checkpoint_path: PathBuf,
    feed_path: PathBuf,
}
impl SessionPaths {
    fn create(sid: &str) -> ServiceResult<Self> {
        let base = program_data_root(sid)?;
        // Build and lock ProgramData\\Kite\\FileIndex\\<SID> component by
        // component. This rejects pre-existing reparse points and untrusted
        // owners before the service creates a session beneath the tree.
        let program_data = base
            .ancestors()
            .nth(3)
            .ok_or_else(|| ServiceError::internal("无法确定 ProgramData 根目录"))?;
        let base = pipe::ensure_metadata_tree(program_data, sid)?;
        let id = format!(
            "session-{}-{}",
            unsafe { GetCurrentProcessId() },
            unique_suffix()
        );
        let root = base.join(id);
        fs::create_dir(&root).map_err(ServiceError::from)?;
        pipe::harden_metadata_directory(&root, sid)?;
        let capture_root = root.join("capture");
        let checkpoint_path = root.join("journal-start.json");
        let feed_path = root.join("journal-feed.jsonl");
        Ok(Self {
            user_root: base,
            root,
            capture_root,
            checkpoint_path,
            feed_path,
        })
    }
    fn contains(&self, path: &Path) -> bool {
        path.canonicalize().ok().is_some_and(|path| {
            self.root
                .canonicalize()
                .ok()
                .is_some_and(|root| path.starts_with(root))
        })
    }
}

fn program_data_root(sid: &str) -> ServiceResult<PathBuf> {
    if sid.is_empty()
        || sid.len() > 184
        || !sid.starts_with("S-")
        || sid
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '-'))
    {
        return Err(ServiceError::authentication("无效的用户 SID"));
    }
    let root = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None) }
        .map_err(ServiceError::from)?;
    let value =
        unsafe { root.to_string() }.map_err(|error| ServiceError::internal(error.to_string()));
    unsafe {
        CoTaskMemFree(Some(root.0 as _));
    }
    Ok(PathBuf::from(value?)
        .join("Kite")
        .join("FileIndex")
        .join(sid))
}

fn validate_volumes(volumes: &[String]) -> ServiceResult<()> {
    if volumes.len() > protocol::MAX_VOLUMES {
        return Err(ServiceError::invalid_request("卷列表超过数量限制"));
    }
    for volume in volumes {
        let bytes = volume.as_bytes();
        if bytes.len() != 2 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
            return Err(ServiceError::invalid_request("卷必须是类似 C: 的盘符"));
        }
    }
    Ok(())
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default()
}

pub(crate) fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

// Keep the imports visible for downstream diagnostics without exposing raw
// HANDLEs in the public client API.
use std::os::windows::io::AsRawHandle;
