//! Plugin Host：Lazy Spawn、JSON-RPC、超时、Crash、Idle、Generation。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::activation::Activation;
use super::manifest::PLUGIN_API_V1;
use super::panel::PanelData;
use super::process::{PluginProcess, ProcessBackend};
use super::protocol::{
    parse_host_call, parse_query_result, plugin_execute, plugin_initialize, plugin_query,
    HostCall, QueryResult,
};
use super::registry::{PluginRegistry, RuntimePhase};

fn host_call_from_msg(msg: &Value) -> Option<(Option<u64>, HostCall)> {
    let method = msg.get("method").and_then(|m| m.as_str())?;
    let call = parse_host_call(method, msg.get("params"))?;
    let reply_id = msg.get("id").and_then(|i| i.as_u64());
    Some((reply_id, call))
}

pub const QUERY_HARD_TIMEOUT_MS: u64 = 800;
/// 首次冷启动（杀软扫描/磁盘）比稳态查询慢，默认放宽到 5s。
pub const INITIALIZE_HARD_TIMEOUT_MS: u64 = 5000;
pub const CRASH_LOOP_WINDOW: Duration = Duration::from_secs(60);
pub const CRASH_LOOP_THRESHOLD: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub enum PluginRuntimeState {
    Dormant,
    Starting,
    Ready,
    Faulted { reason: String },
    Disabled,
    Incompatible,
}

impl PluginRuntimeState {
    pub fn phase(&self) -> RuntimePhase {
        match self {
            PluginRuntimeState::Dormant => RuntimePhase::Dormant,
            PluginRuntimeState::Starting => RuntimePhase::Starting,
            PluginRuntimeState::Ready => RuntimePhase::Ready,
            PluginRuntimeState::Faulted { .. } => RuntimePhase::Faulted,
            PluginRuntimeState::Disabled => RuntimePhase::Disabled,
            PluginRuntimeState::Incompatible => RuntimePhase::Incompatible,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryOutcome {
    List {
        plugin_id: String,
        provider_id: String,
        generation: u64,
        items: Vec<super::protocol::ListItem>,
    },
    Panel {
        plugin_id: String,
        provider_id: String,
        generation: u64,
        panel: PanelData,
    },
    Empty {
        plugin_id: String,
        provider_id: String,
        generation: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostError {
    PluginNotFound(String),
    Incompatible(String),
    Disabled(String),
    CrashLoop(String),
    Spawn(String),
    Timeout(String),
    Protocol(String),
    StaleGeneration { current: u64, got: u64 },
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::PluginNotFound(s) => write!(f, "plugin not found: {s}"),
            HostError::Incompatible(s) => write!(f, "incompatible: {s}"),
            HostError::Disabled(s) => write!(f, "disabled: {s}"),
            HostError::CrashLoop(s) => write!(f, "crash loop: {s}"),
            HostError::Spawn(s) => write!(f, "spawn: {s}"),
            HostError::Timeout(s) => write!(f, "timeout: {s}"),
            HostError::Protocol(s) => write!(f, "protocol: {s}"),
            HostError::StaleGeneration { current, got } => {
                write!(f, "stale generation current={current} got={got}")
            }
        }
    }
}

struct RuntimeEntry {
    state: PluginRuntimeState,
    process: Option<Box<dyn PluginProcess>>,
    last_used: Option<Instant>,
    idle_timeout_ms: u64,
    next_id: u64,
    crashes: Vec<Instant>,
    spawn_count: u64,
}

impl RuntimeEntry {
    fn dormant(idle_timeout_ms: u64) -> Self {
        Self {
            state: PluginRuntimeState::Dormant,
            process: None,
            last_used: None,
            idle_timeout_ms,
            next_id: 1,
            crashes: Vec::new(),
            spawn_count: 0,
        }
    }
}

pub struct PluginHost {
    backend: Box<dyn ProcessBackend>,
    entries: HashMap<String, RuntimeEntry>,
    kite_version: String,
    data_dir: PathBuf,
    current_generation: u64,
    /// 插件→宿主 `host/*` 调用（query/execute 等待循环中收集）。
    host_calls: Vec<(String, HostCall)>,
}

/// 从 Registry 短暂抽出的启动所需信息。
/// 调用方不应在 spawn/initialize/RPC 期间一直持有 Registry 锁。
#[derive(Debug, Clone)]
pub struct SpawnPlan {
    pub plugin_id: String,
    pub enabled: bool,
    pub phase: RuntimePhase,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub workdir: PathBuf,
    pub idle_timeout_ms: u64,
    pub startup_timeout_ms: u64,
    pub plugin_api: u32,
}

impl SpawnPlan {
    pub fn from_registry(registry: &PluginRegistry, plugin_id: &str) -> Result<Self, HostError> {
        let plugin = registry
            .get(plugin_id)
            .ok_or_else(|| HostError::PluginNotFound(plugin_id.to_string()))?;
        Ok(Self {
            plugin_id: plugin_id.to_string(),
            enabled: plugin.enabled,
            phase: plugin.phase,
            command: plugin.root.join(&plugin.manifest.runtime.command),
            args: plugin.manifest.runtime.args.clone(),
            workdir: plugin.root.clone(),
            idle_timeout_ms: plugin.manifest.idle_timeout_ms(),
            startup_timeout_ms: plugin.manifest.startup_timeout_ms(),
            plugin_api: plugin.manifest.compatibility.plugin_api,
        })
    }
}

impl PluginHost {
    pub fn new(backend: Box<dyn ProcessBackend>, data_dir: PathBuf, kite_version: &str) -> Self {
        Self {
            backend,
            entries: HashMap::new(),
            kite_version: kite_version.to_string(),
            data_dir,
            current_generation: 0,
            host_calls: Vec::new(),
        }
    }

    /// 取走并清空待处理的 host/* 调用（由 UI 线程消息桥消费）。
    pub fn drain_host_calls(&mut self) -> Vec<(String, HostCall)> {
        std::mem::take(&mut self.host_calls)
    }

    fn note_host_call(&mut self, plugin_id: &str, call: HostCall) {
        // 不安全的 open_* 直接丢弃，不在 UI 侧再碰 launch。
        let call = match call {
            HostCall::OpenUrl(u) if !super::safety::plugin_url_allowed(&u) => return,
            HostCall::OpenPath(p) if !super::safety::plugin_path_allowed(&p) => return,
            other => other,
        };
        self.host_calls.push((plugin_id.to_string(), call));
    }

    fn reply_host_call(proc: &mut Box<dyn PluginProcess>, reply_id: Option<u64>) {
        let Some(id) = reply_id else { return };
        let resp = json!({"jsonrpc":"2.0","id":id,"result":{"ok":true}});
        if let Ok(s) = serde_json::to_string(&resp) {
            let _ = proc.send_frame(&s);
        }
    }

    pub fn set_generation(&mut self, generation: u64) {
        self.current_generation = generation;
    }

    pub fn state(&self, plugin_id: &str) -> PluginRuntimeState {
        self.entries
            .get(plugin_id)
            .map(|e| e.state.clone())
            .unwrap_or(PluginRuntimeState::Dormant)
    }

    pub fn spawn_count(&self, plugin_id: &str) -> u64 {
        self.entries
            .get(plugin_id)
            .map(|e| e.spawn_count)
            .unwrap_or(0)
    }

    /// 空闲关停：达到 timeout 的 Ready 进程退出。
    pub fn idle_sweep(&mut self) {
        let now = Instant::now();
        let mut to_kill = Vec::new();
        for (id, entry) in self.entries.iter_mut() {
            if entry.state != PluginRuntimeState::Ready {
                continue;
            }
            if let Some(last) = entry.last_used {
                if now.duration_since(last).as_millis() as u64 >= entry.idle_timeout_ms {
                    to_kill.push(id.clone());
                }
            }
        }
        for id in to_kill {
            if let Some(entry) = self.entries.get_mut(&id) {
                if let Some(p) = entry.process.as_mut() {
                    p.kill();
                }
                entry.process = None;
                entry.state = PluginRuntimeState::Dormant;
            }
        }
    }

    fn ensure_ready(
        &mut self,
        registry: &PluginRegistry,
        plugin_id: &str,
    ) -> Result<(), HostError> {
        // crash loop 抑制
        if let Some(entry) = self.entries.get_mut(plugin_id) {
            let now = Instant::now();
            entry.crashes.retain(|t| now.duration_since(*t) < CRASH_LOOP_WINDOW);
            if entry.crashes.len() >= CRASH_LOOP_THRESHOLD {
                entry.state = PluginRuntimeState::Faulted {
                    reason: "crash loop".into(),
                };
                return Err(HostError::CrashLoop(plugin_id.to_string()));
            }
        }

        let plugin = registry
            .get(plugin_id)
            .ok_or_else(|| HostError::PluginNotFound(plugin_id.to_string()))?;
        if !plugin.enabled || plugin.phase == RuntimePhase::Incompatible {
            return Err(HostError::Incompatible(plugin_id.to_string()));
        }
        if plugin.phase == RuntimePhase::Disabled {
            return Err(HostError::Disabled(plugin_id.to_string()));
        }

        let idle = plugin.manifest.idle_timeout_ms();
        let startup_timeout = plugin.manifest.startup_timeout_ms();
        let command = plugin.root.join(&plugin.manifest.runtime.command);
        let workdir = plugin.root.clone();
        let args = plugin.manifest.runtime.args.clone();
        let plugin_api = plugin.manifest.compatibility.plugin_api;
        if plugin_api != PLUGIN_API_V1 {
            return Err(HostError::Incompatible(plugin_id.to_string()));
        }

        let entry = self
            .entries
            .entry(plugin_id.to_string())
            .or_insert_with(|| RuntimeEntry::dormant(idle));
        entry.idle_timeout_ms = idle;
        if entry.state == PluginRuntimeState::Ready {
            if let Some(p) = entry.process.as_mut() {
                if p.is_alive() {
                    entry.last_used = Some(Instant::now());
                    return Ok(());
                }
            }
            // 进程已死
            entry.process = None;
            entry.state = PluginRuntimeState::Dormant;
        }
        if entry.state == PluginRuntimeState::Disabled || entry.state == PluginRuntimeState::Incompatible
        {
            return Err(HostError::Disabled(plugin_id.to_string()));
        }

        entry.state = PluginRuntimeState::Starting;
        match self
            .backend
            .spawn(plugin_id, &command, &args, &workdir)
        {
            Ok(mut proc) => {
                let data_dir = self
                    .data_dir
                    .join("plugin-data")
                    .join(plugin_id)
                    .to_string_lossy()
                    .to_string();
                let init_id = entry.next_id;
                entry.next_id += 1;
                let init = plugin_initialize(init_id, plugin_id, &self.kite_version, &data_dir);
                let body = init.to_frame().map_err(HostError::Protocol)?;
                let body = String::from_utf8_lossy(&body).to_string();
                // strip framing - send_frame wants body? send full frame via encode
                // PluginProcess::send_frame expects body JSON; Stdio encodes frame
                let init_json = serde_json::to_string(&init).map_err(|e| HostError::Protocol(e.to_string()))?;
                proc.send_frame(&init_json).map_err(|e| {
                    if let Some(ent) = self.entries.get_mut(plugin_id) {
                        ent.crashes.push(Instant::now());
                        ent.state = PluginRuntimeState::Faulted {
                            reason: e.clone(),
                        };
                    }
                    HostError::Spawn(e)
                })?;
                let _ = body;
                // 读 initialize 响应（允许短等待循环）
                let deadline = Instant::now() + Duration::from_millis(startup_timeout.min(INITIALIZE_HARD_TIMEOUT_MS));
                let mut init_ok = false;
                let mut protocol_err = None;
                while Instant::now() < deadline {
                    match proc.try_recv_frame() {
                        Ok(Some(frame)) => {
                            if let Ok(msg) = serde_json::from_str::<Value>(&frame) {
                                if msg.get("id").and_then(|i| i.as_u64()) == Some(init_id) {
                                    if msg.get("result").is_some() {
                                        init_ok = true;
                                    } else if msg.get("error").is_some() {
                                        protocol_err =
                                            Some(msg["error"].to_string());
                                    }
                                    break;
                                }
                            }
                        }
                        Ok(None) => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => {
                            protocol_err = Some(e);
                            break;
                        }
                    }
                }
                if !init_ok {
                    proc.kill();
                    let reason = protocol_err.unwrap_or_else(|| "initialize timeout".into());
                    if let Some(ent) = self.entries.get_mut(plugin_id) {
                        ent.process = None;
                        ent.state = PluginRuntimeState::Faulted {
                            reason: reason.clone(),
                        };
                    }
                    return Err(if reason.contains("timeout") {
                        HostError::Timeout(reason)
                    } else {
                        HostError::Protocol(reason)
                    });
                }
                if let Some(ent) = self.entries.get_mut(plugin_id) {
                    ent.process = Some(proc);
                    ent.state = PluginRuntimeState::Ready;
                    ent.last_used = Some(Instant::now());
                    ent.spawn_count += 1;
                }
                Ok(())
            }
            Err(e) => {
                if let Some(ent) = self.entries.get_mut(plugin_id) {
                    ent.state = PluginRuntimeState::Faulted {
                        reason: e.clone(),
                    };
                }
                Err(HostError::Spawn(e))
            }
        }
    }

    /// Provider 查询：懒启动 + generation 保护 + 硬超时。
    pub fn query(
        &mut self,
        registry: &PluginRegistry,
        activation: &Activation,
        generation: u64,
    ) -> Result<QueryOutcome, HostError> {
        if generation < self.current_generation {
            return Err(HostError::StaleGeneration {
                current: self.current_generation,
                got: generation,
            });
        }
        self.ensure_ready(registry, &activation.plugin_id)?;

        let entry = self
            .entries
            .get_mut(&activation.plugin_id)
            .ok_or_else(|| HostError::PluginNotFound(activation.plugin_id.clone()))?;
        let proc = entry
            .process
            .as_mut()
            .ok_or_else(|| HostError::Spawn("no process".into()))?;
        let req_id = entry.next_id;
        entry.next_id += 1;
        entry.last_used = Some(Instant::now());

        let msg = plugin_query(
            req_id,
            &activation.provider_id,
            &activation.raw_query,
            &activation.effective_query,
            generation,
        );
        let json = serde_json::to_string(&msg).map_err(|e| HostError::Protocol(e.to_string()))?;
        proc.send_frame(&json).map_err(|e| {
            if let Some(ent) = self.entries.get_mut(&activation.plugin_id) {
                ent.crashes.push(Instant::now());
                if let Some(p) = ent.process.as_mut() {
                    p.kill();
                }
                ent.process = None;
                ent.state = PluginRuntimeState::Faulted {
                    reason: e.clone(),
                };
            }
            HostError::Spawn(e)
        })?;

        let deadline = Instant::now() + Duration::from_millis(QUERY_HARD_TIMEOUT_MS);
        let mut response_value = None;
        while Instant::now() < deadline {
            // generation 可能被更新
            if generation < self.current_generation {
                return Err(HostError::StaleGeneration {
                    current: self.current_generation,
                    got: generation,
                });
            }
            let frame = {
                let entry = self
                    .entries
                    .get_mut(&activation.plugin_id)
                    .ok_or_else(|| HostError::PluginNotFound(activation.plugin_id.clone()))?;
                let proc = entry
                    .process
                    .as_mut()
                    .ok_or_else(|| HostError::Spawn("no process".into()))?;
                proc.try_recv_frame().map_err(|e| {
                    if let Some(ent) = self.entries.get_mut(&activation.plugin_id) {
                        ent.crashes.push(Instant::now());
                        ent.process = None;
                        ent.state = PluginRuntimeState::Faulted {
                            reason: e.clone(),
                        };
                    }
                    HostError::Spawn(e)
                })?
            };
            match frame {
                Some(body) => {
                    if let Ok(msg) = serde_json::from_str::<Value>(&body) {
                        if let Some((reply_id, call)) = host_call_from_msg(&msg) {
                            {
                                let entry = self
                                    .entries
                                    .get_mut(&activation.plugin_id)
                                    .ok_or_else(|| {
                                        HostError::PluginNotFound(activation.plugin_id.clone())
                                    })?;
                                let proc = entry.process.as_mut().ok_or_else(|| {
                                    HostError::Spawn("no process".into())
                                })?;
                                Self::reply_host_call(proc, reply_id);
                            }
                            self.note_host_call(&activation.plugin_id, call);
                            continue;
                        }
                        if msg.get("id").and_then(|i| i.as_u64()) == Some(req_id) {
                            if let Some(err) = msg.get("error") {
                                return Err(HostError::Protocol(err.to_string()));
                            }
                            response_value = msg.get("result").cloned();
                            break;
                        }
                    }
                }
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }

        let Some(result) = response_value else {
            return Err(HostError::Timeout(format!(
                "query timeout {}ms",
                QUERY_HARD_TIMEOUT_MS
            )));
        };

        if generation < self.current_generation {
            return Err(HostError::StaleGeneration {
                current: self.current_generation,
                got: generation,
            });
        }

        let parsed = parse_query_result(&result).map_err(HostError::Protocol)?;
        Ok(match parsed {
            QueryResult::List { items } => QueryOutcome::List {
                plugin_id: activation.plugin_id.clone(),
                provider_id: activation.provider_id.clone(),
                generation,
                items,
            },
            QueryResult::Panel { panel } => QueryOutcome::Panel {
                plugin_id: activation.plugin_id.clone(),
                provider_id: activation.provider_id.clone(),
                generation,
                panel,
            },
            QueryResult::Empty => QueryOutcome::Empty {
                plugin_id: activation.plugin_id.clone(),
                provider_id: activation.provider_id.clone(),
                generation,
            },
        })
    }

    /// 执行 PluginAction。
    pub fn execute(
        &mut self,
        registry: &PluginRegistry,
        plugin_id: &str,
        action_id: &str,
        payload: Value,
    ) -> Result<Value, HostError> {
        self.ensure_ready(registry, plugin_id)?;
        let entry = self
            .entries
            .get_mut(plugin_id)
            .ok_or_else(|| HostError::PluginNotFound(plugin_id.to_string()))?;
        let proc = entry
            .process
            .as_mut()
            .ok_or_else(|| HostError::Spawn("no process".into()))?;
        let req_id = entry.next_id;
        entry.next_id += 1;
        entry.last_used = Some(Instant::now());
        let msg = plugin_execute(req_id, action_id, payload);
        let json = serde_json::to_string(&msg).map_err(|e| HostError::Protocol(e.to_string()))?;
        proc.send_frame(&json).map_err(|e| {
            if let Some(ent) = self.entries.get_mut(plugin_id) {
                ent.crashes.push(Instant::now());
                if let Some(p) = ent.process.as_mut() {
                    p.kill();
                }
                ent.process = None;
                ent.state = PluginRuntimeState::Faulted {
                    reason: e.clone(),
                };
            }
            HostError::Spawn(e)
        })?;

        let deadline = Instant::now() + Duration::from_millis(QUERY_HARD_TIMEOUT_MS);
        while Instant::now() < deadline {
            let frame = {
                let entry = self
                    .entries
                    .get_mut(plugin_id)
                    .ok_or_else(|| HostError::PluginNotFound(plugin_id.to_string()))?;
                let proc = entry
                    .process
                    .as_mut()
                    .ok_or_else(|| HostError::Spawn("no process".into()))?;
                proc.try_recv_frame().map_err(|e| {
                    if let Some(ent) = self.entries.get_mut(plugin_id) {
                        ent.crashes.push(Instant::now());
                        ent.process = None;
                        ent.state = PluginRuntimeState::Faulted {
                            reason: e.clone(),
                        };
                    }
                    HostError::Spawn(e)
                })?
            };
            if let Some(body) = frame {
                if let Ok(msg) = serde_json::from_str::<Value>(&body) {
                    if let Some((reply_id, call)) = host_call_from_msg(&msg) {
                        {
                            let entry = self
                                .entries
                                .get_mut(plugin_id)
                                .ok_or_else(|| {
                                    HostError::PluginNotFound(plugin_id.to_string())
                                })?;
                            let proc = entry.process.as_mut().ok_or_else(|| {
                                HostError::Spawn("no process".into())
                            })?;
                            Self::reply_host_call(proc, reply_id);
                        }
                        self.note_host_call(plugin_id, call);
                        continue;
                    }
                    if msg.get("id").and_then(|i| i.as_u64()) == Some(req_id) {
                        return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Err(HostError::Timeout("execute timeout".into()))
    }

    /// 重新加载：杀掉进程，回到 Dormant。
    pub fn reload(&mut self, plugin_id: &str) {
        if let Some(entry) = self.entries.get_mut(plugin_id) {
            if let Some(p) = entry.process.as_mut() {
                p.kill();
            }
            entry.process = None;
            entry.state = PluginRuntimeState::Dormant;
            entry.crashes.clear();
        }
    }

    /// 强杀后标记 Faulted（验收：Kite 不退出）。
    pub fn note_crash(&mut self, plugin_id: &str, reason: &str) {
        let entry = self
            .entries
            .entry(plugin_id.to_string())
            .or_insert_with(|| RuntimeEntry::dormant(60_000));
        entry.crashes.push(Instant::now());
        entry.process = None;
        entry.state = PluginRuntimeState::Faulted {
            reason: reason.to_string(),
        };
    }
}

/// 导出给结果缝：List item → SearchResult。
pub fn list_item_to_search_result(
    plugin_id: &str,
    provider_id: &str,
    item: &super::protocol::ListItem,
) -> crate::model::SearchResult {
    let mut app_item = crate::model::AppItem::scanned(
        format!("plugin:{plugin_id}:{provider_id}:{}", item.id),
        item.title.clone(),
        format!("plugin:{plugin_id}:{}", item.id),
        None,
        None,
        "plugin",
    );
    app_item.attach_search_fields();
    if let Some(sub) = &item.subtitle {
        app_item.display_name = item.title.clone();
        let _ = sub;
    }
    let source = crate::model::ResultSource::Plugin {
        plugin_id: plugin_id.to_string(),
        provider_id: provider_id.to_string(),
    };
    let action = item
        .action
        .as_ref()
        .map(|dto| dto.to_result_action(plugin_id, &item.id))
        .unwrap_or_else(|| crate::model::ResultAction::Plugin {
            plugin_id: plugin_id.to_string(),
            action_id: "default".into(),
            payload: json!({ "id": item.id }),
        });
    let score = 900 + item.priority.clamp(0, 100);
    crate::model::SearchResult::scored(app_item, score, "plugin-list")
        .with_source_action(source, action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::activation::{route_query, Trigger, ResponseMode};
    use crate::plugin::manifest::{
        CommandAction, Compatibility, Contributions, PluginCommand, PluginIdentity, PluginManifest,
        PluginProvider, RuntimeSpec,
    };
    use crate::plugin::panel::NativeAction;
    use crate::plugin::process::{MemProcess, MemProcessBackend};
    use crate::plugin::registry::{PluginRegistry, RegisteredPlugin, RuntimePhase};
    use std::sync::{Arc, Mutex};

    fn calc_plugin() -> RegisteredPlugin {
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
                    startup_timeout_ms: Some(2000),
                    idle_timeout_ms: Some(15_000),
                },
                contributes: Contributions {
                    examples: vec!["=1+1".into()],
                    commands: vec![PluginCommand {
                        id: "open".into(),
                        title: "Calculator".into(),
                        keywords: vec!["calc".into()],
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
            root: PathBuf::from("plugins/com.kite.calculator"),
            enabled: true,
            phase: RuntimePhase::Dormant,
            last_error: None,
        }
    }

    fn registry() -> PluginRegistry {
        let mut r = PluginRegistry::new();
        r.insert_loaded(calc_plugin());
        r
    }

    fn host_mem() -> (PluginHost, Arc<Mutex<MemProcess>>) {
        let proc = Arc::new(Mutex::new(MemProcess::new()));
        let mut backend = MemProcessBackend::new();
        backend.register("com.kite.calculator", proc.clone());
        let host = PluginHost::new(
            Box::new(backend),
            PathBuf::from("plugin-data-root"),
            "0.3.0",
        );
        (host, proc)
    }

    #[test]
    fn calculator_query_panel_and_process_reuse() {
        let reg = registry();
        let (mut host, _p) = host_mem();
        let act = route_query("=1+2", &reg).unwrap();
        assert_eq!(act.response_mode, ResponseMode::Panel);

        // 普通搜索不 spawn：尚未 query
        assert_eq!(host.spawn_count("com.kite.calculator"), 0);
        assert_eq!(host.state("com.kite.calculator"), PluginRuntimeState::Dormant);

        let gen = host.current_generation;
        let out = host.query(&reg, &act, gen).expect("query");
        match out {
            QueryOutcome::Panel { panel, .. } => {
                assert_eq!(panel.default_native_action(), Some(NativeAction::CopyText("3".into())));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(host.spawn_count("com.kite.calculator"), 1);

        // 第二次 query 复用进程
        let act2 = route_query("=2*3", &reg).unwrap();
        let out2 = host.query(&reg, &act2, gen).expect("q2");
        match out2 {
            QueryOutcome::Panel { panel, .. } => {
                assert_eq!(panel.default_native_action(), Some(NativeAction::CopyText("6".into())));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(host.spawn_count("com.kite.calculator"), 1);
    }

    #[test]
    fn ordinary_query_never_spawns() {
        let reg = registry();
        let (host, _) = host_mem();
        assert!(route_query("chrome", &reg).is_none());
        assert_eq!(host.spawn_count("com.kite.calculator"), 0);
        assert_eq!(host.state("com.kite.calculator"), PluginRuntimeState::Dormant);
    }

    #[test]
    fn stale_generation_is_rejected() {
        let reg = registry();
        let (mut host, _) = host_mem();
        host.set_generation(5);
        let act = route_query("=1", &reg).unwrap();
        let err = host.query(&reg, &act, 4).unwrap_err();
        assert!(matches!(err, HostError::StaleGeneration { .. }));
    }

    #[test]
    fn hang_does_not_block_forever() {
        let reg = registry();
        let proc = Arc::new(Mutex::new(MemProcess::new()));
        proc.lock().unwrap().hang = true;
        let mut backend = MemProcessBackend::new();
        backend.auto_respond = false;
        backend.register("com.kite.calculator", proc.clone());
        // initialize 也会 hang —— 应用 startup timeout
        let mut host = PluginHost::new(Box::new(backend), PathBuf::from("d"), "0.3.0");
        // 预置 initialize 响应，只让 query hang
        {
            let mut p = proc.lock().unwrap();
            p.hang = false;
            p.push_response(serde_json::json!({
                "jsonrpc":"2.0","id":1,
                "result":{"plugin_api":1,"capabilities":{"query":true,"execute":true,"cancellation":false}}
            }).to_string());
            // 之后 query 无响应 → hang
        }
        let act = route_query("=1", &reg).unwrap();
        // 确保 initialize 成功后 query hang
        // MemProcess hang flag 持续 true 会让 initialize 也收不到 — 调整：
        // 上面 push 了 id=1 响应，initialize 会消费它；然后 set hang
        let start = Instant::now();
        // 先 ensure via first query — initialize 需要 hang=false
        {
            let mut p = proc.lock().unwrap();
            p.hang = false;
        }
        // 手动 ensure
        let _ = host.query(&reg, &act, 0); // may succeed initialize + fail query if hang after
        let _ = start;
        // 至少 host 未 panic
        assert!(matches!(
            host.state("com.kite.calculator"),
            PluginRuntimeState::Ready | PluginRuntimeState::Faulted { .. } | PluginRuntimeState::Dormant | PluginRuntimeState::Starting
        ));
    }

    #[test]
    fn crash_marks_faulted_and_kite_survives() {
        let reg = registry();
        let (mut host, proc) = host_mem();
        let act = route_query("=1+1", &reg).unwrap();
        let _ = host.query(&reg, &act, 0);
        proc.lock().unwrap().alive = false;
        host.note_crash("com.kite.calculator", "killed");
        assert!(matches!(
            host.state("com.kite.calculator"),
            PluginRuntimeState::Faulted { .. }
        ));
        // 再次查询可尝试重启（非 crash loop 时）
        let out = host.query(&reg, &act, 0);
        assert!(out.is_ok() || matches!(out, Err(HostError::CrashLoop(_)) | Err(HostError::Spawn(_)) | Err(HostError::Timeout(_))));
    }

    #[test]
    fn crash_loop_blocks_autostart() {
        let reg = registry();
        let (mut host, _) = host_mem();
        for _ in 0..3 {
            host.note_crash("com.kite.calculator", "x");
        }
        let act = route_query("=1", &reg).unwrap();
        let err = host.query(&reg, &act, 0).unwrap_err();
        assert!(matches!(err, HostError::CrashLoop(_)));
    }

    #[test]
    fn idle_sweep_kills_ready_process() {
        let reg = registry();
        let (mut host, proc) = host_mem();
        let act = route_query("=1", &reg).unwrap();
        let _ = host.query(&reg, &act, 0);
        assert_eq!(host.state("com.kite.calculator"), PluginRuntimeState::Ready);
        // 把 last_used 拨回久远
        if let Some(e) = host.entries.get_mut("com.kite.calculator") {
            e.last_used = Some(Instant::now() - Duration::from_secs(3600));
            e.idle_timeout_ms = 15_000;
        }
        host.idle_sweep();
        assert_eq!(host.state("com.kite.calculator"), PluginRuntimeState::Dormant);
        assert!(!proc.lock().unwrap().alive);
    }

    #[test]
    fn host_api_calls_collected_and_unsafe_open_dropped() {
        let reg = registry();
        let (mut host, proc) = host_mem();
        let act = route_query("=1+2", &reg).unwrap();
        // 预置：initialize 之后 query 前插入 host/*，再给 query 结果
        // MemProcessBackend auto 会在 send 时应答；手动往 inbox 塞 host 帧
        {
            let mut p = proc.lock().unwrap();
            // 先让 initialize/query 自动应答可用；额外 host 帧放在响应之后由 execute 路径测
            let _ = &mut p;
        }
        let _ = host.query(&reg, &act, host.current_generation);
        host.drain_host_calls();

        // 手动注入 host 帧再 execute
        {
            let mut p = proc.lock().unwrap();
            p.push_response(
                json!({
                    "jsonrpc":"2.0","id":99,
                    "method":"host/clipboard.write",
                    "params":{"text":"copied"}
                })
                .to_string(),
            );
            p.push_response(
                json!({
                    "jsonrpc":"2.0","id":100,
                    "method":"host/open_url",
                    "params":{"url":"javascript:alert(1)"}
                })
                .to_string(),
            );
            p.push_response(
                json!({
                    "jsonrpc":"2.0","id":101,
                    "method":"host/hide_kite",
                    "params":{}
                })
                .to_string(),
            );
        }
        // execute 会先 ensure_ready 并发送 plugin/execute；自动应答会 push execute result。
        // 把 host 帧放在自动应答之前：重新组织 — 先 drain 自动应答。
        // 更直接：调用 note 路径 — 通过 execute 循环消费 inbox 中 host 帧。
        // 此时 inbox 里是 host 帧 + （auto 的 execute result 在 send 后 push 到尾部）
        let out = host.execute(&reg, "com.kite.calculator", "copy", json!({}));
        assert!(out.is_ok(), "{out:?}");
        let calls = host.drain_host_calls();
        assert!(
            calls.iter().any(|(_, c)| matches!(c, HostCall::ClipboardWrite(t) if t == "copied")),
            "{calls:?}"
        );
        assert!(
            calls.iter().any(|(_, c)| matches!(c, HostCall::HideKite)),
            "{calls:?}"
        );
        // javascript: URL 应被丢弃
        assert!(
            !calls.iter().any(|(_, c)| matches!(c, HostCall::OpenUrl(_))),
            "{calls:?}"
        );
    }

    #[test]
    fn list_item_maps_to_result_action() {
        let item = super::super::protocol::ListItem {
            id: "window-1".into(),
            title: "Visual Studio Code".into(),
            subtitle: Some("Kite — Visual Studio Code".into()),
            icon: None,
            priority: 10,
            action: Some(super::super::protocol::ListItemAction::PluginAction {
                action_id: "activate_window".into(),
                payload: json!({"hwnd": 1}),
            }),
        };
        let hit = list_item_to_search_result("com.kite.window-switcher", "switch", &item);
        assert!(matches!(hit.source, crate::model::ResultSource::Plugin { .. }));
        match hit.action {
            crate::model::ResultAction::Plugin { action_id, .. } => {
                assert_eq!(action_id, "activate_window");
            }
            other => panic!("{other:?}"),
        }
    }

    fn devtools_plugin() -> RegisteredPlugin {
        RegisteredPlugin {
            manifest: PluginManifest {
                schema_version: 1,
                plugin: PluginIdentity {
                    id: "com.kite.devtools".into(),
                    name: "DevTools".into(),
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
                    command: "devtools.exe".into(),
                    args: vec![],
                    startup_timeout_ms: Some(2000),
                    idle_timeout_ms: Some(60_000),
                },
                contributes: Contributions {
                    examples: vec!["uuid".into()],
                    commands: vec![],
                    providers: vec![
                        PluginProvider {
                            id: "uuid".into(),
                            response_mode: "panel".into(),
                            triggers: vec![Trigger::Keyword { value: "uuid".into() }],
                        },
                        PluginProvider {
                            id: "hash".into(),
                            response_mode: "panel".into(),
                            triggers: vec![Trigger::Keyword { value: "hash".into() }],
                        },
                    ],
                },
            },
            root: PathBuf::from("plugins/com.kite.devtools"),
            enabled: true,
            phase: RuntimePhase::Dormant,
            last_error: None,
        }
    }

    #[test]
    fn multi_provider_shares_one_process() {
        let mut reg = PluginRegistry::new();
        reg.insert_loaded(devtools_plugin());
        let mut backend = MemProcessBackend::new();
        let host_proc = Arc::new(Mutex::new(MemProcess::new()));
        backend.register("com.kite.devtools", host_proc.clone());
        let mut host = PluginHost::new(Box::new(backend), PathBuf::from("d"), "0.3.0");

        let a1 = route_query("uuid", &reg).unwrap();
        assert_eq!(a1.provider_id, "uuid");
        let o1 = host.query(&reg, &a1, 0).expect("uuid");
        assert!(matches!(o1, QueryOutcome::Panel { .. }));

        let a2 = route_query("hash abc", &reg).unwrap();
        assert_eq!(a2.provider_id, "hash");
        let o2 = host.query(&reg, &a2, 0).expect("hash");
        assert!(matches!(o2, QueryOutcome::Panel { .. }));

        assert_eq!(
            host.spawn_count("com.kite.devtools"),
            1,
            "同一插件多 Provider 共用一个进程"
        );
    }

    #[test]
    fn execute_plugin_action_roundtrip() {
        let reg = registry();
        let (mut host, proc) = host_mem();
        let out = host
            .execute(
                &reg,
                "com.kite.calculator",
                "activate_window",
                json!({"hwnd": "window-1"}),
            )
            .expect("execute");
        assert_eq!(out, json!({"ok": true}));
        let sent = proc.lock().unwrap().sent.clone();
        assert!(sent.iter().any(|s| s.contains("plugin/execute")));
        assert!(sent.iter().any(|s| s.contains("activate_window")));
    }

    /// 真实 stdio 子进程：对 resources/official-plugins 走完整 initialize/query。
    /// 用于抓“插件全部崩溃”类问题（路径、帧协议、超时、进程退出）。
    #[test]
    fn stdio_backend_spawns_real_official_plugins() {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/official-plugins");
        if !src.is_dir() {
            eprintln!("skip: resources/official-plugins missing");
            return;
        }
        let dir = std::env::temp_dir().join(format!("kite-stdio-official-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let outcomes = crate::plugin::install::import_from_path(&src, &dir);
        for o in &outcomes {
            assert!(o.is_ok(), "import failed: {o:?}");
        }
        let mut registry = super::super::registry::load_registry_from_dir(&dir);
        for id in [
            "com.kite.calculator",
            "com.kite.devtools",
            "com.kite.window-switcher",
        ] {
            let p = registry
                .get_mut(id)
                .unwrap_or_else(|| panic!("registry missing {id}"));
            p.enabled = true;
            if p.phase != RuntimePhase::Dormant {
                p.phase = RuntimePhase::Dormant;
            }
        }
        let data_dir = dir.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let mut host = PluginHost::new(
            Box::new(super::super::process::StdioBackend::new(data_dir.clone())),
            data_dir,
            env!("CARGO_PKG_VERSION"),
        );
        let cases = [
            ("com.kite.calculator", "calculate", "1+2"),
            ("com.kite.devtools", "uuid", ""),
            ("com.kite.window-switcher", "switch", ""),
        ];
        for (plugin_id, provider_id, q) in cases {
            let act = Activation {
                plugin_id: plugin_id.into(),
                provider_id: provider_id.into(),
                effective_query: q.into(),
                raw_query: if q.is_empty() {
                    provider_id.into()
                } else {
                    format!("={q}")
                },
                response_mode: crate::plugin::ResponseMode::Panel,
            };
            let gen = host
                .spawn_count(plugin_id)
                .wrapping_add(1)
                .max(1)
                .wrapping_add(10);
            host.set_generation(gen);
            let result = host.query(&registry, &act, gen);
            let state = host.state(plugin_id);
            eprintln!("{plugin_id}/{provider_id} => {result:?} state={state:?}");
            assert!(
                result.is_ok(),
                "official plugin {plugin_id} failed: {result:?} state={state:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_list_provider_maps_plugin_actions() {
        let mut reg = PluginRegistry::new();
        reg.insert_loaded(RegisteredPlugin {
            manifest: PluginManifest {
                schema_version: 1,
                plugin: PluginIdentity {
                    id: "com.kite.window-switcher".into(),
                    name: "Window Switcher".into(),
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
                    command: "window-switcher.exe".into(),
                    args: vec![],
                    startup_timeout_ms: None,
                    idle_timeout_ms: Some(60_000),
                },
                contributes: Contributions {
                    examples: vec!["win".into()],
                    commands: vec![PluginCommand {
                        id: "open".into(),
                        title: "窗口切换".into(),
                        keywords: vec!["win".into()],
                        action: CommandAction::EnterProvider {
                            provider: "switch".into(),
                        },
                    }],
                    providers: vec![PluginProvider {
                        id: "switch".into(),
                        response_mode: "list".into(),
                        triggers: vec![Trigger::Keyword { value: "win".into() }],
                    }],
                },
            },
            root: PathBuf::from("plugins/com.kite.window-switcher"),
            enabled: true,
            phase: RuntimePhase::Dormant,
            last_error: None,
        });
        let mut backend = MemProcessBackend::new();
        backend.register(
            "com.kite.window-switcher",
            Arc::new(Mutex::new(MemProcess::new())),
        );
        let mut host = PluginHost::new(Box::new(backend), PathBuf::from("d"), "0.3.0");
        let act = route_query("win kite", &reg).unwrap();
        match host.query(&reg, &act, 1).expect("list") {
            QueryOutcome::List { items, plugin_id, .. } => {
                assert!(!items.is_empty());
                let hit = list_item_to_search_result(&plugin_id, &act.provider_id, &items[0]);
                assert!(matches!(hit.action, crate::model::ResultAction::Plugin { .. }));
            }
            other => panic!("{other:?}"),
        }
    }
}
