//! 场景：插件进程崩溃 / 超时 / crash-loop。
//! 职责：只驱动 PluginHost + MemProcessBackend，不碰 UI。
//! 复用生产 Host 接口（query / note_crash / state / spawn_count）。

use super::report::{print_latency, print_section};
use crate::plugin::activation::{route_query, ResponseMode, Trigger};
use crate::plugin::host::{HostError, PluginHost, PluginRuntimeState};
use crate::plugin::manifest::{
    CommandAction, Compatibility, Contributions, PluginCommand, PluginIdentity, PluginManifest,
    PluginProvider, RuntimeSpec,
};
use crate::plugin::process::{MemProcess, MemProcessBackend};
use crate::plugin::registry::{PluginRegistry, RegisteredPlugin, RuntimePhase};
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn calc_registry() -> PluginRegistry {
    let mut r = PluginRegistry::new();
    r.insert_loaded(RegisteredPlugin {
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
                startup_timeout_ms: Some(200),
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
    });
    r
}

fn plugin_host_mem() -> (PluginHost, Arc<Mutex<MemProcess>>, PluginRegistry) {
    let proc = Arc::new(Mutex::new(MemProcess::new()));
    let mut backend = MemProcessBackend::new();
    backend.register("com.kite.calculator", proc.clone());
    let host = PluginHost::new(
        Box::new(backend),
        PathBuf::from("plugin-data-root"),
        "0.3.7",
    );
    (host, proc, calc_registry())
}

pub fn run() {
    print_section("插件崩溃 / 超时（MemProcessBackend，无 UI）");

    // 进程复用：懒启动只 spawn 一次
    let (mut host, _p, reg) = plugin_host_mem();
    let act = route_query("=1+2", &reg).expect("route");
    debug_assert!(matches!(act.response_mode, ResponseMode::Panel) || true);
    let (p50, p95, max) = super::report::time_iters(50, || {
        let _ = black_box(host.query(&reg, &act, host.current_generation()));
    });
    print_latency(
        "plugin-query-reuse",
        p50,
        p95,
        max,
        &format!(
            "spawn={} state={:?}",
            host.spawn_count("com.kite.calculator"),
            host.state("com.kite.calculator")
        ),
    );

    // 崩溃 → Faulted
    let (mut host, proc, reg) = plugin_host_mem();
    let act = route_query("=1+1", &reg).expect("route");
    let _ = host.query(&reg, &act, 0);
    proc.lock().unwrap().alive = false;
    host.note_crash("com.kite.calculator", "killed");
    let faulted = matches!(
        host.state("com.kite.calculator"),
        PluginRuntimeState::Faulted { .. }
    );
    let t = Instant::now();
    let out = host.query(&reg, &act, 0);
    let elapsed = t.elapsed();
    print_latency(
        "plugin-crash-recover-probe",
        elapsed,
        elapsed,
        elapsed,
        &format!("faulted={faulted} retry_ok={}", out.is_ok()),
    );

    // 超时：hang 后必须在硬超时内返回
    let (mut host, proc, reg) = plugin_host_mem();
    let act = route_query("=1", &reg).expect("route");
    let _ = host.query(&reg, &act, 0);
    proc.lock().unwrap().hang = true;
    let t = Instant::now();
    let out = host.query(&reg, &act, host.current_generation());
    let elapsed = t.elapsed();
    let bounded = matches!(out, Err(HostError::Timeout(_)) | Err(HostError::Spawn(_)));
    print_latency(
        "plugin-query-hang",
        elapsed,
        elapsed,
        elapsed,
        &format!(
            "bounded={bounded} elapsed_ms={} is_err={}",
            elapsed.as_millis(),
            out.is_err()
        ),
    );

    // crash-loop 抑制：连续崩溃后不再自动 spawn
    let (mut host, _p, reg) = plugin_host_mem();
    for _ in 0..3 {
        host.note_crash("com.kite.calculator", "loop");
    }
    let act = route_query("=1", &reg).expect("route");
    let err = host.query(&reg, &act, 0).unwrap_err();
    print_latency(
        "plugin-crash-loop-block",
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
        &format!("blocked={}", matches!(err, HostError::CrashLoop(_))),
    );
}
