//! 内置 Alias：各领域常用软件的缩写 → 应用名片段（小写，单表唯一数据源）。
//! 匹配语义：应用规范化名 == 片段 或 包含片段。
//! 片段必须足够具体，避免误伤（宁可少收一个缩写，不加一个泛词片段）。

/// 该 Query 是否为内置 Alias；是则返回目标名称片段。
/// 每 Query 只解析一次，matcher 逐项复用，避免逐项扫表。
pub fn targets_for(query_norm: &str) -> Option<&'static [&'static str]> {
    ALIAS_TO_NAMES
        .iter()
        .find(|(alias, _)| *alias == query_norm)
        .map(|(_, targets)| *targets)
}

/// Alias → 目标名称片段（OR）。片段命中任意一个即算点名。
const ALIAS_TO_NAMES: &[(&str, &[&str])] = &[
    // ── 开发 / IDE ──
    ("vs", &["visual studio"]), // 兼指 VS 与 VS Code，同分短名优先让 VS 在前
    ("vsc", &["visual studio code"]),
    ("vscode", &["visual studio code"]),
    ("code", &["visual studio code"]),
    ("idea", &["intellij idea"]),
    ("pycharm", &["pycharm"]),
    ("goland", &["goland"]),
    ("clion", &["clion"]),
    ("rider", &["rider"]),
    ("webstorm", &["webstorm"]),
    ("datagrip", &["datagrip"]),
    ("as", &["android studio"]),
    ("androidstudio", &["android studio"]),
    ("dbeaver", &["dbeaver"]),
    ("navicat", &["navicat"]),
    ("postman", &["postman"]),
    ("git", &["git"]),
    ("github", &["github"]),
    ("docker", &["docker"]),
    // ── 浏览器 ──
    ("chrome", &["google chrome"]),
    ("gc", &["google chrome"]),
    ("edge", &["microsoft edge", "edge"]),
    ("firefox", &["firefox", "火狐"]),
    ("brave", &["brave"]),
    ("vivaldi", &["vivaldi"]),
    ("opera", &["opera"]),
    ("qqbrowser", &["qq浏览器", "qqbrowser"]),
    ("360", &["360"]),
    // ── 终端 / 远程 ──
    ("cmd", &["命令提示符", "cmd"]),
    ("powershell", &["powershell"]),
    ("wt", &["terminal"]),
    ("cmder", &["cmder"]),
    ("tabby", &["tabby"]),
    ("mobaxterm", &["mobaxterm"]),
    ("xshell", &["xshell"]),
    ("finalshell", &["finalshell"]),
    ("putty", &["putty"]),
    // ── 办公 / 笔记 ──
    ("word", &["word", "microsoft word"]),
    ("excel", &["excel", "microsoft excel"]),
    ("ppt", &["powerpoint", "microsoft powerpoint"]),
    ("outlook", &["outlook", "microsoft outlook"]),
    ("onenote", &["onenote", "microsoft onenote"]),
    ("wps", &["wps"]),
    ("visio", &["visio"]),
    ("onedrive", &["onedrive"]),
    ("typora", &["typora"]),
    ("obsidian", &["obsidian"]),
    ("notion", &["notion"]),
    ("xmind", &["xmind"]),
    // ── PDF / 阅读 ──
    ("sumatra", &["sumatra"]),
    ("foxit", &["foxit", "福昕"]),
    ("acrobat", &["acrobat"]),
    ("calibre", &["calibre"]),
    // ── 设计 / 创意 ──
    ("ps", &["photoshop", "adobe photoshop"]),
    ("ai", &["illustrator"]),
    ("ae", &["after effects", "adobe after effects"]),
    ("pr", &["premiere", "adobe premiere"]),
    ("lr", &["lightroom"]),
    ("id", &["indesign"]),
    ("xd", &["adobe xd"]),
    ("figma", &["figma"]),
    ("blender", &["blender"]),
    ("c4d", &["cinema 4d", "c4d"]),
    ("maya", &["maya"]),
    ("cad", &["autocad"]),
    ("gimp", &["gimp"]),
    ("krita", &["krita"]),
    ("davinci", &["davinci", "达芬奇"]),
    ("audacity", &["audacity"]),
    // ── 影音 / 音乐 ──
    ("wyy", &["网易云音乐", "cloudmusic", "netease"]),
    ("qqmusic", &["qq音乐", "qqmusic"]),
    ("kugou", &["酷狗", "kugou"]),
    ("kuwo", &["酷我", "kuwo"]),
    ("ximalaya", &["喜马拉雅", "ximalaya"]),
    ("spotify", &["spotify"]),
    ("foobar", &["foobar2000", "foobar"]),
    ("vlc", &["vlc"]),
    ("potplayer", &["potplayer"]),
    ("mpv", &["mpv"]),
    ("iqiyi", &["爱奇艺", "iqiyi"]),
    ("qqlive", &["腾讯视频", "qqlive"]),
    ("youku", &["优酷", "youku"]),
    ("bilibili", &["哔哩哔哩", "bilibili"]),
    // ── 通讯 / 会议 ──
    ("wx", &["微信"]),
    ("weixin", &["微信"]),
    ("wechat", &["微信"]),
    ("qywx", &["企业微信"]),
    ("wxwork", &["企业微信"]),
    ("qq", &["qq"]),
    ("dingtalk", &["钉钉", "dingtalk"]),
    ("feishu", &["飞书", "feishu", "lark"]),
    ("tg", &["telegram"]),
    ("telegram", &["telegram"]),
    ("discord", &["discord"]),
    ("teams", &["teams"]),
    ("slack", &["slack"]),
    ("zoom", &["zoom"]),
    // ── 游戏 / 平台 ──
    ("steam", &["steam"]),
    ("epic", &["epic"]),
    ("wegame", &["wegame"]),
    ("uplay", &["ubisoft", "uplay"]),
    ("eaapp", &["ea app"]),
    ("bnet", &["battle.net", "battle"]),
    ("mc", &["minecraft", "我的世界"]),
    // ── 下载 / 压缩 / 网盘 ──
    ("7z", &["7-zip", "7zip"]),
    ("7zip", &["7-zip", "7zip"]),
    ("winrar", &["winrar"]),
    ("bandizip", &["bandizip"]),
    ("idm", &["internet download manager", "idm"]),
    ("fdm", &["free download manager"]),
    ("qbit", &["qbittorrent", "qbit"]),
    ("qbittorrent", &["qbittorrent", "qbit"]),
    ("thunder", &["迅雷", "thunder"]),
    ("xunlei", &["迅雷", "thunder"]),
    ("bdwp", &["百度网盘", "baidunetdisk"]),
    ("baidunetdisk", &["百度网盘", "baidunetdisk"]),
    ("alipan", &["阿里云盘"]),
    // ── 截图 / 效率 / 系统 ──
    ("everything", &["everything"]),
    ("listary", &["listary"]),
    ("utools", &["utools"]),
    ("ditto", &["ditto"]),
    ("snipaste", &["snipaste"]),
    ("sharex", &["sharex"]),
    ("pixpin", &["pixpin"]),
    ("powertoys", &["powertoys"]),
    ("trafficmonitor", &["trafficmonitor", "traffic monitor"]),
    ("notepad", &["记事本", "notepad"]),
    ("calc", &["计算器", "calculator"]),
    ("mspaint", &["画图", "paint"]),
    ("taskmgr", &["任务管理器", "task manager"]),
    ("regedit", &["注册表编辑器", "regedit"]),
    ("devmgmt", &["设备管理器", "device manager"]),
    ("control", &["控制面板", "control panel"]),
    ("cpuz", &["cpu-z", "cpuz"]),
    ("gpuz", &["gpu-z"]),
    ("hwinfo", &["hwinfo"]),
    ("aida64", &["aida64"]),
    ("diskgenius", &["diskgenius"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_keys_unique() {
        let mut keys: Vec<&str> = ALIAS_TO_NAMES.iter().map(|(a, _)| *a).collect();
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n, "存在重复 Alias 键");
    }

    #[test]
    fn fragments_are_lowercase_and_specific() {
        for &(alias, targets) in ALIAS_TO_NAMES {
            assert!(!targets.is_empty(), "alias {alias} 无目标");
            for t in targets {
                assert!(!t.is_empty(), "alias {alias} 有空片段");
                assert_eq!(*t, t.to_lowercase(), "alias {alias} 片段须小写：{t}");
                // 片段太短（1 字符）必然误伤
                assert!(t.chars().count() >= 2, "alias {alias} 片段过短：{t}");
            }
        }
    }

    #[test]
    fn resolves_spot_checks() {
        assert!(targets_for("wyy").unwrap().contains(&"网易云音乐"));
        assert_eq!(targets_for("steam"), Some(&["steam"][..]));
        assert!(targets_for("calc").unwrap().contains(&"计算器"));
        assert!(targets_for("zzz_not_an_alias").is_none());
    }

    #[test]
    fn known_aliases_cover_domains() {
        // 各领域抽查一条，防止误删
        for alias in ["vscode", "edge", "wt", "wps", "ai", "wyy", "steam", "7z", "snipaste"] {
            assert!(targets_for(alias).is_some(), "缺别名 {alias}");
        }
    }
}
