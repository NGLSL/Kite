//! 入口身份去重与命令别名归并。

use std::collections::HashMap;

use crate::model::AppItem;

use super::commands;
use super::util::{hash_id, normalize_path_key};
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

/// 同一 target 的不同参数可能代表不同的启动语义（例如普通 PowerShell
/// 与 Developer PowerShell），不能仅按 exe 路径合并。工作目录不参与
/// 去重：开始菜单、桌面快捷方式经常只是在快捷方式元数据中提供了
/// 不同的起始位置，而启动器会将无效或未提供的目录统一回落到用户主目录。
fn dedupe_key(item: &AppItem) -> String {
    let target = normalize_path_key(&item.target);
    hash_id(&[&target, item.args.as_deref().unwrap_or("")])
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
}
