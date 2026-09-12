//! UWP / Store 应用：枚举 shell:AppsFolder（对齐 LaunchyQt UWPApp）。
//! 过滤：System.Launcher.AppState 非空；用 AUMID + ActivationManager 启动。

use std::collections::HashSet;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, IBindCtx,
};
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PropVariantToStringAlloc, PROPVARIANT,
};
use windows::Win32::System::Variant::{VT_BSTR, VT_EMPTY, VT_LPWSTR};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PSGetPropertyKeyFromName};
use windows::Win32::UI::Shell::{
    ApplicationActivationManager, BHID_EnumItems, BHID_PropertyStore, IApplicationActivationManager,
    IEnumShellItems, IShellItem, SHCreateItemFromParsingName, AO_NONE, SIGDN_NORMALDISPLAY,
};

use super::scanner::util::{hash_id, normalize_path_key};
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

        let props: IPropertyStore = match unsafe {
            item.BindToHandler(None::<&IBindCtx>, &BHID_PropertyStore)
        } {
            Ok(p) => p,
            Err(_) => continue,
        };

        // 真 UWP 才有非空 Launcher.AppState
        let Some(state) = prop_string(&props, &pk_app_state) else {
            continue;
        };
        if state.trim().is_empty() {
            continue;
        }

        let name = display_name(&item).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
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
        let id = hash_id(&[&normalize_path_key(&target), source]);
        let item = AppItem::scanned(id, name, target, None, None, source);
        out.push((item, icon_src));
    }
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
    unsafe {
        let p: PWSTR = item.GetDisplayName(SIGDN_NORMALDISPLAY).ok()?;
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

/// 启动 shell:AppsFolder / 普通路径 / UWP AUMID。
pub fn launch_shell_path(target: &str) -> Result<(), String> {
    if let Some(aumid) = target.strip_prefix("shell:AppsFolder\\") {
        if !aumid.is_empty() {
            return activate_uwp(aumid);
        }
    }

    let file: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let code = unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            PCWSTR::null(),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    if code.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecute failed for {target}"))
    }
}

fn activate_uwp(aumid: &str) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let aam: IApplicationActivationManager =
            CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("ActivationManager: {e}"))?;
        let wide_aumid = wide(aumid);
        aam.ActivateApplication(
            PCWSTR(wide_aumid.as_ptr()),
            PCWSTR::null(),
            AO_NONE,
        )
        .map_err(|e| format!("ActivateApplication: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_icon_empty() {
        assert!(resolve_uwp_icon("", "").is_none());
        assert!(resolve_uwp_icon(r"C:\nope", "").is_none());
    }

    #[test]
    fn strip_aumid_prefix() {
        assert_eq!(
            "shell:AppsFolder\\Microsoft.WindowsStore_8wekyb3d8bbwe!App"
                .strip_prefix("shell:AppsFolder\\"),
            Some("Microsoft.WindowsStore_8wekyb3d8bbwe!App")
        );
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
            picked(&["Logo.scale-100.png", "Logo.scale-200.png", "Logo.scale-400.png"]).as_deref(),
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
