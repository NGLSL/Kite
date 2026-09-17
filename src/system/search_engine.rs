//! 从本机浏览器配置里嗅探默认搜索引擎 URL 模板。
//! 模板占位符统一为 `{searchTerms}`；Chrome/Edge/Brave 的 Preferences 可直接读。

use std::path::PathBuf;

use serde_json::Value;

/// 可选搜索引擎预设；`auto` 跟随浏览器默认，`custom` 用用户模板。
pub struct EnginePreset {
    pub id: &'static str,
    pub label: &'static str,
    /// 空串表示不写固定模板（auto / custom 由其它逻辑处理）。
    pub template: &'static str,
}

pub const ENGINE_PRESETS: &[EnginePreset] = &[
    EnginePreset {
        id: "auto",
        label: "自动",
        template: "",
    },
    EnginePreset {
        id: "baidu",
        label: "百度",
        template: "https://www.baidu.com/s?wd={searchTerms}",
    },
    EnginePreset {
        id: "bing",
        label: "Bing",
        template: "https://www.bing.com/search?q={searchTerms}",
    },
    EnginePreset {
        id: "google",
        label: "Google",
        template: "https://www.google.com/search?q={searchTerms}",
    },
    EnginePreset {
        id: "duckduckgo",
        label: "DuckDuckGo",
        template: "https://duckduckgo.com/?q={searchTerms}",
    },
    EnginePreset {
        id: "custom",
        label: "自定义",
        template: "",
    },
];

pub fn preset_by_id(id: &str) -> Option<&'static EnginePreset> {
    ENGINE_PRESETS.iter().find(|p| p.id == id)
}

/// 模板是否可直接用于网页搜索。
pub fn is_valid_template(template: &str) -> bool {
    let t = template.trim();
    !t.is_empty() && (t.contains("{searchTerms}") || t.contains("%s"))
}

/// 根据浏览器 id 探测搜索 URL 模板（含 `{searchTerms}`）。结果进程内缓存。
pub fn detect_search_template(browser_id: &str) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(g) = cache.lock() {
        if let Some(v) = g.get(browser_id) {
            return v.clone();
        }
    }
    let found = detect_search_template_uncached(browser_id);
    if let Ok(mut g) = cache.lock() {
        g.insert(browser_id.to_string(), found.clone());
    }
    found
}

fn detect_search_template_uncached(browser_id: &str) -> Option<String> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let pref = match browser_id {
        "chrome" => PathBuf::from(&local).join(r"Google\Chrome\User Data\Default\Preferences"),
        "edge" => PathBuf::from(&local).join(r"Microsoft\Edge\User Data\Default\Preferences"),
        "brave" => {
            PathBuf::from(&local).join(r"BraveSoftware\Brave-Browser\User Data\Default\Preferences")
        }
        "vivaldi" => PathBuf::from(&local).join(r"Vivaldi\User Data\Default\Preferences"),
        _ => return None,
    };
    let text = std::fs::read_to_string(pref).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    extract_template(&v)
}

fn extract_template(v: &Value) -> Option<String> {
    let dsp = v
        .get("default_search_provider")
        .or_else(|| v.pointer("/default_search_provider_data/template_url_data"))?;
    let url = dsp
        .get("search_url")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())?;
    normalize_template(url)
}

/// 把 Chrome 内部占位符收成 `{searchTerms}`；非法则 None。
fn normalize_template(url: String) -> Option<String> {
    let mut s = url;
    // 常见：{searchTerms} / {google:baseURL} 等
    if s.contains("{searchTerms}") {
        return Some(s);
    }
    if s.contains("%s") {
        return Some(s.replace("%s", "{searchTerms}"));
    }
    // Chrome 有时写成 ...?q={searchTerms}&...
    if let Some(i) = s.find("{search") {
        // 找到完整 {xxx} 占位
        if let Some(end) = s[i..].find('}') {
            let ph = s[i..i + end + 1].to_string();
            s = s.replace(&ph, "{searchTerms}");
            return Some(s);
        }
    }
    None
}

/// 用模板 + 已编码关键词生成最终 URL。
pub fn apply_template(template: &str, encoded_query: &str) -> String {
    if template.contains("{searchTerms}") {
        return template.replace("{searchTerms}", encoded_query);
    }
    if template.contains("%s") {
        return template.replace("%s", encoded_query);
    }
    // 退化：直接拼在末尾
    format!("{template}{encoded_query}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_searchterms() {
        let t = normalize_template("https://www.google.com/search?q={searchTerms}".into()).unwrap();
        assert_eq!(t, "https://www.google.com/search?q={searchTerms}");
    }

    #[test]
    fn normalize_percent_s() {
        let t = normalize_template("https://duckduckgo.com/?q=%s".into()).unwrap();
        assert_eq!(t, "https://duckduckgo.com/?q={searchTerms}");
    }

    #[test]
    fn preset_lookup_and_template_validation() {
        assert_eq!(preset_by_id("baidu").unwrap().label, "百度");
        assert!(preset_by_id("nope").is_none());
        assert!(is_valid_template("https://x.com/?q={searchTerms}"));
        assert!(is_valid_template("https://x.com/?q=%s"));
        assert!(!is_valid_template("https://x.com/"));
        assert!(!is_valid_template(""));
    }

    #[test]
    fn apply_replaces() {
        let u = apply_template(
            "https://www.google.com/search?q={searchTerms}",
            "rust%20lang",
        );
        assert_eq!(u, "https://www.google.com/search?q=rust%20lang");
    }

    #[test]
    fn extract_from_json() {
        let v: Value = serde_json::from_str(
            r#"{"default_search_provider":{"search_url":"https://www.bing.com/search?q={searchTerms}"}}"#,
        )
        .unwrap();
        let t = extract_template(&v).unwrap();
        assert!(t.contains("bing.com"));
    }
}
