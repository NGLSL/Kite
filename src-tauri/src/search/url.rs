//! 判断 Query 是否为可打开的网址。

/// 若 Query 像网址，返回规范化后的 URL（补全 https:// 等）。
pub fn normalize_url(query: &str) -> Option<String> {
    let q = query.trim();
    if q.is_empty() || q.contains(char::is_whitespace) {
        return None;
    }
    // 已带 scheme
    let lower = q.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(q.to_string());
    }
    if lower.starts_with("ftp://") {
        return Some(q.to_string());
    }
    // www.example.com
    if lower.starts_with("www.") && looks_like_host(&q[4..]) {
        return Some(format!("https://{q}"));
    }
    // example.com / example.com/path — 要求至少一个点且 TLD 像字母
    if let Some(host) = host_part(q) {
        if looks_like_host(host) {
            return Some(format!("https://{q}"));
        }
    }
    None
}

fn host_part(q: &str) -> Option<&str> {
    let end = q
        .find(['/', '?', '#'])
        .unwrap_or(q.len());
    let host = &q[..end];
    // 去掉端口
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// 主机名是否像真实域名：含点、无中文、末段为 2+ 字母。
fn looks_like_host(host: &str) -> bool {
    if host.is_empty() || host.contains(char::is_whitespace) {
        return false;
    }
    // 中文/全角直接排除
    if !host.is_ascii() {
        return false;
    }
    let Some((name, tld)) = host.rsplit_once('.') else {
        return false;
    };
    if name.is_empty() || tld.is_empty() {
        return false;
    }
    // TLD：2–24 位字母（含 xn-- 等简单情况按字母处理）
    if tld.len() < 2 || tld.len() > 24 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    // 常见文件扩展名不当作 TLD
    if is_file_ext(tld) {
        return false;
    }
    // 主体：字母数字 . - _ 合理字符
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

fn is_file_ext(tld: &str) -> bool {
    const EXTS: &[&str] = &[
        "txt", "exe", "dll", "pdf", "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "zip",
        "rar", "7z", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "md", "rs", "ts", "tsx", "js",
        "jsx", "json", "log", "csv", "xml", "html", "htm", "css", "yml", "yaml", "toml", "ini",
        "bat", "ps1", "msi", "iso", "mp3", "mp4", "mkv", "avi", "mov", "wav", "flac", "lnk", "db",
        "sql",
    ];
    let lower = tld.to_ascii_lowercase();
    EXTS.contains(&lower.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_passthrough() {
        assert_eq!(
            normalize_url("https://example.com/a").as_deref(),
            Some("https://example.com/a")
        );
        assert_eq!(
            normalize_url("http://example.com").as_deref(),
            Some("http://example.com")
        );
    }

    #[test]
    fn www_gets_https() {
        assert_eq!(
            normalize_url("www.example.com").as_deref(),
            Some("https://www.example.com")
        );
    }

    #[test]
    fn bare_domain_gets_https() {
        assert_eq!(
            normalize_url("github.com/torvalds").as_deref(),
            Some("https://github.com/torvalds")
        );
        assert_eq!(
            normalize_url("baidu.com").as_deref(),
            Some("https://baidu.com")
        );
    }

    #[test]
    fn not_url() {
        assert_eq!(normalize_url("微信"), None);
        assert_eq!(normalize_url("vs code"), None);
        assert_eq!(normalize_url("chrome"), None);
        assert_eq!(normalize_url("file.txt"), None);
        assert_eq!(normalize_url(""), None);
        assert_eq!(normalize_url("打开 github.com 浏览器"), None); // 含空格
    }

    #[test]
    fn chinese_tld_rejected() {
        assert_eq!(normalize_url("例子.公司"), None);
    }
}
