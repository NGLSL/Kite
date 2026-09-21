//! 安装族 / 启动归并共享契约。
//!
//! 扫描期 Discovery 吸收与检索期 launch-group 折叠共用同一组纯函数，
//! 避免两处各写一份「是否同一安装 / 能否合并」导致语义漂移。
//!
//! 两套决策入口刻意保持不同（行为与历史一致）：
//! - [`should_merge_family`]：搜索结果归并，同安装根即可合并（产品族）。
//! - [`discovery_absorbs_into`]：扫描期 Discovery 吸收，同根还须参数等价。

use crate::model::AppItem;

use super::util::{
    args_equivalent_for_merge, display_family_key, install_root_dir, launch_identity,
    names_share_install_family, normalize_path_key,
};

/// `shell:AppsFolder\` 前缀剥离（大小写不敏感）。
pub fn strip_shell_target(target: &str) -> &str {
    super::util::strip_shell_appsfolder(target.trim())
}

/// target 是否为 shell:AppsFolder 包（含前缀本身）。
pub fn is_shell_package(target: &str) -> bool {
    let t = target.trim();
    t.len() >= 16
        && t.is_char_boundary(16)
        && t[..16].eq_ignore_ascii_case("shell:appsfolder")
}

/// 正式路径类扫描来源（开始菜单/桌面/便携等）。
pub fn is_friendly_source(source: &str) -> bool {
    crate::model::is_path_formal_source(source)
}

/// 去壳后的 exe 文件名词干（小写）；非路径或无 stem 返回 None。
pub fn exe_stem(target: &str) -> Option<String> {
    let body = strip_shell_target(target);
    std::path::Path::new(body)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .filter(|s| !s.is_empty())
}

/// 搜索侧词干链接：exe stem 与另一条展示名族对齐（`wps` ↔ `WPS Office`）。
/// stem 含空格或短于 3 字符时拒绝，避免噪声链接。
pub fn exe_stem_matches_display_family(target: &str, other_name: &str) -> bool {
    let Some(stem) = exe_stem(target) else {
        return false;
    };
    if stem.len() < 3 || stem.contains(' ') {
        return false;
    }
    let family = display_family_key(other_name);
    family == stem || family.starts_with(&format!("{stem} "))
}

/// 扫描侧词干链接：exe stem 与展示名紧凑 ASCII 对齐。
pub fn exe_stem_links_to_compact_name(target: &str, name: &str) -> bool {
    let Some(stem) = exe_stem(target) else {
        return false;
    };
    if stem.len() < 3 {
        return false;
    }
    let compact_name = crate::search::normalize_for_index(name)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>();
    // 仅允许「展示名包含 exe 词干」或全等；反向前缀（code ← CodeHelper）会误吸收。
    !compact_name.is_empty() && (stem == compact_name || compact_name.starts_with(&stem))
}

/// 检索期：已确认同一安装、同一启动动作的入口才合并；不单靠同名，不无条件忽略参数。
pub fn should_merge_family(a: &AppItem, b: &AppItem) -> bool {
    let same_name = names_share_install_family(&a.name, &b.name);
    let stem_link = exe_stem_matches_display_family(&a.target, &b.name)
        || exe_stem_matches_display_family(&b.target, &a.name);
    if !same_name && !stem_link {
        return false;
    }

    let body_a = strip_shell_target(&a.target);
    let body_b = strip_shell_target(&b.target);
    let ta = normalize_path_key(body_a.trim());
    let tb = normalize_path_key(body_b.trim());
    if !ta.is_empty() && ta == tb {
        // 同一 exe：仅当参数等价（或只差 /from=*）才合并
        return args_equivalent_for_merge(a.args.as_deref(), b.args.as_deref());
    }

    // 同安装根：不同 exe 的产品族（ksolaunch vs wps.exe）
    let root_a = install_root_dir(&a.target);
    let root_b = install_root_dir(&b.target);
    match (root_a, root_b) {
        (Some(x), Some(y)) => x == y,
        (None, Some(_)) | (Some(_), None) => {
            let shell = is_shell_package(&a.target) || is_shell_package(&b.target);
            let pair_ok = (is_friendly_source(&a.source) || is_friendly_source(&b.source))
                || a.source == "app-paths"
                || b.source == "app-paths";
            same_name && shell && pair_ok
        }
        (None, None) => same_name && is_shell_package(&a.target) && is_shell_package(&b.target),
    }
}

/// 扫描期：同一 launch identity / 同一 exe（参数等价）/ 同安装根且名称族或 exe 词干链接。
/// 比 [`should_merge_family`] 更严：同安装根也须参数等价，避免 Discovery 误吸正式入口。
pub fn discovery_absorbs_into(discovery: &AppItem, formal: &AppItem) -> bool {
    if launch_identity(&discovery.target, discovery.args.as_deref())
        == launch_identity(&formal.target, formal.args.as_deref())
    {
        return true;
    }

    let td = normalize_path_key(&discovery.target);
    let tf = normalize_path_key(&formal.target);
    if !td.is_empty() && td == tf {
        return args_equivalent_for_merge(discovery.args.as_deref(), formal.args.as_deref());
    }

    let (Some(rd), Some(rf)) = (
        install_root_dir(&discovery.target),
        install_root_dir(&formal.target),
    ) else {
        return false;
    };
    if rd != rf {
        return false;
    }
    if !args_equivalent_for_merge(discovery.args.as_deref(), formal.args.as_deref()) {
        return false;
    }
    names_share_install_family(&discovery.name, &formal.name)
        || exe_stem_links_to_compact_name(&discovery.target, &formal.name)
        || exe_stem_links_to_compact_name(&formal.target, &discovery.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, target: &str, source: &str) -> AppItem {
        AppItem::scanned(
            format!("{name}:{target}"),
            name.into(),
            target.into(),
            None,
            None,
            source,
        )
    }

    #[test]
    fn shell_prefix_strips_for_stem() {
        assert_eq!(
            exe_stem(r"shell:AppsFolder\C:\Apps\foo.exe").as_deref(),
            Some("foo")
        );
    }

    #[test]
    fn search_family_merges_same_install_root() {
        let a = item(
            "WPS",
            r"D:\Program Files\WPS Office\ksolaunch.exe",
            "start-menu",
        );
        let b = item(
            "WPS Office",
            r"D:\Program Files\WPS Office\12.1.0.28505\office6\wps.exe",
            "start-menu",
        );
        assert!(should_merge_family(&a, &b));
    }

    #[test]
    fn scan_absorb_requires_args_for_same_root() {
        let target = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
        let formal = AppItem::scanned(
            "formal".into(),
            "PowerShell".into(),
            target.into(),
            None,
            None,
            "start-menu",
        );
        let discovery = AppItem::scanned(
            "dev".into(),
            "PowerShell".into(),
            target.into(),
            Some("-NoExit".into()),
            None,
            "app-paths",
        );
        assert!(
            !discovery_absorbs_into(&discovery, &formal),
            "同路径但参数不同不得吸收"
        );
        let same_args = AppItem::scanned(
            "same".into(),
            "PowerShell".into(),
            target.into(),
            None,
            None,
            "app-paths",
        );
        assert!(discovery_absorbs_into(&same_args, &formal));
    }
}
