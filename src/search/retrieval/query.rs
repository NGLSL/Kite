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
    /// 拉丁字母/数字串（可走英文缩写与拼音解释）。
    pub latin: bool,
}

impl ParsedQuery {
    pub fn is_empty(&self) -> bool {
        self.raw_norm.is_empty()
    }
}

pub fn parse(query: &str) -> ParsedQuery {
    let raw_norm = normalize_query(query);
    let compact_q = compact(&raw_norm);
    let toks = tokens(&raw_norm)
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let chars: Vec<char> = raw_norm.chars().collect();
    let latin = !raw_norm.is_empty()
        && raw_norm
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c.is_whitespace());
    ParsedQuery {
        raw_norm,
        compact: compact_q,
        tokens: toks,
        chars,
        latin,
    }
}
