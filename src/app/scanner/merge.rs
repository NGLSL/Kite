//! 入口身份去重与命令别名归并。

use std::collections::HashMap;

use crate::model::AppItem;

use super::commands;
use super::util::{hash_id, launch_identity, normalize_path_key};
use super::RawItem;

pub(crate) fn dedupe(raw: Vec<RawItem>) -> Vec<RawItem> {
    use std::collections::hash_map::Entry;

    let rank = |source: &str| match source {
        "start-menu" => 0,
        "desktop" => 1,
        "portable" => 2,
        "uninstall" => 3,
        "app-paths" => 4,
        "uwp" | "apps-folder" => 5,
        "commands" => 6,
        _ => 7,
    };

    let mut best: HashMap<String, RawItem> = HashMap::new();
    for (item, icon) in raw {
        let key = dedupe_key(&item);
        match best.entry(key) {
            Entry::Vacant(slot) => {
                slot.insert((item, icon));
            }
            Entry::Occupied(mut slot) => {
                let current_rank = rank(&slot.get().0.source);
                let incoming_rank = rank(&item.source);
                let replace = incoming_rank < current_rank
                    || (incoming_rank == current_rank
                        && deterministic_item_key(&item) < deterministic_item_key(&slot.get().0));
                if !replace {
                    keep_shortcut_names(&mut slot.get_mut().0, &item);
                } else {
                    let (old_item, _) = slot.get();
                    let mut replacement = item;
                    keep_shortcut_names(&mut replacement, old_item);
                    slot.insert((replacement, icon));
                }
            }
        }
    }

    let mut list: Vec<_> = best.into_values().collect();
    list.sort_by(|a, b| {
        a.0.name
            .to_lowercase()
            .cmp(&b.0.name.to_lowercase())
            .then_with(|| a.0.source.cmp(&b.0.source))
            .then_with(|| normalize_path_key(&a.0.target).cmp(&normalize_path_key(&b.0.target)))
            .then_with(|| a.0.args.cmp(&b.0.args))
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    list
}

fn deterministic_item_key(item: &AppItem) -> (String, String, String, String, String) {
    (
        item.name.to_lowercase(),
        item.display_name.to_lowercase(),
        normalize_path_key(&item.target),
        item.args.clone().unwrap_or_default(),
        item.id.clone(),
    )
}

pub(crate) fn exclude_system_name_duplicates(apps: &mut Vec<RawItem>, system_entries: &[AppItem]) {
    let system_names: std::collections::HashSet<String> = system_entries
        .iter()
        .map(|item| crate::search::normalize_for_index(&item.name))
        .collect();
    apps.retain(|(item, _)| {
        item.source != "apps-folder"
            || !system_names.contains(&crate::search::normalize_for_index(&item.name))
    });
}

/// WindowsApps often exposes `MediaPlayer.exe` beside an AppsFolder row named
/// `Media Player`. Preserve the executable alias as a keyword on the friendly
/// row instead of showing two visually equivalent applications.
pub(crate) fn merge_command_name_duplicates(items: &mut Vec<RawItem>) {
    let compact_name = |item: &AppItem| {
        crate::search::normalize_for_index(&item.name)
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
    };
    let mut remove = std::collections::HashSet::new();
    for index in 0..items.len() {
        if items[index].0.source != commands::COMMAND_SOURCE {
            continue;
        }
        let key = compact_name(&items[index].0);
        let Some(winner) = items.iter().enumerate().find_map(|(candidate, (item, _))| {
            (candidate != index
                && item.source != commands::COMMAND_SOURCE
                && compact_name(item) == key)
                .then_some(candidate)
        }) else {
            continue;
        };
        let alias = items[index].0.clone();
        keep_shortcut_names(&mut items[winner].0, &alias);
        remove.insert(index);
    }
    if !remove.is_empty() {
        let mut index = 0usize;
        items.retain(|_| {
            let keep = !remove.contains(&index);
            index += 1;
            keep
        });
    }
}

fn formal_source_rank(source: &str) -> Option<u8> {
    match source {
        "start-menu" => Some(0),
        "desktop" => Some(1),
        "portable" => Some(2),
        "apps-folder" | "uwp" => Some(3),
        _ => None,
    }
}

fn is_shell_app_source(source: &str) -> bool {
    matches!(source, "apps-folder" | "uwp")
}

fn is_url_shortcut(target: &str) -> bool {
    std::path::Path::new(target)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("url"))
}

fn compact_item_name(item: &AppItem) -> String {
    crate::search::normalize_for_index(&item.name)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

/// Formal 同名是否允许归并。**不单靠同名**——需 shell 孪生、双 .url，
/// 或与 discovery 相同的同安装/同启动身份规则。
fn formal_pair_merges(a: &AppItem, b: &AppItem) -> bool {
    let ca = compact_item_name(a);
    if ca.is_empty() || ca != compact_item_name(b) {
        return false;
    }
    let a_shell = is_shell_app_source(&a.source);
    let b_shell = is_shell_app_source(&b.source);
    // shell AUMID + 路径正式入口：Chrome、Application Verifier 等双行。
    if a_shell != b_shell {
        return true;
    }
    // 桌面 .url + 开始菜单 Steam .url：同名游戏快捷方式。
    if !a_shell && is_url_shortcut(&a.target) && is_url_shortcut(&b.target) {
        return true;
    }
    // 同安装/同启动身份（与 discovery 归并同一套规则）。
    discovery_absorbs_into(a, b) || discovery_absorbs_into(b, a)
}

/// 同名 Formal 入口归并成一行（shell 孪生 / 双 .url / 同安装）。
/// winner 按 start-menu > desktop > portable > apps/uwp。
/// 同名不同安装根/不同 exe 的产品保持两行，避免误杀。
pub(crate) fn merge_same_name_formal_duplicates(items: &mut Vec<RawItem>) {
    let mut groups: std::collections::HashMap<String, Vec<usize>> = Default::default();
    for (index, (item, _)) in items.iter().enumerate() {
        if formal_source_rank(&item.source).is_none() {
            continue;
        }
        let key = compact_item_name(item);
        if key.is_empty() {
            continue;
        }
        groups.entry(key).or_default().push(index);
    }

    let mut remove = std::collections::HashSet::new();
    for indices in groups.into_values() {
        if indices.len() < 2 {
            continue;
        }
        let Some(&winner) = indices
            .iter()
            .min_by(|&&a, &&b| {
                formal_source_rank(&items[a].0.source)
                    .unwrap_or(u8::MAX)
                    .cmp(&formal_source_rank(&items[b].0.source).unwrap_or(u8::MAX))
                    .then_with(|| items[a].0.id.cmp(&items[b].0.id))
            })
        else {
            continue;
        };
        for &loser in &indices {
            if loser == winner || remove.contains(&loser) {
                continue;
            }
            if !formal_pair_merges(&items[winner].0, &items[loser].0) {
                continue;
            }
            let alias = items[loser].0.clone();
            keep_shortcut_names(&mut items[winner].0, &alias);
            if items[winner].1.is_none() {
                items[winner].1 = items[loser].1.clone();
            }
            remove.insert(loser);
        }
    }

    if !remove.is_empty() {
        let mut index = 0usize;
        items.retain(|_| {
            let keep = !remove.contains(&index);
            index += 1;
            keep
        });
    }
}

/// App Paths / Uninstall 能确认与正式/系统入口同一产品时只补关键词/图标，不独立成行。
/// 未吸收且通过启发式的 Discovery 行保留为 Tier C 兜底。不吸入 commands。
pub(crate) fn absorb_discovery_rows(items: &mut Vec<RawItem>) {
    use crate::model::{is_discovery_source, source_layer, SourceLayer};

    let mut remove = std::collections::HashSet::new();
    for i in 0..items.len() {
        if !is_discovery_source(&items[i].0.source) || remove.contains(&i) {
            continue;
        }
        let discovery = items[i].0.clone();
        let discovery_icon = items[i].1.clone();
        let Some(winner) = items.iter().enumerate().find_map(|(j, (host, _))| {
            (j != i
                && !remove.contains(&j)
                && !is_discovery_source(&host.source)
                && matches!(
                    source_layer(&host.source),
                    SourceLayer::Formal | SourceLayer::System
                )
                && discovery_absorbs_into(&discovery, host))
            .then_some(j)
        }) else {
            continue;
        };
        keep_shortcut_names(&mut items[winner].0, &discovery);
        if items[winner].1.is_none() {
            items[winner].1 = discovery_icon;
        }
        remove.insert(i);
    }
    if !remove.is_empty() {
        let mut index = 0usize;
        items.retain(|_| {
            let keep = !remove.contains(&index);
            index += 1;
            keep
        });
    }
}

/// 同一 launch identity / 同一 exe（参数等价）/ 同安装根且名称族或 exe 词干链接。
fn discovery_absorbs_into(discovery: &AppItem, formal: &AppItem) -> bool {
    use super::util::{args_equivalent_for_merge, install_root_dir, launch_identity, names_share_install_family};

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

    let (Some(rd), Some(rf)) = (install_root_dir(&discovery.target), install_root_dir(&formal.target))
    else {
        return false;
    };
    if rd != rf {
        return false;
    }
    if !args_equivalent_for_merge(discovery.args.as_deref(), formal.args.as_deref()) {
        return false;
    }
    names_share_install_family(&discovery.name, &formal.name)
        || exe_stem_links_to_name(&discovery.target, &formal.name)
        || exe_stem_links_to_name(&formal.target, &discovery.name)
}

fn exe_stem_links_to_name(target: &str, name: &str) -> bool {
    let Some(stem) = std::path::Path::new(target)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
    else {
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

/// 同一 target 的不同参数可能代表不同的启动语义（例如普通 PowerShell
/// 与 Developer PowerShell），不能仅按 exe 路径合并。工作目录不参与
/// 去重：开始菜单、桌面快捷方式经常只是在快捷方式元数据中提供了
/// 不同的起始位置，而启动器会将无效或未提供的目录统一回落到用户主目录。
/// 使用 `launch_identity`：`shell:AppsFolder\<绝对 exe 路径>` 与直接路径同一键。
fn dedupe_key(item: &AppItem) -> String {
    hash_id(&[
        &launch_identity(&item.target, item.args.as_deref()),
    ])
}

pub(crate) fn keep_shortcut_names(winner: &mut AppItem, other: &AppItem) {
    for name in std::iter::once(&other.name)
        .chain(std::iter::once(&other.display_name))
        .chain(other.search_keywords.iter())
    {
        if !name.eq_ignore_ascii_case(&winner.name)
            && !name.eq_ignore_ascii_case(&winner.display_name)
            && !winner
                .search_keywords
                .iter()
                .any(|kw| kw.eq_ignore_ascii_case(name))
        {
            winner.search_keywords.push(name.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_winner_is_independent_of_source_enumeration_order() {
        let make = |name: &str| {
            AppItem::scanned(
                "same-id".to_string(),
                name.to_string(),
                r"C:\Apps\same.exe".to_string(),
                None,
                None,
                "start-menu",
            )
        };
        let forward = dedupe(vec![(make("Zeta"), None), (make("Alpha"), None)]);
        let reverse = dedupe(vec![(make("Alpha"), None), (make("Zeta"), None)]);

        assert_eq!(forward[0].0.name, reverse[0].0.name);
        assert_eq!(forward[0].0.name, "Alpha");
    }

    #[test]
    fn dedupe_preserves_distinct_launch_arguments() {
        let target = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
        let normal = AppItem::scanned(
            "normal".into(),
            "PowerShell".into(),
            target.into(),
            None,
            Some(r"C:\Users\admin".into()),
            "start-menu",
        );
        let normal_duplicate = AppItem::scanned(
            "normal-duplicate".into(),
            "PowerShell (desktop)".into(),
            target.into(),
            None,
            Some(r"C:\Users\admin\Desktop".into()),
            "desktop",
        );
        let developer = AppItem::scanned(
            "developer".into(),
            "Developer PowerShell".into(),
            target.into(),
            Some("-NoExit -Command Enter-VsDevShell".into()),
            Some(r"C:\Users\admin".into()),
            "start-menu",
        );

        let items = dedupe(vec![
            (normal, None),
            (normal_duplicate, None),
            (developer, None),
        ]);

        assert_eq!(items.len(), 2, "相同启动语义应合并，不同参数必须保留");
        assert!(items
            .iter()
            .any(|(item, _)| item.name == "Developer PowerShell"));
    }

    #[test]
    fn duplicate_shortcut_names_remain_searchable() {
        let target = r"C:\Program Files\Example\example.exe";
        let primary = AppItem::scanned(
            "same-id".into(),
            "Primary Launcher".into(),
            target.into(),
            None,
            None,
            "start-menu",
        );
        let alternate = AppItem::scanned(
            "same-id".into(),
            "Alternate Console".into(),
            target.into(),
            None,
            None,
            "desktop",
        );
        for raw in [
            vec![(primary.clone(), None), (alternate.clone(), None)],
            vec![(alternate.clone(), None), (primary.clone(), None)],
        ] {
            let mut items = dedupe(raw);
            assert_eq!(items.len(), 1);
            let mut item = items.pop().unwrap().0;
            item.attach_search_fields();
            let index = crate::search::RetrievalIndex::build(&[item], &[]);
            assert!(
                index
                    .search("Alternate Console", &[], 10)
                    .iter()
                    .any(|hit| hit.item.name == "Primary Launcher"),
                "another installed shortcut name must still find the same target"
            );
        }
    }

    #[test]
    fn compact_command_alias_merges_into_friendly_application() {
        let friendly = AppItem::scanned(
            "friendly".into(),
            "Media Player".into(),
            r"shell:AppsFolder\MediaPlayer".into(),
            None,
            None,
            "uwp",
        );
        let command = AppItem::scanned(
            "command".into(),
            "MediaPlayer".into(),
            r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\MediaPlayer.exe".into(),
            None,
            None,
            commands::COMMAND_SOURCE,
        );
        let mut items = vec![(friendly, None), (command, None)];
        merge_command_name_duplicates(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0.name, "Media Player");
        assert!(items[0]
            .0
            .search_keywords
            .iter()
            .any(|keyword| keyword == "MediaPlayer"));
    }

    #[test]
    fn uninstall_display_name_wins_and_executable_alias_remains_searchable() {
        let target = r"C:\Apps\Editor\editor.exe";
        let app_path = AppItem::scanned(
            "app-path".into(),
            "editor".into(),
            target.into(),
            None,
            None,
            "app-paths",
        );
        let uninstall = AppItem::scanned(
            "uninstall".into(),
            "Acme Document Editor".into(),
            target.into(),
            None,
            None,
            "uninstall",
        );
        let mut items = dedupe(vec![(app_path, None), (uninstall, None)]);
        assert_eq!(items.len(), 1);
        let mut item = items.pop().unwrap().0;
        assert_eq!(item.name, "Acme Document Editor");
        item.attach_search_fields();
        let index = crate::search::RetrievalIndex::build(&[item], &[]);
        for query in ["acme", "editor"] {
            assert_eq!(index.search(query, &[], 10).len(), 1);
        }
    }

    #[test]
    fn apps_folder_classic_does_not_duplicate_builtin_system_entry() {
        let app = AppItem::scanned(
            "apps-folder-control-panel".into(),
            "控制面板".into(),
            "shell:AppsFolder\\Microsoft.Windows.ControlPanel".into(),
            None,
            None,
            "apps-folder",
        );
        let builtin = AppItem::scanned(
            "builtin-control-panel".into(),
            "控制面板".into(),
            "shell:ControlPanelFolder".into(),
            None,
            None,
            "builtin-system",
        );
        let mut apps = vec![(app, None)];
        exclude_system_name_duplicates(&mut apps, &[builtin]);
        assert!(
            apps.is_empty(),
            "same-named AppsFolder row duplicates an existing system entry"
        );
    }

    #[test]
    fn discovery_same_target_is_absorbed_into_formal_row() {
        let target = r"C:\Program Files\Example\example.exe";
        let formal = AppItem::scanned(
            "formal".into(),
            "Example App".into(),
            target.into(),
            None,
            None,
            "start-menu",
        );
        let discovery = AppItem::scanned(
            "discovery".into(),
            "example".into(),
            target.into(),
            None,
            None,
            "app-paths",
        );
        let mut items = vec![(formal, None), (discovery, None)];
        absorb_discovery_rows(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0.source, "start-menu");
        assert!(items[0]
            .0
            .search_keywords
            .iter()
            .any(|kw| kw.eq_ignore_ascii_case("example")));
    }

    #[test]
    fn discovery_same_install_and_stem_family_is_absorbed() {
        let formal = AppItem::scanned(
            "formal".into(),
            "WPS Office".into(),
            r"D:\Program Files\WPS Office\ksolaunch.exe".into(),
            None,
            None,
            "start-menu",
        );
        let discovery = AppItem::scanned(
            "discovery".into(),
            "wps".into(),
            r"D:\Program Files\WPS Office\12.1.0\office6\wps.exe".into(),
            None,
            None,
            "app-paths",
        );
        let mut items = vec![(formal, None), (discovery, None)];
        absorb_discovery_rows(&mut items);
        assert_eq!(
            items.len(),
            1,
            "same install product should absorb app-paths, got {:?}",
            items.iter().map(|(i, _)| (&i.name, &i.source)).collect::<Vec<_>>()
        );
        assert_eq!(items[0].0.source, "start-menu");
    }

    #[test]
    fn unabsorbed_discovery_fallback_row_is_kept() {
        let formal = AppItem::scanned(
            "formal".into(),
            "Chrome".into(),
            r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
            None,
            None,
            "start-menu",
        );
        let discovery = AppItem::scanned(
            "discovery".into(),
            "ExamplePlayer".into(),
            r"C:\Apps\Example\ExamplePlayer.exe".into(),
            None,
            None,
            "app-paths",
        );
        let mut items = vec![(formal, None), (discovery, None)];
        absorb_discovery_rows(&mut items);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|(i, _)| i.source == "app-paths"));
    }

    #[test]
    fn discovery_with_different_launch_args_is_not_absorbed() {
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
            "discovery".into(),
            "powershell".into(),
            target.into(),
            Some("-NoExit -Command Enter-VsDevShell".into()),
            None,
            "app-paths",
        );
        let mut items = vec![(formal, None), (discovery, None)];
        absorb_discovery_rows(&mut items);
        assert_eq!(
            items.len(),
            2,
            "different launch args must stay separate rows"
        );
    }

    #[test]
    fn discovery_is_not_absorbed_into_command_alias() {
        let target = r"C:\Apps\Example\example.exe";
        let command = AppItem::scanned(
            "cmd".into(),
            "Example".into(),
            target.into(),
            None,
            None,
            "commands",
        );
        let discovery = AppItem::scanned(
            "discovery".into(),
            "Example App".into(),
            target.into(),
            None,
            None,
            "app-paths",
        );
        let mut items = vec![(command, None), (discovery, None)];
        absorb_discovery_rows(&mut items);
        assert_eq!(items.len(), 2, "commands are not absorption hosts");
        assert!(items.iter().any(|(i, _)| i.source == "app-paths"));
    }

    #[test]
    fn shell_apps_folder_path_merges_with_direct_exe() {
        let path = r"C:\Program Files\Adobe\Adobe Lightroom Classic\Lightroom.exe";
        let direct = AppItem::scanned(
            "direct".into(),
            "Adobe Lightroom Classic".into(),
            path.into(),
            None,
            None,
            "start-menu",
        );
        let shell = AppItem::scanned(
            "shell".into(),
            "Adobe Lightroom Classic".into(),
            format!(r"shell:AppsFolder\{path}"),
            None,
            None,
            "apps-folder",
        );
        let items = dedupe(vec![(direct, None), (shell, None)]);
        assert_eq!(
            items.len(),
            1,
            "shell-wrapped absolute path must dedupe with direct exe"
        );
        assert_eq!(items[0].0.source, "start-menu");
    }

    #[test]
    fn apps_folder_aumid_row_merges_into_same_name_path_entry() {
        let path = AppItem::scanned(
            "path".into(),
            "Application Verifier (WOW)".into(),
            r"C:\Windows\SysWOW64\appverif.exe".into(),
            None,
            None,
            "start-menu",
        );
        let shell = AppItem::scanned(
            "shell".into(),
            "Application Verifier (WOW)".into(),
            r"shell:AppsFolder\{D65231B0-1234-4E5E-A8E7C6EA7D27}\appverif.exe".into(),
            None,
            None,
            "apps-folder",
        );
        let mut items = vec![(shell, None), (path, None)];
        merge_same_name_formal_duplicates(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0.source, "start-menu");
        assert_eq!(items[0].0.name, "Application Verifier (WOW)");
    }

    #[test]
    fn desktop_and_start_menu_same_name_url_merge() {
        let start = AppItem::scanned(
            "sm".into(),
            "Dying Light".into(),
            r"C:\Users\admin\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Steam\Dying Light.url".into(),
            None,
            None,
            "start-menu",
        );
        let desk = AppItem::scanned(
            "desk".into(),
            "Dying Light".into(),
            r"C:\Users\admin\Desktop\Games\Dying Light.url".into(),
            None,
            None,
            "desktop",
        );
        let mut items = vec![(desk, None), (start, None)];
        merge_same_name_formal_duplicates(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0.source, "start-menu");
    }

    #[test]
    fn store_uwp_without_path_twin_is_kept() {
        let store = AppItem::scanned(
            "store".into(),
            "Contoso App".into(),
            r"shell:AppsFolder\Contoso.App_abc!App".into(),
            None,
            None,
            "uwp",
        );
        let mut items = vec![(store, None)];
        merge_same_name_formal_duplicates(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0.source, "uwp");
    }

    #[test]
    fn chrome_aumid_and_start_menu_merge() {
        let start = AppItem::scanned(
            "sm".into(),
            "Google Chrome".into(),
            r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
            None,
            None,
            "start-menu",
        );
        let shell = AppItem::scanned(
            "shell".into(),
            "Google Chrome".into(),
            r"shell:AppsFolder\Chrome".into(),
            None,
            None,
            "apps-folder",
        );
        let mut items = vec![(shell, None), (start, None)];
        merge_same_name_formal_duplicates(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0.source, "start-menu");
    }

    #[test]
    fn same_name_different_targets_are_both_kept() {
        let wechat = AppItem::scanned(
            "a".into(),
            "微信".into(),
            r"C:\A\WeChat\WeChat.exe".into(),
            None,
            None,
            "start-menu",
        );
        let weixin = AppItem::scanned(
            "b".into(),
            "微信".into(),
            r"C:\B\Weixin\Weixin.exe".into(),
            None,
            None,
            "start-menu",
        );
        let mut items = vec![(wechat, None), (weixin, None)];
        merge_same_name_formal_duplicates(&mut items);
        assert_eq!(
            items.len(),
            2,
            "同名不同安装根不得合并: {:?}",
            items.iter().map(|(i, _)| &i.target).collect::<Vec<_>>()
        );
    }

    /// 本机 formal 归并 dry-run：`cargo test --lib dryrun_formal -- --ignored --nocapture`
    /// 用于在收紧 shell 同名吸收前，先看真实 Start Menu/Desktop/UWP 会不会错合并。
    #[test]
    #[ignore = "本机数据 dry-run，不进默认 CI"]
    fn dryrun_formal_pair_merges_on_this_machine() {
        use super::super::cache::ScanCache;
        use super::super::pass::{
            FULL_MAX_PER_DIR, FULL_MAX_TOTAL, FULL_OTHER_SOURCE_MAX_DEPTH, FULL_START_MENU_MAX_DEPTH,
        };
        use super::super::walk::collect_from_dir;
        use std::collections::BTreeMap;
        use std::path::PathBuf;
        use std::time::Instant;

        let icon_dir = std::env::temp_dir().join("kite-formal-merge-dryrun");
        let mut cache = ScanCache::load(&icon_dir);
        let mut items: Vec<RawItem> = Vec::new();
        let t0 = Instant::now();

        let user_start = crate::app::scanner::util::user_start_menu_dir()
            .unwrap_or_else(|| {
                dirs::data_dir()
                    .unwrap_or_default()
                    .join("Microsoft/Windows/Start Menu")
            });
        let common_start = crate::app::scanner::util::common_start_menu_dir()
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu"));
        let roots: [(PathBuf, &str); 4] = [
            (user_start, "start-menu"),
            (common_start, "start-menu"),
            (dirs::desktop_dir().unwrap_or_default(), "desktop"),
            (PathBuf::from(r"C:\Users\Public\Desktop"), "desktop"),
        ];
        for (root, source) in roots {
            let depth = if source == "start-menu" {
                FULL_START_MENU_MAX_DEPTH
            } else {
                FULL_OTHER_SOURCE_MAX_DEPTH
            };
            collect_from_dir(
                &root,
                source,
                depth,
                None,
                t0,
                FULL_MAX_PER_DIR,
                FULL_MAX_TOTAL,
                &mut items,
                &mut cache,
            );
        }
        crate::app::uwp::collect_uwp("uwp", &mut items);

        println!("=== formal dry-run: collected {} raw items ===", items.len());

        // 只看 formal 源的同名组
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, (item, _)) in items.iter().enumerate() {
            if formal_source_rank(&item.source).is_none() {
                continue;
            }
            let key = compact_item_name(item);
            if key.is_empty() {
                continue;
            }
            groups.entry(key).or_default().push(i);
        }

        let multi: Vec<_> = groups.into_values().filter(|v| v.len() >= 2).collect();
        println!("same-name formal groups (n>=2): {}", multi.len());

        let mut would_merge = 0usize;
        let mut keep_both = 0usize;
        let mut high_risk = 0usize;
        let mut diff_target_merges = 0usize;
        for indices in &multi {
            // 按与 merge 相同的 winner 规则
            let Some(&winner) = indices.iter().min_by(|&&a, &&b| {
                formal_source_rank(&items[a].0.source)
                    .unwrap_or(u8::MAX)
                    .cmp(&formal_source_rank(&items[b].0.source).unwrap_or(u8::MAX))
                    .then_with(|| items[a].0.id.cmp(&items[b].0.id))
            }) else {
                continue;
            };
            let mut lines = Vec::new();
            lines.push(format!(
                "GROUP name={} winner=[{}] {} -> {} args={:?} id={}",
                items[winner].0.name,
                items[winner].0.source,
                items[winner].0.id,
                items[winner].0.target,
                items[winner].0.args,
                launch_identity(&items[winner].0.target, items[winner].0.args.as_deref())
            ));
            for &loser in indices {
                if loser == winner {
                    continue;
                }
                let merges = formal_pair_merges(&items[winner].0, &items[loser].0);
                let shell_pair = is_shell_app_source(&items[winner].0.source)
                    != is_shell_app_source(&items[loser].0.source);
                if merges {
                    would_merge += 1;
                } else {
                    keep_both += 1;
                }
                if shell_pair && merges {
                    high_risk += 1;
                }
                if merges
                    && normalize_path_key(&items[winner].0.target)
                        != normalize_path_key(&items[loser].0.target)
                {
                    diff_target_merges += 1;
                }
                lines.push(format!(
                    "  {} [{}] {} -> {} args={:?} id={}{}",
                    if merges { "MERGE " } else { "KEEP  " },
                    items[loser].0.source,
                    items[loser].0.id,
                    items[loser].0.target,
                    items[loser].0.args,
                    launch_identity(&items[loser].0.target, items[loser].0.args.as_deref()),
                    if shell_pair { "  (shell+path)" } else { "" }
                ));
            }
            // 打印：任意 KEEP、任意 shell+path、或任意不同 target 的 MERGE（错合并候选）
            let any_keep = indices
                .iter()
                .any(|&l| l != winner && !formal_pair_merges(&items[winner].0, &items[l].0));
            let any_shell_merge = indices.iter().any(|&l| {
                l != winner
                    && is_shell_app_source(&items[winner].0.source)
                        != is_shell_app_source(&items[l].0.source)
                    && formal_pair_merges(&items[winner].0, &items[l].0)
            });
            let any_diff_target_merge = indices.iter().any(|&l| {
                l != winner
                    && formal_pair_merges(&items[winner].0, &items[l].0)
                    && normalize_path_key(&items[winner].0.target)
                        != normalize_path_key(&items[l].0.target)
            });
            if any_keep || any_shell_merge || any_diff_target_merge {
                for line in &lines {
                    println!("{line}");
                }
            }
        }
        println!(
            "summary: groups={} merge_pairs={} keep_pairs={} shell_path_merges={} diff_target_merges={}",
            multi.len(),
            would_merge,
            keep_both,
            high_risk,
            diff_target_merges
        );

        let mut after = items.clone();
        merge_same_name_formal_duplicates(&mut after);
        println!(
            "merge_same_name_formal_duplicates: {} -> {}",
            items.len(),
            after.len()
        );

        // 断言只是防静默失败；真实结论看 --nocapture 输出
        assert!(!items.is_empty(), "本机 formal 扫描不应为空");
    }
}
