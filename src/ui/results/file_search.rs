//! Ordinary file search delegates to the same Loci process as its management UI.

use crate::model::{AppItem, ResultAction, ResultSource, SearchResult};
use crate::plugin::{
    Activation, ListItemAction, PluginHost, PluginRegistry, QueryOutcome, ResponseMode,
};
use crate::system::file_filter::FileFilter;
use crate::{search, system};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) const LOCI_CONFIGURE_ID: &str = "loci:configure";

#[cfg(test)]
pub(crate) fn build_file_results(
    query: &str,
    icon_dir: &std::path::Path,
    filter: FileFilter,
    registry: &Arc<Mutex<PluginRegistry>>,
    host: &Arc<Mutex<PluginHost>>,
) -> Vec<SearchResult> {
    build_file_results_if_current(query, icon_dir, filter, registry, host, || true)
}

fn build_file_results_if_current(
    query: &str,
    icon_dir: &std::path::Path,
    filter: FileFilter,
    registry: &Arc<Mutex<PluginRegistry>>,
    host: &Arc<Mutex<PluginHost>>,
    current: impl Fn() -> bool,
) -> Vec<SearchResult> {
    let filter = match filter {
        FileFilter::All => "all",
        FileFilter::Images => "images",
        FileFilter::Documents => "documents",
        FileFilter::Videos => "videos",
        FileFilter::Audio => "audio",
        FileFilter::Archives => "archives",
        FileFilter::Folders => "folders",
    };
    let activation = Activation {
        plugin_id: "com.kite.loci".into(),
        provider_id: "file_search".into(),
        raw_query: query.into(),
        effective_query: serde_json::json!({"query":query,"filter":filter}).to_string(),
        response_mode: ResponseMode::List,
    };
    let outcome = {
        let plugin = registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get("com.kite.loci")
            .cloned();
        match &plugin {
            None => return vec![loci_status_result("Loci 插件未安装，请在插件设置中导入")],
            Some(plugin) if !plugin.enabled => {
                return vec![loci_status_result(
                    "Loci 文件搜索已禁用，请在插件设置中启用",
                )]
            }
            _ => {}
        }
        let mut registry = PluginRegistry::new();
        registry.insert_loaded(plugin.unwrap());
        let mut host = host.lock().unwrap_or_else(|error| error.into_inner());
        if !current() {
            return Vec::new();
        }
        // The UI file generation is independent of Provider Mode's RPC generation.
        let generation = host.current_generation();
        host.query(&registry, &activation, generation)
    };
    match outcome {
        Ok(QueryOutcome::List { items, .. }) => {
            let mut results = Vec::new();
            let mut files = 0;
            for item in items {
                if item.id == "loci-status" || item.id == "loci-pending" {
                    let mut status = loci_status_result(&item.title);
                    if item.id == "loci-pending" {
                        status.matched_by = "loci-pending".into();
                    }
                    results.push(status);
                } else if let Some(ListItemAction::OpenFile { path }) = item.action {
                    if files >= 20 || !crate::plugin::safety::plugin_path_allowed(&path) {
                        continue;
                    }
                    let mut app_item = AppItem::scanned(
                        format!("file:{}", path.to_lowercase()),
                        item.title.clone(),
                        path.clone(),
                        None,
                        None,
                        "loci",
                    );
                    app_item.attach_search_fields();
                    app_item.icon = system::icons::cache_type_icon(
                        icon_dir,
                        &item.title,
                        std::path::Path::new(&path).is_dir(),
                    );
                    results.push(
                        SearchResult::scored(app_item, 400, "file").with_source_action(
                            ResultSource::File,
                            ResultAction::OpenFile { path },
                        ),
                    );
                    files += 1;
                }
            }
            results
        }
        Ok(QueryOutcome::Panel { .. }) => {
            vec![loci_status_result("Loci 尚未就绪，点击配置并查看状态")]
        }
        Ok(QueryOutcome::Empty { .. }) => Vec::new(),
        Err(error) => vec![loci_status_result(&format!("Loci 文件搜索不可用：{error}"))],
    }
}

pub(in crate::ui) struct FileSearchJob {
    pub generation: u64,
    pub query: String,
    pub icon_dir: std::path::PathBuf,
    pub filter: FileFilter,
    pub registry: Arc<Mutex<PluginRegistry>>,
    pub host: Arc<Mutex<PluginHost>>,
    pub on_done: Arc<dyn Fn(u64, String, Vec<SearchResult>, u128) + Send + Sync>,
}
pub(in crate::ui) struct FileSearchWorker {
    slot: Arc<crate::search::service::LatestSlot<Option<FileSearchJob>>>,
    stop: Arc<AtomicBool>,
}
impl FileSearchWorker {
    pub fn spawn() -> Self {
        let slot: Arc<crate::search::service::LatestSlot<Option<FileSearchJob>>> =
            Arc::new(crate::search::service::LatestSlot::new());
        let worker = slot.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        std::thread::spawn(move || {
            let mut last = 0;
            while !stopping.load(Ordering::Acquire) {
                worker.wait_newer_than(last, std::time::Duration::from_millis(200));
                let Some((sequence, job)) = worker.take() else {
                    continue;
                };
                last = sequence;
                let Some(job) = job else {
                    continue;
                };
                while !stopping.load(Ordering::Acquire) {
                    let started = std::time::Instant::now();
                    let results = build_file_results_if_current(
                        &job.query,
                        &job.icon_dir,
                        job.filter,
                        &job.registry,
                        &job.host,
                        || !stopping.load(Ordering::Acquire) && worker.latest_seq() == sequence,
                    );
                    if stopping.load(Ordering::Acquire) || worker.latest_seq() != sequence {
                        break;
                    }
                    let pending = results
                        .iter()
                        .any(|result| result.matched_by == "loci-pending");
                    (job.on_done)(
                        job.generation,
                        job.query.clone(),
                        results,
                        started.elapsed().as_micros(),
                    );
                    if !pending {
                        break;
                    }
                    // Service scans/recovery continue independently. Refresh the current
                    // query automatically while retaining a latest-only request slot.
                    worker.wait_newer_than(sequence, std::time::Duration::from_millis(500));
                    if worker.latest_seq() != sequence {
                        break;
                    }
                }
            }
        });
        Self { slot, stop }
    }
    pub fn submit(&self, job: FileSearchJob) {
        self.slot.submit(Some(job));
    }
    pub fn cancel_current(&self) {
        self.slot.submit(None);
    }
}
impl Drop for FileSearchWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.slot.submit(None);
    }
}

pub(crate) fn loci_status_result(message: &str) -> SearchResult {
    let mut item = AppItem::scanned(
        LOCI_CONFIGURE_ID.into(),
        message.into(),
        LOCI_CONFIGURE_ID.into(),
        None,
        None,
        "loci-status",
    );
    item.attach_search_fields();
    SearchResult::scored(item, 0, "loci-status")
}
pub(crate) fn prepend_dependency_status(
    results: &mut Vec<SearchResult>,
    file_results: &[SearchResult],
) {
    if let Some(status) = file_results
        .iter()
        .find(|result| result.item.source == "loci-status")
    {
        results.insert(0, status.clone());
        results.truncate(search::MAX_RESULTS);
    }
}
pub(crate) fn is_current_file_response(
    files_mode: bool,
    current_generation: u64,
    current_query: &str,
    response_generation: u64,
    response_query: &str,
) -> bool {
    files_mode
        && current_generation == response_generation
        && current_query.trim() == response_query
}
