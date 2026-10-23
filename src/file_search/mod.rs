//! Ordinary-permission controller for the opt-in native file-search capability.
//!
//! One worker owns the live index. Only the metadata connection crosses the
//! service boundary; the UI never performs volume I/O or waits for a build.

mod build;

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
    build::dispatch_entry()
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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Stopped,
    Starting,
    Indexing,
    CatchingUp,
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
    response: mpsc::SyncSender<Result<Vec<Hit>, String>>,
}

struct Running {
    cancel: Arc<AtomicBool>,
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
        if !rebuild
            && slot.as_ref().is_some_and(|run| {
                !run.cancel.load(Ordering::Acquire) && !run.done.load(Ordering::Acquire)
            })
        {
            return;
        }
        let previous = slot.take();
        if let Some(previous) = &previous {
            cancel_running(previous);
        }
        let id = self.shared.active.fetch_add(1, Ordering::AcqRel) + 1;
        let cancel = Arc::new(AtomicBool::new(false));
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
            service,
            requests,
            done,
            thread: worker,
        });
    }

    /// Nonblocking cancellation; the service also cancels on process death.
    pub fn stop(&self) {
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
                response,
            })
            .map_err(|_| "文件搜索已停止".to_string())?;
        receiver
            .recv_timeout(Duration::from_secs(15))
            .map_err(|error| -> String {
                match error {
                    mpsc::RecvTimeoutError::Disconnected => "查询已更新".into(),
                    mpsc::RecvTimeoutError::Timeout => "文件查询耗时较长，请重试".into(),
                }
            })?
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop();
    }
}

fn cancel_running(run: &Running) {
    run.cancel.store(true, Ordering::Release);
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

    fn progress(&self, id: u64, event: Value) {
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
        self.update(
            id,
            phase,
            &detail,
            event.get("entries").and_then(Value::as_u64),
        );
    }
}

type RunResult<T> = Result<T, (Phase, String)>;

fn service_error(error: ServiceError) -> (Phase, String) {
    let phase = match error.kind {
        ErrorKind::NeedsInstall
        | ErrorKind::NotInstalled
        | ErrorKind::PathMismatch
        | ErrorKind::AccessDenied
        | ErrorKind::Cancelled => Phase::NeedsAuthorization,
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
    requests: mpsc::Receiver<SearchRequest>,
) -> RunResult<()> {
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
            Phase::Starting,
            "请在 Windows 授权窗口中确认安装索引组件",
            None,
        );
        kite_index_service::request_install().map_err(service_error)?;
    }
    check_cancel(&cancel)?;
    let mut client = Client::connect_current(false).map_err(service_error)?;
    *service_handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(client.cancel_handle());
    check_cancel(&cancel)?;
    let progress_shared = Arc::clone(shared);
    let progress = move |event: Value| progress_shared.progress(id, event);
    let existing = if rebuild { None } else { load_current(root)? };
    let (generation, mut watch, new_generation, protected_checkpoint) =
        if let Some((generation, checkpoint)) = existing {
            shared.update(
                id,
                Phase::CatchingUp,
                "正在补齐退出期间的文件变化",
                Some(generation.entries),
            );
            let watch = client
                .watch_path(&checkpoint, None)
                .map_err(checkpoint_error)?;
            (generation, watch, false, checkpoint)
        } else {
            shared.update(id, Phase::Indexing, "正在读取本机 NTFS 磁盘的文件名", None);
            let capture = client
                .capture(&[], Some(&progress))
                .map_err(service_error)?;
            check_cancel(&cancel)?;
            // Begin tracking before ordinary-rights projection/build to cover that gap.
            let checkpoint = capture.checkpoint_path;
            let watch = client
                .watch_path(&checkpoint, None)
                .map_err(checkpoint_error)?;
            fs::create_dir_all(root.join("generations")).map_err(io_error)?;
            let destination = new_generation_path(root)?;
            let generation = build::run(
                &capture.capture_root,
                &destination,
                Arc::clone(&cancel),
                &progress,
            )
            .map_err(|error| (Phase::Error, error.to_string()))?;
            (generation, watch, true, checkpoint)
        };
    check_cancel(&cancel)?;
    let feed = watch.feed_path().to_path_buf();
    let watch_error = Arc::new(Mutex::new(None::<String>));
    let error_slot = Arc::clone(&watch_error);
    let watch_cancel = Arc::clone(&cancel);
    let watch_shared = Arc::clone(shared);
    let watch_worker = thread::spawn(move || {
        loop {
            match watch.next(None) {
                Ok(true) => {}
                Ok(false) => {
                    if !watch_cancel.load(Ordering::Acquire) {
                        let message = "后台读取已停止，请重新启用文件搜索";
                        *error_slot.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(message.into());
                        watch_shared.update(id, Phase::Error, message, None);
                        watch_cancel.store(true, Ordering::Release);
                    }
                    break;
                }
                Err(error) => {
                    if !watch_cancel.load(Ordering::Acquire) {
                        let message = error.to_string();
                        *error_slot.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(message.clone());
                        watch_shared.update(
                            id,
                            Phase::NeedsRebuild,
                            &format!("同步已停止：{message}"),
                            None,
                        );
                        // Also interrupt LiveSearch::open if failure happened
                        // before its initial catch-up barrier was emitted.
                        watch_cancel.store(true, Ordering::Release);
                    }
                    break;
                }
            }
        }
    });
    let outcome = (|| {
        shared.update(
            id,
            Phase::CatchingUp,
            "正在同步最新变化",
            Some(generation.entries),
        );
        let mut engine = LiveSearch::open_with_cancel(&generation, &feed, Arc::clone(&cancel))
            .map_err(|error| (Phase::NeedsRebuild, format!("索引需要重建：{error}")))?;
        check_cancel(&cancel)?;
        live_health(&engine.status(), &watch_error)?;
        if new_generation {
            publish_current(root, &generation, &protected_checkpoint)?;
        }
        shared.update(
            id,
            Phase::Ready,
            "文件索引已就绪，正在实时同步",
            Some(generation.entries),
        );
        let mut changes = engine.status()["changes"].as_u64().unwrap_or(0);
        while !cancel.load(Ordering::Acquire) {
            let state = engine.status();
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
                        request = newer;
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
                        .query(&query)
                        .map_err(|error| error.to_string())
                        .and_then(decode_hits);
                    let _ = request.response.send(result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(())
    })();
    // Explicit cancel also covers failure before Ready. Join before this worker exits.
    if let Some(handle) = service_handle
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        let _ = handle.cancel();
    }
    let _ = watch_worker.join();
    outcome
}

fn live_health(state: &Value, watch_error: &Mutex<Option<String>>) -> RunResult<()> {
    if state["stale"] == true || state["rebuild_required"] == true {
        let detail = state["error"].as_str().unwrap_or("变更日志已失效");
        return Err((Phase::NeedsRebuild, format!("索引需要重建：{detail}")));
    }
    if let Some(error) = watch_error
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return Err((Phase::Error, error));
    }
    if state["journal_stopped"] == true {
        return Err((Phase::Error, "后台同步已停止，点击重新连接".into()));
    }
    Ok(())
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

fn checkpoint_error(error: ServiceError) -> (Phase, String) {
    if matches!(
        error.kind,
        ErrorKind::PathMismatch | ErrorKind::InvalidRequest | ErrorKind::Backend
    ) {
        (
            Phase::NeedsRebuild,
            format!("索引起点已失效，请重建：{error}"),
        )
    } else {
        service_error(error)
    }
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
            Phase::NeedsRebuild,
            format!("索引清单损坏，请重建：{error}"),
        )
    })?;
    if !generation_name(&manifest.generation) {
        return Err((Phase::NeedsRebuild, "索引清单路径无效，请重建".into()));
    }
    Generation::load(&root.join("generations").join(manifest.generation))
        .map(|generation| Some((generation, manifest.protected_checkpoint)))
        .map_err(|error| {
            (
                Phase::NeedsRebuild,
                format!("索引文件不完整，请重建：{error}"),
            )
        })
}

fn generation_name(name: &str) -> bool {
    name.starts_with("gen-")
        && name.len() > 4
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
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
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(io_error)?;
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
