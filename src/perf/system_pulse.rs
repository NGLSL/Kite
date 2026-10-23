//! 场景：系统脉冲恢复。
//! 职责：Recovery 深模块的四种原生系统脉冲耗时基线；不碰真实 OS / UI。
//! 生产接线边界：UI 消费 `RecoveryEffects.actions`，本模块只测接口。

use super::report::{percentile, print_latency, print_section};
use crate::system::recovery::{Recovery, SystemPulse};
use std::hint::black_box;
use std::time::Instant;

pub fn run() {
    print_section("系统脉冲恢复（代码注入，无 UI）");

    for pulse in [
        SystemPulse::ResumeFromSleep,
        SystemPulse::ExplorerRestarted,
        SystemPulse::DisplayTopologyChanged,
        SystemPulse::HotkeyLost,
    ] {
        let mut rec = Recovery::new();
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
