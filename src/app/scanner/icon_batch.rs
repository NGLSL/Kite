//! 图标批处理：收集缺失项并并行提取。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use crate::model::AppIndex;
use crate::system::icons;

/// 收集缺少图标的条目 (id, 首选源, target 回退)。调用方短暂持锁后即可释放。
pub fn missing_icon_targets(index: &AppIndex) -> Vec<(String, Option<String>, Option<String>)> {
    index
        .apps
        .iter()
        .filter(|a| a.icon.is_none())
        .map(|a| {
            let src = a.icon_src.clone().filter(|s| !s.is_empty());
            let target = Some(a.target.clone()).filter(|s| !s.is_empty());
            (a.id.clone(), src, target)
        })
        .collect()
}

/// 并行提取图标，返回 id → PNG 路径；提取过程不持任何锁，调用方拿结果后短暂持锁合并。
pub fn extract_icons_parallel(
    pending: &[(String, Option<String>, Option<String>)],
    icon_dir: &Path,
) -> HashMap<String, Option<String>> {
    use std::sync::Arc;
    let results = Arc::new(Mutex::new(HashMap::<String, Option<String>>::new()));
    if pending.is_empty() {
        return HashMap::new();
    }
    let chunk = pending.len().div_ceil(8).max(1);

    std::thread::scope(|s| {
        for part in pending.chunks(chunk) {
            let part: Vec<(String, Option<String>, Option<String>)> = part.to_vec();
            let icon_dir = icon_dir.to_path_buf();
            let results = Arc::clone(&results);
            s.spawn(move || {
                for (id, src, target) in &part {
                    let path = icons::cache_icon(&icon_dir, id, src.as_deref(), target.as_deref());
                    if let Ok(mut g) = results.lock() {
                        g.insert(id.clone(), path);
                    }
                }
            });
        }
    });

    results.lock().map(|g| g.clone()).unwrap_or_default()
}
