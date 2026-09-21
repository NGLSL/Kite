//! 绝对路径直达结果（打开文件 / 打开所在目录）。

use crate::app;
use crate::model::{AppItem, ResultAction, ResultSource, SearchResult};
use crate::search;

/// 把用户输入解析为本机真实路径时，生成直达动作；不存在的路径返回空。
pub(crate) fn build_direct_path_results(query: &str) -> Vec<SearchResult> {
    let Some(path) = app::actions::direct_path_candidate(query) else {
        return Vec::new();
    };
    let Ok(metadata) = std::fs::metadata(&path) else {
        return Vec::new();
    };
    if !metadata.is_file() && !metadata.is_dir() {
        return Vec::new();
    }
    let target = path.to_string_lossy().into_owned();
    let result = |id: &str, title: &str, action: ResultAction| {
        let item = AppItem::scanned(
            format!("direct-path:{id}:{target}"),
            title.to_string(),
            target.clone(),
            None,
            None,
            "direct-path",
        );
        SearchResult::scored(item, 2000, "direct-path")
            .with_source_action(ResultSource::File, action)
    };
    let mut hits = Vec::with_capacity(if metadata.is_file() { 2 } else { 1 });
    if metadata.is_file() {
        hits.push(result(
            "open",
            "打开文件",
            ResultAction::OpenLocalPath {
                path: target.clone(),
            },
        ));
    }
    hits.push(result(
        "reveal",
        "打开文件路径",
        ResultAction::RevealPath {
            path: target.clone(),
        },
    ));
    hits
}

/// 直达动作置顶，并去掉同 target 的重复文件/应用行。
pub(crate) fn prepend_direct_path_results(
    results: &mut Vec<SearchResult>,
    direct: &[SearchResult],
) {
    if direct.is_empty() {
        return;
    }
    results.retain(|result| {
        !direct.iter().any(|path_result| {
            result
                .item
                .target
                .replace('/', "\\")
                .eq_ignore_ascii_case(&path_result.item.target.replace('/', "\\"))
        })
    });
    results.splice(0..0, direct.iter().cloned());
    results.truncate(search::MAX_RESULTS);
}
