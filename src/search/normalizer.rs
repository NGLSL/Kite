//! Query / 名称规范化：大小写、空白、基础符号。

/// 搜索 Query 规范化：trim、小写、连续空白压成单空格。
pub fn normalize_query(q: &str) -> String {
    collapse_ws(&q.trim().to_lowercase())
}

/// 索引侧名称规范化。
pub fn normalize_name(n: &str) -> String {
    collapse_ws(&n.trim().to_lowercase())
}

/// 连写形式：去掉所有空白（`To Do` → `todo`，`vs code` → `vscode`）。
pub fn compact(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 按空白切词；空输入返回空列表。
pub fn tokens(s: &str) -> Vec<&str> {
    s.split_whitespace().filter(|t| !t.is_empty()).collect()
}

/// CamelCase / PascalCase 拆词：`XTerminal` → `x` + `terminal`。
pub fn split_camel(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_uppercase() && !cur.is_empty() && chars.get(i + 1).is_some_and(|n| n.is_lowercase()) {
            out.push(std::mem::take(&mut cur).to_lowercase());
        }
        if ch.is_alphanumeric() {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur).to_lowercase());
        }
    }
    if !cur.is_empty() {
        out.push(cur.to_lowercase());
    }
    out
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

    #[test]
    fn compact_strips_spaces() {
        assert_eq!(compact("microsoft to do"), "microsofttodo");
        assert_eq!(compact("vs code"), "vscode");
    }

    #[test]
    fn tokens_split_whitespace() {
        assert_eq!(
            tokens("visual studio code"),
            vec!["visual", "studio", "code"]
        );
    }

    #[test]
    fn split_camel_finds_words() {
        assert_eq!(split_camel("XTerminal"), vec!["x", "terminal"]);
        assert_eq!(split_camel("Notepad++"), vec!["notepad"]);
    }
}
