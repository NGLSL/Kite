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

/// 查询命中内置项时返回（Kite 设置优先，其后 Windows 设置页）。
pub fn collect_builtin_hits(query_norm: &str, icon_dir: &Path) -> Vec<SearchResult> {
    let mut hits = Vec::new();
    if let Some(mut h) = kite_settings_hit(query_norm) {
        fill_kite_icon(&mut h.item, icon_dir);
        hits.push(h);
    }
    hits.extend(windows_settings_hits(query_norm, icon_dir));
    hits
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
        item.icon = icons::cache_icon(icon_dir, &item.id, item.icon_src.as_deref());
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

fn match_page(query_norm: &str, page: &WinPage) -> Option<i32> {
    let name_norm = page.name.to_lowercase();
    if name_norm == query_norm {
        return Some(900);
    }
    if name_norm.contains(query_norm) || query_norm.contains(&name_norm) {
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
        k == query_norm || k.contains(query_norm) || query_norm.contains(k.as_str())
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
}
