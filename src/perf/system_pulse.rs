//! 场景：Everything 降级与系统脉冲恢复。
//! 职责：EverythingClient seam + Recovery 深模块；不碰真实 IPC / UI。
//! 生产接线边界：UI 消息桥消费 `RecoveryEffects.actions`，本模块只测接口。

use super::report::{percentile, print_latency, print_section, time_iters, ITERS};
use crate::system::everything::{
    Availability, EverythingClient, EverythingHit, FileFilter, ScriptedEverything,
};
use crate::system::recovery::{Recovery, SystemPulse};
use std::hint::black_box;
use std::time::Instant;

fn sample_hits(n: usize) -> Vec<EverythingHit> {
    (0..n)
        .map(|i| EverythingHit {
            path: format!(r"C:\files\f{i}.txt"),
            is_folder: false,
            name: format!("f{i}.txt"),
        })
        .collect()
}

pub fn run() {
    print_section("Everything 降级 / 系统脉冲恢复（代码注入，无 UI）");

    let scripted = ScriptedEverything::ready().with_hits(sample_hits(20));
    let (p50, p95, max) = time_iters(ITERS, || {
        black_box(EverythingClient::search(
            &scripted,
            "f",
            20,
            FileFilter::All,
        ));
    });
    print_latency(
        "everything-search-ready",
        p50,
        p95,
        max,
        &format!("search_calls={}", scripted.search_count()),
    );

    // Everything 重启：Ready → InstalledButNotRunning，查询立即空且不阻塞
    scripted.set_availability(Availability::InstalledButNotRunning);
    let (p50, p95, max) = time_iters(ITERS, || {
        let hits = EverythingClient::search(&scripted, "f", 20, FileFilter::All);
        black_box(hits);
    });
    print_latency(
        "everything-search-stopped",
        p50,
        p95,
        max,
        "empty when not running",
    );

    // 脉冲处理：状态翻转时 ClearFileResults
    let scripted = ScriptedEverything::ready();
    let mut rec = Recovery::with_client(scripted.clone());
    let mut probe_durs = Vec::new();
    for i in 0..40 {
        if i == 20 {
            scripted.set_availability(Availability::InstalledButNotRunning);
        }
        let t = Instant::now();
        let effects = rec.on_system_pulse(SystemPulse::EverythingStatusChanged);
        probe_durs.push(t.elapsed());
        black_box(effects);
    }
    probe_durs.sort();
    print_latency(
        "recovery-everything-pulse",
        percentile(&probe_durs, 0.5),
        percentile(&probe_durs, 0.95),
        *probe_durs.last().unwrap(),
        &format!(
            "last={:?} probes={}",
            rec.last_everything(),
            scripted.probe_count()
        ),
    );

    for pulse in [
        SystemPulse::ResumeFromSleep,
        SystemPulse::ExplorerRestarted,
        SystemPulse::DisplayTopologyChanged,
        SystemPulse::HotkeyLost,
    ] {
        let mut rec = Recovery::with_client(ScriptedEverything::ready());
        let mut durs = Vec::new();
        for _ in 0..50 {
            let t = Instant::now();
            let effects = rec.on_system_pulse(pulse);
            durs.push(t.elapsed());
            black_box(effects);
        }
        durs.sort();
        print_latency(
            &format!("recovery-pulse/{pulse:?}"),
            percentile(&durs, 0.5),
            percentile(&durs, 0.95),
            *durs.last().unwrap(),
            "actions non-empty",
        );
    }
}
