//! 系统脉冲恢复：睡眠/唤醒、Explorer 重启、DPI/多屏与热键丢失。
//!
//! 恢复协调器只描述 Kite 自己需要重新装配的动作。内置文件索引由
//! `file_search::Controller` 管理其服务生命周期，不再由系统恢复层探测
//! 或维护第三方文件搜索后端状态。

use std::time::{Duration, Instant};

/// 触发恢复的系统事件。一次只处理一种，避免动作集含糊。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPulse {
    /// 睡眠唤醒：热键、watcher 与应用索引路径都可能失效。
    ResumeFromSleep,
    /// Explorer 重启：桌面/开始菜单相关监听与扫描源可能需重臂。
    ExplorerRestarted,
    /// 显示器拓扑或 DPI 变化。
    DisplayTopologyChanged,
    /// 热键注册失效（被占用/会话切换）。
    HotkeyLost,
}

/// 恢复动作：UI/后台按此执行；基线只断言集合与耗时，不绑 iced 消息类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    RebuildIndex,
    RearmWatchers,
    ReRegisterHotkey,
    RepositionWindow,
}

/// 一次脉冲的处理结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryEffects {
    pub pulse: SystemPulse,
    pub actions: Vec<RecoveryAction>,
    pub elapsed: Duration,
}

/// 恢复协调器：只负责把系统脉冲映射为 Kite 自己的恢复动作。
#[derive(Default)]
pub struct Recovery;

impl Recovery {
    pub fn new() -> Self {
        Self
    }

    /// 处理系统脉冲，不执行外部 I/O。
    pub fn on_system_pulse(&mut self, pulse: SystemPulse) -> RecoveryEffects {
        let started = Instant::now();
        let actions = match pulse {
            SystemPulse::ResumeFromSleep => vec![
                RecoveryAction::ReRegisterHotkey,
                RecoveryAction::RearmWatchers,
                RecoveryAction::RebuildIndex,
            ],
            SystemPulse::ExplorerRestarted => vec![
                RecoveryAction::RearmWatchers,
                RecoveryAction::RebuildIndex,
            ],
            SystemPulse::DisplayTopologyChanged => vec![RecoveryAction::RepositionWindow],
            SystemPulse::HotkeyLost => vec![RecoveryAction::ReRegisterHotkey],
        };
        RecoveryEffects {
            pulse,
            actions,
            elapsed: started.elapsed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_restart_requests_watch_and_rebuild() {
        let mut rec = Recovery::new();
        let effects = rec.on_system_pulse(SystemPulse::ExplorerRestarted);
        assert_eq!(effects.pulse, SystemPulse::ExplorerRestarted);
        assert!(effects.actions.contains(&RecoveryAction::RearmWatchers));
        assert!(effects.actions.contains(&RecoveryAction::RebuildIndex));
    }

    #[test]
    fn resume_from_sleep_requests_full_recovery_actions() {
        let mut rec = Recovery::new();
        let effects = rec.on_system_pulse(SystemPulse::ResumeFromSleep);
        assert!(effects.actions.contains(&RecoveryAction::ReRegisterHotkey));
        assert!(effects.actions.contains(&RecoveryAction::RearmWatchers));
        assert!(effects.actions.contains(&RecoveryAction::RebuildIndex));
    }

    #[test]
    fn dpi_pulse_only_requests_reposition() {
        let mut rec = Recovery::new();
        let effects = rec.on_system_pulse(SystemPulse::DisplayTopologyChanged);
        assert_eq!(effects.actions, vec![RecoveryAction::RepositionWindow]);
    }

    #[test]
    fn hotkey_lost_only_requests_reregister() {
        let mut rec = Recovery::new();
        let effects = rec.on_system_pulse(SystemPulse::HotkeyLost);
        assert_eq!(effects.actions, vec![RecoveryAction::ReRegisterHotkey]);
    }
}
