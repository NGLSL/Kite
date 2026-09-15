//! 从包管理器和 Windows 命令别名目录收集少量可启动入口。
//!
//! 这里只读取三个固定目录的直接子项，不扫描整个 `PATH`，也不递归进入
//! 其他目录。`.lnk` 保留快捷方式文件本身作为 target；启动阶段交给
//! Windows ShellExecute 处理，避免在此处复制快捷方式 COM 解析逻辑。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::model::AppItem;

use super::util::{app_display_name, normalize_path_key, stable_item_id};

/// 与扫描器其他来源相同的原始条目形状：应用和可选图标源。
pub type RawItem = (AppItem, Option<String>);

/// 命令入口建议使用的 source 标签；调用方可以传入自己的标签。
pub const COMMAND_SOURCE: &str = "commands";

const SUPPORTED_EXTENSIONS: &[&str] = &["exe", "cmd", "bat", "com", "lnk"];

/// 读取 WindowsApps、WinGet Links 和 Chocolatey 的直接命令入口。
///
/// 每个候选都必须是现存文件，并且只允许 `SUPPORTED_EXTENSIONS` 中的扩展名。
/// 单个目录或文件读取失败时静默跳过，其他来源仍继续收集。
pub fn collect_command_entries(source: &str, out: &mut Vec<RawItem>) {
    let roots = configured_command_roots();
    collect_from_roots(&roots, source, out);
}

/// 根据当前进程环境变量构造命令入口目录。
pub fn configured_command_roots() -> Vec<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let program_data = std::env::var_os("ProgramData").map(PathBuf::from);
    command_roots(local_app_data.as_deref(), program_data.as_deref())
}

/// 构造固定的命令入口目录；参数独立出来便于无环境副作用地测试。
pub fn command_roots(local_app_data: Option<&Path>, program_data: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(3);
    if let Some(local_app_data) = local_app_data {
        roots.push(local_app_data.join(r"Microsoft\WindowsApps"));
        roots.push(local_app_data.join(r"Microsoft\WinGet\Links"));
    }
    if let Some(program_data) = program_data {
        roots.push(program_data.join(r"chocolatey\bin"));
    }
    roots
}

/// 判断文件扩展名是否属于命令入口集合（大小写不敏感）。
pub fn is_supported_command_extension(extension: &str) -> bool {
    SUPPORTED_EXTENSIONS
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

/// 判断命令文件名是否值得作为应用入口显示。
pub fn should_index_command_name(name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    // App execution aliases are meant to be typed. Very long package-facing
    // names are normally implementation endpoints also exposed elsewhere by
    // AppsFolder, not useful launcher entries.
    if name.chars().count() > 24 {
        return false;
    }

    // Use a compact form so both `Foo-Updater` and `FooUpdater` are covered.
    // These names are maintenance or implementation helpers, not user-facing
    // launch entries.
    let compact = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect::<String>();
    const HELPER_MARKERS: &[&str] = &[
        "uninstall",
        "uninstaller",
        "unins",
        "update",
        "updater",
        "setup",
        "installer",
        "installhelper",
        "repair",
        "maintenance",
        "crashpad",
        "crashreport",
        "helper",
        "mcphost",
        "mcpserver",
        "elevated",
        "adminserver",
        "packaging",
        "pcappce",
    ];
    if HELPER_MARKERS.iter().any(|marker| compact.contains(marker)) {
        return false;
    }

    // Windows SDK/debugger aliases use a joined architecture suffix such as
    // `cdbARM64` or `ntsdX86`. Keep user-facing names like `tool-x64`, while
    // suppressing the unseparated architecture variants that otherwise appear
    // as a cluster of implementation helpers.
    const ARCH_SUFFIXES: &[&str] = &[
        "arm64", "arm32", "aarch64", "x86_64", "amd64", "x64", "x86", "ia32", "win64", "win32",
    ];
    let lower = name.to_ascii_lowercase();
    let joined_architecture_suffix = ARCH_SUFFIXES.iter().any(|suffix| {
        lower.ends_with(suffix)
            && lower.len() > suffix.len()
            && lower.as_bytes()[lower.len() - suffix.len() - 1] != b'-'
            && lower.as_bytes()[lower.len() - suffix.len() - 1] != b'_'
            && lower.as_bytes()[lower.len() - suffix.len() - 1] != b'.'
    });
    if joined_architecture_suffix {
        return false;
    }

    // Separated names are filtered only when they explicitly identify an SDK
    // or implementation helper. This avoids hiding a legitimate user command
    // merely because its product has an x64 build.
    let has_architecture_marker = ARCH_SUFFIXES.iter().any(|suffix| {
        lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|part| part == *suffix)
    });
    let has_sdk_helper_marker = [
        "sdk",
        "toolchain",
        "debug",
        "dbg",
        "runtime",
        "redist",
        "redistributable",
        "helper",
        "host",
    ]
    .iter()
    .any(|marker| compact.contains(marker));
    !(has_architecture_marker && has_sdk_helper_marker)
}

/// 从命令文件名生成展示名；不支持的扩展名或明显辅助程序返回 `None`。
pub fn command_display_name(file_name: &str) -> Option<String> {
    let path = Path::new(file_name);
    let extension = path.extension()?.to_string_lossy();
    if !is_supported_command_extension(&extension) {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy();
    let stem = stem.trim();
    if !should_index_command_name(stem) {
        return None;
    }
    // Keep the existing scanner's display-name normalization (in particular,
    // preserve spaces and non-ASCII names); only the command suffix is removed.
    let display_name = app_display_name(file_name);
    (!display_name.trim().is_empty()).then(|| display_name.trim().to_string())
}

fn collect_from_roots(roots: &[PathBuf], source: &str, out: &mut Vec<RawItem>) {
    let mut seen = HashSet::new();
    for root in roots {
        if !seen.insert(normalize_path_key(&root.to_string_lossy())) {
            continue;
        }
        collect_from_root(root, source, out);
    }
}

fn collect_from_root(root: &Path, source: &str, out: &mut Vec<RawItem>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    // Sort the small direct-child list so snapshots and tests are deterministic.
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    paths.sort_by(|a, b| {
        normalize_path_key(&a.to_string_lossy()).cmp(&normalize_path_key(&b.to_string_lossy()))
    });

    for path in paths {
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().map(|name| name.to_string_lossy()) else {
            continue;
        };
        let Some(name) = command_display_name(&file_name) else {
            continue;
        };
        let target = path.to_string_lossy().into_owned();
        let working_dir = path
            .parent()
            .filter(|parent| parent.is_dir())
            .map(|parent| parent.to_string_lossy().into_owned());
        let id = stable_item_id(&target, None);
        let item = AppItem::scanned(id, name, target.clone(), None, working_dir, source);
        out.push((item, Some(target)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_extension_is_case_insensitive_and_bounded() {
        for extension in ["exe", "EXE", "cmd", "BAT", "com", "LNK"] {
            assert!(is_supported_command_extension(extension), "{}", extension);
        }
        for extension in ["dll", "ps1", "msi", "", "exe "] {
            assert!(!is_supported_command_extension(extension), "{}", extension);
        }
    }

    #[test]
    fn command_display_name_strips_only_the_command_extension() {
        assert_eq!(
            command_display_name("Visual Studio Code.CMD"),
            Some("Visual Studio Code".into())
        );
        assert_eq!(command_display_name("tool.v2.exe"), Some("tool.v2".into()));
        assert_eq!(command_display_name("notes.txt"), None);
        assert_eq!(command_display_name(".exe"), None);
    }

    #[test]
    fn command_name_filter_removes_sdk_architecture_and_maintenance_helpers() {
        for helper in [
            "cdbARM64",
            "dbgsrvX64",
            "kdX86",
            "ntsdARM64",
            "my-sdk-tool-x64",
            "ExampleUpdater",
            "uninstall",
            "repair-helper",
            "ActionsMcpHost",
            "WindowsPackageManagerMCPServer",
            "MicrosoftWindows.DesktopStickerEditorCentennial",
            "XboxPcAppCE",
        ] {
            assert!(
                !should_index_command_name(helper),
                "{} should be hidden",
                helper
            );
        }
        for application in ["winget", "wt", "VisualAssist", "WinDbgX", "legacy-tool"] {
            assert!(
                should_index_command_name(application),
                "{} should remain searchable",
                application
            );
        }
    }

    #[test]
    fn command_roots_follow_fixed_order() {
        let local = Path::new(r"C:\Users\dev\AppData\Local");
        let program_data = Path::new(r"C:\ProgramData");
        assert_eq!(
            command_roots(Some(local), Some(program_data)),
            vec![
                local.join(r"Microsoft\WindowsApps"),
                local.join(r"Microsoft\WinGet\Links"),
                program_data.join(r"chocolatey\bin"),
            ]
        );
        assert_eq!(command_roots(None, None), Vec::<PathBuf>::new());
    }

    #[test]
    fn command_collector_keeps_existing_supported_files_and_lnk_path() {
        let root = std::env::temp_dir().join(format!("kite-command-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for file_name in [
            "Terminal.exe",
            "build.CMD",
            "legacy.bat",
            "old.com",
            "Game.lnk",
            "ignore.txt",
            "cdbARM64.exe",
            "Updater.cmd",
        ] {
            std::fs::write(root.join(file_name), b"fixture").unwrap();
        }

        let mut out = Vec::new();
        collect_from_roots(std::slice::from_ref(&root), COMMAND_SOURCE, &mut out);
        let mut names: Vec<_> = out.iter().map(|(item, _)| item.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["Game", "Terminal", "build", "legacy", "old"]);
        let lnk_target = root.join("Game.lnk").to_string_lossy().to_string();
        let lnk = out
            .iter()
            .find(|(item, _)| item.name == "Game")
            .expect(".lnk must be searchable");
        assert_eq!(lnk.0.target, lnk_target);
        assert_eq!(lnk.0.source, COMMAND_SOURCE);
        assert!(lnk.0.working_dir.is_some());

        let _ = std::fs::remove_dir_all(root);
    }
}
