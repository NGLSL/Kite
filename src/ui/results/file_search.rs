//! 内置文件搜索结果与启用/恢复状态；保留旧 Everything 兼容检查。

use crate::model::{AppItem, ResultAction, ResultSource, SearchResult};
use crate::search;
use crate::system;
use std::fmt::Write as _;
use std::sync::Arc;

/// 按当前 Query 与类型过滤器查询 Everything，组装文件结果。
/// 依赖不可用时只返回状态入口；短 Query 不发起 IPC。
#[cfg(test)]
pub(crate) fn build_file_results(
    query: &str,
    icon_dir: &std::path::Path,
    filter: system::everything::FileFilter,
) -> Vec<SearchResult> {
    if let Some(status) = everything_status_result(system::everything::availability()) {
        return vec![status];
    }
    if search::normalize_for_index(query).chars().count() < 2 {
        return Vec::new();
    }
    system::everything::search_files(query, 20, filter)
        .into_iter()
        .map(|hit| {
            let id = format!("file:{}", hit.path.to_lowercase());
            let mut item =
                AppItem::scanned(id, hit.name.clone(), hit.path, None, None, "everything");
            item.attach_search_fields();
            item.icon = system::icons::cache_type_icon(icon_dir, &hit.name, hit.is_folder);
            SearchResult::scored(item, 400, "file")
        })
        .collect()
}

/// 查询 Kite 内置索引。旧 Everything 适配器的 `build_file_results` 保留给
/// 兼容诊断和迁移代码；正式 UI 路径只走 Controller，不会在服务不可用时
/// 静默回退到 Everything。
pub(crate) fn build_indexed_file_results(
    query: &str,
    icon_dir: &std::path::Path,
    filter: system::everything::FileFilter,
    controller: &Arc<crate::file_search::Controller>,
    enabled: bool,
) -> Vec<SearchResult> {
    if !enabled {
        return vec![file_control_result(
            "kite:file-search:enable",
            "启用文件搜索",
            "点击右侧“启用”，开始建立文件索引",
        )];
    }

    let snapshot = controller.snapshot();
    use crate::file_search::Phase;
    match snapshot.phase {
        Phase::Ready => {}
        Phase::NeedsAuthorization => {
            return vec![file_control_result(
                "kite:file-search:authorize",
                "需要管理员授权",
                "点击右侧“授权”，继续建立文件索引",
            )];
        }
        Phase::NeedsRebuild => {
            return vec![file_control_result(
                "kite:file-search:rebuild",
                "需要重建文件索引",
                "索引已过期，点击重建即可恢复文件搜索",
            )];
        }
        Phase::Starting | Phase::Indexing | Phase::CatchingUp => {
            let detail = if snapshot.detail.trim().is_empty() {
                "正在建立文件索引，请稍候…".to_string()
            } else {
                snapshot.detail.clone()
            };
            let title = if snapshot.entries > 0 {
                format!("正在建立文件索引 · {} 条", snapshot.entries)
            } else {
                "正在建立文件索引".to_string()
            };
            return vec![file_control_result(
                "kite:file-search:progress",
                &title,
                &detail,
            )];
        }
        Phase::Error => {
            let detail = if snapshot.detail.trim().is_empty() {
                "索引读取失败，点击重试".to_string()
            } else {
                snapshot.detail.clone()
            };
            return vec![file_control_result(
                "kite:file-search:error",
                "文件索引暂不可用",
                &format!("{detail}；点击重试，不会自动重建整个索引"),
            )];
        }
        Phase::Stopped => {
            return vec![file_control_result(
                "kite:file-search:authorize",
                "文件搜索已停止",
                "点击此处重新启用文件搜索",
            )];
        }
    }

    if search::normalize_for_index(query).chars().count() < 2 {
        return Vec::new();
    }

    match controller.query(query, 20, filter) {
        Ok(hits) => hits
            .into_iter()
            .map(|hit| {
                let (id, source, action) = if let Some(raw_path) = hit.raw_path {
                    (
                        raw_file_id(&raw_path),
                        "file-search-raw",
                        ResultAction::OpenFileUtf16 { path: raw_path },
                    )
                } else {
                    // Keep the long-standing file:<lowercase-path> identity so
                    // existing pins and history continue to resolve after the
                    // Everything adapter is replaced by the built-in index.
                    let path = hit.path.clone();
                    (
                        format!("file:{}", path.to_lowercase()),
                        "file-search",
                        ResultAction::OpenFile { path },
                    )
                };
                let mut item = AppItem::scanned(id, hit.name.clone(), hit.path, None, None, source);
                item.attach_search_fields();
                item.icon = system::icons::cache_type_icon(icon_dir, &hit.name, hit.is_folder);
                SearchResult::scored(item, 400, "file")
                    .with_source_action(ResultSource::File, action)
            })
            .collect(),
        Err(error) => vec![file_control_result(
            "kite:file-search:error",
            "文件搜索暂不可用",
            &format!("{error}；点击重试"),
        )],
    }
}

fn raw_file_id(path: &[u16]) -> String {
    let mut id = String::with_capacity("file:utf16:".len() + path.len() * 4);
    id.push_str("file:utf16:");
    for unit in path {
        let _ = write!(&mut id, "{unit:04x}");
    }
    id
}

fn file_control_result(id: &str, title: &str, detail: &str) -> SearchResult {
    let mut item = AppItem::scanned(
        id.to_string(),
        title.to_string(),
        detail.to_string(),
        None,
        None,
        "file-search-status",
    );
    item.attach_search_fields();
    let action = ResultAction::LaunchApp {
        item_id: id.to_string(),
    };
    SearchResult::scored(item, 760, "file-search-status")
        .with_source_action(ResultSource::Builtin, action)
}

/// Everything 未安装 / 未运行时的状态结果（Ready 时无状态行）。
#[cfg(test)]
pub(crate) fn everything_status_result(
    availability: system::everything::Availability,
) -> Option<SearchResult> {
    use system::everything::{
        Availability, DOWNLOAD_RESULT_ID, DOWNLOAD_URL, NOT_RUNNING_RESULT_ID,
    };

    let (id, name, target) = match availability {
        Availability::Ready => return None,
        Availability::InstalledButNotRunning => (
            NOT_RUNNING_RESULT_ID,
            "Everything 未运行，文件搜索不可用",
            NOT_RUNNING_RESULT_ID,
        ),
        Availability::NotInstalled => (
            DOWNLOAD_RESULT_ID,
            "未安装 Everything，点击前往官方下载",
            DOWNLOAD_URL,
        ),
    };
    let mut item = AppItem::scanned(
        id.to_string(),
        name.to_string(),
        target.to_string(),
        None,
        None,
        "everything-status",
    );
    item.attach_search_fields();
    Some(SearchResult::scored(item, 0, "everything-status"))
}

/// 文件模式下把依赖状态行置顶，保证未安装/未运行始终可见。
pub(crate) fn prepend_dependency_status(
    results: &mut Vec<SearchResult>,
    file_results: &[SearchResult],
) {
    if let Some(status) = file_results.iter().find(|result| {
        result.item.source == "everything-status" || result.item.source == "file-search-status"
    }) {
        results.insert(0, status.clone());
        results.truncate(search::MAX_RESULTS);
    }
}

/// 文件查询落地校验：模式、代际与 Query 文本须同时匹配。
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
