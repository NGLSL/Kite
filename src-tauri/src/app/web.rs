//! 网址 / 网页搜索 → 已安装浏览器打开。

use crate::model::{AppItem, SearchResult};
use crate::system::browsers::{self, Browser};

/// URL 结果 id：`url:{browser_id}:{url}`；系统默认为 `url:default:{url}`。
pub fn make_id(browser_id: &str, url: &str) -> String {
    format!("url:{browser_id}:{url}")
}

/// 网页搜索结果 id：`websearch:{browser_id}:{query}`。
pub fn make_search_id(browser_id: &str, query: &str) -> String {
    format!("websearch:{browser_id}:{query}")
}

/// 解析 launch 用的 id → (kind, browser_id, payload)。
/// kind: "url" | "websearch"。
pub fn parse_id(id: &str) -> Option<(String, String, String)> {
    if let Some(rest) = id.strip_prefix("url:") {
        let (browser_id, url) = rest.split_once(':')?;
        if browser_id.is_empty() || url.is_empty() {
            return None;
        }
        return Some(("url".into(), browser_id.to_string(), url.to_string()));
    }
    if let Some(rest) = id.strip_prefix("websearch:") {
        let (browser_id, query) = rest.split_once(':')?;
        if browser_id.is_empty() || query.is_empty() {
            return None;
        }
        return Some((
            "websearch".into(),
            browser_id.to_string(),
            query.to_string(),
        ));
    }
    None
}

/// 组装「用浏览器打开」结果列表：偏好优先，其后为其余已安装浏览器 + 系统默认。
pub fn build_hits(url: &str, preferred: Option<&str>, icon_dir: &std::path::Path) -> Vec<SearchResult> {
    let installed = browsers::sort_preferred(browsers::discover_installed(), preferred);
    let mut hits = Vec::with_capacity(installed.len() + 2);

    // 偏好或第一项分最高，其余略低但仍高于普通应用
    for (i, b) in installed.iter().enumerate() {
        hits.push(browser_hit(b, url, if i == 0 { 980 } else { 960 }, icon_dir));
    }

    hits.push(default_hit(url));
    hits
}

/// 网页搜索固定槽位：1-based 第 5 项 → 下标 4。
pub const WEB_SEARCH_SLOT: usize = 4;

/// 只取偏好浏览器一条「网页搜索」；无已装浏览器则退回系统默认。
pub fn build_primary_search_hit(
    query: &str,
    preferred: Option<&str>,
    icon_dir: &std::path::Path,
    url_template: Option<&str>,
) -> Option<SearchResult> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    let installed = browsers::sort_preferred(browsers::discover_installed(), preferred);
    match installed.first() {
        Some(b) => Some(search_hit(b, q, 0, icon_dir, url_template)),
        None => Some(default_search_hit(q, url_template)),
    }
}

/// 把网页搜索项插入固定槽位（已有应用结果时排第 5）。
pub fn insert_at_slot(
    mut hits: Vec<SearchResult>,
    item: SearchResult,
    slot: usize,
) -> Vec<SearchResult> {
    let at = slot.min(hits.len());
    hits.insert(at, item);
    hits
}

/// 无应用结果时：「用 xx 搜索关键词」。优先浏览器默认引擎，否则中文百度/英文 Bing。
pub fn build_search_hits(
    query: &str,
    preferred: Option<&str>,
    icon_dir: &std::path::Path,
    url_template: Option<&str>,
) -> Vec<SearchResult> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let installed = browsers::sort_preferred(browsers::discover_installed(), preferred);
    let mut hits = Vec::with_capacity(installed.len() + 1);
    for (i, b) in installed.iter().enumerate() {
        hits.push(search_hit(b, q, if i == 0 { 920 } else { 900 }, icon_dir, url_template));
    }
    hits.push(default_search_hit(q, url_template));
    hits
}

fn search_hit(
    b: &Browser,
    query: &str,
    score: i32,
    icon_dir: &std::path::Path,
    url_template: Option<&str>,
) -> SearchResult {
    let id = make_search_id(&b.id, query);
    let name = format!("用 {} 搜索「{}」", b.name, query);
    let target = resolve_search_url(url_template, b.id.as_str(), query);
    let mut item = AppItem::scanned(
        id,
        name,
        target.clone(),
        Some(target),
        b.exe.parent().map(|p| p.to_string_lossy().to_string()),
        "websearch",
    );
    item.icon_src = Some(b.exe.to_string_lossy().to_string());
    item.icon = crate::system::icons::cache_icon(icon_dir, &item.id, item.icon_src.as_deref());
    SearchResult {
        item,
        score,
        matched_by: "websearch".into(),
    }
}

fn default_search_hit(query: &str, url_template: Option<&str>) -> SearchResult {
    let id = make_search_id("default", query);
    let target = resolve_search_url(url_template, "default", query);
    let item = AppItem::scanned(
        id,
        format!("用系统默认浏览器搜索「{query}」"),
        target.clone(),
        Some(target),
        None,
        "websearch",
    );
    SearchResult {
        item,
        score: 880,
        matched_by: "websearch".into(),
    }
}

/// 生成搜索 URL：缓存模板 → 浏览器偏好探测 → 中文百度/英文 Bing。
pub fn resolve_search_url(
    cached_template: Option<&str>,
    browser_id: &str,
    query: &str,
) -> String {
    let q = query.trim();
    let encoded = encode_query_component(q);
    if let Some(t) = cached_template.filter(|s| !s.is_empty()) {
        return crate::system::search_engine::apply_template(t, &encoded);
    }
    // 按当前打开的浏览器嗅探（进程内 OnceLock 缓存由 detect 侧保证轻量）
    if let Some(t) = crate::system::search_engine::detect_search_template(browser_id) {
        return crate::system::search_engine::apply_template(&t, &encoded);
    }
    if let Some(pref) = crate::system::browsers::discover_installed()
        .first()
        .map(|b| b.id.clone())
    {
        if let Some(t) = crate::system::search_engine::detect_search_template(&pref) {
            return crate::system::search_engine::apply_template(&t, &encoded);
        }
    }
    search_url(q)
}

/// 搜索引擎 URL：中文用百度，其它用 Bing。
pub fn search_url(query: &str) -> String {
    let encoded = encode_query_component(query.trim());
    if query.chars().any(is_cjk) {
        format!("https://www.baidu.com/s?wd={encoded}")
    } else {
        format!("https://www.bing.com/search?q={encoded}")
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF | 0x3000..=0x303F
    )
}

/// 简易 percent-encoding（query 组件）。
fn encode_query_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 实际打开。返回偏好应记住的浏览器 id（default 不记）。
pub fn launch_url(browser_id: &str, url: &str) -> Result<Option<String>, String> {
    open_with_browser(browser_id, url)
}

/// 用浏览器执行网页搜索（可选缓存模板）。
pub fn launch_websearch(
    browser_id: &str,
    query: &str,
    cached_template: Option<&str>,
) -> Result<Option<String>, String> {
    let url = resolve_search_url(cached_template, browser_id, query);
    open_with_browser(browser_id, &url)
}

fn open_with_browser(browser_id: &str, url: &str) -> Result<Option<String>, String> {
    if browser_id == "default" {
        browsers::open_with_default(url)?;
        return Ok(None);
    }
    let installed = browsers::discover_installed();
    let Some(b) = installed.iter().find(|b| b.id == browser_id) else {
        return Err(format!("browser not installed: {browser_id}"));
    };
    browsers::open_url(&b.exe, url)?;
    Ok(Some(b.id.clone()))
}

fn browser_hit(b: &Browser, url: &str, score: i32, icon_dir: &std::path::Path) -> SearchResult {
    let id = make_id(&b.id, url);
    let name = format!("用 {} 打开", b.name);
    let mut item = AppItem::scanned(
        id,
        name,
        url.to_string(),
        Some(url.to_string()),
        b.exe.parent().map(|p| p.to_string_lossy().to_string()),
        "browser",
    );
    item.icon_src = Some(b.exe.to_string_lossy().to_string());
    item.icon = crate::system::icons::cache_icon(icon_dir, &item.id, item.icon_src.as_deref());
    SearchResult {
        item,
        score,
        matched_by: "url".into(),
    }
}

fn default_hit(url: &str) -> SearchResult {
    let id = make_id("default", url);
    let item = AppItem::scanned(
        id,
        "用系统默认浏览器打开".into(),
        url.to_string(),
        Some(url.to_string()),
        None,
        "browser",
    );
    SearchResult {
        item,
        score: 940,
        matched_by: "url".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrip() {
        let id = make_id("chrome", "https://example.com");
        assert_eq!(id, "url:chrome:https://example.com");
        let (kind, b, u) = parse_id(&id).unwrap();
        assert_eq!(kind, "url");
        assert_eq!(b, "chrome");
        assert_eq!(u, "https://example.com");
    }

    #[test]
    fn parse_default() {
        let (kind, b, u) = parse_id("url:default:https://a.com").unwrap();
        assert_eq!(kind, "url");
        assert_eq!(b, "default");
        assert_eq!(u, "https://a.com");
    }

    #[test]
    fn parse_websearch() {
        let id = make_search_id("edge", "rust 教程");
        let (kind, b, q) = parse_id(&id).unwrap();
        assert_eq!(kind, "websearch");
        assert_eq!(b, "edge");
        assert_eq!(q, "rust 教程");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_id("app:chrome").is_none());
        assert!(parse_id("url:").is_none());
        assert!(parse_id("url:chrome").is_none());
        assert!(parse_id("websearch:").is_none());
    }

    #[test]
    fn chinese_uses_baidu() {
        let u = search_url("微信");
        assert!(u.starts_with("https://www.baidu.com/s?wd="));
        assert!(u.contains("%E5%BE%AE%E4%BF%A1"));
    }

    #[test]
    fn english_uses_bing() {
        let u = search_url("rust lang");
        assert!(u.starts_with("https://www.bing.com/search?q="));
        assert!(u.contains("rust%20lang"));
    }

    #[test]
    fn encode_keeps_safe_ascii() {
        assert_eq!(encode_query_component("abc-123"), "abc-123");
        assert_eq!(encode_query_component("a b"), "a%20b");
    }

    #[test]
    fn insert_at_fifth_slot() {
        let mut hits = Vec::new();
        for i in 0..8 {
            let item = AppItem::scanned(
                format!("app-{i}"),
                format!("App{i}"),
                format!("C:\\a{i}.exe"),
                None,
                None,
                "start-menu",
            );
            hits.push(SearchResult {
                item,
                score: 100 - i as i32,
                matched_by: "name".into(),
            });
        }
        let web = SearchResult {
            item: AppItem::scanned(
                "websearch:chrome:q".into(),
                "用 Chrome 搜索「q」".into(),
                "https://www.bing.com/search?q=q".into(),
                None,
                None,
                "websearch",
            ),
            score: 0,
            matched_by: "websearch".into(),
        };
        let out = insert_at_slot(hits, web, WEB_SEARCH_SLOT);
        assert_eq!(out.len(), 9);
        assert_eq!(out[4].item.source, "websearch");
        assert_eq!(out[0].item.name, "App0");
        assert_eq!(out[3].item.name, "App3");
        assert_eq!(out[5].item.name, "App4");
    }

    #[test]
    fn insert_slot_clamped_when_short() {
        let hits = vec![SearchResult {
            item: AppItem::scanned(
                "a".into(),
                "A".into(),
                "C:\\a.exe".into(),
                None,
                None,
                "start-menu",
            ),
            score: 1,
            matched_by: "name".into(),
        }];
        let web = SearchResult {
            item: AppItem::scanned(
                "websearch:default:q".into(),
                "搜索".into(),
                "https://x".into(),
                None,
                None,
                "websearch",
            ),
            score: 0,
            matched_by: "websearch".into(),
        };
        let out = insert_at_slot(hits, web, WEB_SEARCH_SLOT);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].item.source, "websearch");
    }
}
