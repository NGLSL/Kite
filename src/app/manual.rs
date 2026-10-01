//! Validation and materialisation of user-registered application entries.
//!
//! Manual entries deliberately use the same `AppItem` launch contract as the
//! scanner.  The persisted path is the original `.exe`/`.lnk` entry, while
//! the returned item contains the currently verified launch target.  This
//! lets a later scan notice a shortcut whose target changed without treating
//! the old identity as the new one.

use std::path::Path;

use crate::model::AppItem;

/// A validated persisted entry and the launch item it produced.
#[derive(Debug, Clone)]
pub struct ValidatedManualApp {
    /// The user supplied entry path after the shared path validator removed
    /// optional surrounding quotes.
    pub path: String,
    /// The currently verified launch item.  Its `id` is based on target and
    /// arguments, never on the manual record id or display name.
    pub item: AppItem,
}

/// Validate and materialise a user-registered `.exe` or `.lnk` entry.
///
/// The path is passed through the same absolute-path validator used by direct
/// path results.  No user text is placed in a shell command or launch
/// arguments.  An `.lnk` must resolve to a launchable target which still
/// exists when it is a filesystem path; shell and namespace targets are
/// accepted by the existing launcher contract.
pub fn validate_entry(path: &str, display_name: &str) -> Result<ValidatedManualApp, String> {
    let entry = crate::app::actions::direct_path_candidate(path)
        .ok_or_else(|| "请输入有效的绝对路径".to_owned())?;
    if !entry.is_file() {
        return Err(format!("目标文件不存在: {}", entry.display()));
    }

    let extension = entry
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .ok_or_else(|| "只支持 .exe 或 .lnk 文件".to_owned())?;

    let entry_path = entry.to_string_lossy().into_owned();
    let fallback_name = entry_stem(&entry).ok_or_else(|| "无法确定应用名称".to_owned())?;
    let name = display_name.trim().to_owned();
    let name = if name.is_empty() { fallback_name } else { name };

    let (target, args, working_dir, icon_src, is_lnk) = match extension.as_str() {
        "exe" => {
            // `entry.is_file()` above is the same existence check used by
            // direct path results; the extension is the scanner's executable
            // contract and does not require parsing PE metadata.
            (
                entry_path.clone(),
                None,
                None,
                Some(entry_path.clone()),
                false,
            )
        }
        "lnk" => {
            let (target, args, working_dir, icon_src) = crate::app::scanner::resolve_lnk(&entry)
                .ok_or_else(|| "无法解析快捷方式".to_owned())?;
            validate_lnk_target(&target)?;
            (target, args, working_dir, icon_src, true)
        }
        _ => return Err("只支持 .exe 或 .lnk 文件".to_owned()),
    };

    let id = crate::app::scanner::util::stable_item_id(&target, args.as_deref());
    let mut item = AppItem::scanned(id, name, target, args, working_dir, "manual");
    item.is_lnk = is_lnk;
    item.icon_src = icon_src;
    item.attach_search_fields();

    Ok(ValidatedManualApp {
        path: entry_path,
        item,
    })
}

fn entry_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().trim().to_owned())
        .filter(|stem| !stem.is_empty())
}

fn validate_lnk_target(target: &str) -> Result<(), String> {
    if !crate::app::scanner::util::is_launchable_lnk_target(target) {
        return Err("快捷方式目标不是可启动入口".to_owned());
    }

    let trimmed = target.trim();
    if trimmed.starts_with("shell:") || trimmed.starts_with("::") {
        return Ok(());
    }

    let target_path = crate::app::actions::direct_path_candidate(trimmed)
        .ok_or_else(|| "快捷方式目标不是有效的绝对路径".to_owned())?;
    if target_path.is_file() {
        Ok(())
    } else {
        Err(format!("快捷方式目标不存在: {}", target_path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use windows::core::{Interface, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kite-manual-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn canonical_fixture_path(path: &Path) -> PathBuf {
        let canonical = std::fs::canonicalize(path).unwrap();
        let canonical = canonical.to_string_lossy();
        PathBuf::from(canonical.strip_prefix(r"\\?\").unwrap_or(&canonical))
    }

    #[test]
    fn validates_exe_and_uses_stem_when_name_is_empty() {
        let dir = temp_dir("exe");
        let exe = dir.join("中文 Editor.exe");
        std::fs::write(&exe, b"fixture").unwrap();

        let app = validate_entry(&format!("\"{}\"", exe.display()), " ").unwrap();
        assert_eq!(app.path, exe.to_string_lossy());
        assert_eq!(app.item.name, "中文 Editor");
        assert_eq!(app.item.display_name, "中文 Editor");
        assert_eq!(app.item.target, exe.to_string_lossy());
        assert_eq!(app.item.source, "manual");
        assert_eq!(app.item.args, None);
        assert!(!app.item.is_lnk);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn custom_name_and_slash_variants_keep_one_launch_identity() {
        let dir = temp_dir("identity");
        let exe = dir.join("same.exe");
        std::fs::write(&exe, b"fixture").unwrap();
        let backslash = exe.to_string_lossy().into_owned();
        let slash = backslash.replace('\\', "/");

        let first = validate_entry(&backslash, "我的编辑器").unwrap();
        let second = validate_entry(&slash, "另一个名字").unwrap();
        assert_eq!(first.item.id, second.item.id);
        assert_eq!(first.item.name, "我的编辑器");
        assert_eq!(second.item.name, "另一个名字");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_non_absolute_missing_and_non_application_paths() {
        assert!(validate_entry("relative.exe", "App").is_err());
        assert!(validate_entry(r"\\?\C:\app.exe", "App").is_err());

        let dir = temp_dir("reject");
        let text = dir.join("readme.txt");
        std::fs::write(&text, b"fixture").unwrap();
        assert!(validate_entry(&text.to_string_lossy(), "Readme").is_err());
        assert!(validate_entry(&dir.join("missing.exe").to_string_lossy(), "Missing").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn write_shortcut(
        path: &Path,
        target: &Path,
        args: &str,
        working_dir: &Path,
        icon_source: &Path,
    ) -> windows::core::Result<()> {
        let to_wide = |value: &Path| {
            use std::os::windows::ffi::OsStrExt;

            value
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>()
        };
        let to_wide_str = |value: &str| {
            use std::os::windows::ffi::OsStrExt;

            std::ffi::OsStr::new(value)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>()
        };

        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        let result = unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let target = to_wide(target);
            let args = to_wide_str(args);
            let working_dir = to_wide(working_dir);
            let icon_source = to_wide(icon_source);
            link.SetPath(PCWSTR(target.as_ptr()))?;
            link.SetArguments(PCWSTR(args.as_ptr()))?;
            link.SetWorkingDirectory(PCWSTR(working_dir.as_ptr()))?;
            link.SetIconLocation(PCWSTR(icon_source.as_ptr()), 0)?;
            let persist: IPersistFile = link.cast()?;
            let path = to_wide(path);
            persist.Save(PCWSTR(path.as_ptr()), true)?;
            Ok(())
        };
        if initialized {
            unsafe { CoUninitialize() };
        }
        result
    }

    #[test]
    fn validates_real_lnk_metadata_and_recomputes_identity_after_retarget() {
        let dir = canonical_fixture_path(&temp_dir("real-lnk"));
        let first_target = dir.join("first-target.exe");
        let second_target = dir.join("second-target.exe");
        let first_cwd = dir.join("first-cwd");
        let second_cwd = dir.join("second-cwd");
        let shortcut = dir.join("registered shortcut.lnk");
        for path in [&first_target, &second_target] {
            std::fs::write(path, b"fixture").unwrap();
        }
        std::fs::create_dir_all(&first_cwd).unwrap();
        std::fs::create_dir_all(&second_cwd).unwrap();

        write_shortcut(
            &shortcut,
            &first_target,
            r#"--profile "first profile""#,
            &first_cwd,
            &first_target,
        )
        .expect("Windows COM should create the fixture shortcut");
        let first = validate_entry(&shortcut.to_string_lossy(), "Registered shortcut").unwrap();
        assert_eq!(first.item.target, first_target.to_string_lossy());
        assert_eq!(
            first.item.args.as_deref(),
            Some(r#"--profile "first profile""#)
        );
        assert_eq!(
            first.item.working_dir,
            Some(first_cwd.to_string_lossy().into_owned())
        );
        assert!(first.item.is_lnk);
        assert_eq!(
            first.item.icon_src,
            Some(format!("{},0", first_target.to_string_lossy()))
        );
        assert_eq!(
            first.item.id,
            crate::app::scanner::util::stable_item_id(
                &first_target.to_string_lossy(),
                Some(r#"--profile "first profile""#),
            )
        );

        write_shortcut(
            &shortcut,
            &second_target,
            "--profile second",
            &second_cwd,
            &second_target,
        )
        .expect("Windows COM should retarget the fixture shortcut");
        let second = validate_entry(&shortcut.to_string_lossy(), "Registered shortcut").unwrap();
        assert_eq!(second.item.target, second_target.to_string_lossy());
        assert_eq!(second.item.args.as_deref(), Some("--profile second"));
        assert_eq!(
            second.item.working_dir,
            Some(second_cwd.to_string_lossy().into_owned())
        );
        assert_eq!(
            second.item.icon_src,
            Some(format!("{},0", second_target.to_string_lossy()))
        );
        assert_ne!(second.item.id, first.item.id);
        assert_ne!(
            second.item.id,
            crate::app::scanner::util::stable_item_id(
                &first_target.to_string_lossy(),
                Some(r#"--profile "first profile""#),
            )
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
