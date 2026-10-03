//! Provider Mode 查询常驻 worker：与 `AppSearchWorker` 同构。
//!
//! 单线程只跑最新请求；UI 通过 `LatestSlot` 提交/作废，不在 update 里临时 spawn。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::model::SearchResult;
use crate::search::service::LatestSlot;

use super::host::{list_item_to_search_result, HostError, PluginHost, QueryOutcome};
use super::panel::PanelData;
use super::protocol::HostCall;
use super::registry::PluginRegistry;
use super::Activation;

/// 插件查询落地载荷（UI `PluginQueryReady` 使用）。
#[derive(Debug, Clone)]
pub enum PluginQueryPayload {
    List {
        plugin_id: String,
        provider_id: String,
        items: Vec<SearchResult>,
    },
    Panel {
        plugin_id: String,
        provider_id: String,
        panel: PanelData,
    },
    Empty {
        plugin_id: String,
        provider_id: String,
    },
    Error(String),
}

/// 一次 Provider 查询请求。
pub struct PluginQueryJob {
    pub generation: u64,
    pub activation: Activation,
    pub registry: Arc<Mutex<PluginRegistry>>,
    pub host: Arc<Mutex<PluginHost>>,
    /// Host API 回调（clipboard / open_url / …），由 UI 映射为 Message。
    pub on_host_call: Arc<dyn Fn(String, HostCall) + Send + Sync>,
    /// 查询完成回调；取消/覆盖时不会调用。
    pub on_done: Arc<dyn Fn(u64, PluginQueryPayload) + Send + Sync>,
}

/// 常驻插件查询 worker：单线程，只跑最新请求。
pub struct PluginQueryWorker {
    slot: Arc<LatestSlot<Option<PluginQueryJob>>>,
}

impl PluginQueryWorker {
    pub fn spawn() -> Self {
        let slot = Arc::new(LatestSlot::new());
        let worker_slot = slot.clone();
        std::thread::spawn(move || {
            let mut last_handled = 0u64;
            loop {
                worker_slot.wait_newer_than(last_handled, Duration::from_millis(200));
                let Some((slot_gen, entry)) = worker_slot.take() else {
                    continue;
                };
                last_handled = last_handled.max(slot_gen);
                let Some(job) = entry else {
                    // 显式作废：代际已推进。
                    continue;
                };
                run_job(job, || worker_slot.latest_seq() == slot_gen);
            }
        });
        Self { slot }
    }

    pub fn submit(&self, job: PluginQueryJob) {
        self.slot.submit(Some(job));
    }

    /// 推进代际，作废在途查询。
    pub fn cancel_current(&self) {
        self.slot.submit(None);
    }

    pub fn latest_seq(&self) -> u64 {
        self.slot.latest_seq()
    }
}

fn run_job(job: PluginQueryJob, is_current: impl Fn() -> bool) {
    if !is_current() {
        return;
    }
    let PluginQueryJob {
        generation,
        activation,
        registry,
        host,
        on_host_call,
        on_done,
    } = job;
    let plugin = registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&activation.plugin_id)
        .cloned();
    let mut reg = PluginRegistry::new();
    if let Some(plugin) = plugin {
        reg.insert_loaded(plugin);
    }
    let (payload, host_calls) = {
        let mut host = host.lock().unwrap_or_else(|e| e.into_inner());
        if !is_current() {
            return;
        }
        host.set_generation(generation);
        host.idle_sweep();
        let payload = match host.query(&reg, &activation, generation) {
            Ok(QueryOutcome::List {
                plugin_id,
                provider_id,
                items,
                ..
            }) => {
                let mapped = items
                    .iter()
                    .map(|it| list_item_to_search_result(&plugin_id, &provider_id, it))
                    .collect();
                PluginQueryPayload::List {
                    plugin_id,
                    provider_id,
                    items: mapped,
                }
            }
            Ok(QueryOutcome::Panel {
                plugin_id,
                provider_id,
                panel,
                ..
            }) => PluginQueryPayload::Panel {
                plugin_id,
                provider_id,
                panel,
            },
            Ok(QueryOutcome::Empty {
                plugin_id,
                provider_id,
                ..
            }) => PluginQueryPayload::Empty {
                plugin_id,
                provider_id,
            },
            // 过期代际不是插件故障：静默丢弃，避免 UI 误标「崩溃」。
            Err(HostError::StaleGeneration { .. }) => PluginQueryPayload::Empty {
                plugin_id: activation.plugin_id.clone(),
                provider_id: activation.provider_id.clone(),
            },
            Err(e) => PluginQueryPayload::Error(e.to_string()),
        };
        let host_calls = host.drain_host_calls();
        (payload, host_calls)
    };
    if !is_current() {
        return;
    }
    for (pid, call) in host_calls {
        if !is_current() {
            return;
        }
        on_host_call(pid, call);
    }
    if is_current() {
        on_done(generation, payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::process::{MemProcessBackend, PluginProcess, ProcessBackend};
    use crate::plugin::registry::{RegisteredPlugin, RuntimePhase};
    use std::path::Path;
    use std::sync::mpsc;

    struct SlowRpcBackend {
        inner: MemProcessBackend,
        started: mpsc::Sender<()>,
        release: Option<mpsc::Receiver<()>>,
    }
    struct SlowRpcProcess {
        inner: Box<dyn PluginProcess>,
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl ProcessBackend for SlowRpcBackend {
        fn spawn(
            &mut self,
            id: &str,
            command: &Path,
            args: &[String],
            workdir: &Path,
        ) -> Result<Box<dyn PluginProcess>, String> {
            Ok(Box::new(SlowRpcProcess {
                inner: self.inner.spawn(id, command, args, workdir)?,
                started: self.started.clone(),
                release: self.release.take().unwrap(),
            }))
        }
    }
    impl PluginProcess for SlowRpcProcess {
        fn send_frame(&mut self, body: &str) -> Result<(), String> {
            let request: serde_json::Value = serde_json::from_str(body).unwrap();
            if request.get("method").and_then(|value| value.as_str()) == Some("plugin/query") {
                self.started.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(2)).unwrap();
            }
            self.inner.send_frame(body)
        }
        fn try_recv_frame(&mut self) -> Result<Option<String>, String> {
            self.inner.try_recv_frame()
        }
        fn is_alive(&mut self) -> bool {
            self.inner.is_alive()
        }
        fn kill(&mut self) {
            self.inner.kill();
        }
    }

    #[test]
    fn slow_provider_rpc_keeps_live_registry_available() {
        let root = std::env::temp_dir().join(format!(
            "kite-provider-registry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let manifest =
            crate::plugin::manifest::parse_manifest(include_str!("testdata/loci.json")).unwrap();
        std::fs::write(root.join(&manifest.runtime.command), b"MZ").unwrap();
        let mut registry = PluginRegistry::new();
        registry.insert_loaded(RegisteredPlugin {
            manifest,
            root: root.clone(),
            enabled: true,
            phase: RuntimePhase::Dormant,
            last_error: None,
        });
        let registry = Arc::new(Mutex::new(registry));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let host = Arc::new(Mutex::new(PluginHost::new(
            Box::new(SlowRpcBackend {
                inner: MemProcessBackend::new(),
                started: started_tx,
                release: Some(release_rx),
            }),
            root.clone(),
            env!("CARGO_PKG_VERSION"),
        )));
        let job = PluginQueryJob {
            generation: 7,
            activation: Activation {
                plugin_id: "com.kite.loci".into(),
                provider_id: "files".into(),
                raw_query: "loci status".into(),
                effective_query: "status".into(),
                response_mode: super::super::activation::ResponseMode::List,
            },
            registry: registry.clone(),
            host,
            on_host_call: Arc::new(|_, _| {}),
            on_done: Arc::new(move |generation, payload| {
                done_tx.send((generation, payload)).unwrap();
            }),
        };
        let worker = std::thread::spawn(move || run_job(job, || true));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let accessible = registry.try_lock().is_ok();
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        let (generation, result) = done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            accessible,
            "the live registry stayed locked during plugin RPC"
        );
        assert_eq!(generation, 7);
        assert!(matches!(result, PluginQueryPayload::Empty { .. }));
        std::fs::remove_dir_all(root).unwrap();
    }

    struct NoSpawnBackend(Arc<std::sync::atomic::AtomicUsize>);
    impl ProcessBackend for NoSpawnBackend {
        fn spawn(
            &mut self,
            _: &str,
            _: &Path,
            _: &[String],
            _: &Path,
        ) -> Result<Box<dyn PluginProcess>, String> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err("cancelled job must not spawn".into())
        }
    }

    #[test]
    fn cancelled_job_waiting_for_host_does_not_spawn_or_publish() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let slot = Arc::new(LatestSlot::new());
        let worker = PluginQueryWorker { slot: slot.clone() };
        let spawns = Arc::new(AtomicUsize::new(0));
        let callbacks = Arc::new(AtomicUsize::new(0));
        let host = Arc::new(Mutex::new(PluginHost::new(
            Box::new(NoSpawnBackend(spawns.clone())),
            std::env::temp_dir(),
            env!("CARGO_PKG_VERSION"),
        )));
        let mut registry = PluginRegistry::new();
        registry.insert_loaded(RegisteredPlugin {
            manifest: crate::plugin::manifest::parse_manifest(include_str!("testdata/loci.json"))
                .unwrap(),
            root: std::env::temp_dir(),
            enabled: true,
            phase: RuntimePhase::Dormant,
            last_error: None,
        });
        let done_callbacks = callbacks.clone();
        let call_callbacks = callbacks.clone();
        slot.submit(Some(PluginQueryJob {
            generation: 1,
            activation: Activation {
                plugin_id: "com.kite.loci".into(),
                provider_id: "files".into(),
                raw_query: "loci status".into(),
                effective_query: "status".into(),
                response_mode: super::super::activation::ResponseMode::List,
            },
            registry: Arc::new(Mutex::new(registry)),
            host: host.clone(),
            on_host_call: Arc::new(move |_, _| {
                call_callbacks.fetch_add(1, Ordering::SeqCst);
            }),
            on_done: Arc::new(move |_, _| {
                done_callbacks.fetch_add(1, Ordering::SeqCst);
            }),
        }));
        let (slot_gen, Some(job)) = slot.take().unwrap() else {
            panic!("expected job");
        };
        let host_guard = host.lock().unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let runner_slot = slot.clone();
        let runner = std::thread::spawn(move || {
            run_job(job, || {
                let current = runner_slot.latest_seq() == slot_gen;
                entered_tx.send(()).ok();
                current
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.cancel_current();
        drop(host_guard);
        runner.join().unwrap();
        assert_eq!(spawns.load(Ordering::SeqCst), 0);
        assert_eq!(callbacks.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn missing_plugin_snapshot_preserves_host_error() {
        let (done_tx, done_rx) = mpsc::channel();
        run_job(
            PluginQueryJob {
                generation: 1,
                activation: Activation {
                    plugin_id: "missing-plugin".into(),
                    provider_id: "files".into(),
                    raw_query: String::new(),
                    effective_query: String::new(),
                    response_mode: super::super::activation::ResponseMode::List,
                },
                registry: Arc::new(Mutex::new(PluginRegistry::new())),
                host: Arc::new(Mutex::new(PluginHost::new(
                    Box::new(MemProcessBackend::new()),
                    std::env::temp_dir(),
                    env!("CARGO_PKG_VERSION"),
                ))),
                on_host_call: Arc::new(|_, _| {}),
                on_done: Arc::new(move |_, payload| {
                    done_tx.send(payload).unwrap();
                }),
            },
            || true,
        );
        let PluginQueryPayload::Error(error) = done_rx.recv().unwrap() else {
            panic!("expected HostError");
        };
        assert!(error.contains("missing-plugin"));
    }
}
