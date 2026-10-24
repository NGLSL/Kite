//! Ordinary-permission controller for the opt-in native file-search capability.
//!
//! One worker owns the live index. Only the metadata connection crosses the
//! service boundary; the UI never performs volume I/O or waits for a build.

mod build;
mod diagnostics;

use std::fmt;

/// File-search categories shared by the UI and the native index.
///
/// This type belongs to the built-in file-search feature rather than to an
/// operating-system integration or a third-party backend.  The native index
/// applies the extension rules in its own query layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileFilter {
    #[default]
    All,
    Images,
    Documents,
    Videos,
    Audio,
    Archives,
    Folders,
}

impl FileFilter {
    pub const ALL: [Self; 7] = [
        Self::All,
        Self::Images,
        Self::Documents,
        Self::Videos,
        Self::Audio,
        Self::Archives,
        Self::Folders,
    ];
}

impl fmt::Display for FileFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::All => "全部",
            Self::Images => "图片",
            Self::Documents => "文档",
            Self::Videos => "视频",
            Self::Audio => "音频",
            Self::Archives => "压缩包",
            Self::Folders => "文件夹",
        })
    }
}

/// Dispatch the ordinary-permission, short-lived builder before claiming a UI instance.
pub fn dispatch_entry() -> bool {
    diagnostics::dispatch_entry() || build::dispatch_entry()
}

use kite_file_index::{Generation, LiveSearch, Query};
use kite_index_service::{CancelHandle, Client, ErrorKind, ServiceError, ServiceState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Stopped,
    Starting,
    Indexing,
    CatchingUp,
    Recovering,
    RetryWaiting,
    Ready,
    NeedsAuthorization,
    NeedsRebuild,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub phase: Phase,
    pub detail: String,
    pub entries: u64,
    /// Changes only when visible state, configuration, or actual metadata changes.
    pub revision: u64,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub path: String,
    pub name: String,
    pub is_folder: bool,
    /// Present only when a filename cannot be represented losslessly as UTF-8.
    /// Display uses `path`; execution must use these original Windows code units.
    pub raw_path: Option<Vec<u16>>,
}

struct SearchRequest {
    query: String,
    limit: usize,
    filter: FileFilter,
    cancel: kite_file_index::CancelToken,
    response: mpsc::SyncSender<Result<Vec<Hit>, String>>,
}

struct Running {
    cancel: Arc<AtomicBool>,
    attempt_cancel: Arc<Mutex<Option<kite_file_index::CancelToken>>>,
    retry_now: Arc<AtomicBool>,
    service: Arc<Mutex<Option<CancelHandle>>>,
    requests: mpsc::Sender<SearchRequest>,
    done: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
}

struct Shared {
    state: Mutex<Snapshot>,
    exclusions: RwLock<Vec<String>>,
    active: AtomicU64,
    running: Mutex<Option<Running>>,
    query_cancel: Mutex<Option<kite_file_index::CancelToken>>,
    diagnostics: Mutex<Value>,
    pending_candidate: Mutex<Option<OwnedCandidate>>,
}

pub struct Controller {
    root: PathBuf,
    shared: Arc<Shared>,
}

#[derive(Serialize, Deserialize)]
struct CurrentGeneration {
    generation: String,
    /// Service-owned immutable begin retained across application sessions.
    protected_checkpoint: PathBuf,
}

impl Controller {
    pub fn new(data_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            root: data_dir.join("file-search"),
            shared: Arc::new(Shared {
                state: Mutex::new(Snapshot {
                    phase: Phase::Stopped,
                    detail: "文件搜索已关闭".into(),
                    entries: 0,
                    revision: 0,
                }),
                exclusions: RwLock::new(Vec::new()),
                active: AtomicU64::new(0),
                running: Mutex::new(None),
                query_cancel: Mutex::new(None),
                diagnostics: Mutex::new(serde_json::json!({})),
                pending_candidate: Mutex::new(None),
            }),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_exclusions(&self, exclusions: Vec<String>) {
        let mut current = self
            .shared
            .exclusions
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if *current != exclusions {
            *current = exclusions;
            self.shared
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .revision += 1;
        }
    }

    pub fn start(self: &Arc<Self>, allow_install: bool) {
        self.launch(false, allow_install);
    }

    pub fn rebuild(self: &Arc<Self>, allow_install: bool) {
        self.launch(true, allow_install);
    }

    fn launch(self: &Arc<Self>, rebuild: bool, allow_install: bool) {
        let mut slot = self
            .shared
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !rebuild {
            let state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(run) = slot.as_ref().filter(|run| {
                !run.cancel.load(Ordering::Acquire) && !run.done.load(Ordering::Acquire)
            }) {
                if state.phase == Phase::RetryWaiting {
                    run.retry_now.store(true, Ordering::Release);
                    run.thread.thread().unpark();
                }
                return;
            }
        }
        self.cancel_query();
        let previous = slot.take();
        if let Some(previous) = &previous {
            cancel_running(previous);
        }
        let id = self.shared.active.fetch_add(1, Ordering::AcqRel) + 1;
        self.shared.set_diagnostics(id, serde_json::json!({}));
        let cancel = Arc::new(AtomicBool::new(false));
        let attempt_cancel = Arc::new(Mutex::new(None));
        let retry_now = Arc::new(AtomicBool::new(false));
        let retry_signal = Arc::clone(&retry_now);
        let attempt_slot = Arc::clone(&attempt_cancel);
        let service = Arc::new(Mutex::new(None));
        let done = Arc::new(AtomicBool::new(false));
        let (requests, receiver) = mpsc::channel();
        self.shared
            .update(id, Phase::Starting, "正在连接文件索引组件", Some(0));
        let shared = Arc::clone(&self.shared);
        let root = self.root.clone();
        let token = Arc::clone(&cancel);
        let service_handle = Arc::clone(&service);
        let finished = Arc::clone(&done);
        let worker = thread::spawn(move || {
            // A quick off/on or rebuild cannot create overlapping metadata sessions.
            if let Some(previous) = previous {
                let _ = previous.thread.join();
            }
            if !token.load(Ordering::Acquire) {
                let result = run(
                    &shared,
                    id,
                    &root,
                    rebuild,
                    allow_install,
                    Arc::clone(&token),
                    &service_handle,
                    &attempt_slot,
                    &retry_signal,
                    receiver,
                );
                if let Err((phase, error)) = result {
                    if !token.load(Ordering::Acquire) {
                        crate::log::info(&format!("file index {phase:?}: {error}"));
                        shared.update(id, phase, &error, None);
                    }
                }
            }
            if let Some(handle) = service_handle
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
            {
                let _ = handle.cancel();
            }
            finished.store(true, Ordering::Release);
        });
        *slot = Some(Running {
            cancel,
            attempt_cancel,
            retry_now,
            service,
            requests,
            done,
            thread: worker,
        });
    }

    /// Nonblocking cancellation; the service also cancels on process death.
    pub fn stop(&self) {
        self.cancel_query();
        let id = self.shared.active.fetch_add(1, Ordering::AcqRel) + 1;
        if let Some(run) = self
            .shared
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            cancel_running(run);
        }
        self.shared
            .update(id, Phase::Stopped, "文件搜索已关闭", None);
    }

    /// Run from the existing asynchronous file-query path, never an Iced update.
    pub fn query(&self, query: &str, limit: usize, filter: FileFilter) -> Result<Vec<Hit>, String> {
        self.query_with_cancel(query, limit, filter, self.begin_query())
    }

    /// Capture on the UI thread, before spawning: an older task cannot cancel
    /// a newer query merely because the scheduler starts it later.
    pub fn begin_query(&self) -> kite_file_index::CancelToken {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut current = self
            .shared
            .query_cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = current.replace(Arc::clone(&cancel)) {
            previous.store(true, Ordering::Release);
        }
        cancel
    }

    pub fn cancel_query(&self) {
        if let Some(cancel) = self
            .shared
            .query_cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            cancel.store(true, Ordering::Release);
        }
    }

    pub fn query_with_cancel(
        &self,
        query: &str,
        limit: usize,
        filter: FileFilter,
        cancel: kite_file_index::CancelToken,
    ) -> Result<Vec<Hit>, String> {
        if cancel.load(Ordering::Acquire) {
            return Ok(Vec::new());
        }
        if self.snapshot().phase != Phase::Ready {
            return Err(self.snapshot().detail);
        }
        let (response, receiver) = mpsc::sync_channel(1);
        let requests = self
            .shared
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|run| run.requests.clone())
            .ok_or("文件搜索尚未启动")?;
        requests
            .send(SearchRequest {
                query: query.to_owned(),
                limit,
                filter,
                cancel: Arc::clone(&cancel),
                response,
            })
            .map_err(|_| "文件搜索已停止".to_string())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(15))
            .map_err(|error| -> String {
                match error {
                    mpsc::RecvTimeoutError::Disconnected => "查询已更新".into(),
                    mpsc::RecvTimeoutError::Timeout => "文件查询耗时较长，请重试".into(),
                }
            });
        if cancel.load(Ordering::Acquire) {
            Ok(Vec::new())
        } else {
            result?
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop();
    }
}

fn cancel_running(run: &Running) {
    run.cancel.store(true, Ordering::Release);
    run.thread.thread().unpark();
    if let Some(cancel) = run
        .attempt_cancel
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        cancel.store(true, Ordering::Release);
    }
    if let Some(handle) = run
        .service
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        // Pipe writes are a small cancel message, not a wait for capture completion.
        let _ = handle.cancel();
    }
}

impl Shared {
    fn update(&self, id: u64, phase: Phase, detail: &str, entries: Option<u64>) {
        if self.active.load(Ordering::Acquire) != id {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.active.load(Ordering::Acquire) != id {
            return;
        }
        let entries = entries.unwrap_or(state.entries);
        if state.phase != phase || state.detail != detail || state.entries != entries {
            state.phase = phase;
            state.detail = detail.into();
            state.entries = entries;
            state.revision += 1;
            #[cfg(feature = "isolated-profile")]
            crate::log::info(&format!(
                "file-index phase={phase:?} unix_us={} entries={entries}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_micros())
                    .unwrap_or(0)
            ));
        }
    }

    fn set_diagnostics(&self, id: u64, value: Value) {
        if self.active.load(Ordering::Acquire) != id {
            return;
        }
        let mut current = self
            .diagnostics
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.active.load(Ordering::Acquire) == id {
            *current = value;
        }
    }

    fn progress(&self, id: u64, event: Value, recovering: bool) {
        let volume = event.get("volume").and_then(Value::as_str).unwrap_or("");
        let (phase, detail) = match event.get("stage").and_then(Value::as_str) {
            Some("capturing" | "capturing-fast-records") => {
                (Phase::Indexing, format!("正在读取 {volume} 盘的文件名"))
            }
            Some("captured") => (Phase::Indexing, "文件名读取完成，准备构建索引".into()),
            Some("projecting") => (
                Phase::Indexing,
                format!("正在整理 {volume} 盘的目录与硬链接"),
            ),
            Some("building") => (Phase::Indexing, "正在构建压缩索引".into()),
            Some("built") => (Phase::CatchingUp, "正在同步建库期间的文件变化".into()),
            _ => return,
        };
        let (phase, detail) = if recovering {
            (Phase::Recovering, format!("正在恢复文件索引：{detail}"))
        } else {
            (phase, detail)
        };
        self.update(
            id,
            phase,
            &detail,
            event.get("entries").and_then(Value::as_u64),
        );
    }
}

type RunFailure = (Phase, String);
type RunResult<T> = Result<T, RunFailure>;

fn service_error(error: ServiceError) -> (Phase, String) {
    let phase = match error.kind {
        ErrorKind::NeedsInstall
        | ErrorKind::NotInstalled
        | ErrorKind::PathMismatch
        | ErrorKind::AccessDenied => Phase::NeedsAuthorization,
        ErrorKind::Cancelled => Phase::Error,
        ErrorKind::NeedsRebuild => Phase::NeedsRebuild,
        _ => Phase::Error,
    };
    (phase, error.to_string())
}

fn run(
    shared: &Arc<Shared>,
    id: u64,
    root: &Path,
    rebuild: bool,
    allow_install: bool,
    cancel: Arc<AtomicBool>,
    service_handle: &Arc<Mutex<Option<CancelHandle>>>,
    attempt_slot: &Arc<Mutex<Option<kite_file_index::CancelToken>>>,
    retry_now: &AtomicBool,
    requests: mpsc::Receiver<SearchRequest>,
) -> RunResult<()> {
    let _cleanup = CandidateCleanup { shared, root };
    let mut force_rebuild = rebuild;
    let mut recovery_reason = None::<String>;
    let mut initial_attempt = true;
    let mut backoff_step = 0usize;
    let mut retry_attempt = 0u64;
    loop {
        check_cancel(&cancel)?;
        {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if shared.active.load(Ordering::Acquire) == id && !initial_attempt {
                state.phase = if recovery_reason.is_some() {
                    Phase::Recovering
                } else {
                    Phase::Starting
                };
                state.detail = if recovery_reason.is_some() {
                    "正在恢复文件索引：正在重新建立索引"
                } else {
                    "正在重新连接文件索引"
                }
                .into();
                state.revision += 1;
            }
            retry_now.store(false, Ordering::Release);
        }
        if !initial_attempt {
            let mut diagnostic = shared
                .diagnostics
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            diagnostic["retry_remaining_secs"] = Value::from(0);
            shared.set_diagnostics(id, diagnostic);
        }
        let attempt_cancel = Arc::new(AtomicBool::new(false));
        {
            let mut slot = attempt_slot
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *slot = Some(attempt_cancel.clone());
            // stop() may have run between the initial check and token install.
            if cancel.load(Ordering::Acquire) {
                attempt_cancel.store(true, Ordering::Release);
            }
        }
        let mut reached_ready = false;
        let outcome = match cleanup_pending_candidate(shared, root) {
            Err(error) => Err(error),
            Ok(()) => run_attempt(
                shared,
                id,
                root,
                force_rebuild,
                allow_install && initial_attempt,
                Arc::clone(&cancel),
                attempt_cancel.clone(),
                service_handle,
                &requests,
                recovery_reason.as_deref(),
                &mut reached_ready,
            ),
        };
        initial_attempt = false;
        // All engine/watch/session cleanup finishes before scheduling another
        // attempt. A waiting operation owns no live metadata session.
        attempt_cancel.store(true, Ordering::Release);
        if let Some(handle) = service_handle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = handle.cancel();
        }
        attempt_slot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let outcome = match cleanup_pending_candidate(shared, root) {
            Ok(()) => outcome,
            Err(error) => Err(error),
        };
        check_cancel(&cancel)?;
        let failed_rebuild = force_rebuild && !reached_ready;
        if reached_ready {
            backoff_step = 0;
            retry_attempt = 0;
            force_rebuild = false;
            recovery_reason = None;
        }
        let (mut phase, reason) = match outcome {
            Ok(()) => return Ok(()),
            Err(failure) => failure,
        };
        if phase == Phase::NeedsAuthorization {
            return Err((phase, reason));
        }
        // Global cancellation was checked above. An unexpected Interrupted
        // from a session is a reconnectable failure while the feature is on.
        if phase == Phase::Stopped {
            phase = Phase::Error;
        }
        if phase == Phase::NeedsRebuild && !failed_rebuild {
            // A newly confirmed journal/base gap needs a fresh generation now.
            // A failed rebuild, by contrast, must back off before scanning again.
            force_rebuild = true;
            recovery_reason = Some(reason.clone());
            crate::log::info(&format!("file index automatic recovery: {reason}"));
            let mut diagnostic = shared
                .diagnostics
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            diagnostic["automatic_rebuild"] = Value::Bool(true);
            diagnostic["recovery_reason"] = Value::String(reason);
            diagnostic["retry_remaining_secs"] = Value::from(0);
            shared.set_diagnostics(id, diagnostic);
            shared.update(
                id,
                Phase::Recovering,
                "正在恢复文件索引：正在重新建立索引",
                None,
            );
            drain_retry_requests(&requests);
            continue;
        }
        if phase == Phase::NeedsRebuild {
            force_rebuild = true;
        }
        if force_rebuild {
            recovery_reason = Some(reason.clone());
        }
        const RETRY_DELAYS: [u64; 4] = [30, 60, 120, 300];
        let delay = RETRY_DELAYS[backoff_step];
        backoff_step = backoff_step.saturating_add(1).min(RETRY_DELAYS.len() - 1);
        retry_attempt = retry_attempt.saturating_add(1);
        crate::log::info(&format!(
            "file index retry {retry_attempt} in {delay}s: {reason}"
        ));
        wait_for_retry(
            shared,
            id,
            &cancel,
            &requests,
            &reason,
            retry_attempt,
            delay,
            retry_now,
        )?;
    }
}

fn drain_retry_requests(requests: &mpsc::Receiver<SearchRequest>) {
    while let Ok(request) = requests.try_recv() {
        let _ = request
            .response
            .send(Err("文件索引暂时不可用，请稍后重试".into()));
    }
}

fn wait_for_retry(
    shared: &Shared,
    id: u64,
    cancel: &AtomicBool,
    requests: &mpsc::Receiver<SearchRequest>,
    reason: &str,
    attempt: u64,
    delay: u64,
    retry_now: &AtomicBool,
) -> RunResult<()> {
    let deadline = Instant::now() + Duration::from_secs(delay);
    loop {
        check_cancel(cancel)?;
        drain_retry_requests(requests);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || retry_now.load(Ordering::Acquire) {
            return Ok(());
        }
        let seconds = remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0);
        let mut diagnostic = shared
            .diagnostics
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        diagnostic["retry_attempt"] = Value::from(attempt);
        diagnostic["retry_delay_secs"] = Value::from(delay);
        diagnostic["retry_remaining_secs"] = Value::from(seconds);
        diagnostic["last_error"] = Value::String(reason.to_owned());
        shared.set_diagnostics(id, diagnostic);
        shared.update(
            id,
            Phase::RetryWaiting,
            &format!("文件索引暂时不可用，{seconds} 秒后自动重试：{reason}"),
            None,
        );
        // park uses monotonic time, and stop()/manual retry unparks this worker.
        thread::park_timeout(remaining.min(Duration::from_secs(1)));
    }
}

fn run_attempt(
    shared: &Arc<Shared>,
    id: u64,
    root: &Path,
    rebuild: bool,
    allow_install: bool,
    user_cancel: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    service_handle: &Arc<Mutex<Option<CancelHandle>>>,
    requests: &mpsc::Receiver<SearchRequest>,
    recovery_reason: Option<&str>,
    reached_ready: &mut bool,
) -> RunResult<()> {
    check_cancel(&cancel)?;
    let recovering = recovery_reason.is_some();
    let state = kite_index_service::current_state().map_err(service_error)?;
    if matches!(state, ServiceState::Missing | ServiceState::PathMismatch) {
        if !allow_install {
            return Err((
                Phase::NeedsAuthorization,
                "首次使用需要授权安装文件索引组件，点击继续".into(),
            ));
        }
        shared.update(
            id,
            if recovering {
                Phase::Recovering
            } else {
                Phase::Starting
            },
            if recovering {
                "正在恢复文件索引：请确认安装索引组件"
            } else {
                "请在 Windows 授权窗口中确认安装索引组件"
            },
            None,
        );
        kite_index_service::request_install().map_err(installation_error)?;
    }
    check_cancel(&cancel)?;
    let mut client = match Client::connect_current(false) {
        Err(error) if allow_install && error.kind == ErrorKind::NeedsInstall => {
            // A registered component may still speak an older feed protocol.
            // Only the user's explicit authorization may replace that image.
            shared.update(
                id,
                if recovering {
                    Phase::Recovering
                } else {
                    Phase::Starting
                },
                if recovering {
                    "正在恢复文件索引：请确认更新索引组件"
                } else {
                    "请在 Windows 授权窗口中确认更新索引组件"
                },
                None,
            );
            kite_index_service::request_install().map_err(installation_error)?;
            check_cancel(&cancel)?;
            Client::connect_current(false).map_err(service_error)?
        }
        result => result.map_err(service_error)?,
    };
    *service_handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(client.cancel_handle());
    let acknowledge_handle = client.cancel_handle();
    check_cancel(&cancel)?;
    let progress_shared = Arc::clone(shared);
    let progress = move |event: Value| progress_shared.progress(id, event, recovering);
    let existing = if rebuild { None } else { load_current(root)? };
    let (generation, mut watch, new_generation, protected_checkpoint, loaded_resume_cursors) =
        if let Some((generation, checkpoint)) = existing {
            shared.update(
                id,
                Phase::CatchingUp,
                "正在补齐退出期间的文件变化",
                Some(generation.entries),
            );
            let resume = LiveSearch::resume_cursors(&generation).map_err(kernel_error)?;
            let loaded_resume_cursors = resume.clone().unwrap_or(Value::Null);
            let watch = client
                .watch_path_from(&checkpoint, resume, None)
                .map_err(checkpoint_error)?;
            (generation, watch, false, checkpoint, loaded_resume_cursors)
        } else {
            shared.update(
                id,
                if recovering {
                    Phase::Recovering
                } else {
                    Phase::Indexing
                },
                if recovering {
                    "正在恢复文件索引：正在读取文件名"
                } else {
                    "正在读取本机 NTFS 磁盘的文件名"
                },
                None,
            );
            let capture = client
                .capture(&[], Some(&progress))
                .map_err(service_error)?;
            check_cancel(&cancel)?;
            // Begin tracking before ordinary-rights projection/build to cover that gap.
            let checkpoint = capture.checkpoint_path;
            let watch = client
                .watch_path(&checkpoint, None)
                .map_err(checkpoint_error)?;
            let candidate = OwnedCandidate::claim(root)?;
            let destination = candidate.path.clone();
            *shared
                .pending_candidate
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(candidate);
            let generation = build::run(
                &capture.capture_root,
                &destination,
                Arc::clone(&cancel),
                &progress,
            )
            .map_err(|error| (Phase::Error, error.to_string()))?;
            (generation, watch, true, checkpoint, Value::Null)
        };
    check_cancel(&cancel)?;
    let feed = watch.feed_path().to_path_buf();
    let watch_error = Arc::new(Mutex::new(None::<RunFailure>));
    let error_slot = Arc::clone(&watch_error);
    let watch_cancel = Arc::clone(&cancel);
    let watch_user_cancel = Arc::clone(&user_cancel);
    let watch_worker = thread::spawn(move || {
        loop {
            if watch_cancel.load(Ordering::Acquire) || watch_user_cancel.load(Ordering::Acquire) {
                break;
            }
            let failure = match watch.next(None) {
                Ok(true) => continue,
                Ok(false) => (Phase::Error, "后台读取已停止".into()),
                Err(error) => service_error(error),
            };
            if !watch_cancel.load(Ordering::Acquire) && !watch_user_cancel.load(Ordering::Acquire) {
                error_slot
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .get_or_insert(failure);
                // Interrupt this attempt's bootstrap/build without cancelling
                // the user's enabled operation or its background retry scheduler.
                watch_cancel.store(true, Ordering::Release);
            }
            break;
        }
    });
    let outcome = (|| {
        shared.update(
            id,
            if recovering {
                Phase::Recovering
            } else {
                Phase::CatchingUp
            },
            if recovering {
                "正在恢复文件索引：正在同步最新变化"
            } else {
                "正在同步最新变化"
            },
            Some(generation.entries),
        );
        let acknowledge = Arc::new(move |seq| {
            let _ = acknowledge_handle.acknowledge(seq);
        });
        let mut engine = LiveSearch::open_with_resume_and_ack(
            &generation,
            &feed,
            Arc::clone(&cancel),
            acknowledge,
        )
        .map_err(|error| watch_failure(&watch_error).unwrap_or_else(|| kernel_error(error)))?;
        // The watcher cause is authoritative when its cancellation interrupts
        // LiveSearch::open; do not replace it with an Interrupted fallback.
        live_health(&engine.status(), &watch_error)?;
        check_cancel(&cancel)?;
        if new_generation {
            publish_current(
                root,
                &generation,
                &protected_checkpoint,
                &shared.pending_candidate,
            )?;
        }
        *reached_ready = true;
        shared.set_diagnostics(
            id,
            serde_json::json!({
                "generation_root":generation.root.to_string_lossy(),
                "protected_checkpoint":protected_checkpoint.to_string_lossy(),
                "feed_path":feed.to_string_lossy(),
                "loaded_resume_cursors":loaded_resume_cursors,
                "automatic_rebuild":recovering,
                "recovery_reason":recovery_reason,
                "retry_attempt":0,"retry_delay_secs":0,"retry_remaining_secs":0,"last_error":null,
                "live":engine.status(),
            }),
        );
        shared.update(
            id,
            Phase::Ready,
            "文件索引已就绪，正在实时同步",
            Some(generation.entries),
        );
        let mut changes = engine.status()["changes"].as_u64().unwrap_or(0);
        while !cancel.load(Ordering::Acquire) {
            let state = engine.status();
            shared.set_diagnostics(
                id,
                serde_json::json!({
                    "generation_root":generation.root.to_string_lossy(),
                    "protected_checkpoint":protected_checkpoint.to_string_lossy(),
                    "feed_path":feed.to_string_lossy(),
                    "loaded_resume_cursors":loaded_resume_cursors,
                    "automatic_rebuild":recovering,
                    "recovery_reason":recovery_reason,
                    "retry_attempt":0,"retry_delay_secs":0,"retry_remaining_secs":0,"last_error":null,
                    "live":state,
                }),
            );
            live_health(&state, &watch_error)?;
            let current_changes = state["changes"].as_u64().unwrap_or(0);
            if current_changes != changes {
                changes = current_changes;
                if shared.active.load(Ordering::Acquire) == id {
                    shared
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .revision += 1;
                }
            }
            match requests.recv_timeout(Duration::from_millis(300)) {
                Ok(mut request) => {
                    // Obsolete keystrokes must not queue behind expensive queries.
                    while let Ok(newer) = requests.try_recv() {
                        if !newer.cancel.load(Ordering::Acquire) {
                            request = newer;
                        }
                    }
                    if request.cancel.load(Ordering::Acquire) {
                        continue;
                    }
                    let exclusions = shared
                        .exclusions
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .iter()
                        .map(PathBuf::from)
                        .collect();
                    let query = Query {
                        query: request.query,
                        limit: request.limit,
                        filter: native_filter(request.filter),
                        excluded_folders: exclusions,
                    };
                    let result = engine
                        .query_limited_with_cancel(&query, Arc::clone(&request.cancel))
                        .map_err(|error| error.to_string())
                        .and_then(decode_hits);
                    let _ = request.response.send(result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if let Some(failure) = watch_failure(&watch_error) {
            return Err(failure);
        }
        check_cancel(&cancel)?;
        Ok(())
    })();
    // Every attempt releases its engine/client and joins its watcher before
    // the outer loop may establish the replacement metadata session.
    cancel.store(true, Ordering::Release);
    if let Some(handle) = service_handle
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    {
        let _ = handle.cancel();
    }
    let _ = watch_worker.join();
    outcome
}

fn watch_failure(watch_error: &Mutex<Option<RunFailure>>) -> Option<RunFailure> {
    watch_error
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

fn live_health(state: &Value, watch_error: &Mutex<Option<RunFailure>>) -> RunResult<()> {
    if let Some(failure) = watch_failure(watch_error) {
        return Err(failure);
    }
    if state["rebuild_required"] == true || state["error_code"] == "NeedsRebuild" {
        let detail = state["error"].as_str().unwrap_or("变更日志已失效");
        return Err((Phase::NeedsRebuild, format!("索引需要重建：{detail}")));
    }
    if state["stale"] == true {
        return Err((
            Phase::Error,
            state["error"]
                .as_str()
                .unwrap_or("文件同步已停止")
                .to_owned(),
        ));
    }
    if state["journal_stopped"] == true {
        return Err((Phase::Error, "后台同步已停止".into()));
    }
    Ok(())
}

fn kernel_error(error: Box<dyn std::error::Error + Send + Sync>) -> RunFailure {
    let phase = if error
        .downcast_ref::<kite_file_index::NeedsRebuild>()
        .is_some()
    {
        Phase::NeedsRebuild
    } else {
        Phase::Error
    };
    (phase, error.to_string())
}

fn installation_error(error: ServiceError) -> RunFailure {
    // Declined elevation and component installation failures need another
    // explicit user action, never a scheduled installer or repeated UAC.
    (Phase::NeedsAuthorization, error.to_string())
}

fn check_cancel(cancel: &AtomicBool) -> RunResult<()> {
    if cancel.load(Ordering::Acquire) {
        Err((Phase::Stopped, "文件搜索已关闭".into()))
    } else {
        Ok(())
    }
}

fn io_error(error: std::io::Error) -> (Phase, String) {
    (Phase::Error, error.to_string())
}

fn checkpoint_error(error: ServiceError) -> RunFailure {
    service_error(error)
}

fn load_current(root: &Path) -> RunResult<Option<(Generation, PathBuf)>> {
    let path = root.join("current.json");
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    let manifest: CurrentGeneration = serde_json::from_reader(file).map_err(|error| {
        (
            if error.is_io() {
                Phase::Error
            } else {
                Phase::NeedsRebuild
            },
            format!("索引清单读取失败：{error}"),
        )
    })?;
    if !generation_name(&manifest.generation) {
        return Err((Phase::NeedsRebuild, "索引清单路径无效，请重建".into()));
    }
    let generation = Generation::load(&root.join("generations").join(manifest.generation))
        .map_err(|error| {
            let invalid = error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData
                )
            }) || error
                .downcast_ref::<serde_json::Error>()
                .is_some_and(|error| !error.is_io());
            (
                if invalid {
                    Phase::NeedsRebuild
                } else {
                    Phase::Error
                },
                format!("索引文件读取失败：{error}"),
            )
        })?;
    // Generation::load has validated these fixed paths against its root.
    // Check existence without converting permission/device errors into a scan.
    for path in [
        &generation.snapshot,
        &generation.index,
        &generation.identities,
        &generation.checkpoint,
    ] {
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                return Err((
                    Phase::NeedsRebuild,
                    format!("索引文件无效：{}", path.display()),
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err((
                    Phase::NeedsRebuild,
                    format!("索引文件缺失：{}", path.display()),
                ))
            }
            Err(error) => return Err(io_error(error)),
        }
    }
    let provenance = [
        PathBuf::from(format!(
            "{}.provenance.json",
            generation.identities.display()
        )),
        generation.identities.with_extension("provenance.json"),
    ];
    let mut found = false;
    let mut metadata_error = None;
    for path in provenance {
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => {
                found = true;
                break;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                metadata_error.get_or_insert(error);
            }
        }
    }
    if !found {
        if let Some(error) = metadata_error {
            return Err(io_error(error));
        }
        return Err((Phase::NeedsRebuild, "索引身份来源文件缺失或无效".into()));
    }
    Ok(Some((generation, manifest.protected_checkpoint)))
}

fn generation_name(name: &str) -> bool {
    name.starts_with("gen-")
        && name.len() > 4
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Only a successful atomic create_dir grants ownership. This slot survives
/// failed attempts and manual retries; no pre-existing generation is adopted.
struct OwnedCandidate {
    path: PathBuf,
    parent: PathBuf,
    publish_temp: Option<PathBuf>,
}
impl OwnedCandidate {
    fn claim(root: &Path) -> RunResult<Self> {
        use std::os::windows::fs::MetadataExt;
        let parent = root.join("generations");
        fs::create_dir_all(&parent).map_err(io_error)?;
        let metadata = fs::symlink_metadata(&parent).map_err(io_error)?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err((Phase::Error, "候选索引父目录无效".into()));
        }
        let parent = parent.canonicalize().map_err(io_error)?;
        let proposed = new_generation_path(root)?;
        let name = proposed
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| generation_name(name))
            .ok_or((Phase::Error, "候选索引目录名称无效".into()))?;
        let path = parent.join(name);
        fs::create_dir(&path).map_err(io_error)?;
        Ok(Self {
            path,
            parent,
            publish_temp: None,
        })
    }
    fn cleanup(&mut self, root: &Path) -> RunResult<()> {
        use std::os::windows::fs::MetadataExt;
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| generation_name(name))
            .ok_or((Phase::Error, "候选索引目录名称无效".into()))?;
        if self.path.parent() != Some(self.parent.as_path())
            || root.join("generations").canonicalize().map_err(io_error)? != self.parent
        {
            return Err((Phase::Error, "候选索引目录超出管理范围".into()));
        }
        match File::open(root.join("current.json")) {
            Ok(file) => {
                // An armed candidate is known unpublished: our atomic rename
                // disarms it immediately. A pre-existing malformed manifest
                // cannot turn cleanup into a permanent recovery deadlock.
                match serde_json::from_reader::<_, Value>(file) {
                    Ok(current) if current["generation"].as_str() == Some(name) => {
                        return Err((Phase::Error, "候选索引已被发布，保留现有目录".into()))
                    }
                    Ok(_) => {}
                    Err(error) if !error.is_io() => {}
                    Err(error) => {
                        return Err((Phase::Error, format!("无法核对已发布索引：{error}")))
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
        if let Some(temporary) = self.publish_temp.as_ref() {
            if *temporary != root.join(format!("current-{name}.tmp")) {
                return Err((Phase::Error, "候选索引临时清单路径无效".into()));
            }
            match fs::symlink_metadata(temporary) {
                Ok(metadata) if metadata.is_file() && metadata.file_attributes() & 0x400 == 0 => {
                    fs::remove_file(temporary).map_err(io_error)?
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => return Err((Phase::Error, "候选索引临时清单类型无效".into())),
                Err(error) => return Err(io_error(error)),
            }
            self.publish_temp = None;
        }
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) => {
                if !metadata.is_dir()
                    || metadata.file_attributes() & 0x400 != 0
                    || self.path.canonicalize().map_err(io_error)?.parent()
                        != Some(self.parent.as_path())
                {
                    return Err((Phase::Error, "候选索引目录无效，保留目录".into()));
                }
                reject_candidate_reparse(&self.path)?;
                fs::remove_dir_all(&self.path).map_err(io_error)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
        Ok(())
    }
}
fn reject_candidate_reparse(path: &Path) -> RunResult<()> {
    use std::os::windows::fs::MetadataExt;
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err((Phase::Error, "候选索引包含重解析点，保留目录".into()));
        }
        if metadata.is_dir() {
            reject_candidate_reparse(&entry.path())?;
        } else if !metadata.is_file() {
            return Err((Phase::Error, "候选索引文件类型无效，保留目录".into()));
        }
    }
    Ok(())
}
fn cleanup_pending_candidate(shared: &Shared, root: &Path) -> RunResult<()> {
    let mut slot = shared
        .pending_candidate
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(candidate) = slot.as_mut() {
        candidate.cleanup(root).map_err(|(_, error)| {
            (
                Phase::Error,
                format!("未完成索引清理失败，将先重试清理：{error}"),
            )
        })?;
        slot.take();
    }
    Ok(())
}
struct CandidateCleanup<'a> {
    shared: &'a Shared,
    root: &'a Path,
}
impl Drop for CandidateCleanup<'_> {
    fn drop(&mut self) {
        if let Err((_, error)) = cleanup_pending_candidate(self.shared, self.root) {
            crate::log::info(&error);
        }
    }
}

fn new_generation_path(root: &Path) -> RunResult<PathBuf> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| (Phase::Error, error.to_string()))?;
    Ok(root
        .join("generations")
        .join(format!("gen-{}-{}", now.as_nanos(), std::process::id())))
}

fn publish_current(
    root: &Path,
    generation: &Generation,
    protected_checkpoint: &Path,
    owned: &Mutex<Option<OwnedCandidate>>,
) -> RunResult<()> {
    let previous = File::open(root.join("current.json"))
        .ok()
        .and_then(|file| serde_json::from_reader::<_, CurrentGeneration>(file).ok());
    let name = generation
        .root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|name| generation_name(name))
        .ok_or((Phase::Error, "新索引目录无效".into()))?;
    let temporary = root.join(format!("current-{}.tmp", name));
    let mut ownership = owned.lock().unwrap_or_else(|error| error.into_inner());
    let candidate = ownership
        .as_mut()
        .ok_or((Phase::Error, "候选索引所有权缺失".into()))?;
    if candidate.path != generation.root {
        return Err((Phase::Error, "候选索引所有权不匹配".into()));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(io_error)?;
    candidate.publish_temp = Some(temporary.clone());
    drop(ownership);
    serde_json::to_writer(
        &mut file,
        &CurrentGeneration {
            generation: name.into(),
            protected_checkpoint: protected_checkpoint.to_path_buf(),
        },
    )
    .map_err(|error| (Phase::Error, error.to_string()))?;
    file.write_all(b"\n").map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    drop(file);
    fs::rename(temporary, root.join("current.json")).map_err(io_error)?;
    // Successful publication ends ownership before any old-generation work.
    owned
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some(previous) = previous.filter(|previous| previous.generation != name) {
        if let Err(error) = remove_previous_generation(root, &previous.generation) {
            crate::log::info(&format!(
                "file index old generation cleanup deferred: {error}"
            ));
        }
    }
    Ok(())
}

/// Delete only the fixed artifacts of the formerly published generation. No
/// recursive deletion, directory traversal, or user-selected paths are involved.
fn remove_previous_generation(root: &Path, name: &str) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    if !generation_name(name) {
        return Err("invalid old generation name".into());
    }
    let parent = root
        .join("generations")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let directory = parent.join(name);
    let metadata = fs::symlink_metadata(&directory).map_err(|error| error.to_string())?;
    if !metadata.is_dir()
        || metadata.file_attributes() & 0x400 != 0
        || directory
            .canonicalize()
            .map_err(|error| error.to_string())?
            .parent()
            != Some(parent.as_path())
    {
        return Err("old generation is outside the managed directory".into());
    }
    Generation::load(&directory).map_err(|error| error.to_string())?;
    let files = [
        "corpus.bin",
        "names.index",
        "identities.sorted",
        "identities.sorted.provenance.json",
        "journal-start.json",
        "generation.json",
        "live-state.sqlite",
        "live-state.sqlite-wal",
        "live-state.sqlite-shm",
    ];
    for name in files {
        let path = directory.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && metadata.file_attributes() & 0x400 == 0 => {
                fs::remove_file(path).map_err(|error| error.to_string())?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => return Err("unexpected old generation artifact".into()),
            Err(error) => return Err(error.to_string()),
        }
    }
    fs::remove_dir(directory).map_err(|error| error.to_string())
}

fn native_filter(filter: FileFilter) -> kite_file_index::FileFilter {
    use kite_file_index::FileFilter as Native;
    match filter {
        FileFilter::All => Native::All,
        FileFilter::Images => Native::Images,
        FileFilter::Documents => Native::Documents,
        FileFilter::Videos => Native::Videos,
        FileFilter::Audio => Native::Audio,
        FileFilter::Archives => Native::Archives,
        FileFilter::Folders => Native::Folders,
    }
}

fn decode_hits(value: Value) -> Result<Vec<Hit>, String> {
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(error.into());
    }
    let paths = value["paths"].as_array().ok_or("文件查询未返回路径")?;
    let mut hits = Vec::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        let raw_path = value["paths_utf16"]
            .get(index)
            .filter(|raw| !raw.is_null())
            .map(|raw| -> Result<Vec<u16>, String> {
                raw.as_array()
                    .ok_or("原始文件路径格式无效")?
                    .iter()
                    .map(|unit| {
                        unit.as_u64()
                            .and_then(|unit| u16::try_from(unit).ok())
                            .ok_or_else(|| "原始文件路径含无效码元".to_string())
                    })
                    .collect()
            })
            .transpose()?;
        let path = path.as_str().ok_or("文件路径格式无效")?;
        let name = path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned();
        let is_folder = value["is_dirs"]
            .get(index)
            .and_then(Value::as_bool)
            .ok_or("文件查询缺少类型标记")?;
        hits.push(Hit {
            path: path.into(),
            is_folder,
            name,
            raw_path,
        });
    }
    Ok(hits)
}
