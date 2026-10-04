//! Everything 文件搜索结果组装与依赖状态入口。

use crate::model::{AppItem, SearchResult};
use crate::search;
use crate::system;

/// 按当前 Query 与类型过滤器查询 Everything，组装文件结果。
/// 依赖不可用时只返回状态入口；短 Query 不发起 IPC。
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

/// Everything 未安装 / 未运行时的状态结果（Ready 时无状态行）。
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
    if let Some(status) = file_results
        .iter()
        .find(|result| result.item.source == "everything-status")
    {
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
