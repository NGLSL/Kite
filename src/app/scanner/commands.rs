//! 从包管理器和命令目录收集可启动入口。
//!
//! 读取固定目录及 PATH 中非 Windows 系统目录的直接子项，并补充 Codex Desktop
//! 的版本化 CLI 目录。`.lnk` 保留快捷方式文件本身作为 target；启动阶段交给
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

/// 读取固定目录、PATH 和 Codex Desktop 的直接命令入口。
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
    let windows_dir = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map(PathBuf::from);
    let mut roots = command_roots(local_app_data.as_deref(), program_data.as_deref());
    roots.extend(additional_command_roots(
        local_app_data.as_deref(),
        &crate::system::env::effective_path(),
        windows_dir.as_deref(),
    ));
    roots
}

pub fn codex_bin_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|local| local.join(r"OpenAI\Codex\bin"))
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

fn additional_command_roots(
    local_app_data: Option<&Path>,
    effective_path: &str,
    windows_dir: Option<&Path>,
) -> Vec<PathBuf> {
    const MAX_PATH_ROOTS: usize = 128;
    let windows_key = windows_dir.map(|dir| normalize_path_key(&dir.to_string_lossy()));
    let mut roots = Vec::new();
    for segment in effective_path.split(';').take(MAX_PATH_ROOTS) {
        let segment = segment.trim().trim_matches('"');
        if segment.is_empty() {
            continue;
        }
        let expanded = crate::system::env::expand_env(segment);
        let root = PathBuf::from(expanded);
        if !root.is_absolute() || root.to_string_lossy().starts_with(r"\\") {
            continue;
        }
        let key = normalize_path_key(&root.to_string_lossy());
        if windows_key.as_ref().is_some_and(|windows| {
            key == *windows || key.starts_with(&format!("{}\\", windows.trim_end_matches('\\')))
        }) {
            continue;
        }
        roots.push(root);
    }

    // Codex Desktop 管理版本目录，并只向它启动的终端注入 CLI PATH；Kite 可能
    // 早于 Codex 启动，因此还要从安装目录选一个当前存在的版本。
    if let Some(bin) = local_app_data.map(|local| local.join(r"OpenAI\Codex\bin")) {
        let latest = std::fs::read_dir(bin).ok().and_then(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|dir| dir.join("codex.exe").is_file())
                .max_by_key(|dir| {
                    dir.join("codex.exe")
                        .metadata()
                        .and_then(|metadata| metadata.modified())
                        .ok()
                })
        });
        if let Some(root) = latest {
            roots.push(root);
        }
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
    // Shared helper markers live in util; this list is WindowsApps/package-only noise.
    if super::util::is_helper_name(name) {
        return false;
    }
    let compact = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect::<String>();
    const COMMAND_ONLY_HELPERS: &[&str] = &[
        "mcphost",
        "mcpserver",
        "adminserver",
        "packaging",
        "pcappce",
    ];
    if COMMAND_ONLY_HELPERS
        .iter()
        .any(|marker| compact.contains(marker))
    {
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
    fn path_and_codex_cli_entries_are_discovered() {
        let root = std::env::temp_dir().join(format!("kite-cli-roots-{}", std::process::id()));
        let grok_bin = root.join(".grok").join("bin");
        let codex_bin = root
            .join("AppData")
            .join("Local")
            .join("OpenAI")
            .join("Codex")
            .join("bin");
        let codex_version = codex_bin.join("version-a");
        let windows_dir = root.join("Windows");
        std::fs::create_dir_all(&grok_bin).unwrap();
        std::fs::create_dir_all(&codex_version).unwrap();
        std::fs::create_dir_all(windows_dir.join("System32")).unwrap();
        std::fs::write(grok_bin.join("grok.exe"), b"fixture").unwrap();
        std::fs::write(codex_version.join("codex.exe"), b"fixture").unwrap();
        std::fs::write(windows_dir.join("System32").join("cmd.exe"), b"fixture").unwrap();

        let roots = additional_command_roots(
            Some(&root.join("AppData").join("Local")),
            &format!(
                "{};{};relative-bin",
                grok_bin.display(),
                windows_dir.join("System32").display()
            ),
            Some(&windows_dir),
        );
        let mut out = Vec::new();
        collect_from_roots(&roots, COMMAND_SOURCE, &mut out);
        let names: HashSet<_> = out.iter().map(|(item, _)| item.name.as_str()).collect();
        assert!(names.contains("grok"), "user PATH CLI must be indexed");
        assert!(names.contains("codex"), "Codex Desktop CLI must be indexed");
        assert!(!names.contains("cmd"), "Windows system commands must stay excluded");

        let _ = std::fs::remove_dir_all(root);
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
