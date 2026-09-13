//! Query / 名称规范化：大小写、空白、基础符号。

/// 搜索 Query 规范化：trim、小写、连续空白压成单空格。
pub fn normalize_query(q: &str) -> String {
    collapse_ws(&q.trim().to_lowercase())
}

/// 索引侧名称规范化。
pub fn normalize_name(n: &str) -> String {
    collapse_ws(&n.trim().to_lowercase())
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_ws && !out.is_empty() {
                out.push(' ');
            }
            last_ws = true;
        } else {
            out.push(ch);
            last_ws = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_lowers() {
        assert_eq!(normalize_query("  VS Code  "), "vs code");
    }

    #[test]
    fn collapses_spaces() {
        assert_eq!(normalize_query("a   b\tc"), "a b c");
    }
}
