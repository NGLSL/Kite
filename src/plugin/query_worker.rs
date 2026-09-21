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
                run_job(job);
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

fn run_job(job: PluginQueryJob) {
    let PluginQueryJob {
        generation,
        activation,
        registry,
        host,
        on_host_call,
        on_done,
    } = job;
    let (payload, host_calls) = {
        let reg = registry.lock().unwrap_or_else(|e| e.into_inner());
        let mut host = host.lock().unwrap_or_else(|e| e.into_inner());
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
    for (pid, call) in host_calls {
        on_host_call(pid, call);
    }
    on_done(generation, payload);
}
