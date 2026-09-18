//! Plugin Registry：扫描、校验、启用状态、Command/Provider 注册。
//!
//! 注册表只保留 Manifest 元数据，不加载插件 exe/DLL。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::model::{AppItem, ResultAction, ResultSource, SearchResult};

use super::manifest::{
    clamp_idle_timeout_ms, parse_manifest, path_inside_plugin_root, CommandAction, Compatibility,
    Contributions, PluginIdentity, PluginManifest, RuntimeSpec, PLUGIN_API_V1, SCHEMA_V1,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePhase {
    Discovered,
    Disabled,
    Dormant,
    Starting,
    Ready,
    Faulted,
    Incompatible,
}

#[derive(Debug, Clone)]
pub struct RegisteredPlugin {
    pub manifest: PluginManifest,
    pub root: PathBuf,
    pub enabled: bool,
    pub phase: RuntimePhase,
    pub last_error: Option<String>,
}

#[derive(Debug, Default)]
pub struct PluginRegistry {
    plugins: BTreeMap<String, RegisteredPlugin>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_loaded(&mut self, plugin: RegisteredPlugin) {
        self.plugins.insert(plugin.manifest.plugin.id.clone(), plugin);
    }

    pub fn get(&self, id: &str) -> Option<&RegisteredPlugin> {
        self.plugins.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut RegisteredPlugin> {
        self.plugins.get_mut(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &RegisteredPlugin> {
        self.plugins.values()
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn enabled_plugins(&self) -> impl Iterator<Item = &RegisteredPlugin> {
        self.plugins.values().filter(|p| p.enabled)
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if let Some(p) = self.plugins.get_mut(id) {
            p.enabled = enabled;
            if !enabled {
                p.phase = RuntimePhase::Disabled;
            } else if p.phase == RuntimePhase::Disabled || p.phase == RuntimePhase::Discovered {
                p.phase = RuntimePhase::Dormant;
            }
            return true;
        }
        false
    }

    /// 从插件根目录扫描。单个插件失败不影响其它插件。
    pub fn scan_dir(&mut self, plugins_dir: &Path) {
        let Ok(entries) = std::fs::read_dir(plugins_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let root = entry.path();
            if !root.is_dir() {
                continue;
            }
            let manifest_path = root.join("plugin.json");
            let Ok(text) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            match load_one(&root, &text) {
                Ok(plugin) => {
                    // 重复 ID：后扫到的不覆盖已启用项
                    let id = plugin.manifest.plugin.id.clone();
                    if !self.plugins.contains_key(&id) {
                        self.plugins.insert(id, plugin);
                    }
                }
                Err((id, phase, err)) => {
                    mark_incompatible(&mut self.plugins, &root, &id, phase, err);
                }
            }
        }
    }

    /// 按当前 Manifest 重新注册；保留启用状态。进程重启交给 Host。
    pub fn reload_from_disk(&mut self, id: &str) -> Result<(), String> {
        let Some(existing) = self.plugins.get(id) else {
            return Err(format!("plugin not found: {id}"));
        };
        let root = existing.root.clone();
        let enabled = existing.enabled;
        let text = std::fs::read_to_string(root.join("plugin.json"))
            .map_err(|e| format!("manifest read failed: {e}"))?;
        match load_one(&root, &text) {
            Ok(mut plugin) => {
                plugin.enabled = enabled;
                if !enabled {
                    plugin.phase = RuntimePhase::Disabled;
                }
                self.plugins.insert(id.to_string(), plugin);
                Ok(())
            }
            Err((err_id, phase, err)) => {
                mark_incompatible(&mut self.plugins, &root, &err_id, phase, err.clone());
                Err(err)
            }
        }
    }

    /// 从磁盘插件根导入/覆盖注册（保留原启用状态）。
    pub fn upsert_from_root(&mut self, root: &Path) -> Result<String, String> {
        let text = std::fs::read_to_string(root.join("plugin.json"))
            .map_err(|e| format!("manifest read failed: {e}"))?;
        match load_one(root, &text) {
            Ok(mut plugin) => {
                let id = plugin.manifest.plugin.id.clone();
                if let Some(old) = self.plugins.get(&id) {
                    plugin.enabled = old.enabled;
                    if !plugin.enabled {
                        plugin.phase = RuntimePhase::Disabled;
                    }
                }
                self.plugins.insert(id.clone(), plugin);
                Ok(id)
            }
            Err((err_id, phase, err)) => {
                mark_incompatible(&mut self.plugins, root, &err_id, phase, err.clone());
                Err(err)
            }
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<RegisteredPlugin> {
        self.plugins.remove(id)
    }

    /// 仅允许删除插件根目录下的包目录，并从 Registry 移除。
    pub fn uninstall(&mut self, id: &str, plugins_dir: &Path) -> Result<(), String> {
        self.uninstall_dir(id, plugins_dir)?;
        self.plugins.remove(id);
        Ok(())
    }

    /// 仅允许删除插件根目录下的包目录。
    pub fn uninstall_dir(&self, id: &str, plugins_dir: &Path) -> Result<(), String> {
        let plugin = self
            .plugins
            .get(id)
            .ok_or_else(|| format!("plugin not found: {id}"))?;
        let root = &plugin.root;
        if !root.starts_with(plugins_dir) || root.as_path() == plugins_dir {
            return Err("plugin root outside plugins dir".into());
        }
        std::fs::remove_dir_all(root).map_err(|e| format!("remove failed: {e}"))
    }
}

fn placeholder_manifest(id: &str) -> PluginManifest {
    PluginManifest {
        schema_version: SCHEMA_V1,
        plugin: PluginIdentity {
            id: id.to_string(),
            name: id.to_string(),
            version: "0.0.0".into(),
            description: String::new(),
            usage: String::new(),
            author: String::new(),
        },
        compatibility: Compatibility {
            plugin_api: 0,
            minimum_kite_version: None,
        },
        runtime: RuntimeSpec {
            command: String::new(),
            args: Vec::new(),
            startup_timeout_ms: None,
            idle_timeout_ms: None,
        },
        contributes: Contributions::default(),
    }
}

fn mark_incompatible(
    plugins: &mut BTreeMap<String, RegisteredPlugin>,
    root: &Path,
    id: &str,
    phase: RuntimePhase,
    err: String,
) {
    let key = if id.is_empty() {
        root.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".into())
    } else {
        id.to_string()
    };
    let entry = plugins.entry(key).or_insert_with(|| RegisteredPlugin {
        manifest: placeholder_manifest(root.file_name().and_then(|s| s.to_str()).unwrap_or("unknown")),
        root: root.to_path_buf(),
        enabled: false,
        phase: RuntimePhase::Incompatible,
        last_error: None,
    });
    entry.phase = phase;
    entry.last_error = Some(err);
    entry.enabled = false;
}

fn load_one(root: &Path, text: &str) -> Result<RegisteredPlugin, (String, RuntimePhase, String)> {
    let manifest = parse_manifest(text).map_err(|e| {
        (
            root.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            RuntimePhase::Incompatible,
            e,
        )
    })?;
    if manifest.schema_version != SCHEMA_V1 || manifest.compatibility.plugin_api != PLUGIN_API_V1 {
        return Err((
            manifest.plugin.id.clone(),
            RuntimePhase::Incompatible,
            format!(
                "incompatible schema={} api={}",
                manifest.schema_version, manifest.compatibility.plugin_api
            ),
        ));
    }
    if !path_inside_plugin_root(root, &manifest.runtime.command) {
        return Err((
            manifest.plugin.id.clone(),
            RuntimePhase::Incompatible,
            "command escapes plugin root".into(),
        ));
    }
    // command 文件应位于插件根（相对）
    Ok(RegisteredPlugin {
        manifest,
        root: root.to_path_buf(),
        enabled: true,
        phase: RuntimePhase::Dormant,
        last_error: None,
    })
}

pub fn load_registry_from_dir(plugins_dir: &Path) -> PluginRegistry {
    let mut registry = PluginRegistry::new();
    registry.scan_dir(plugins_dir);
    registry
}

/// Command 静态命中：不启动进程。search source = plugin-command。
pub fn command_hits(registry: &PluginRegistry, query: &str) -> Vec<SearchResult> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for plugin in registry.enabled_plugins() {
        // Disabled / Incompatible 不进搜索。
        if matches!(
            plugin.phase,
            RuntimePhase::Disabled | RuntimePhase::Incompatible
        ) {
            continue;
        }
        for cmd in &plugin.manifest.contributes.commands {
            let title = cmd.title.to_lowercase();
            let kw_hit = cmd
                .keywords
                .iter()
                .any(|k| k.to_lowercase() == q || k.to_lowercase().starts_with(&q));
            let title_hit = title.contains(&q);
            if !(title_hit || kw_hit) {
                continue;
            }
            let score = if title == q || cmd.keywords.iter().any(|k| k.to_lowercase() == q) {
                // 精确 Command：低于应用 Name Exact(1000)/内置别名，高于普通前缀
                crate::search::ranker::SCORE_NAME_EXACT - 60
            } else {
                // 弱命中：不高于应用词前缀，避免普通搜索被插件顶掉
                crate::search::ranker::SCORE_PREFIX - 40
            };
            let mut item = AppItem::scanned(
                format!("plugin-command:{}:{}", plugin.manifest.plugin.id, cmd.id),
                cmd.title.clone(),
                format!("plugin-command:{}:{}", plugin.manifest.plugin.id, cmd.id),
                None,
                None,
                "plugin-command",
            );
            item.attach_search_fields();
            item.search_keywords = cmd
                .keywords
                .iter()
                .map(|k| k.to_lowercase())
                .collect();
            let source = ResultSource::Plugin {
                plugin_id: plugin.manifest.plugin.id.clone(),
                provider_id: String::new(),
            };
            let action = match &cmd.action {
                CommandAction::EnterProvider { provider } => ResultAction::plugin(
                    &plugin.manifest.plugin.id,
                    "enter_provider",
                    serde_json::json!({
                        "command_id": cmd.id,
                        "provider": provider,
                    }),
                ),
                CommandAction::PluginAction { action_id } => ResultAction::plugin(
                    &plugin.manifest.plugin.id,
                    action_id,
                    serde_json::json!({ "command_id": cmd.id }),
                ),
            };
            let hit = SearchResult::scored(item, score, "plugin-command")
                .with_source_action(source, action);
            hits.push(hit);
        }
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.item.id.cmp(&b.item.id)));
    hits.truncate(8);
    hits
}

/// 管理页状态标签（含 crash loop），面向用户显示中文。
pub fn runtime_phase_label(phase: RuntimePhase, last_error: Option<&str>, host_faulted: Option<&str>) -> String {
    if let Some(reason) = host_faulted {
        if reason.contains("crash loop") {
            return "崩溃循环".into();
        }
        return format!("故障：{reason}");
    }
    match phase {
        RuntimePhase::Discovered => "已发现".into(),
        RuntimePhase::Disabled => "已禁用".into(),
        RuntimePhase::Dormant => "待命".into(),
        RuntimePhase::Starting => "启动中".into(),
        RuntimePhase::Ready => "运行中".into(),
        RuntimePhase::Faulted => match last_error {
            Some(e) if e.contains("crash loop") => "崩溃循环".into(),
            Some(e) => format!("故障：{e}"),
            None => "故障".into(),
        },
        RuntimePhase::Incompatible => match last_error {
            Some(e) => format!("不兼容：{e}"),
            None => "不兼容".into(),
        },
    }
}

/// idle clamp 的公开断言入口（规格：15–300s，0 不代表永不关闭）。
pub fn effective_idle_timeout_ms(manifest: &PluginManifest) -> u64 {
    clamp_idle_timeout_ms(manifest.runtime.idle_timeout_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::activation::Trigger;
    use crate::plugin::manifest::{
        CommandAction, Compatibility, Contributions, PluginCommand, PluginIdentity, PluginProvider,
        RuntimeSpec,
    };

    fn write_plugin(dir: &Path, id: &str, command: &str) {
        let root = dir.join(id);
        std::fs::create_dir_all(&root).unwrap();
        let json = serde_json::json!({
            "schema_version": 1,
            "plugin": { "id": id, "name": id, "version": "0.1.0", "description": "测试插件说明" },
            "compatibility": { "plugin_api": 1 },
            "runtime": { "command": command, "idle_timeout_ms": 60000 },
            "contributes": {
                "examples": ["=1+1"],
                "commands": [{
                    "id": "open",
                    "title": "计算器",
                    "keywords": ["calc", "calculator", "计算器"],
                    "action": { "type": "enter_provider", "provider": "calculate" }
                }],
                "providers": [{
                    "id": "calculate",
                    "response_mode": "panel",
                    "triggers": [{ "type": "prefix", "value": "=" }]
                }]
            }
        });
        std::fs::write(root.join("plugin.json"), json.to_string()).unwrap();
    }

    #[test]
    fn scans_valid_plugin_and_rejects_escape() {
        let dir = std::env::temp_dir().join(format!("kite-plugin-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_plugin(&dir, "com.kite.calculator", "calculator.exe");
        write_plugin(&dir, "com.kite.evil", "..\\..\\evil.exe");
        let reg = load_registry_from_dir(&dir);
        assert!(reg.get("com.kite.calculator").is_some());
        assert!(reg.get("com.kite.calculator").unwrap().enabled);
        assert_eq!(
            reg.get("com.kite.calculator").unwrap().phase,
            RuntimePhase::Dormant
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_hits_without_spawning() {
        let mut reg = PluginRegistry::new();
        reg.insert_loaded(RegisteredPlugin {
            manifest: PluginManifest {
                schema_version: 1,
                plugin: PluginIdentity {
                    id: "com.kite.calculator".into(),
                    name: "Calculator".into(),
                    version: "0.1.0".into(),
                    description: String::new(),
                    usage: String::new(),
                    author: String::new(),
                },
                compatibility: Compatibility {
                    plugin_api: 1,
                    minimum_kite_version: None,
                },
                runtime: RuntimeSpec {
                    command: "calculator.exe".into(),
                    args: vec![],
                    startup_timeout_ms: None,
                    idle_timeout_ms: Some(60_000),
                },
                contributes: Contributions {
                    examples: vec!["=1+1".into()],
                    commands: vec![PluginCommand {
                        id: "open".into(),
                        title: "计算器".into(),
                        keywords: vec!["calc".into(), "计算器".into()],
                        action: CommandAction::EnterProvider {
                            provider: "calculate".into(),
                        },
                    }],
                    providers: vec![PluginProvider {
                        id: "calculate".into(),
                        response_mode: "panel".into(),
                        triggers: vec![Trigger::Prefix { value: "=".into() }],
                    }],
                },
            },
            root: PathBuf::from("x"),
            enabled: true,
            phase: RuntimePhase::Dormant,
            last_error: None,
        });

        let hits = command_hits(&reg, "calc");
        assert_eq!(hits.len(), 1);
        assert!(matches!(hits[0].source, ResultSource::Plugin { .. }));
        assert!(matches!(hits[0].action, ResultAction::Plugin { .. }));
        // 普通查询无 Command
        assert!(command_hits(&reg, "chrome").is_empty());
        // 禁用后不再命中
        assert!(reg.set_enabled("com.kite.calculator", false));
        assert!(command_hits(&reg, "calc").is_empty());
    }

    #[test]
    fn incompatible_manifest_is_visible_and_disabled() {
        let dir = std::env::temp_dir().join(format!(
            "kite-plugin-incompat-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_plugin(&dir, "com.kite.evil", "..\\..\\evil.exe");
        let reg = load_registry_from_dir(&dir);
        let p = reg.get("com.kite.evil").expect("incompatible kept visible");
        assert_eq!(p.phase, RuntimePhase::Incompatible);
        assert!(!p.enabled);
        assert!(command_hits(&reg, "calc").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_from_disk_preserves_enabled_and_uninstall_guard() {
        let dir = std::env::temp_dir().join(format!("kite-plugin-reload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_plugin(&dir, "com.kite.calculator", "calculator.exe");
        let mut reg = load_registry_from_dir(&dir);
        assert!(reg.set_enabled("com.kite.calculator", false));
        reg.reload_from_disk("com.kite.calculator").expect("reload");
        let p = reg.get("com.kite.calculator").unwrap();
        assert!(!p.enabled, "reload 保留启用状态");
        assert_eq!(p.phase, RuntimePhase::Disabled);
        assert!(reg.set_enabled("com.kite.calculator", true));
        reg.reload_from_disk("com.kite.calculator").unwrap();
        assert!(reg.get("com.kite.calculator").unwrap().enabled);

        // 卸载护栏：不在 plugins 根下拒绝
        assert!(reg
            .uninstall("com.kite.calculator", std::path::Path::new("other-root"))
            .is_err());
        let plugins_root = dir.clone();
        // 真实卸载
        assert!(reg.uninstall("com.kite.calculator", &plugins_root).is_ok());
        assert!(!dir.join("com.kite.calculator").exists());
        assert!(reg.get("com.kite.calculator").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runtime_phase_label_marks_crash_loop() {
        assert_eq!(runtime_phase_label(RuntimePhase::Dormant, None, None), "待命");
        assert_eq!(
            runtime_phase_label(RuntimePhase::Faulted, None, Some("crash loop")),
            "崩溃循环"
        );
        assert!(runtime_phase_label(RuntimePhase::Incompatible, Some("api"), None)
            .contains("不兼容"));
    }
}
