//! 多通道候选生成：各通道独立召回，最后取并集。
//! 允许假阳性（验证阶段剔除）；不得因“已有普通结果”停止其他通道。

use std::collections::HashSet;

use crate::search::alias;
use crate::search::matcher::UserTarget;

use super::doc::{DocId, RetrievalIndex};
use super::query::{ChannelStats, ParsedQuery};

/// 最多从词典枚举的前缀词元数（控制爆炸）。
const MAX_PREFIX_TERMS: usize = 16;

#[derive(Debug, Default)]
pub struct Candidates {
    pub ids: HashSet<DocId>,
    pub stats: ChannelStats,
}

pub fn collect(index: &RetrievalIndex, q: &ParsedQuery, user_targets: &[UserTarget]) -> Candidates {
    let mut out = Candidates::default();
    if q.is_empty() {
        return out;
    }

    // 用户 Alias：稳定 id 直达，不扫全集。
    for t in user_targets {
        if let Some(id) = &t.id {
            if let Some(doc) = index.lookup_stable(id) {
                out.ids.insert(doc);
                out.stats.user_alias += 1;
            }
        } else {
            let n = t.name.to_lowercase();
            for doc in &index.docs {
                if doc.name.contains(&n) || doc.display.contains(&n) {
                    out.ids.insert(doc.id);
                    out.stats.user_alias += 1;
                }
            }
        }
    }

    // 内置 Alias
    if let Some(fragments) = alias::targets_for(&q.raw_norm) {
        for frag in fragments {
            if let Some(list) = index.postings(frag) {
                for &id in list {
                    out.ids.insert(id);
                    out.stats.alias += 1;
                }
            }
            // 片段包含：用 gram 通道兜底
            add_gram_candidates(index, frag, &mut out);
        }
    }

    // 词元精确 / 前缀
    let mut seed_terms: Vec<String> = q.tokens.clone();
    if seed_terms.is_empty() && !q.raw_norm.is_empty() {
        seed_terms.push(q.raw_norm.clone());
    }
    for term in &seed_terms {
        if let Some(list) = index.postings(term) {
            for &id in list {
                out.ids.insert(id);
                out.stats.exact += 1;
            }
        }
        // 前缀枚举
        if term.chars().count() >= 2 {
            for t in index.prefix_terms(term, MAX_PREFIX_TERMS) {
                if let Some(list) = index.postings(&t) {
                    for &id in list {
                        out.ids.insert(id);
                        out.stats.prefix += 1;
                    }
                }
            }
        }
    }

    // compact 精确/前缀（todo ↔ To Do）
    if q.compact.len() >= 2 {
        if let Some(list) = index.compact_postings(&q.compact) {
            for &id in list {
                out.ids.insert(id);
                out.stats.compact += 1;
            }
        }
        // compact 的前缀也走 gram 连续验证
        add_gram_candidates(index, &q.compact, &mut out);
    }

    // 多词：各词 postings 求交（最短优先），不因跨字段过早求交丢失——验证阶段再核
    if q.tokens.len() >= 2 {
        let mut lists: Vec<&[DocId]> = Vec::new();
        for t in &q.tokens {
            if let Some(list) = index.postings(t) {
                lists.push(list);
            } else {
                // 某词无精确词元：用前缀扩展
                let mut ids: Vec<DocId> = Vec::new();
                for pt in index.prefix_terms(t, MAX_PREFIX_TERMS) {
                    if let Some(list) = index.postings(&pt) {
                        ids.extend_from_slice(list);
                    }
                }
                ids.sort_unstable();
                ids.dedup();
                if ids.is_empty() {
                    // 交集为空则跳过该严格交，退化为并集+验证
                    lists.clear();
                    break;
                }
                // 无法借用临时 vec——改为直接收集候选并集
                for id in ids {
                    out.ids.insert(id);
                }
            }
        }
        if lists.len() == q.tokens.len() {
            let shortest = lists.iter().min_by_key(|l| l.len()).copied().unwrap();
            for &id in shortest {
                if lists.iter().all(|l| l.binary_search(&id).is_ok()) {
                    out.ids.insert(id);
                }
            }
        }
    }

    // n-gram 中段片段（短 query 优先 bigram，长 query 优先 trigram）
    add_gram_candidates(index, &q.raw_norm, &mut out);

    // 字符跳字：仅对较长 Query 启用；用最稀有 Query 字符的倒排作锚
    // （3 字以下选择性差，交给精确/前缀/gram）
    if q.chars.len() >= 4 {
        let mut seed: Option<&[DocId]> = None;
        for &c in &q.chars {
            if let Some(list) = index.char_postings(c) {
                if seed.map(|s| list.len() < s.len()).unwrap_or(true) {
                    seed = Some(list);
                }
            }
        }
        if let Some(list) = seed {
            for &id in list {
                let Some(doc) = index.doc(id) else { continue };
                if doc.name_bits.contains_all(&q.chars) || doc.display_bits.contains_all(&q.chars) {
                    out.ids.insert(id);
                    out.stats.skip += 1;
                }
            }
        }
    }

    // 拼音 / 拼音首字母：词元已在 postings；再对 syllable 混合解释做种子
    if q.latin {
        add_pinyin_candidates(index, q, &mut out);
    }

    // SymSpell 纠错：独立通道，不被其他必要条件删除
    if q.chars.len() >= 3 {
        let max_d = crate::search::fuzzy::max_distance(q.raw_norm.len());
        if max_d > 0 {
            for term in &seed_terms {
                for (orig, dist) in index.deletes().expand(term, max_d) {
                    if dist == 0 {
                        continue;
                    }
                    if let Some(list) = index.postings(&orig) {
                        for &id in list {
                            out.ids.insert(id);
                            out.stats.symspell += 1;
                        }
                    }
                }
            }
            // 整串也查删除索引（chorme → chrome）
            for (orig, dist) in index.deletes().expand(&q.raw_norm, max_d) {
                if dist == 0 {
                    continue;
                }
                if let Some(list) = index.postings(&orig) {
                    for &id in list {
                        out.ids.insert(id);
                        out.stats.symspell += 1;
                    }
                }
            }
        }
    }

    // 成本模型：极短 Query 用首字符倒排兜底，而不是无条件全集扫描
    if q.chars.len() <= 2 {
        if let Some(&first) = q.chars.first() {
            let list = if q.chars.len() == 1 {
                index.char_postings(first)
            } else {
                index.first_char_postings(first)
            };
            if let Some(list) = list {
                for &id in list {
                    out.ids.insert(id);
                    out.stats.scanned_fallback += 1;
                }
            }
        }
        // 拼音首字母前缀：k → 控制面板（kzmb）；名称首字符是汉字时 first_char 盖不到
        if q.latin {
            for doc in &index.docs {
                if doc.pinyin_initials.starts_with(&q.raw_norm) {
                    out.ids.insert(doc.id);
                    out.stats.pinyin += 1;
                }
            }
        }
    }

    out
}

fn add_gram_candidates(index: &RetrievalIndex, text: &str, out: &mut Candidates) {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 2 {
        return;
    }
    // 选最稀有 gram 缩小候选
    if chars.len() >= 3 {
        let mut best: Option<(&[DocId], (char, char, char))> = None;
        for w in chars.windows(3) {
            let g = (w[0], w[1], w[2]);
            if let Some(list) = index.gram3_postings(g) {
                if best.map(|(b, _)| list.len() < b.len()).unwrap_or(true) {
                    best = Some((list, g));
                }
            }
        }
        if let Some((list, _)) = best {
            for &id in list {
                out.ids.insert(id);
                out.stats.gram += 1;
            }
        }
    }
    // bigram 全覆盖预过滤由验证完成；这里取稀有 bigram 作锚
    let mut rare: Option<(&[DocId], (char, char))> = None;
    for w in chars.windows(2) {
        let g = (w[0], w[1]);
        if let Some(list) = index.gram2_postings(g) {
            if rare.map(|(b, _)| list.len() < b.len()).unwrap_or(true) {
                rare = Some((list, g));
            }
        }
    }
    if let Some((list, _)) = rare {
        for &id in list {
            out.ids.insert(id);
            out.stats.gram += 1;
        }
    }
}

fn add_pinyin_candidates(index: &RetrievalIndex, q: &ParsedQuery, out: &mut Candidates) {
    // 整串作为全拼/首字母词元
    if let Some(list) = index.postings(&q.raw_norm) {
        for &id in list {
            out.ids.insert(id);
            out.stats.pinyin += 1;
        }
    }
    for t in &q.tokens {
        if let Some(list) = index.postings(t) {
            for &id in list {
                out.ids.insert(id);
                out.stats.pinyin += 1;
            }
        }
    }
    // 混合全拼/简拼和多音字的所有读音在建索引时展开为字符倒排。
    // 这里只取可能包含输入字符的中文文档，最终仍由 ib-pinyin 验证。
    if q.chars.len() >= 2 {
        for &ch in &q.chars {
            if let Some(list) = index.pinyin_char_postings(ch) {
                for &id in list {
                    out.ids.insert(id);
                    out.stats.pinyin += 1;
                }
            }
        }
    }
}
