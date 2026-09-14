//! 内置可搜索动作：Kite 设置、Windows 系统设置页。

use std::path::Path;

use crate::model::{AppItem, SearchResult};
use crate::search::pinyin_of;
use crate::system::icons;

/// Kite 自带图标（裁边满幅），避免内置项落到字母兜底。
const KITE_ICON_PNG: &[u8] = include_bytes!("../../icons/32x32.png");

/// Windows 设置页：显示名、ms-settings URI、匹配关键词。
struct WinPage {
    name: &'static str,
    uri: &'static str,
    keywords: &'static [&'static str],
}

enum SystemToolTarget {
    Shell(&'static str),
    SystemFile(&'static str),
}

struct SystemTool {
    id: &'static str,
    name: &'static str,
    target: SystemToolTarget,
    args: Option<&'static str>,
    keywords: &'static [&'static str],
}

/// 常用系统设置页（中文名 + 英文/拼音关键词）。
const WIN_PAGES: &[WinPage] = &[
    WinPage { name: "系统", uri: "ms-settings:system", keywords: &["system", "xitong", "关于", "系统信息"] },
    WinPage { name: "显示", uri: "ms-settings:display", keywords: &["display", "xianshi", "屏幕", "分辨率", "显示器"] },
    WinPage { name: "声音", uri: "ms-settings:sound", keywords: &["sound", "shengyin", "音量", "音频"] },
    WinPage { name: "通知", uri: "ms-settings:notifications", keywords: &["notification", "tongzhi", "通知"] },
    WinPage { name: "专注助手", uri: "ms-settings:quiethours", keywords: &["focus", "zhuanzhu", "勿扰"] },
    WinPage { name: "电源", uri: "ms-settings:powersleep", keywords: &["power", "dianyuan", "睡眠", "电池"] },
    WinPage { name: "存储", uri: "ms-settings:storagesense", keywords: &["storage", "cunchu", "磁盘"] },
    WinPage { name: "多任务处理", uri: "ms-settings:multitasking", keywords: &["multitask", "duorenwu", "贴靠"] },
    WinPage { name: "激活", uri: "ms-settings:activation", keywords: &["activation", "jihuo", "许可证"] },
    WinPage { name: "查找我的设备", uri: "ms-settings:findmydevice", keywords: &["find", "chazhao"] },
    WinPage { name: "远程桌面", uri: "ms-settings:remotedesktop", keywords: &["remote", "yuancheng", "rdp"] },
    WinPage { name: "可选功能", uri: "ms-settings:optionalfeatures", keywords: &["optional", "kexuan", "windows 功能"] },
    WinPage { name: "设备", uri: "ms-settings:devices", keywords: &["devices", "shebei", "打印机", "鼠标", "键盘"] },
    WinPage { name: "蓝牙和其他设备", uri: "ms-settings:bluetooth", keywords: &["bluetooth", "lanya", "蓝牙"] },
    WinPage { name: "打印机和扫描仪", uri: "ms-settings:printers", keywords: &["printer", "dayinji", "扫描"] },
    WinPage { name: "鼠标", uri: "ms-settings:mousetouchpad", keywords: &["mouse", "shubiao"] },
    WinPage { name: "触摸板", uri: "ms-settings:devices-touchpad", keywords: &["touchpad", "chumoban"] },
    WinPage { name: "网络和 Internet", uri: "ms-settings:network", keywords: &["network", "wangluo", "wifi", "internet", "网络"] },
    WinPage { name: "WLAN", uri: "ms-settings:network-wifi", keywords: &["wlan", "wifi", "无线"] },
    WinPage { name: "VPN", uri: "ms-settings:network-vpn", keywords: &["vpn"] },
    WinPage { name: "代理", uri: "ms-settings:network-proxy", keywords: &["proxy", "daili"] },
    WinPage { name: "飞行模式", uri: "ms-settings:network-airplanemode", keywords: &["airplane", "feixing"] },
    WinPage { name: "移动热点", uri: "ms-settings:network-mobilehotspot", keywords: &["hotspot", "redian"] },
    WinPage { name: "个性化", uri: "ms-settings:personalization", keywords: &["personalization", "geren", "主题", "壁纸"] },
    WinPage { name: "背景", uri: "ms-settings:personalization-background", keywords: &["background", "beijing", "壁纸"] },
    WinPage { name: "颜色", uri: "ms-settings:personalization-colors", keywords: &["color", "yanse", "深色", "浅色"] },
    WinPage { name: "锁屏界面", uri: "ms-settings:lockscreen", keywords: &["lockscreen", "suoping"] },
    WinPage { name: "开始", uri: "ms-settings:personalization-start", keywords: &["start", "kaishi", "开始菜单"] },
    WinPage { name: "任务栏", uri: "ms-settings:taskbar", keywords: &["taskbar", "renwulan"] },
    WinPage { name: "字体", uri: "ms-settings:fonts", keywords: &["fonts", "ziti"] },
    WinPage { name: "应用", uri: "ms-settings:appsfeatures", keywords: &["apps", "yingyong", "卸载", "应用和功能"] },
    WinPage { name: "默认应用", uri: "ms-settings:defaultapps", keywords: &["default", "moren", "默认"] },
    WinPage { name: "启动", uri: "ms-settings:startupapps", keywords: &["startup", "qidong", "自启"] },
    WinPage { name: "账户", uri: "ms-settings:yourinfo", keywords: &["account", "zhanghu", "用户"] },
    WinPage { name: "登录选项", uri: "ms-settings:signinoptions", keywords: &["signin", "denglu", "密码", "指纹", "人脸"] },
    WinPage { name: "时间和语言", uri: "ms-settings:dateandtime", keywords: &["time", "shijian", "日期", "时区"] },
    WinPage { name: "语言", uri: "ms-settings:regionlanguage", keywords: &["language", "yuyan", "输入法"] },
    WinPage { name: "游戏", uri: "ms-settings:gaming", keywords: &["gaming", "youxi", "xbox"] },
    WinPage { name: "辅助功能", uri: "ms-settings:easeofaccess", keywords: &["accessibility", "wuzhangai", "辅助", "放大镜", "讲述人"] },
    WinPage { name: "隐私", uri: "ms-settings:privacy", keywords: &["privacy", "yinsi", "权限"] },
    WinPage { name: "摄像头", uri: "ms-settings:privacy-webcam", keywords: &["camera", "shexiangtou"] },
    WinPage { name: "麦克风", uri: "ms-settings:privacy-microphone", keywords: &["microphone", "maikefeng"] },
    WinPage { name: "更新和安全", uri: "ms-settings:windowsupdate", keywords: &["update", "gengxin", "windows update", "补丁"] },
    WinPage { name: "Windows 安全中心", uri: "ms-settings:windowsdefender", keywords: &["defender", "anquan", "病毒", "防火墙"] },
    WinPage { name: "开发者选项", uri: "ms-settings:developers", keywords: &["developer", "kaifazhe", "开发"] },
    WinPage { name: "剪贴板", uri: "ms-settings:clipboard", keywords: &["clipboard", "jiantieban"] },
];

/// Windows 常用管理入口。它们不一定出现在开始菜单里，因此作为稳定的
/// 内置结果提供；文件型目标只在当前系统确实存在时显示。
const SYSTEM_TOOLS: &[SystemTool] = &[
    SystemTool {
        id: "file-explorer",
        name: "文件资源管理器",
        target: SystemToolTarget::SystemFile("explorer.exe"),
        args: None,
        keywords: &[
            "explorer",
            "file explorer",
            "exp",
            "wenjianziyuanguanliqi",
            "资源管理器",
            "this pc",
            "此电脑",
        ],
    },
    SystemTool {
        id: "recycle-bin",
        name: "回收站",
        target: SystemToolTarget::Shell("shell:RecycleBinFolder"),
        args: None,
        keywords: &["recycle", "recycle bin", "trash", "huishouzhan"],
    },
    SystemTool {
        id: "control-panel",
        name: "控制面板",
        target: SystemToolTarget::Shell("shell:ControlPanelFolder"),
        args: None,
        keywords: &["control", "control panel", "kongzhimianban"],
    },
    SystemTool {
        id: "registry-editor",
        name: "注册表编辑器",
        target: SystemToolTarget::SystemFile("regedit.exe"),
        args: None,
        keywords: &["registry", "regedit", "zhucebiaobianjiqi", "注册表"],
    },
    SystemTool {
        id: "task-manager",
        name: "任务管理器",
        target: SystemToolTarget::SystemFile("System32\\Taskmgr.exe"),
        args: None,
        keywords: &["taskmgr", "task manager", "renwuguanliqi"],
    },
    SystemTool {
        id: "device-manager",
        name: "设备管理器",
        target: SystemToolTarget::SystemFile("System32\\devmgmt.msc"),
        args: None,
        keywords: &["devmgmt", "device manager", "shebeiguanliqi"],
    },
    SystemTool {
        id: "services",
        name: "服务",
        target: SystemToolTarget::SystemFile("System32\\services.msc"),
        args: None,
        keywords: &["services", "service", "fuwu"],
    },
    SystemTool {
        id: "event-viewer",
        name: "事件查看器",
        target: SystemToolTarget::SystemFile("System32\\eventvwr.msc"),
        args: None,
        keywords: &["event viewer", "eventvwr", "shijianchakanqi"],
    },
    SystemTool {
        id: "computer-management",
        name: "计算机管理",
        target: SystemToolTarget::SystemFile("System32\\compmgmt.msc"),
        args: None,
        keywords: &["computer management", "compmgmt", "jisuanjiguanli"],
    },
    SystemTool {
        id: "disk-management",
        name: "磁盘管理",
        target: SystemToolTarget::SystemFile("System32\\diskmgmt.msc"),
        args: None,
        keywords: &["disk management", "diskmgmt", "ciguanli"],
    },
    SystemTool {
        id: "programs-and-features",
        name: "程序和功能",
        target: SystemToolTarget::SystemFile("System32\\control.exe"),
        args: Some("/name Microsoft.ProgramsAndFeatures"),
        keywords: &["programs and features", "appwiz", "chengxugongneng"],
    },
    SystemTool {
        id: "network-connections",
        name: "网络连接",
        target: SystemToolTarget::Shell("shell:ConnectionsFolder"),
        args: None,
        keywords: &["network connections", "ncpa", "wangluolianjie"],
    },
    SystemTool {
        id: "printers",
        name: "打印机",
        target: SystemToolTarget::Shell("shell:PrintersFolder"),
        args: None,
        keywords: &["printers", "printer", "dayinji"],
    },
    SystemTool {
        id: "windows-tools",
        name: "Windows 工具",
        target: SystemToolTarget::Shell("shell:Administrative Tools"),
        args: None,
        keywords: &["windows tools", "administrative tools", "gongju"],
    },
];

/// 查询命中内置项时返回（Kite 设置、Windows 设置页和常用系统工具）。
pub fn collect_builtin_hits(query_norm: &str, icon_dir: &Path) -> Vec<SearchResult> {
    let mut hits = Vec::new();
    if let Some(mut h) = kite_settings_hit(query_norm) {
        fill_kite_icon(&mut h.item, icon_dir);
        hits.push(h);
    }
    hits.extend(windows_settings_hits(query_norm, icon_dir));
    hits.extend(system_tool_hits(query_norm, icon_dir));
    hits
}

/// 物化全部系统入口为可搜索 AppItem（快照构建时调用一次）。
/// 不在此同步提取/校验图标——`icon_dir` 提供时仅写入 kite 图标与 settings 源路径。
/// 文件型系统工具仅在目标存在时纳入。
pub fn materialize_system_entries(icon_dir: Option<&Path>) -> Vec<AppItem> {
    let mut out = Vec::new();

    // Kite 设置
    let mut kite = AppItem::scanned(
        "kite:settings".into(),
        "Kite 设置".into(),
        "kite:settings".into(),
        None,
        None,
        "builtin",
    );
    kite.search_keywords = vec![
        "setting".into(),
        "settings".into(),
        "shezhi".into(),
        "sz".into(),
        "kite".into(),
    ];
    kite.attach_search_fields();
    if let Some(dir) = icon_dir {
        fill_kite_icon(&mut kite, dir);
    }
    out.push(kite);

    // Windows 设置页
    let settings_exe = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())
        + r"\ImmersiveControlPanel\SystemSettings.exe";
    for page in WIN_PAGES {
        let mut item = AppItem::scanned(
            format!("winsettings:{}", page.uri),
            format!("Windows · {}", page.name),
            page.uri.into(),
            None,
            None,
            "win-settings",
        );
        item.icon_src = Some(settings_exe.clone());
        item.search_keywords = page.keywords.iter().map(|k| k.to_lowercase()).collect();
        item.attach_search_fields();
        out.push(item);
    }

    // 系统工具（文件目标须存在）
    let system_root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    for tool in SYSTEM_TOOLS {
        let target = match tool.target {
            SystemToolTarget::Shell(uri) => uri.to_string(),
            SystemToolTarget::SystemFile(relative) => {
                let path = system_root.join(relative);
                if !path.is_file() {
                    continue;
                }
                path.to_string_lossy().to_string()
            }
        };
        let mut item = AppItem::scanned(
            format!("system-tool:{}", tool.id),
            tool.name.into(),
            target.clone(),
            tool.args.map(str::to_string),
            std::path::Path::new(&target)
                .parent()
                .filter(|p| p.is_dir())
                .map(|p| p.to_string_lossy().to_string()),
            "builtin-system",
        );
        item.search_keywords = tool.keywords.iter().map(|k| k.to_lowercase()).collect();
        item.attach_search_fields();
        item.icon_src = Some(target);
        out.push(item);
    }

    if let Some(dir) = icon_dir {
        fill_entry_icons(&mut out, dir);
    }

    out
}

/// 展示阶段补齐系统入口图标（不在按键匹配路径调用）。
pub fn fill_entry_icons(entries: &mut [AppItem], icon_dir: &Path) {
    for item in entries.iter_mut() {
        if item.icon.is_some() {
            continue;
        }
        if item.id == "kite:settings" {
            fill_kite_icon(item, icon_dir);
            continue;
        }
        if let Some(src) = item.icon_src.clone() {
            item.icon = icons::cache_icon(icon_dir, &item.id, Some(&src), None);
        }
    }
}

fn fill_kite_icon(item: &mut AppItem, icon_dir: &Path) {
    let _ = std::fs::create_dir_all(icon_dir);
    let out = icon_dir.join("builtin-kite-settings.png");
    if !out.exists() {
        let _ = std::fs::write(&out, KITE_ICON_PNG);
    }
    item.icon = Some(out.to_string_lossy().to_string());
    item.icon_src = Some(out.to_string_lossy().to_string());
}

fn kite_settings_hit(query_norm: &str) -> Option<SearchResult> {
    let matched = query_norm == "设置"
        || query_norm == "setting"
        || query_norm == "settings"
        || query_norm == "shezhi"
        || query_norm == "sz"
        || query_norm == "kite"
        || query_norm.starts_with("设置")
        || query_norm.starts_with("setting")
        || query_norm.starts_with("kite设置")
        || query_norm.starts_with("kite setting");
    if !matched {
        return None;
    }
    let mut item = AppItem::scanned(
        "kite:settings".into(),
        "Kite 设置".into(),
        "kite:settings".into(),
        None,
        None,
        "builtin",
    );
    item.attach_search_fields();
    Some(SearchResult {
        item,
        score: 930,
        matched_by: "builtin".into(),
    })
}

fn windows_settings_hits(query_norm: &str, icon_dir: &Path) -> Vec<SearchResult> {
    if query_norm.chars().count() < 2 && !query_norm.is_ascii() {
        return Vec::new();
    }
    let settings_exe = std::env::var("SystemRoot")
        .unwrap_or_else(|_| r"C:\Windows".into())
        + r"\ImmersiveControlPanel\SystemSettings.exe";

    let mut hits = Vec::new();
    for page in WIN_PAGES {
        let Some(score) = match_page(query_norm, page) else {
            continue;
        };
        let mut item = AppItem::scanned(
            format!("winsettings:{}", page.uri),
            format!("Windows · {}", page.name),
            page.uri.into(),
            None,
            None,
            "win-settings",
        );
        item.icon_src = Some(settings_exe.clone());
        item.attach_search_fields();
        item.icon = icons::cache_icon(
            icon_dir,
            &item.id,
            item.icon_src.as_deref(),
            None,
        );
        hits.push(SearchResult {
            item,
            score,
            matched_by: "win-settings".into(),
        });
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score));
    hits.truncate(6);
    hits
}

fn system_tool_hits(query_norm: &str, icon_dir: &Path) -> Vec<SearchResult> {
    if query_norm.chars().count() < 2 && !query_norm.is_ascii() {
        return Vec::new();
    }

    let system_root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    let mut hits = Vec::new();
    for tool in SYSTEM_TOOLS {
        let target = match tool.target {
            SystemToolTarget::Shell(uri) => uri.to_string(),
            SystemToolTarget::SystemFile(relative) => {
                let path = system_root.join(relative);
                if !path.is_file() {
                    continue;
                }
                path.to_string_lossy().to_string()
            }
        };
        let Some(score) = match_system_tool(query_norm, tool) else {
            continue;
        };
        let mut item = AppItem::scanned(
            format!("system-tool:{}", tool.id),
            tool.name.into(),
            target.clone(),
            tool.args.map(str::to_string),
            std::path::Path::new(&target)
                .parent()
                .filter(|p| p.is_dir())
                .map(|p| p.to_string_lossy().to_string()),
            "builtin-system",
        );
        item.attach_search_fields();
        item.icon_src = Some(target);
        item.icon = icons::cache_icon(
            icon_dir,
            &item.id,
            item.icon_src.as_deref(),
            None,
        );
        hits.push(SearchResult {
            item,
            score,
            matched_by: "builtin-system".into(),
        });
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score));
    hits
}

/// 英文按单词前缀匹配，避免 `ter` 命中 Internet / printer 这类无关尾部。
/// 中文仍允许片段匹配；完整名称包含在较长查询中时也继续召回。
fn matches_name_or_keyword(text: &str, query: &str) -> bool {
    if text == query || query.contains(text) {
        return true;
    }
    if query.is_ascii() {
        text.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| !word.is_empty() && word.starts_with(query))
    } else {
        text.contains(query)
    }
}

fn match_system_tool(query_norm: &str, tool: &SystemTool) -> Option<i32> {
    let name_norm = tool.name.to_lowercase();
    if name_norm == query_norm {
        return Some(900);
    }
    if matches_name_or_keyword(&name_norm, query_norm) {
        return Some(860);
    }
    let (full, initials) = pinyin_of(tool.name);
    if !full.is_empty() && (full == query_norm || full.starts_with(query_norm)) {
        return Some(840);
    }
    if !initials.is_empty() && (initials == query_norm || initials.starts_with(query_norm)) {
        return Some(830);
    }
    if tool.keywords.iter().any(|keyword| {
        let keyword = keyword.to_lowercase();
        matches_name_or_keyword(&keyword, query_norm)
    }) {
        return Some(820);
    }
    None
}

fn match_page(query_norm: &str, page: &WinPage) -> Option<i32> {
    let name_norm = page.name.to_lowercase();
    if name_norm == query_norm {
        return Some(900);
    }
    if matches_name_or_keyword(&name_norm, query_norm) {
        return Some(860);
    }
    let (full, ini) = pinyin_of(page.name);
    if !full.is_empty() && (full == query_norm || full.starts_with(query_norm)) {
        return Some(840);
    }
    if !ini.is_empty() && ini == query_norm {
        return Some(830);
    }
    if page.keywords.iter().any(|k| {
        let k = k.to_lowercase();
        matches_name_or_keyword(&k, query_norm)
    }) {
        return Some(820);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kite-builtin-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn kite_settings_exact() {
        let dir = tmp_dir();
        let hits = collect_builtin_hits("设置", &dir);
        assert!(hits.iter().any(|h| h.item.id == "kite:settings"));
        assert!(hits
            .iter()
            .find(|h| h.item.id == "kite:settings")
            .unwrap()
            .item
            .icon
            .is_some());
    }

    #[test]
    fn kite_settings_english() {
        let dir = tmp_dir();
        let hits = collect_builtin_hits("setting", &dir);
        assert!(hits.iter().any(|h| h.item.id == "kite:settings"));
    }

    #[test]
    fn win_display() {
        let dir = tmp_dir();
        let hits = collect_builtin_hits("显示", &dir);
        assert!(hits
            .iter()
            .any(|h| h.item.target.contains("ms-settings:display")));
    }

    #[test]
    fn win_wifi_keyword() {
        let dir = tmp_dir();
        let hits = collect_builtin_hits("wifi", &dir);
        assert!(hits
            .iter()
            .any(|h| h.item.name.contains("WLAN") || h.item.target.contains("wifi")));
    }

    #[test]
    fn no_spam_on_random() {
        let dir = tmp_dir();
        let hits = collect_builtin_hits("zzzzqqq", &dir);
        assert!(hits.is_empty());
    }

    #[test]
    fn system_tools_include_recycle_bin_and_control_panel() {
        let dir = tmp_dir();
        let recycle = collect_builtin_hits("回收站", &dir);
        assert!(
            recycle
                .iter()
                .any(|h| h.item.target == "shell:RecycleBinFolder" && h.item.icon.is_some()),
            "回收站应作为带系统图标的可启动工具返回"
        );

        let control = collect_builtin_hits("control panel", &dir);
        assert!(
            control.iter().any(|h| h.item.target == "shell:ControlPanelFolder"),
            "控制面板应支持英文搜索"
        );
    }

    #[test]
    fn system_tools_include_registry_editor() {
        let dir = tmp_dir();
        let hits = collect_builtin_hits("注册表", &dir);
        assert!(
            hits.iter().any(|h| h.item.name == "注册表编辑器"),
            "注册表应返回注册表编辑器"
        );
    }

    #[test]
    fn english_word_suffix_does_not_crowd_out_app_results() {
        let dir = tmp_dir();
        assert!(
            collect_builtin_hits("ter", &dir).is_empty(),
            "ter 不应召回 Internet、printer、computer 等词尾"
        );
        assert!(
            collect_builtin_hits("internet", &dir)
                .iter()
                .any(|h| h.item.name == "Windows · 网络和 Internet")
        );
        assert!(
            collect_builtin_hits("print", &dir)
                .iter()
                .any(|h| h.item.name == "打印机")
        );
    }
}
