//! 内置 Alias。原则：少而准，不为冷门词扩容。

use crate::search::ranker::SCORE_BUILTIN_ALIAS_EXACT;

/// 是否为内置 Alias（Query 已 normalize）。
pub fn is_builtin_alias(query_norm: &str) -> bool {
    BUILTIN_EXACT.iter().any(|a| *a == query_norm)
}

/// 该 Alias 是否点名当前应用（名称已 normalize）。
pub fn alias_targets_name(query_norm: &str, name_norm: &str) -> bool {
    let Some((_, targets)) = ALIAS_TO_NAMES.iter().find(|(alias, _)| *alias == query_norm) else {
        return false;
    };
    targets.iter().any(|t| name_norm == *t || name_norm.contains(t))
}

/// 内置 Alias 分数在 matcher 中直接引用 ranker 常量。

/// 高频英文/拼音缩写。
const BUILTIN_EXACT: &[&str] = &[
    "vs", "vsc", "vscode", "code", "wx", "weixin", "wechat", "qywx", "wxwork", "idea", "chrome",
    "gc", "ps", "ae", "pr", "word", "excel", "ppt", "outlook", "onenote",
];

/// Alias → 应用名（小写），用于把 alias 命中绑定到具体条目。
const ALIAS_TO_NAMES: &[(&str, &[&str])] = &[
    ("vs", &["visual studio code"]),
    ("vsc", &["visual studio code"]),
    ("vscode", &["visual studio code"]),
    ("code", &["visual studio code"]),
    ("wx", &["微信"]),
    ("weixin", &["微信"]),
    ("wechat", &["微信"]),
    ("qywx", &["企业微信"]),
    ("wxwork", &["企业微信"]),
    ("idea", &["intellij idea"]),
    ("chrome", &["google chrome"]),
    ("gc", &["google chrome"]),
    ("ps", &["photoshop", "adobe photoshop"]),
    ("ae", &["after effects", "adobe after effects"]),
    ("pr", &["premiere", "adobe premiere"]),
    ("word", &["word", "microsoft word"]),
    ("excel", &["excel", "microsoft excel"]),
    ("ppt", &["powerpoint", "microsoft powerpoint"]),
    ("outlook", &["outlook", "microsoft outlook"]),
    ("onenote", &["onenote", "microsoft onenote"]),
];
