//! plugin.json 解析与校验。

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const SCHEMA_V1: u32 = 1;
pub const PLUGIN_API_V1: u32 = 1;
pub const IDLE_TIMEOUT_MIN_MS: u64 = 15_000;
pub const IDLE_TIMEOUT_MAX_MS: u64 = 300_000;
pub const IDLE_TIMEOUT_DEFAULT_MS: u64 = 60_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub plugin: PluginIdentity,
    pub compatibility: Compatibility,
    pub runtime: RuntimeSpec,
    #[serde(default)]
    pub contributes: Contributions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginIdentity {
    pub id: String,
    pub name: String,
    pub version: String,
    /// 一句话能力说明（设置列表展示；必填非空）。
    #[serde(default)]
    pub description: String,
    /// 详细使用说明（设置页点开展示；多行中文/用户语言）。
    #[serde(default)]
    pub usage: String,
    #[serde(default)]
    pub author: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Compatibility {
    pub plugin_api: u32,
    #[serde(default)]
    pub minimum_kite_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub startup_timeout_ms: Option<u64>,
    #[serde(default)]
    pub idle_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Contributions {
    #[serde(default)]
    pub commands: Vec<PluginCommand>,
    #[serde(default)]
    pub providers: Vec<PluginProvider>,
    /// 用户可点的试用查询，如 `=1+1`、`win`。发现路径优先读这里。
    #[serde(default)]
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginCommand {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub action: CommandAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandAction {
    EnterProvider { provider: String },
    PluginAction { action_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginProvider {
    pub id: String,
    /// list | panel
    pub response_mode: String,
    #[serde(default)]
    pub triggers: Vec<crate::plugin::activation::Trigger>,
}

/// V1 允许的 Plugin ID：`[a-z0-9._-]+`，点分 reverse-domain 推荐。
pub fn validate_plugin_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    id.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
}

/// idle timeout clamp：默认 60s，范围 15–300s；0 不代表永不关闭。
pub fn clamp_idle_timeout_ms(requested: Option<u64>) -> u64 {
    match requested {
        None => IDLE_TIMEOUT_DEFAULT_MS,
        Some(0) => IDLE_TIMEOUT_MIN_MS,
        Some(ms) => ms.clamp(IDLE_TIMEOUT_MIN_MS, IDLE_TIMEOUT_MAX_MS),
    }
}

/// 路径是否落在插件根目录内（command/icon/asset 用）。
/// 相对路径拒绝 `..`/绝对路径/盘符；root 可用时再做 canonicalize 二次确认。
pub fn path_inside_plugin_root(root: &Path, relative: &str) -> bool {
    let rel = relative.trim();
    if rel.is_empty() {
        return false;
    }
    let mut path = PathBuf::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(s) => path.push(s),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    if path.as_os_str().is_empty() {
        return false;
    }
    // 拒绝绝对路径与盘符
    if Path::new(rel).is_absolute() || rel.contains(':') {
        return false;
    }
    if root.as_os_str().is_empty() {
        return true;
    }
    // parse_manifest 阶段 root 常为 "."：相对组件检查已足够
    if root.components().all(|c| matches!(c, Component::CurDir)) {
        return true;
    }
    let joined = root.join(&path);
    if let (Ok(root_c), Ok(joined_c)) = (std::fs::canonicalize(root), std::fs::canonicalize(&joined))
    {
        return joined_c.starts_with(&root_c);
    }
    // 文件尚未落地时做词法包含检查（`..` 已在组件层拒绝）
    let root_n = lexical_normalize(root);
    let joined_n = lexical_normalize(&joined);
    if root_n.as_os_str().is_empty() {
        return true;
    }
    joined_n.starts_with(&root_n)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(s) => out.push(s),
        }
    }
    out
}

pub fn parse_manifest(text: &str) -> Result<PluginManifest, String> {
    let manifest: PluginManifest =
        serde_json::from_str(text).map_err(|e| format!("manifest json invalid: {e}"))?;
    if manifest.schema_version != SCHEMA_V1 {
        return Err(format!(
            "schema_version unsupported: {}",
            manifest.schema_version
        ));
    }
    if manifest.compatibility.plugin_api != PLUGIN_API_V1 {
        return Err(format!(
            "plugin_api unsupported: {}",
            manifest.compatibility.plugin_api
        ));
    }
    if !validate_plugin_id(&manifest.plugin.id) {
        return Err(format!("invalid plugin id: {}", manifest.plugin.id));
    }
    if manifest.plugin.name.trim().is_empty() {
        return Err("plugin.name is required".into());
    }
    // 产品约束：插件必须自带用户可读说明，否则设置页/发现路径无法使用。
    if manifest.plugin.description.trim().is_empty() {
        return Err(
            "plugin.description is required：必须写清插件能做什么、在搜索框怎么触发".into(),
        );
    }
    // 至少一条可发现/可触发入口，否则装了也用不到。
    let has_entry = !manifest.contributes.examples.is_empty()
        || !manifest.contributes.commands.is_empty()
        || manifest
            .contributes
            .providers
            .iter()
            .any(|p| !p.triggers.is_empty());
    if !has_entry {
        return Err(
            "plugin must contribute examples, commands, or provider triggers".into(),
        );
    }
    if !path_inside_plugin_root(Path::new("."), &manifest.runtime.command) {
        return Err("runtime command escapes plugin root".into());
    }
    Ok(manifest)
}

impl PluginManifest {
    pub fn idle_timeout_ms(&self) -> u64 {
        clamp_idle_timeout_ms(self.runtime.idle_timeout_ms)
    }

    pub fn startup_timeout_ms(&self) -> u64 {
        self.runtime
            .startup_timeout_ms
            .filter(|&ms| ms > 0)
            .unwrap_or(5000)
            .clamp(100, 10_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_json(command: &str) -> String {
        format!(
            r#"{{
            "schema_version": 1,
            "plugin": {{
                "id": "com.kite.calculator",
                "name": "计算器",
                "version": "0.1.0",
                "description": "在搜索框输入 = 直接计算",
                "usage": "输入 = 加上算式。"
            }},
            "compatibility": {{ "plugin_api": 1, "minimum_kite_version": "0.3.0" }},
            "runtime": {{ "command": "{command}", "idle_timeout_ms": 60000 }},
            "contributes": {{
                "examples": ["=1+1"],
                "commands": [{{
                    "id": "open",
                    "title": "计算器",
                    "keywords": ["calc", "calculator", "计算器"],
                    "action": {{ "type": "enter_provider", "provider": "calculate" }}
                }}],
                "providers": [{{
                    "id": "calculate",
                    "response_mode": "panel",
                    "triggers": [{{ "type": "prefix", "value": "=" }}]
                }}]
            }}
        }}"#
        )
    }

    #[test]
    fn parses_valid_manifest() {
        let m = parse_manifest(&sample_json("calculator.exe")).expect("parse");
        assert_eq!(m.plugin.id, "com.kite.calculator");
        assert_eq!(m.contributes.commands.len(), 1);
        assert_eq!(m.contributes.providers[0].id, "calculate");
        assert_eq!(m.contributes.examples, vec!["=1+1".to_string()]);
        assert_eq!(m.idle_timeout_ms(), 60_000);
    }

    #[test]
    fn rejects_missing_description() {
        let mut json = sample_json("calc.exe");
        json = json.replace("\"description\": \"在搜索框输入 = 直接计算\"", "\"description\": \"  \"");
        let err = parse_manifest(&json).expect_err("blank description");
        assert!(err.contains("description"), "{err}");

        let mut json = sample_json("calc.exe");
        json = json.replace("\"description\": \"在搜索框输入 = 直接计算\",", "");
        if json.contains("\"description\"") {
            json = json.replace("\"description\": \"在搜索框输入 = 直接计算\"", "");
        }
        assert!(parse_manifest(&json).is_err());
    }

    #[test]
    fn rejects_no_activation_entry() {
        let json = r#"{
            "schema_version": 1,
            "plugin": {
                "id": "com.kite.demo",
                "name": "Demo",
                "version": "0.1.0",
                "description": "说明"
            },
            "compatibility": { "plugin_api": 1 },
            "runtime": { "command": "demo.exe" },
            "contributes": {}
        }"#;
        assert!(parse_manifest(json).is_err());
    }

    #[test]
    fn rejects_path_escape() {
        assert!(parse_manifest(&sample_json("../../evil.exe")).is_err());
        assert!(parse_manifest(&sample_json(r"C:\evil.exe")).is_err());
        assert!(!path_inside_plugin_root(Path::new("x"), "..\\evil.exe"));
        assert!(path_inside_plugin_root(Path::new("x"), "sub/calc.exe"));
    }

    #[test]
    fn rejects_bad_api_and_id() {
        let mut json = sample_json("calc.exe");
        json = json.replace("\"plugin_api\": 1", "\"plugin_api\": 2");
        assert!(parse_manifest(&json).is_err());

        let mut json = sample_json("calc.exe");
        json = json.replace("com.kite.calculator", "Bad Id!");
        assert!(parse_manifest(&json).is_err());
        assert!(validate_plugin_id("com.kite.calculator"));
        assert!(!validate_plugin_id("Com.Kite"));
    }

    #[test]
    fn clamps_idle_timeout() {
        assert_eq!(clamp_idle_timeout_ms(None), 60_000);
        assert_eq!(clamp_idle_timeout_ms(Some(0)), 15_000);
        assert_eq!(clamp_idle_timeout_ms(Some(1)), 15_000);
        assert_eq!(clamp_idle_timeout_ms(Some(120_000)), 120_000);
        assert_eq!(clamp_idle_timeout_ms(Some(999_999)), 300_000);
    }
}
