//! Activation Router：Query → 是否命中插件 Trigger。
//!
//! 必须非常轻：普通搜索热路径只增加少量 prefix/keyword 判断。
//! V1 禁止 Global Provider。

use serde::{Deserialize, Serialize};

use super::registry::PluginRegistry;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    Prefix { value: String },
    Keyword { value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    List,
    Panel,
    Empty,
}

impl ResponseMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "list" => ResponseMode::List,
            "panel" => ResponseMode::Panel,
            _ => ResponseMode::Empty,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    pub plugin_id: String,
    pub provider_id: String,
    pub effective_query: String,
    pub raw_query: String,
    pub response_mode: ResponseMode,
}

/// 路由缝：纯函数。未命中返回 None，普通搜索不受影响。
pub fn route_query(query: &str, registry: &PluginRegistry) -> Option<Activation> {
    route_with_contributions(query, registry.enabled_plugins())
}

pub fn route_with_contributions<'a>(
    query: &str,
    plugins: impl Iterator<Item = &'a super::registry::RegisteredPlugin>,
) -> Option<Activation> {
    let raw = query;
    for plugin in plugins {
        if !plugin.enabled {
            continue;
        }
        for provider in &plugin.manifest.contributes.providers {
            for trigger in &provider.triggers {
                if let Some(effective) = match_trigger(raw, trigger) {
                    return Some(Activation {
                        plugin_id: plugin.manifest.plugin.id.clone(),
                        provider_id: provider.id.clone(),
                        effective_query: effective,
                        raw_query: raw.to_string(),
                        response_mode: ResponseMode::parse(&provider.response_mode),
                    });
                }
            }
        }
    }
    None
}

/// Trigger 匹配；返回 effective query（去掉 prefix/keyword）。
pub fn match_trigger(query: &str, trigger: &Trigger) -> Option<String> {
    match trigger {
        Trigger::Prefix { value } => {
            if value.is_empty() {
                return None;
            }
            query.strip_prefix(value.as_str()).map(|s| s.to_string())
        }
        Trigger::Keyword { value } => {
            if value.is_empty() {
                return None;
            }
            if query == value {
                return Some(String::new());
            }
            let rest = query.strip_prefix(value.as_str())?;
            // 仅 keyword 或 keyword+空白 才匹配；tree 不能命中 tr
            if rest.starts_with(' ') || rest.starts_with('\t') {
                return Some(rest.trim_start().to_string());
            }
            None
        }
    }
}

/// Command 命中后进入 Provider Mode 的激活描述（不要求 Query 已含 trigger）。
pub fn activation_for_command(
    registry: &PluginRegistry,
    plugin_id: &str,
    command_id: &str,
) -> Option<Activation> {
    let plugin = registry.get(plugin_id)?;
    if !plugin.enabled {
        return None;
    }
    let cmd = plugin
        .manifest
        .contributes
        .commands
        .iter()
        .find(|c| c.id == command_id)?;
    let provider_id = match &cmd.action {
        super::manifest::CommandAction::EnterProvider { provider } => provider.clone(),
        super::manifest::CommandAction::PluginAction { .. } => return None,
    };
    let provider = plugin
        .manifest
        .contributes
        .providers
        .iter()
        .find(|p| p.id == provider_id)?;
    Some(Activation {
        plugin_id: plugin_id.to_string(),
        provider_id,
        effective_query: String::new(),
        raw_query: String::new(),
        response_mode: ResponseMode::parse(&provider.response_mode),
    })
}

/// Provider 触发前缀：写入搜索框，便于继续输入并再次命中 Trigger。
/// prefix → `=`；keyword → `win `（含尾空格）。
pub fn trigger_query_prefix(provider: &super::manifest::PluginProvider) -> String {
    provider
        .triggers
        .iter()
        .find_map(|t| match t {
            Trigger::Prefix { value } => Some(value.clone()),
            Trigger::Keyword { value } => Some(format!("{value} ")),
        })
        .unwrap_or_default()
}

/// 根据 Registry 为 Command 拼出进入 Provider Mode 后的初始 Query。
pub fn initial_query_for_activation(registry: &PluginRegistry, activation: &Activation) -> String {
    let Some(plugin) = registry.get(&activation.plugin_id) else {
        return String::new();
    };
    plugin
        .manifest
        .contributes
        .providers
        .iter()
        .find(|p| p.id == activation.provider_id)
        .map(trigger_query_prefix)
        .unwrap_or_default()
}

/// Command → Provider Mode：解析 Activation 与应写入搜索框的初始 Query。
/// UI 不得自己翻 manifest；List/Panel 共用本函数。
pub fn resolve_command_entry(
    registry: &PluginRegistry,
    plugin_id: &str,
    command_id: &str,
    provider_fallback: Option<&str>,
) -> Option<(Activation, String)> {
    let activation = activation_for_command(registry, plugin_id, command_id).or_else(|| {
        let provider = provider_fallback?;
        let p = registry.get(plugin_id)?;
        if !p.enabled {
            return None;
        }
        let prov = p
            .manifest
            .contributes
            .providers
            .iter()
            .find(|x| x.id == provider)?;
        Some(Activation {
            plugin_id: plugin_id.to_string(),
            provider_id: prov.id.clone(),
            effective_query: String::new(),
            raw_query: String::new(),
            response_mode: ResponseMode::parse(&prov.response_mode),
        })
    })?;
    let query = initial_query_for_activation(registry, &activation);
    Some((activation, query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manifest::{
        CommandAction, Compatibility, Contributions, PluginCommand, PluginIdentity, PluginManifest,
        PluginProvider, RuntimeSpec,
    };
    use crate::plugin::registry::{PluginRegistry, RegisteredPlugin, RuntimePhase};

    fn plugin_with_triggers() -> RegisteredPlugin {
        RegisteredPlugin {
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
                    providers: vec![
                        PluginProvider {
                            id: "calculate".into(),
                            response_mode: "panel".into(),
                            triggers: vec![Trigger::Prefix { value: "=".into() }],
                        },
                        PluginProvider {
                            id: "translate".into(),
                            response_mode: "list".into(),
                            triggers: vec![Trigger::Keyword { value: "tr".into() }],
                        },
                    ],
                },
            },
            root: std::path::PathBuf::from("plugins/com.kite.calculator"),
            enabled: true,
            phase: RuntimePhase::Dormant,
            last_error: None,
        }
    }

    fn registry() -> PluginRegistry {
        let mut r = PluginRegistry::new();
        r.insert_loaded(plugin_with_triggers());
        r
    }

    #[test]
    fn prefix_trigger_enters_provider_mode() {
        let act = route_query("=100*1.13", &registry()).expect("hit");
        assert_eq!(act.plugin_id, "com.kite.calculator");
        assert_eq!(act.provider_id, "calculate");
        assert_eq!(act.effective_query, "100*1.13");
        assert_eq!(act.response_mode, ResponseMode::Panel);
    }

    #[test]
    fn keyword_requires_boundary() {
        assert!(route_query("tr hello", &registry()).is_some());
        assert_eq!(
            route_query("tr hello", &registry()).unwrap().effective_query,
            "hello"
        );
        assert_eq!(route_query("tr", &registry()).unwrap().effective_query, "");
        assert!(route_query("tree", &registry()).is_none());
        assert!(route_query("chrome", &registry()).is_none());
        assert!(route_query("", &registry()).is_none());
    }

    #[test]
    fn ordinary_query_does_not_activate() {
        assert!(route_query("chrome", &registry()).is_none());
        assert!(route_query("wx", &registry()).is_none());
    }

    #[test]
    fn command_enter_provider_activation() {
        let act = activation_for_command(&registry(), "com.kite.calculator", "open")
            .expect("command activation");
        assert_eq!(act.provider_id, "calculate");
        assert_eq!(act.effective_query, "");
    }

    #[test]
    fn resolve_command_entry_sets_trigger_prefix() {
        let reg = registry();
        // Panel prefix：初始 Query = "="
        let (act, q) = resolve_command_entry(&reg, "com.kite.calculator", "open", None)
            .expect("panel command");
        assert_eq!(act.provider_id, "calculate");
        assert_eq!(act.response_mode, ResponseMode::Panel);
        assert_eq!(q, "=");

        // List keyword：初始 Query 必须是 "tr "，否则 route_query 失效
        let mut reg = registry();
        if let Some(p) = reg.get_mut("com.kite.calculator") {
            p.manifest.contributes.commands[0].action = CommandAction::EnterProvider {
                provider: "translate".into(),
            };
        }
        let (act, q) = resolve_command_entry(&reg, "com.kite.calculator", "open", None)
            .expect("list command");
        assert_eq!(act.provider_id, "translate");
        assert_eq!(act.response_mode, ResponseMode::List);
        assert_eq!(q, "tr ");
        assert!(route_query(&q, &reg).is_some(), "initial query must re-enter provider");
    }

    #[test]
    fn match_trigger_boundaries() {
        assert_eq!(
            match_trigger("=1+2", &Trigger::Prefix { value: "=".into() }),
            Some("1+2".into())
        );
        assert_eq!(
            match_trigger("x=1", &Trigger::Prefix { value: "=".into() }),
            None
        );
        assert_eq!(
            match_trigger("tr  x", &Trigger::Keyword { value: "tr".into() }),
            Some("x".into())
        );
    }
}
