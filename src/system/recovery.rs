//! 系统脉冲恢复：睡眠/唤醒、Explorer 重启、DPI/多屏、热键丢失。
//!
//! `on_system_pulse` 输出应执行的恢复动作，由 UI 消息桥消费。

/// 触发恢复的系统事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPulse {
    /// 睡眠唤醒：热键、watcher 与索引路径可能失效。
    ResumeFromSleep,
    /// Explorer 重启：桌面/开始菜单相关监听与扫描源可能需重臂。
    ExplorerRestarted,
    /// 显示器拓扑或 DPI 变化。
    DisplayTopologyChanged,
    /// 热键注册失效（被占用/会话切换）。
    HotkeyLost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    RebuildIndex,
    RearmWatchers,
    ReRegisterHotkey,
    RepositionWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryEffects {
    pub pulse: SystemPulse,
    pub actions: Vec<RecoveryAction>,
}

/// 恢复协调器：只声明系统恢复动作，不探测外部文件搜索服务。
#[derive(Default)]
pub struct Recovery;

impl Recovery {
    pub fn new() -> Self {
        Self
    }

    pub fn on_system_pulse(&mut self, pulse: SystemPulse) -> RecoveryEffects {
        let actions = match pulse {
            SystemPulse::ResumeFromSleep => vec![
                RecoveryAction::ReRegisterHotkey,
                RecoveryAction::RearmWatchers,
                RecoveryAction::RebuildIndex,
            ],
            SystemPulse::ExplorerRestarted => {
                vec![RecoveryAction::RearmWatchers, RecoveryAction::RebuildIndex]
            }
            SystemPulse::DisplayTopologyChanged => vec![RecoveryAction::RepositionWindow],
            SystemPulse::HotkeyLost => vec![RecoveryAction::ReRegisterHotkey],
        };
        RecoveryEffects { pulse, actions }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_pulses_preserve_required_recovery_actions() {
        let mut recovery = Recovery::new();
        for (pulse, expected) in [
            (
                SystemPulse::ResumeFromSleep,
                vec![
                    RecoveryAction::ReRegisterHotkey,
                    RecoveryAction::RearmWatchers,
                    RecoveryAction::RebuildIndex,
                ],
            ),
            (
                SystemPulse::ExplorerRestarted,
                vec![RecoveryAction::RearmWatchers, RecoveryAction::RebuildIndex],
            ),
            (
                SystemPulse::DisplayTopologyChanged,
                vec![RecoveryAction::RepositionWindow],
            ),
            (
                SystemPulse::HotkeyLost,
                vec![RecoveryAction::ReRegisterHotkey],
            ),
        ] {
            let effects = recovery.on_system_pulse(pulse);
            assert_eq!(effects.pulse, pulse);
            assert_eq!(effects.actions, expected);
            assert_eq!(recovery.on_system_pulse(pulse), effects);
        }
    }
}
