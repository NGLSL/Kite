//! 扫描档位与预算常量。

use std::path::PathBuf;
use std::time::Duration;

/// 快速扫描时间预算；超时后用已收集结果。
pub(crate) const FAST_BUDGET: Duration = Duration::from_millis(1500);
pub(crate) const MAX_PER_DIR: usize = 400;
pub(crate) const MAX_TOTAL: usize = 1500;
/// Start Menu 应用通常位于 Programs/<分类>/<应用>，开发工具还可能再嵌套一层。
pub(crate) const START_MENU_MAX_DEPTH: usize = 5;
pub(crate) const OTHER_SOURCE_MAX_DEPTH: usize = 2;
/// 后台完整扫描：不受快扫时间预算限制，递归更深；安全上限触发时写日志。
pub(crate) const FULL_START_MENU_MAX_DEPTH: usize = 16;
pub(crate) const FULL_OTHER_SOURCE_MAX_DEPTH: usize = 8;
pub(crate) const FULL_MAX_PER_DIR: usize = 2000;
pub(crate) const FULL_MAX_TOTAL: usize = 8000;

/// 扫描档位：Bootstrap 首屏 vs 后台完整补扫。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPass {
    /// 首屏 Bootstrap：只扫高价值正式入口（开始菜单/桌面/配置 Portable/系统入口），
    /// 有时间预算并批量提图标；不做 metadata、UWP、uninstall、Scoop/commands、App Paths、protocol。
    Bootstrap,
    /// 有时间预算、浅层、不提图标、不含 UWP（测试与兼容路径，不作生产首屏）。
    Fast,
    /// 无时间预算、深层递归、含 UWP 与图标，原子替换快照。
    Full,
}

impl ScanPass {
    pub(crate) fn timed_budget(self) -> bool {
        !matches!(self, ScanPass::Full)
    }

    /// Scoop / commands / uninstall 等发现与命令来源。
    pub(crate) fn include_supplemental_sources(self) -> bool {
        !matches!(self, ScanPass::Bootstrap)
    }

    /// App Paths：Fast/Full 收集；Bootstrap 只保留 A+B 高价值入口，不含 App Paths。
    pub(crate) fn include_app_paths(self) -> bool {
        matches!(self, ScanPass::Fast | ScanPass::Full)
    }

    pub(crate) fn include_uwp(self) -> bool {
        matches!(self, ScanPass::Full)
    }

    pub(crate) fn include_metadata(self) -> bool {
        matches!(self, ScanPass::Full)
    }

    /// Bootstrap 也提图标：Cold Start 第一印象优先于省 200–300ms。
    /// Warm Start 走 snapshot，不付这笔成本。
    pub(crate) fn include_icons(self) -> bool {
        matches!(self, ScanPass::Bootstrap | ScanPass::Full)
    }

    pub(crate) fn include_protocols(self) -> bool {
        !matches!(self, ScanPass::Bootstrap)
    }

    pub(crate) fn start_menu_depth(self) -> usize {
        if matches!(self, ScanPass::Full) {
            FULL_START_MENU_MAX_DEPTH
        } else {
            START_MENU_MAX_DEPTH
        }
    }

    pub(crate) fn other_source_depth(self) -> usize {
        if matches!(self, ScanPass::Full) {
            FULL_OTHER_SOURCE_MAX_DEPTH
        } else {
            OTHER_SOURCE_MAX_DEPTH
        }
    }

    pub(crate) fn max_per_dir(self) -> usize {
        if matches!(self, ScanPass::Full) {
            FULL_MAX_PER_DIR
        } else {
            MAX_PER_DIR
        }
    }

    pub(crate) fn max_total(self) -> usize {
        if matches!(self, ScanPass::Full) {
            FULL_MAX_TOTAL
        } else {
            MAX_TOTAL
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub extra_scoop_shim_dirs: Vec<PathBuf>,
    pub portable_dirs: Vec<PathBuf>,
    /// 设置/托盘「重新扫描」：强制重枚举 UWP，忽略结果缓存 TTL。
    pub force_uwp_refresh: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_budget_is_timed_like_fast() {
        assert!(ScanPass::Bootstrap.timed_budget());
        assert!(ScanPass::Fast.timed_budget());
        assert!(!ScanPass::Full.timed_budget());
        assert!(!ScanPass::Bootstrap.include_supplemental_sources());
        assert!(ScanPass::Fast.include_supplemental_sources());
        assert!(ScanPass::Full.include_supplemental_sources());
        assert!(!ScanPass::Bootstrap.include_app_paths());
        assert!(ScanPass::Fast.include_app_paths());
        assert!(ScanPass::Full.include_app_paths());
        assert!(!ScanPass::Bootstrap.include_uwp());
        assert!(!ScanPass::Bootstrap.include_metadata());
        assert!(ScanPass::Bootstrap.include_icons());
        assert!(!ScanPass::Fast.include_icons());
        assert!(ScanPass::Full.include_icons());
        assert!(!ScanPass::Bootstrap.include_protocols());
        assert!(ScanPass::Full.include_uwp());
        assert_eq!(ScanPass::Bootstrap.start_menu_depth(), START_MENU_MAX_DEPTH);
        assert_eq!(ScanPass::Full.start_menu_depth(), FULL_START_MENU_MAX_DEPTH);
    }
}
