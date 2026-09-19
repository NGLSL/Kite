//! 插件导入：校验后拷入 `%APPDATA%\...\plugins\`，再交给 Registry/Host。
//!
//! V1 不做 Plugin Store / 在线市场；支持本地文件夹导入与官方样例 seed。
//! 包形态 `.kiteplugin`（ZIP）不在本模块范围。

use std::path::{Path, PathBuf};

use super::manifest::{parse_manifest, path_inside_plugin_root, validate_plugin_id};
use super::registry::PluginRegistry;

/// 官方插件打包目录名（resources/ 下，由 build-official-plugins.ps1 生成）。
pub const OFFICIAL_PLUGINS_DIR_NAME: &str = "official-plugins";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    pub plugin_id: String,
    pub dest: PathBuf,
    pub replaced: bool,
}

/// 定位捆绑的官方样例插件目录（安装目录或开发仓库 resources/）。
pub fn official_plugins_source_dir() -> Option<PathBuf> {
    crate::system::resources::candidates(OFFICIAL_PLUGINS_DIR_NAME)
        .into_iter()
        .find(|p| p.is_dir() && has_any_plugin_manifest(p))
}

fn has_any_plugin_manifest(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten().any(|e| {
                e.path().is_dir() && e.path().join("plugin.json").is_file()
            })
        })
        .unwrap_or(false)
}

/// 导入单个插件根目录（内含 plugin.json）到 plugins_dir。
/// 调用方应先 host.reload(plugin_id)，成功后再更新 Registry。
pub fn import_plugin_dir(source: &Path, plugins_dir: &Path) -> Result<ImportOutcome, String> {
    if !source.is_dir() {
        return Err(format!("插件目录不存在: {}", source.display()));
    }
    let manifest_path = source.join("plugin.json");
    if !manifest_path.is_file() {
        return Err("缺少 plugin.json".into());
    }
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("读取 plugin.json 失败: {e}"))?;
    let manifest = parse_manifest(&text)?;
    if !validate_plugin_id(&manifest.plugin.id) {
        return Err(format!("非法 plugin id: {}", manifest.plugin.id));
    }
    if !path_inside_plugin_root(source, &manifest.runtime.command) {
        return Err("runtime.command 逃逸插件根目录".into());
    }
    let command_path = source.join(&manifest.runtime.command);
    if !command_path.is_file() {
        return Err(format!(
            "runtime.command 不存在: {}",
            manifest.runtime.command
        ));
    }

    std::fs::create_dir_all(plugins_dir)
        .map_err(|e| format!("创建插件目录失败: {e}"))?;
    let plugins_canon = plugins_dir
        .canonicalize()
        .map_err(|e| format!("插件目录不可用: {e}"))?;
    let id = manifest.plugin.id.clone();
    let dest = plugins_dir.join(&id);
    // 禁止导入到 plugins 根之外（id 已校验，这里再兜一层）
    if dest.parent().map(|p| p != plugins_dir).unwrap_or(true) {
        return Err("目标路径不在插件目录下".into());
    }

    let replaced = dest.exists();
    if replaced {
        // 源即目标（已安装且路径相同）时无需拷贝
        if let (Ok(s), Ok(d)) = (source.canonicalize(), dest.canonicalize()) {
            if s == d {
                return Ok(ImportOutcome {
                    plugin_id: id,
                    dest,
                    replaced: false,
                });
            }
        }
        if dest.starts_with(&plugins_canon) || dest.starts_with(plugins_dir) {
            std::fs::remove_dir_all(&dest)
                .map_err(|e| format!("覆盖已有插件失败: {e}"))?;
        } else {
            return Err("拒绝删除插件目录之外的路径".into());
        }
    }
    copy_dir_recursive(source, &dest)
        .map_err(|e| format!("复制插件失败: {e}"))?;
    Ok(ImportOutcome {
        plugin_id: id,
        dest,
        replaced,
    })
}

/// 从路径导入：路径本身是插件根，或其子目录各自为插件根。
pub fn import_from_path(source: &Path, plugins_dir: &Path) -> Vec<Result<ImportOutcome, String>> {
    if source.join("plugin.json").is_file() {
        return vec![import_plugin_dir(source, plugins_dir)];
    }
    let Ok(rd) = std::fs::read_dir(source) else {
        return vec![Err(format!("路径不可读: {}", source.display()))];
    };
    let mut out = Vec::new();
    let mut found = false;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() && p.join("plugin.json").is_file() {
            found = true;
            out.push(import_plugin_dir(&p, plugins_dir));
        }
    }
    if !found {
        out.push(Err("路径下未找到 plugin.json".into()));
    }
    out
}

/// 安装捆绑的官方样例；返回每条导入结果。
pub fn install_official_plugins(plugins_dir: &Path) -> Vec<Result<ImportOutcome, String>> {
    let Some(src) = official_plugins_source_dir() else {
        return vec![Err(
            "未找到官方示例插件目录（resources/official-plugins）；请先运行 .\\scripts\\build-official-plugins.ps1"
                .into(),
        )];
    };
    import_from_path(&src, plugins_dir)
}

/// 启动时同步安装包内置的官方插件。
///
/// 官方插件随 Kite 一起发布，升级时必须覆盖用户数据目录中的旧版本，
/// 否则安装目录里的修复永远不会被当前宿主加载。第三方/用户插件不在
/// `official-plugins` 源目录中，不会被此函数触碰。
pub fn sync_official_plugins(plugins_dir: &Path) -> Vec<Result<ImportOutcome, String>> {
    install_official_plugins(plugins_dir)
}

/// 仅补齐缺失的安装包内置官方插件，不覆盖已有包。
/// 需要覆盖升级时使用 [`sync_official_plugins`]；源目录缺失时静默跳过。
pub fn seed_official_plugins_if_missing(plugins_dir: &Path) -> Vec<Result<ImportOutcome, String>> {
    let Some(src) = official_plugins_source_dir() else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(&src) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let manifest_path = p.join("plugin.json");
        if !manifest_path.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = parse_manifest(&text) else {
            continue;
        };
        if plugins_dir.join(&manifest.plugin.id).exists() {
            continue;
        }
        out.push(import_plugin_dir(&p, plugins_dir));
    }
    out
}

/// 导入后更新 Registry：保留原启用状态，替换 Manifest/Root。
pub fn apply_import_to_registry(
    registry: &mut PluginRegistry,
    dest: &Path,
) -> Result<String, String> {
    registry.upsert_from_root(dest)
}

fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if ty.is_file() {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manifest::{
        CommandAction, Compatibility, Contributions, PluginCommand, PluginIdentity, PluginManifest,
        PluginProvider, RuntimeSpec,
    };
    use crate::plugin::activation::Trigger;

    fn write_sample_plugin(root: &Path, id: &str, command: &str) {
        std::fs::create_dir_all(root).unwrap();
        let manifest = PluginManifest {
            schema_version: 1,
            plugin: PluginIdentity {
                id: id.into(),
                name: id.into(),
                version: "0.1.0".into(),
                description: "测试插件：输入 = 触发".into(),
                usage: "输入 = 加上内容。".into(),
                author: String::new(),
            },
            compatibility: Compatibility {
                plugin_api: 1,
                minimum_kite_version: None,
            },
            runtime: RuntimeSpec {
                command: command.into(),
                args: vec![],
                startup_timeout_ms: None,
                idle_timeout_ms: Some(30_000),
            },
            contributes: Contributions {
                examples: vec!["=1".into()],
                commands: vec![PluginCommand {
                    id: "open".into(),
                    title: "Test".into(),
                    keywords: vec!["t".into()],
                    action: CommandAction::EnterProvider {
                        provider: "main".into(),
                    },
                }],
                providers: vec![PluginProvider {
                    id: "main".into(),
                    response_mode: "panel".into(),
                    triggers: vec![Trigger::Prefix { value: "=".into() }],
                }],
            },
        };
        std::fs::write(
            root.join("plugin.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(root.join(command), b"@echo off\n").unwrap();
    }

    #[test]
    fn imports_folder_and_upserts_registry() {
        let tmp = std::env::temp_dir().join(format!("kite-plugin-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let src = tmp.join("src/com.kite.demo");
        let plugins = tmp.join("plugins");
        write_sample_plugin(&src, "com.kite.demo", "run.cmd");

        let out = import_plugin_dir(&src, &plugins).expect("import");
        assert_eq!(out.plugin_id, "com.kite.demo");
        assert!(out.dest.join("plugin.json").is_file());
        assert!(out.dest.join("run.cmd").is_file());
        assert!(!out.replaced);

        let mut reg = PluginRegistry::new();
        let id = apply_import_to_registry(&mut reg, &out.dest).expect("upsert");
        assert_eq!(id, "com.kite.demo");
        assert!(reg.get("com.kite.demo").is_some());

        // 再次导入 = 覆盖
        let out2 = import_plugin_dir(&src, &plugins).expect("reimport");
        assert!(out2.replaced);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_missing_manifest_and_command_escape() {
        let tmp = std::env::temp_dir().join(format!("kite-plugin-import-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let empty = tmp.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let plugins = tmp.join("plugins");
        assert!(import_plugin_dir(&empty, &plugins).unwrap_err().contains("plugin.json"));

        let evil = tmp.join("evil");
        write_sample_plugin(&evil, "com.kite.evil", "run.cmd");
        // 篡改 command 逃逸（合法 JSON，非法路径）
        let p = evil.join("plugin.json");
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        v["runtime"]["command"] = serde_json::json!("..\\..\\evil.exe");
        std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        let err = import_plugin_dir(&evil, &plugins).unwrap_err();
        assert!(err.contains("逃逸") || err.contains("command"), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn import_from_path_scans_children() {
        let tmp = std::env::temp_dir().join(format!("kite-plugin-import-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        write_sample_plugin(&tmp.join("pack/a"), "com.kite.a", "run.cmd");
        write_sample_plugin(&tmp.join("pack/b"), "com.kite.b", "run.cmd");
        let plugins = tmp.join("plugins");
        let results = import_from_path(&tmp.join("pack"), &plugins);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.is_ok()));
        assert!(plugins.join("com.kite.a/plugin.json").is_file());
        assert!(plugins.join("com.kite.b/plugin.json").is_file());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn official_package_manifests_are_importable() {
        // 官方插件源在 official-plugins/*/plugin.json；组装临时包（manifest + 占位 exe）验证导入
        let tmp = std::env::temp_dir().join(format!("kite-plugin-official-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let src_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("official-plugins");
        let pack = tmp.join("pack");
        let plugins = tmp.join("plugins");
        let crates = [
            ("calculator", "com.kite.calculator"),
            ("window-switcher", "com.kite.window-switcher"),
            ("devtools", "com.kite.devtools"),
        ];
        for (crate_dir, plugin_id) in crates {
            let manifest_src = src_root.join(crate_dir).join("plugin.json");
            assert!(manifest_src.is_file(), "missing {}", manifest_src.display());
            let text = std::fs::read_to_string(&manifest_src).unwrap();
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            let command = v["runtime"]["command"].as_str().unwrap().to_string();
            assert_eq!(v["plugin"]["id"].as_str().unwrap(), plugin_id);
            assert!(command.ends_with(".exe"), "official command should be exe: {command}");
            let dest = pack.join(plugin_id);
            std::fs::create_dir_all(&dest).unwrap();
            std::fs::write(dest.join("plugin.json"), &text).unwrap();
            std::fs::write(dest.join(&command), b"MZ").unwrap();
        }
        let results = import_from_path(&pack, &plugins);
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.is_ok()), "{results:?}");
        assert!(plugins.join("com.kite.calculator/plugin.json").is_file());
        assert!(plugins.join("com.kite.window-switcher/plugin.json").is_file());
        assert!(plugins.join("com.kite.devtools/plugin.json").is_file());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn seed_official_plugins_copies_once_and_keeps_existing() {
        let tmp = std::env::temp_dir().join(format!("kite-plugin-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let plugins = tmp.join("plugins");
        crate::system::resources::init(PathBuf::from(env!("CARGO_MANIFEST_DIR")));

        // 无捆绑源时静默
        let empty = tmp.join("empty-data");
        let _ = empty;

        let first = seed_official_plugins_if_missing(&plugins);
        let ok: Vec<_> = first.into_iter().filter_map(|r| r.ok()).collect();
        assert!(ok.len() >= 3, "首次 seed 应装入官方插件: {ok:?}");
        assert!(plugins.join("com.kite.calculator/plugin.json").is_file());

        // 已存在则不覆盖：改 calculator 的 name，再 seed 应保持
        let calc_manifest = plugins.join("com.kite.calculator/plugin.json");
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&calc_manifest).unwrap()).unwrap();
        v["plugin"]["name"] = serde_json::json!("User Tweaked");
        std::fs::write(&calc_manifest, serde_json::to_string_pretty(&v).unwrap()).unwrap();

        let second = seed_official_plugins_if_missing(&plugins);
        assert!(
            second.iter().all(|r| r.is_ok()),
            "重复 seed 不应失败: {second:?}"
        );
        let v2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&calc_manifest).unwrap()).unwrap();
        assert_eq!(v2["plugin"]["name"], "User Tweaked", "已存在包不得被 seed 覆盖");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn sync_official_plugins_replaces_existing_official_package() {
        let tmp = std::env::temp_dir().join(format!("kite-plugin-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let plugins = tmp.join("plugins");
        crate::system::resources::init(PathBuf::from(env!("CARGO_MANIFEST_DIR")));

        let first = sync_official_plugins(&plugins);
        assert!(first.iter().all(|r| r.is_ok()), "首次同步失败: {first:?}");

        let calc_manifest = plugins.join("com.kite.calculator/plugin.json");
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&calc_manifest).unwrap()).unwrap();
        v["plugin"]["name"] = serde_json::json!("Stale Official Package");
        std::fs::write(&calc_manifest, serde_json::to_string_pretty(&v).unwrap()).unwrap();

        let second = sync_official_plugins(&plugins);
        assert!(second.iter().all(|r| r.is_ok()), "重复同步失败: {second:?}");
        let v2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&calc_manifest).unwrap()).unwrap();
        assert_eq!(v2["plugin"]["name"], "计算器", "官方包应由安装包版本覆盖");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
