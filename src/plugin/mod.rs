//! Kite Plugin System V1：能力外置，体验内联。
//!
//! 插件负责计算/查询/执行能力；Kite 负责触发、生命周期、结果展示与主要交互。
//! 单 crate 内模块，不拆 SDK crate。

pub mod activation;
pub mod host;
pub mod install;
pub mod manifest;
pub mod panel;
pub mod process;
pub mod protocol;
pub mod registry;
pub mod safety;

pub use activation::{
    activation_for_command, initial_query_for_activation, resolve_command_entry, route_query,
    trigger_query_prefix, Activation, ResponseMode, Trigger,
};
pub use host::{HostError, PluginHost, PluginRuntimeState, QueryOutcome};
pub use install::{
    apply_import_to_registry, import_from_path, import_plugin_dir, install_official_plugins,
    official_plugins_source_dir, seed_official_plugins_if_missing, sync_official_plugins,
    ImportOutcome,
    OFFICIAL_PLUGINS_DIR_NAME,
};
pub use manifest::{
    clamp_idle_timeout_ms, parse_manifest, validate_plugin_id, CommandAction, PluginCommand,
    PluginManifest, PluginProvider, RuntimeSpec, PLUGIN_API_V1, SCHEMA_V1,
};
pub use panel::{parse_panel, NativeAction, PanelAction, PanelBlock, PanelData, PanelError};
pub use process::{MemProcess, MemProcessBackend, PluginProcess, ProcessBackend, StdioBackend};
pub use protocol::{
    cancel_notification, decode_frames, encode_frame, parse_host_call, plugin_execute,
    plugin_initialize, plugin_query, ContentLengthCodec, HostCall, JsonRpcMessage, ListItemAction,
    PluginResponse, QueryResult,
};
pub use registry::{
    command_hits, load_registry_from_dir, runtime_phase_label, PluginRegistry, RegisteredPlugin,
    RuntimePhase,
};
pub use safety::{plugin_path_allowed, plugin_url_allowed};

/// 默认插件根目录（相对应用数据目录）。
pub const PLUGINS_DIR_NAME: &str = "plugins";
/// 插件私有数据目录名（相对应用数据目录）。
pub const PLUGIN_DATA_DIR_NAME: &str = "plugin-data";

/// `%APPDATA%\com.kite.launcher\plugins`
pub fn default_plugins_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(PLUGINS_DIR_NAME)
}

/// `%APPDATA%\com.kite.launcher\plugin-data\<plugin_id>`
pub fn plugin_data_dir(data_dir: &std::path::Path, plugin_id: &str) -> std::path::PathBuf {
    data_dir.join(PLUGIN_DATA_DIR_NAME).join(plugin_id)
}

/// 插件 stderr 日志路径（管理页「查看日志」）。
pub fn plugin_log_path(data_dir: &std::path::Path, plugin_id: &str) -> std::path::PathBuf {
    plugin_data_dir(data_dir, plugin_id).join("stderr.log")
}

/// stderr 日志大小上限（超过后轮转为 .1）。
pub const PLUGIN_LOG_MAX_BYTES: u64 = 256 * 1024;

/// 用户可读能力句：优先 manifest.description，空则退回 name。
pub fn capability_blurb(manifest: &PluginManifest) -> String {
    let d = manifest.plugin.description.trim();
    if d.is_empty() {
        manifest.plugin.name.clone()
    } else {
        d.to_string()
    }
}

/// 设置页「点开说明」正文：优先 plugin.usage，否则由 description/triggers/examples 拼装。
pub fn usage_doc(manifest: &PluginManifest) -> String {
    let usage = manifest.plugin.usage.trim();
    if !usage.is_empty() {
        return usage.to_string();
    }
    let mut out = String::new();
    out.push_str(&capability_blurb(manifest));
    out.push_str("\n\n触发方式\n");
    let mut any_trigger = false;
    for provider in &manifest.contributes.providers {
        for trigger in &provider.triggers {
            match trigger {
                Trigger::Prefix { value } if !value.is_empty() => {
                    out.push_str(&format!("  在搜索框以 {value} 开头输入内容\n"));
                    any_trigger = true;
                }
                Trigger::Keyword { value } if !value.is_empty() => {
                    out.push_str(&format!("  在搜索框输入 {value}\n"));
                    any_trigger = true;
                }
                _ => {}
            }
        }
    }
    if !any_trigger {
        out.push_str("  见下方命令入口\n");
    }
    if !manifest.contributes.commands.is_empty() {
        out.push_str("\n命令入口\n");
        for cmd in &manifest.contributes.commands {
            let kw = if cmd.keywords.is_empty() {
                String::new()
            } else {
                format!("（{}）", cmd.keywords.join(" / "))
            };
            out.push_str(&format!("  {}{}\n", cmd.title, kw));
        }
    }
    let examples = try_examples(manifest);
    if !examples.is_empty() {
        out.push_str("\n示例\n");
        for e in examples {
            out.push_str(&format!("  {e}\n"));
        }
    }
    out
}

/// 试用示例查询：manifest.examples 优先，否则从 Trigger/Command 推导。
pub fn try_examples(manifest: &PluginManifest) -> Vec<String> {
    if !manifest.contributes.examples.is_empty() {
        return manifest
            .contributes
            .examples
            .iter()
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .collect();
    }
    let mut out: Vec<String> = Vec::new();
    for provider in &manifest.contributes.providers {
        for trigger in &provider.triggers {
            let sample = match trigger {
                Trigger::Prefix { value } if !value.is_empty() => format!("{value}1+1"),
                Trigger::Keyword { value } if !value.is_empty() => value.clone(),
                _ => continue,
            };
            if !out.contains(&sample) {
                out.push(sample);
            }
        }
    }
    if out.is_empty() {
        for cmd in &manifest.contributes.commands {
            let sample = cmd
                .keywords
                .iter()
                .find(|k| !k.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| cmd.title.clone());
            if !sample.is_empty() && !out.contains(&sample) {
                out.push(sample);
            }
        }
    }
    out
}

/// 主界面发现用短提示：每个启用插件取一条较短示例，最多 max 条。
pub fn short_try_samples(registry: &PluginRegistry, max: usize) -> Vec<String> {
    if max == 0 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for plugin in registry.enabled_plugins() {
        if matches!(
            plugin.phase,
            RuntimePhase::Disabled | RuntimePhase::Incompatible
        ) {
            continue;
        }
        let samples = try_examples(&plugin.manifest);
        let pick = samples
            .iter()
            .filter(|s| s.chars().count() <= 10)
            .min_by_key(|s| s.chars().count())
            .or_else(|| samples.iter().min_by_key(|s| s.chars().count()))
            .cloned();
        if let Some(s) = pick {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

/// 首启能力提示文案；无可用示例时返回 None。
pub fn first_run_hint_text(hints: &[String]) -> Option<String> {
    if hints.is_empty() {
        None
    } else {
        Some(format!("插件已就绪：{}", hints.join(" · ")))
    }
}

/// 官方插件中文展示元数据：(显示名, 一句话说明, 详细用法)。
/// AppData 里仍是旧英文包时，设置页用这里兜底，避免用户看到开发者英文。
pub fn official_display_meta(id: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match id {
        "com.kite.calculator" => Some((
            "计算器",
            "在搜索框输入 = 直接计算，支持括号与优先级",
            "在搜索框输入 = 加上算式即可计算。\n\n示例\n  =1+1\n  =1+1*(2+1)\n  =1000,000+1\n  =2^3\n  =√9+1\n\n支持\n  + - * / %、括号、优先级、千分位\n  隐式乘法（如 1+1(1+1)=3）\n  ^ 乘方、√ 开方\n\n操作\n  Enter 复制结果\n  也可搜索「计算器」进入计算模式",
        )),
        "com.kite.window-switcher" => Some((
            "窗口切换",
            "输入 win 搜索并切换打开的窗口",
            "在搜索框输入 win，列出当前打开的窗口。\n\n用法\n  win          列出全部窗口\n  win 关键字   按标题过滤\n\n操作\n  ↑↓ 选择\n  Enter 激活选中窗口\n\n命令入口\n  窗口切换（win / window / 窗口）",
        )),
        "com.kite.devtools" => Some((
            "开发者工具",
            "uuid / hash / 时间戳 / JSON / Base64 等常用开发工具",
            "在搜索框输入关键词，触发对应开发工具。\n\n工具\n  uuid\n    生成一条 UUID，Enter 复制。\n\n  hash <文本>\n    打开独立 Hash 工具窗，预填文本并计算 SHA-256。\n\n  ts\n    时间戳相关查询。\n\n  json\n    搜索 json，列表只显示「JSON 工具」，Enter 打开独立窗。\n    也可在本插件说明页点「打开 JSON 工具」（需确认）。\n    左原始、右结果：格式化 / 压缩 / 复制。\n    不改动启动器主窗口；Esc 或 × 只关工具窗。\n\n  json <JSON>\n    搜索后 Enter 打开工具窗，并自动格式化预填。\n    示例：json {\"name\":\"kite\",\"n\":1}\n\n  base64\n    搜索 base64，列表只显示「Base64 工具」，Enter 打开独立窗。\n    可在文本与 Base64 之间编码或解码；Esc 或 × 只关工具窗。\n\n  base64 <文本>\n    搜索后 Enter 打开工具窗，预填文本并自动编码。\n    示例：base64 hello\n\n说明\n  uuid / ts 为内联结果。Hash、JSON 和 Base64 使用独立窗口。\n  命令入口：搜索「生成 UUID」。",
        )),
        _ => None,
    }
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{4e00}'..='\u{9fff}'))
}

fn looks_untranslated(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true;
    }
    if t.contains("Official plugin") {
        return true;
    }
    !has_cjk(t)
}

/// 设置页显示名：旧英文官方包时用中文兜底。
pub fn display_plugin_name(manifest: &PluginManifest) -> String {
    if let Some((name, _, _)) = official_display_meta(&manifest.plugin.id) {
        if looks_untranslated(&manifest.plugin.name)
            || looks_untranslated(&manifest.plugin.description)
        {
            return name.to_string();
        }
    }
    let n = manifest.plugin.name.trim();
    if n.is_empty() {
        manifest.plugin.id.clone()
    } else {
        n.to_string()
    }
}

/// 设置页一句话说明。官方插件以宿主文案为准，避免 AppData 旧包继续展示过时用法。
pub fn display_plugin_blurb(manifest: &PluginManifest) -> String {
    if let Some((_, blurb, _)) = official_display_meta(&manifest.plugin.id) {
        return blurb.to_string();
    }
    capability_blurb(manifest)
}

/// 设置页详细用法正文。官方插件以宿主文案为准。
pub fn display_plugin_usage(manifest: &PluginManifest) -> String {
    if let Some((_, _, usage)) = official_display_meta(&manifest.plugin.id) {
        return usage.to_string();
    }
    usage_doc(manifest)
}

#[cfg(test)]
mod discover_tests {
    use super::*;
    use crate::plugin::manifest::{
        CommandAction, Compatibility, Contributions, PluginCommand, PluginIdentity, PluginManifest,
        PluginProvider, RuntimeSpec,
    };
    use crate::plugin::registry::{PluginRegistry, RegisteredPlugin, RuntimePhase};
    use std::path::PathBuf;

    fn manifest(desc: &str, examples: &[&str]) -> PluginManifest {
        PluginManifest {
            schema_version: 1,
            plugin: PluginIdentity {
                id: "com.kite.demo".into(),
                name: "演示".into(),
                version: "0.1.0".into(),
                description: desc.into(),
                usage: String::new(),
                author: "Kite".into(),
            },
            compatibility: Compatibility {
                plugin_api: 1,
                minimum_kite_version: None,
            },
            runtime: RuntimeSpec {
                command: "demo.exe".into(),
                args: vec![],
                startup_timeout_ms: None,
                idle_timeout_ms: None,
            },
            contributes: Contributions {
                examples: examples.iter().map(|s| s.to_string()).collect(),
                commands: vec![PluginCommand {
                    id: "open".into(),
                    title: "打开".into(),
                    keywords: vec!["demo".into()],
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
        }
    }

    #[test]
    fn blurb_prefers_description() {
        let m = manifest("用 = 计算", &["=1+1"]);
        assert_eq!(capability_blurb(&m), "用 = 计算");
        let m = manifest("", &[]);
        assert_eq!(capability_blurb(&m), "演示");
    }

    #[test]
    fn examples_fallback_to_trigger() {
        let m = manifest("x", &[]);
        assert_eq!(try_examples(&m), vec!["=1+1".to_string()]);
        let m = manifest("x", &["=1+1", "win"]);
        assert_eq!(try_examples(&m), vec!["=1+1".to_string(), "win".to_string()]);
    }

    #[test]
    fn short_samples_from_registry() {
        let mut reg = PluginRegistry::new();
        reg.insert_loaded(RegisteredPlugin {
            manifest: manifest("计算", &["=1+1*(2+1)", "=1+1"]),
            root: PathBuf::from("."),
            enabled: true,
            phase: RuntimePhase::Dormant,
            last_error: None,
        });
        assert_eq!(short_try_samples(&reg, 3), vec!["=1+1".to_string()]);
        assert_eq!(
            first_run_hint_text(&short_try_samples(&reg, 3)).as_deref(),
            Some("插件已就绪：=1+1")
        );
    }

    #[test]
    fn official_display_meta_covers_builtin_plugins() {
        let (name, blurb, usage) = official_display_meta("com.kite.calculator").unwrap();
        assert_eq!(name, "计算器");
        assert!(blurb.contains('='));
        assert!(usage.contains("示例"));
        let m = {
            let mut m = manifest("Official plugin: calculator", &[]);
            m.plugin.id = "com.kite.calculator".into();
            m.plugin.name = "Calculator".into();
            m
        };
        assert_eq!(display_plugin_name(&m), "计算器");
        assert!(display_plugin_blurb(&m).contains("计算"));
        assert!(display_plugin_usage(&m).contains("Enter"));

        // AppData 里仍是旧中文包时，说明也必须跟宿主产品文案。
        let mut dt = manifest("旧说明", &[]);
        dt.plugin.id = "com.kite.devtools".into();
        dt.plugin.usage = "独立面板：设置 → 插件 → 打开 JSON 面板。".into();
        let usage = display_plugin_usage(&dt);
        assert!(usage.contains("JSON 工具"), "官方用法不得沿用旧「面板」文案");
        assert!(!usage.contains("打开 JSON 面板"), "旧入口名应被宿主文案覆盖");
        assert!(display_plugin_blurb(&dt).contains("Base64"));
        assert!(usage.contains("base64 hello"));
    }

    #[test]
    fn usage_doc_prefers_manifest_usage() {
        let mut m = manifest("一句话", &["=1+1"]);
        assert!(usage_doc(&m).contains("触发方式"));
        m.plugin.usage = "详细用法\n第二行".into();
        assert_eq!(usage_doc(&m), "详细用法\n第二行");
    }

    #[test]
    fn parse_manifest_requires_description() {
        let ok = parse_manifest(
            r#"{
            "schema_version": 1,
            "plugin": {"id":"com.kite.a","name":"A","version":"1","description":"说明"},
            "compatibility": {"plugin_api":1},
            "runtime": {"command":"a.exe"},
            "contributes": {"examples":["x"]}
        }"#,
        );
        assert!(ok.is_ok());
        let bad = parse_manifest(
            r#"{
            "schema_version": 1,
            "plugin": {"id":"com.kite.a","name":"A","version":"1","description":""},
            "compatibility": {"plugin_api":1},
            "runtime": {"command":"a.exe"},
            "contributes": {"examples":["x"]}
        }"#,
        );
        assert!(bad.unwrap_err().contains("description"));
    }
}
