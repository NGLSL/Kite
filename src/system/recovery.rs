//! 系统脉冲恢复：睡眠/唤醒、Explorer 重启、Everything 状态变化、DPI/多屏、热键丢失。
//!
//! 小接口：`on_system_pulse` 吃进脉冲，吐出应执行的动作与探测结果。
//! UI 消息桥只消费 `RecoveryEffects.actions`；性能基线与单测在无窗口进程内压测同一接口。
//! Everything 探测走 `EverythingClient` seam，测试可注入脚本化状态。

use std::time::{Duration, Instant};

use super::everything::{Availability, EverythingClient, NativeEverything};

/// 触发恢复的系统事件。一次只处理一种，避免动作集含糊。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPulse {
    /// 睡眠唤醒：热键/watcher/索引路径与 Everything 状态都可能失效。
    ResumeFromSleep,
    /// Explorer 重启：桌面/开始菜单相关监听与扫描源可能需重臂。
    ExplorerRestarted,
    /// Everything 进程起停或安装状态变化。
    EverythingStatusChanged,
    /// 显示器拓扑或 DPI 变化。
    DisplayTopologyChanged,
    /// 热键注册失效（被占用/会话切换）。
    HotkeyLost,
}

/// 恢复动作：UI/后台按此执行；基线只断言集合与耗时，不绑 iced 消息类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    ProbeEverything,
    ClearFileResults,
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
    /// 本次探测到（或沿用上次）的 Everything 可用性；未探测且无历史时为 None 语义上的默认。
    pub everything_availability: Availability,
    /// 相对上次探测是否变化（含「首次探测到非 Ready」）。
    pub everything_changed: bool,
    pub probe_elapsed: Duration,
}

/// 恢复协调器：依赖 EverythingClient，其余动作以声明式 effects 输出。
pub struct Recovery {
    everything: Box<dyn EverythingClient>,
    last_everything: Option<Availability>,
}

impl Recovery {
    pub fn new(everything: Box<dyn EverythingClient>) -> Self {
        Self {
            everything,
            last_everything: None,
        }
    }

    pub fn with_client(client: impl EverythingClient + 'static) -> Self {
        Self::new(Box::new(client))
    }

    /// 生产默认：原生 Everything 探测。
    pub fn with_native() -> Self {
        Self::new(Box::new(NativeEverything))
    }

    pub fn last_everything(&self) -> Option<Availability> {
        self.last_everything
    }

    /// 处理系统脉冲。探测类脉冲会调用 EverythingClient；非探测脉冲不触碰外部。
    pub fn on_system_pulse(&mut self, pulse: SystemPulse) -> RecoveryEffects {
        let started = Instant::now();
        let mut actions = Vec::new();
        let mut needs_probe = false;

        match pulse {
            SystemPulse::ResumeFromSleep => {
                needs_probe = true;
                actions.push(RecoveryAction::ProbeEverything);
                actions.push(RecoveryAction::ReRegisterHotkey);
                actions.push(RecoveryAction::RearmWatchers);
                actions.push(RecoveryAction::RebuildIndex);
            }
            SystemPulse::ExplorerRestarted => {
                actions.push(RecoveryAction::RearmWatchers);
                actions.push(RecoveryAction::RebuildIndex);
            }
            SystemPulse::EverythingStatusChanged => {
                needs_probe = true;
                actions.push(RecoveryAction::ProbeEverything);
            }
            SystemPulse::DisplayTopologyChanged => {
                actions.push(RecoveryAction::RepositionWindow);
            }
            SystemPulse::HotkeyLost => {
                actions.push(RecoveryAction::ReRegisterHotkey);
            }
        }

        let mut everything_availability = self.last_everything.unwrap_or(Availability::NotInstalled);
        let mut everything_changed = false;

        if needs_probe {
            everything_availability = self.everything.availability();
            everything_changed = match self.last_everything {
                Some(prev) => prev != everything_availability,
                // 首次探测：非 Ready 视为需要清文件结果
                None => everything_availability != Availability::Ready,
            };
            if everything_changed {
                actions.push(RecoveryAction::ClearFileResults);
            }
            self.last_everything = Some(everything_availability);
        } else if let Some(last) = self.last_everything {
            everything_availability = last;
        }

        RecoveryEffects {
            pulse,
            actions,
            everything_availability,
            everything_changed,
            probe_elapsed: started.elapsed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::everything::{EverythingHit, ScriptedEverything};

    fn hit(name: &str) -> EverythingHit {
        EverythingHit {
            path: format!(r"C:\tmp\{name}"),
            is_folder: false,
            name: name.to_string(),
        }
    }

    #[test]
    fn explorer_restart_requests_watch_and_rebuild_without_everything_probe() {
        let scripted = ScriptedEverything::ready();
        let mut rec = Recovery::with_client(scripted.clone());
        let effects = rec.on_system_pulse(SystemPulse::ExplorerRestarted);
        assert_eq!(effects.pulse, SystemPulse::ExplorerRestarted);
        assert!(effects.actions.contains(&RecoveryAction::RearmWatchers));
        assert!(effects.actions.contains(&RecoveryAction::RebuildIndex));
        assert!(!effects.actions.contains(&RecoveryAction::ProbeEverything));
        assert_eq!(scripted.probe_count(), 0);
    }

    #[test]
    fn everything_down_pulse_clears_file_results() {
        let scripted = ScriptedEverything::ready();
        let mut rec = Recovery::with_client(scripted.clone());
        let first = rec.on_system_pulse(SystemPulse::EverythingStatusChanged);
        assert_eq!(first.everything_availability, Availability::Ready);
        assert!(!first.everything_changed);

        scripted.set_availability(Availability::InstalledButNotRunning);
        let second = rec.on_system_pulse(SystemPulse::EverythingStatusChanged);
        assert_eq!(
            second.everything_availability,
            Availability::InstalledButNotRunning
        );
        assert!(second.everything_changed);
        assert!(second.actions.contains(&RecoveryAction::ClearFileResults));
        assert_eq!(scripted.probe_count(), 2);
    }

    #[test]
    fn scripted_search_degrades_when_everything_stops() {
        let scripted = ScriptedEverything::ready().with_hits(vec![hit("a.txt"), hit("b.txt")]);
        assert_eq!(
            EverythingClient::search(&scripted, "a", 10, crate::system::everything::FileFilter::All)
                .len(),
            2
        );
        scripted.set_availability(Availability::InstalledButNotRunning);
        assert!(
            EverythingClient::search(&scripted, "a", 10, crate::system::everything::FileFilter::All)
                .is_empty()
        );
        assert_eq!(scripted.search_count(), 2);
    }

    #[test]
    fn resume_from_sleep_probes_and_lists_full_recovery_actions() {
        let scripted = ScriptedEverything::new(Availability::InstalledButNotRunning);
        let mut rec = Recovery::with_client(scripted.clone());
        let effects = rec.on_system_pulse(SystemPulse::ResumeFromSleep);
        assert!(effects.actions.contains(&RecoveryAction::ProbeEverything));
        assert!(effects.actions.contains(&RecoveryAction::ReRegisterHotkey));
        assert!(effects.actions.contains(&RecoveryAction::RearmWatchers));
        assert!(effects.actions.contains(&RecoveryAction::RebuildIndex));
        // 首次探测到非 Ready → 清文件结果
        assert!(effects.actions.contains(&RecoveryAction::ClearFileResults));
        assert_eq!(
            effects.everything_availability,
            Availability::InstalledButNotRunning
        );
        assert!(scripted.probe_count() >= 1);
    }

    #[test]
    fn dpi_pulse_only_requests_reposition() {
        let mut rec = Recovery::with_client(ScriptedEverything::ready());
        let effects = rec.on_system_pulse(SystemPulse::DisplayTopologyChanged);
        assert_eq!(effects.actions, vec![RecoveryAction::RepositionWindow]);
    }

    #[test]
    fn hotkey_lost_only_requests_reregister() {
        let mut rec = Recovery::with_client(ScriptedEverything::ready());
        let effects = rec.on_system_pulse(SystemPulse::HotkeyLost);
        assert_eq!(effects.actions, vec![RecoveryAction::ReRegisterHotkey]);
    }
}
