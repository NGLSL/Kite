//! 枚举 shell:AppsFolder 中的 Store 应用和经典系统工具。
//! Store 应用使用 AUMID；经典入口使用 Shell 已验证的解析路径。

mod activate;

pub use activate::{launch_runas, launch_shell_path};

use std::collections::HashSet;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PropVariantToStringAlloc, PROPVARIANT,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::{VT_BSTR, VT_EMPTY, VT_LPWSTR};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PSGetPropertyKeyFromName};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, BHID_PropertyStore, IEnumShellItems, IShellItem, SHCreateItemFromParsingName,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};

use super::scanner::util::stable_item_id;
use crate::model::AppItem;

type RawItem = (AppItem, Option<String>);

/// 补充 Store / 系统 UWP 应用（失败静默）。
pub fn collect_uwp(source: &str, out: &mut Vec<RawItem>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        collect_apps_folder(source, out)
    }));
    if result.is_err() {
        crate::log::info("uwp collect panicked; skip");
    }
    unsafe {
        let _ = CoUninitialize();
    }
}

fn collect_apps_folder(source: &str, out: &mut Vec<RawItem>) {
    let mut seen: HashSet<String> = HashSet::new();

    let folder: IShellItem = unsafe {
        match SHCreateItemFromParsingName(
            PCWSTR(wide("shell:AppsFolder").as_ptr()),
            None::<&IBindCtx>,
        ) {
            Ok(f) => f,
            Err(e) => {
                crate::log::info(&format!("uwp: open AppsFolder failed: {e}"));
                return;
            }
        }
    };

    let enum_items: IEnumShellItems = unsafe {
        match folder.BindToHandler(None::<&IBindCtx>, &BHID_EnumItems) {
            Ok(e) => e,
            Err(e) => {
                crate::log::info(&format!("uwp: bind enum failed: {e}"));
                return;
            }
        }
    };

    let pk_app_state = prop_key("System.Launcher.AppState");
    let pk_aumid = prop_key("System.AppUserModel.ID");
    let pk_logo = prop_key("System.Tile.SmallLogoPath");
    let pk_install = prop_key("System.AppUserModel.PackageInstallPath");

    loop {
        let mut fetched = 0u32;
        let mut slot: Option<IShellItem> = None;
        let ok = unsafe {
            enum_items
                .Next(std::slice::from_mut(&mut slot), Some(&mut fetched))
                .is_ok()
        };
        if !ok || fetched == 0 {
            break;
        }
        let Some(item) = slot else { continue };

        let name = display_name(&item).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let props: Option<IPropertyStore> = unsafe {
            item.BindToHandler(None::<&IBindCtx>, &BHID_PropertyStore)
                .ok()
        };
        let state = props
            .as_ref()
            .and_then(|props| prop_string(props, &pk_app_state));
        // Classic AppsFolder items have no UWP launcher state. Keep only Shell
        // targets that resolve, and avoid a second row for the same name.
        if state.as_deref().is_none_or(|state| state.trim().is_empty()) {
            if is_auxiliary_display_name(&name) {
                continue;
            }
            let Some(target) = classic_shell_target(&item) else {
                continue;
            };
            if out
                .iter()
                .any(|(existing, _)| existing.name.eq_ignore_ascii_case(&name))
                || !seen.insert(target.to_lowercase())
            {
                continue;
            }
            let id = stable_item_id(&target, None);
            let classic = AppItem::scanned(id, name, target.clone(), None, None, "apps-folder");
            out.push((classic, Some(target)));
            continue;
        }

        let Some(props) = props else { continue };
        let Some(aumid) = prop_string(&props, &pk_aumid) else {
            continue;
        };
        if aumid.is_empty() || !seen.insert(aumid.to_lowercase()) {
            continue;
        }

        let install = prop_string(&props, &pk_install).unwrap_or_default();
        let logo = prop_string(&props, &pk_logo).unwrap_or_default();
        let icon_src = resolve_uwp_icon(&install, &logo);

        let target = format!("shell:AppsFolder\\{aumid}");
        let id = stable_item_id(&target, None);
        let item = AppItem::scanned(id, name, target, None, None, source);
        out.push((item, icon_src));
    }
}

fn classic_shell_target(item: &IShellItem) -> Option<String> {
    let parsing_name = display_name_with_flag(item, SIGDN_DESKTOPABSOLUTEPARSING)?;
    if is_web_identifier(&parsing_name) {
        return None;
    }
    let target = format!("shell:AppsFolder\\{parsing_name}");
    let _: IShellItem = unsafe {
        SHCreateItemFromParsingName(PCWSTR(wide(&target).as_ptr()), None::<&IBindCtx>).ok()?
    };
    Some(target)
}

fn is_web_identifier(value: &str) -> bool {
    if [
        "http:",
        "https:",
        "ftp:",
        "file:",
        "mailto:",
        "data:",
        "javascript:",
    ]
    .iter()
    .any(|prefix| {
        value
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    }) {
        return true;
    }

    let lower = value.to_ascii_lowercase();
    let suffix = Path::new(&lower)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    if matches!(suffix, "url" | "html" | "htm" | "pdf" | "txt" | "chm") {
        return true;
    }
    ["uninstall", "unins", "readme", "documentation", "manual"]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn is_auxiliary_display_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "uninstall",
        "unins",
        "readme",
        "documentation",
        "manual",
        "卸载",
        "文档",
        "说明",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn prop_key(name: &str) -> PROPERTYKEY {
    let mut key = PROPERTYKEY::default();
    let w = wide(name);
    let _ = unsafe { PSGetPropertyKeyFromName(PCWSTR(w.as_ptr()), &mut key) };
    key
}

fn prop_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let mut pv: PROPVARIANT = store.GetValue(key).ok()?;
        let s = prop_variant_to_string(&pv);
        let _ = PropVariantClear(&mut pv);
        s
    }
}

fn prop_variant_to_string(pv: &PROPVARIANT) -> Option<String> {
    unsafe {
        let vt = pv.Anonymous.Anonymous.vt;
        if vt == VT_EMPTY {
            return None;
        }
        if vt == VT_LPWSTR {
            let p = pv.Anonymous.Anonymous.Anonymous.pwszVal;
            if !p.is_null() {
                return p.to_string().ok();
            }
            return None;
        }
        if vt == VT_BSTR {
            let b = &*pv.Anonymous.Anonymous.Anonymous.bstrVal;
            return Some(b.to_string());
        }
        let out = PropVariantToStringAlloc(pv).ok()?;
        if out.is_null() {
            return None;
        }
        let s = out.to_string().ok();
        let _ = CoTaskMemFree(Some(out.0 as *const _));
        s
    }
}

fn display_name(item: &IShellItem) -> Option<String> {
    display_name_with_flag(item, SIGDN_NORMALDISPLAY)
}

fn display_name_with_flag(
    item: &IShellItem,
    flag: windows::Win32::UI::Shell::SIGDN,
) -> Option<String> {
    unsafe {
        let p: PWSTR = item.GetDisplayName(flag).ok()?;
        if p.is_null() {
            return None;
        }
        let s = p.to_string().ok();
        let _ = CoTaskMemFree(Some(p.0 as *const _));
        s.filter(|x| !x.trim().is_empty())
    }
}

/// LaunchyQt 风格：安装目录里找 Tile logo；同 stem 多个资产时选分辨率最高的。
fn resolve_uwp_icon(install_path: &str, logo_rel: &str) -> Option<String> {
    if install_path.is_empty() || logo_rel.is_empty() {
        return None;
    }
    let base = PathBuf::from(install_path);
    let rel = logo_rel.replace('/', "\\");
    let full = base.join(&rel);
    let parent = full.parent()?;
    let stem = full.file_stem()?.to_string_lossy().to_string();

    let rd = std::fs::read_dir(parent).ok()?;
    let candidates: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .map(|x| x.eq_ignore_ascii_case("png"))
                    .unwrap_or(false)
                && p.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .map(|s| s.starts_with(&stem))
                    .unwrap_or(false)
        })
        .collect();
    pick_asset(&candidates).map(|p| p.to_string_lossy().to_string())
}

/// 从同 stem 的资产变体里挑最佳：分辨率最高 → 非高对比度 → 文件名短（变体后缀少）。
/// 纯函数便于测试；scale-N/targetsize-N 解析为名义像素。
fn pick_asset(candidates: &[PathBuf]) -> Option<&PathBuf> {
    fn score(p: &Path) -> (u32, u8, usize) {
        let name = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        // scale-200 → 200；targetsize-256 → 256；无标注按 100
        let mut px = 100u32;
        for tag in name.split('.') {
            if let Some(n) = tag.strip_prefix("scale-") {
                if let Ok(v) = n.parse::<u32>() {
                    px = px.max(v);
                }
            } else if let Some(n) = tag.strip_prefix("targetsize-") {
                if let Ok(v) = n.parse::<u32>() {
                    px = px.max(v);
                }
            }
        }
        // contrast-black/white 是无障碍变体，仅在别无选择时使用
        let contrast = u8::from(name.contains("contrast-"));
        (px, contrast, name.len())
    }
    candidates.iter().max_by(|a, b| {
        let (pa, ca, la) = score(a);
        let (pb, cb, lb) = score(b);
        // 标准外观优先于无障碍变体,再取分辨率最高,同分取名字短的
        cb.cmp(&ca)
            .then_with(|| pa.cmp(&pb))
            .then_with(|| lb.cmp(&la))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websites_in_apps_folder_are_not_software() {
        assert!(is_web_identifier("https://example.com"));
        assert!(is_web_identifier("HTTP://example.com"));
        assert!(is_web_identifier("javascript:alert(1)"));
        assert!(!is_web_identifier(
            "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\charmap.exe"
        ));
        assert!(is_web_identifier(
            r"D:\Program Files\SDK\DesktopAppsDocumentation.url"
        ));
        assert!(is_web_identifier(r"D:\Program Files\Tool\ReadMe.pdf"));
        assert!(is_web_identifier(r"D:\Program Files\Tool\uninstall.exe"));
        assert!(is_auxiliary_display_name("英雄联盟卸载"));
    }

    #[test]
    fn classic_apps_folder_item_is_searchable_by_shell_name() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let folder: IShellItem = unsafe {
            SHCreateItemFromParsingName(
                PCWSTR(wide("shell:AppsFolder").as_ptr()),
                None::<&IBindCtx>,
            )
            .unwrap()
        };
        let entries: IEnumShellItems = unsafe {
            folder
                .BindToHandler(None::<&IBindCtx>, &BHID_EnumItems)
                .unwrap()
        };
        let mut expected = None;
        loop {
            let mut fetched = 0;
            let mut slot: Option<IShellItem> = None;
            if unsafe { entries.Next(std::slice::from_mut(&mut slot), Some(&mut fetched)) }.is_err()
                || fetched == 0
            {
                break;
            }
            if let Some(shell_item) = slot {
                if let Some(parsing) =
                    display_name_with_flag(&shell_item, SIGDN_DESKTOPABSOLUTEPARSING)
                {
                    if parsing.to_ascii_lowercase().ends_with("cleanmgr.exe") {
                        expected = Some((
                            display_name(&shell_item).unwrap(),
                            classic_shell_target(&shell_item).unwrap(),
                        ));
                        break;
                    }
                }
            }
        }
        let (expected_name, expected_target) =
            expected.expect("Disk Cleanup visible in AppsFolder");
        let mut raw = Vec::new();
        collect_uwp("uwp", &mut raw);
        let mut item = raw
            .into_iter()
            .map(|(item, _)| item)
            .find(|item| item.target.eq_ignore_ascii_case(&expected_target))
            .expect("classic AppsFolder item must enter the index");
        assert_eq!(item.name, expected_name);
        item.attach_search_fields();
        let index = crate::search::RetrievalIndex::build(&[item], &[]);
        assert!(index
            .search(&expected_name, &[], 10)
            .iter()
            .any(|hit| hit.item.name == expected_name));
        unsafe {
            CoUninitialize();
        }
    }

    #[test]
    fn resolve_icon_empty() {
        assert!(resolve_uwp_icon("", "").is_none());
        assert!(resolve_uwp_icon(r"C:\nope", "").is_none());
    }

    fn names(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn picked(paths: &[&str]) -> Option<String> {
        pick_asset(&names(paths)).map(|p| p.to_string_lossy().to_string())
    }

    #[test]
    fn picks_highest_scale() {
        assert_eq!(
            picked(&[
                "Logo.scale-100.png",
                "Logo.scale-200.png",
                "Logo.scale-400.png"
            ])
            .as_deref(),
            Some("Logo.scale-400.png")
        );
    }

    #[test]
    fn picks_largest_targetsize() {
        assert_eq!(
            picked(&[
                "Logo.targetsize-16.png",
                "Logo.targetsize-24.png",
                "Logo.targetsize-256.png"
            ])
            .as_deref(),
            Some("Logo.targetsize-256.png")
        );
    }

    #[test]
    fn skips_contrast_variant_when_alternative_exists() {
        assert_eq!(
            picked(&[
                "Logo.contrast-black_scale-100.png",
                "Logo.contrast-white_scale-400.png",
                "Logo.scale-100.png"
            ])
            .as_deref(),
            Some("Logo.scale-100.png")
        );
    }

    #[test]
    fn falls_back_to_contrast_when_only_variant() {
        assert_eq!(
            picked(&["Logo.contrast-black.png"]).as_deref(),
            Some("Logo.contrast-black.png")
        );
    }

    #[test]
    fn plain_logo_wins_over_unplated_tie() {
        // 同为 scale-100：altform-unplated 名字更长,退居其次
        assert_eq!(
            picked(&["Logo.altform-unplated_scale-100.png", "Logo.scale-100.png"]).as_deref(),
            Some("Logo.scale-100.png")
        );
    }

    #[test]
    fn scale_beats_targetsize_of_same_number() {
        assert_eq!(
            picked(&["Logo.targetsize-44.png", "Logo.scale-200.png"]).as_deref(),
            Some("Logo.scale-200.png")
        );
    }

    #[test]
    fn empty_candidates() {
        assert!(pick_asset(&[]).is_none());
    }
}
