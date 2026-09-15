//! Query 一次解析：词元、解释集合与通道预算。

use crate::search::normalizer::{compact, normalize_query, tokens};

/// 诊断：各通道候选量。
#[derive(Debug, Default, Clone)]
pub struct ChannelStats {
    pub exact: usize,
    pub prefix: usize,
    pub compact: usize,
    pub gram: usize,
    pub skip: usize,
    pub symspell: usize,
    pub pinyin: usize,
    pub alias: usize,
    pub user_alias: usize,
    pub scanned_fallback: usize,
}

#[derive(Debug, Clone)]
pub struct ParsedQuery {
    pub raw_norm: String,
    pub compact: String,
    pub tokens: Vec<String>,
    pub chars: Vec<char>,
    /// 全串为拉丁字母/数字（可走英文缩写与纯拼音解释）。
    pub latin: bool,
    /// 含 ASCII 字母/数字即可参与拼音解释（允许汉字+拼音/英文混输）。
    pub has_ascii_alnum: bool,
    /// 同时含 CJK 与 ASCII 字母数字（混输）。
    pub mixed: bool,
    /// 混输时的 CJK 连续段（用于名称精确/包含锚定）。
    pub cjk_parts: Vec<String>,
    /// 混输时的拉丁/数字段（用于拼音/英文解释）。
    pub latin_parts: Vec<String>,
}

impl ParsedQuery {
    pub fn is_empty(&self) -> bool {
        self.raw_norm.is_empty()
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4e00..=0x9fff | 0x3400..=0x4dbf)
}

fn split_script_parts(s: &str) -> (Vec<String>, Vec<String>) {
    let mut cjk_parts = Vec::new();
    let mut latin_parts = Vec::new();
    let mut cjk_buf = String::new();
    let mut latin_buf = String::new();
    let flush_cjk = |buf: &mut String, out: &mut Vec<String>| {
        if !buf.is_empty() {
            out.push(std::mem::take(buf));
        }
    };
    let flush_latin = |buf: &mut String, out: &mut Vec<String>| {
        if !buf.is_empty() {
            out.push(std::mem::take(buf));
        }
    };
    for c in s.chars() {
        if is_cjk(c) {
            flush_latin(&mut latin_buf, &mut latin_parts);
            cjk_buf.push(c);
        } else if c.is_ascii_alphanumeric() {
            flush_cjk(&mut cjk_buf, &mut cjk_parts);
            latin_buf.push(c);
        } else {
            flush_cjk(&mut cjk_buf, &mut cjk_parts);
            flush_latin(&mut latin_buf, &mut latin_parts);
        }
    }
    flush_cjk(&mut cjk_buf, &mut cjk_parts);
    flush_latin(&mut latin_buf, &mut latin_parts);
    (cjk_parts, latin_parts)
}

pub fn parse(query: &str) -> ParsedQuery {
    let raw_norm = normalize_query(query);
    let compact_q = compact(&raw_norm);
    let toks = tokens(&raw_norm)
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let chars: Vec<char> = raw_norm.chars().collect();
    let has_ascii_alnum = chars.iter().any(|c| c.is_ascii_alphanumeric());
    let has_cjk = chars.iter().copied().any(is_cjk);
    let latin = !raw_norm.is_empty()
        && raw_norm
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c.is_whitespace());
    let mixed = has_ascii_alnum && has_cjk;
    let (cjk_parts, latin_parts) = if mixed {
        split_script_parts(&raw_norm)
    } else {
        (Vec::new(), Vec::new())
    };
    ParsedQuery {
        raw_norm,
        compact: compact_q,
        tokens: toks,
        chars,
        latin,
        has_ascii_alnum,
        mixed,
        cjk_parts,
        latin_parts,
    }
}