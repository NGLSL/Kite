//! 0.3.7 代码级性能基线：不主动唤起 Kite 窗口。
//!
//! ```text
//! cargo test --release report_perf_baseline -- --ignored --nocapture
//! ```
//!
//! ## 职责拆分
//!
//! | 模块 | 职责 | 依赖的生产 seam |
//! |------|------|----------------|
//! | `report` | 采样与 markdown 输出 | 无 |
//! | `search` | 搜索热路径 / 索引构建代理 | `RetrievalIndex` |
//! | `plugin` | 插件崩溃 / 超时 / crash-loop | `PluginHost` + `ProcessBackend` |
//! | `system_pulse` | Everything 降级、恢复脉冲 | `EverythingClient` + `Recovery` |
//! | `dpi` | 多屏 / DPI 几何 | `window_place` 纯函数 |
//! | `storage` | 历史库迁移、快照、原子写 | `HistoryDb` / `snapshot` / `atomic_file` |
//!
//! 真 UI 唤起延迟仍用 `scripts/measure-performance.ps1` 发布抽检，不混入本报告。
//! 生产接线边界：UI 只消费 `RecoveryEffects.actions` 与 `EverythingClient`，
//! 不把 iced 消息类型写进基线模块。

mod dpi;
mod plugin;
mod report;
mod search;
mod storage;
mod system_pulse;

use crate::system::everything::ScriptedEverything;
use crate::system::recovery::{Recovery, RecoveryAction, SystemPulse};

fn print_os_only() {
    println!("\n## OS-only（不在代码基线内）\n");
    println!("| 场景 | 方式 | 说明 |");
    println!("|---|---|---|");
    println!(
        "| 真 UI 唤起延迟 | scripts/measure-performance.ps1 | Alt+Space→可见→Esc，发布抽检 |"
    );
    println!("| 真实睡眠/唤醒 | 实机 | Recovery effects 已代码覆盖；电源事件需 OS |");
    println!("| Explorer 真重启 | 实机 | 脉冲动作集已代码覆盖 |");
    println!("| Everything 真进程重启 | 实机 | 降级路径已 ScriptedEverything 覆盖 |");
    println!("| 多屏热插拔 / 系统 DPI | 实机 | 几何矩阵已代码覆盖 |");
    println!(
        "| 覆盖安装 / 旧版升级实机 | installer + 实机 | 静态契约见 test-installer-upgrade.ps1 |"
    );
}

/// 0.3.7 默认代码级性能基线入口（不启动 kite.exe 窗口）。
#[test]
#[ignore = "性能基线，手动运行：cargo test --release report_perf_baseline -- --ignored --nocapture"]
fn report_perf_baseline() {
    println!("\n# Kite 0.3.7 代码级性能基线");
    println!("\n不主动唤起 UI。真窗口唤起见 scripts/measure-performance.ps1。\n");
    search::run();
    plugin::run();
    system_pulse::run();
    dpi::run();
    storage::run();
    print_os_only();
}

#[cfg(test)]
mod smoke {
    use super::*;

    #[test]
    fn recovery_actions_stay_on_production_interface() {
        let mut rec = Recovery::with_client(ScriptedEverything::ready());
        let effects = rec.on_system_pulse(SystemPulse::ResumeFromSleep);
        assert!(effects.actions.contains(&RecoveryAction::ProbeEverything));
        assert!(effects.actions.contains(&RecoveryAction::RebuildIndex));
    }

    #[test]
    fn search_scenario_smoke() {
        search::smoke_hits_nonempty();
    }

    #[test]
    fn report_helpers_percentile_empty_is_zero() {
        assert_eq!(report::percentile(&[], 0.5), std::time::Duration::ZERO);
    }
}
