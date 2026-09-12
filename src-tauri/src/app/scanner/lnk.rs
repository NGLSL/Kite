//! 解析 Windows .lnk 快捷方式为可启动目标。

use std::path::Path;

/// 将 .lnk 解析为 (target, args, working_dir, icon_path)。
pub fn resolve_lnk(path: &Path) -> Option<(String, Option<String>, Option<String>, Option<String>)> {
    let shortcut = lnk::ShellLink::open(path).ok()?;

    let target = shortcut
        .link_info()
        .as_ref()
        .and_then(|info| {
            info.local_base_path_unicode()
                .as_ref()
                .or_else(|| info.local_base_path().as_ref())
        })
        .cloned()
        .or_else(|| shortcut.relative_path().clone())?;

    let args = shortcut
        .arguments()
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let working_dir = shortcut
        .working_dir()
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let icon = shortcut
        .icon_location()
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let target = if !Path::new(&target).is_absolute() {
        path.parent()
            .map(|parent| parent.join(&target))
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(target)
    } else {
        target
    };

    Some((target, args, working_dir, icon))
}
