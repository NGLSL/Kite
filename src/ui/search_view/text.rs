//! 结果标题与路径的纯文本处理。
pub(super) fn first_char(s: &str) -> String {
    s.chars().next().map(|c| c.to_string()).unwrap_or_default()
}

pub(crate) fn truncate_display_label(s: &str, max_units: usize) -> String {
    let total_units: usize = s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum();
    if total_units <= max_units {
        return s.to_string();
    }
    let mut current_units = 0;
    let mut chars = Vec::new();
    for c in s.chars() {
        let u = if c.is_ascii() { 1 } else { 2 };
        if current_units + u + 1 > max_units {
            break;
        }
        current_units += u;
        chars.push(c);
    }
    let truncated: String = chars.into_iter().collect();
    format!("{truncated}…")
}

pub(super) fn truncate_middle(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    let keep_head = max_chars.saturating_sub(1) / 2;
    let keep_tail = max_chars.saturating_sub(1) - keep_head;
    let head: String = chars[..keep_head].iter().collect();
    let tail: String = chars[chars.len() - keep_tail..].iter().collect();
    format!("{head}…{tail}")
}
