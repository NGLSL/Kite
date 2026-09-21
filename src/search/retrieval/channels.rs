//! 多通道候选生成：各通道独立召回，最后取并集。
//! 允许假阳性（验证阶段剔除）；不得因“已有普通结果”停止其他通道。
//!
//! `collect` 只做编排；单通道实现在各 `collect_*` 中，便于单独测试与观测。

use std::collections::HashSet;

use crate::search::alias;
use crate::search::matcher::UserTarget;

use super::doc::{DocId, RetrievalIndex};
use super::query::{ChannelStats, ParsedQuery};

/// 前缀词元枚举上限：足够覆盖同前缀词典，避免与相关性无关的硬截断漏召回。
/// 更短前缀主要靠 char/first_char/gram 倒排收窄，不依赖本枚举扫全库。
const MAX_PREFIX_TERMS: usize = 512;

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

    let seed_terms = seed_terms(q);

    collect_user_alias(index, user_targets, &mut out);
    collect_builtin_alias(index, q, &mut out);
    collect_token_exact_prefix(index, q, &seed_terms, &mut out);
    collect_compact(index, q, &mut out);
    collect_multi_token(index, q, &mut out);
    add_gram_candidates(index, &q.raw_norm, &mut out);
    collect_char_skip(index, q, &mut out);
    if q.has_ascii_alnum {
        add_pinyin_candidates(index, q, &mut out);
    }
    collect_mixed(index, q, &mut out);
    collect_symspell(index, q, &seed_terms, &mut out);
    collect_short_fallback(index, q, &mut out);

    out
}

fn seed_terms(q: &ParsedQuery) -> Vec<String> {
    let mut seed_terms: Vec<String> = q.tokens.clone();
    if seed_terms.is_empty() && !q.raw_norm.is_empty() {
        seed_terms.push(q.raw_norm.clone());
    }
    seed_terms
}

fn collect_user_alias(
    index: &RetrievalIndex,
    user_targets: &[UserTarget],
    out: &mut Candidates,
) {
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
}

fn collect_builtin_alias(index: &RetrievalIndex, q: &ParsedQuery, out: &mut Candidates) {
    let Some(fragments) = alias::targets_for(&q.raw_norm) else {
        return;
    };
    for frag in fragments {
        if let Some(list) = index.postings(frag) {
            for &id in list {
                out.ids.insert(id);
                out.stats.alias += 1;
            }
        }
        // 片段包含：用 gram 通道兜底
        add_gram_candidates(index, frag, out);
    }
}

fn collect_token_exact_prefix(
    index: &RetrievalIndex,
    q: &ParsedQuery,
    seed_terms: &[String],
    out: &mut Candidates,
) {
    let _ = q;
    for term in seed_terms {
        if let Some(list) = index.postings(term) {
            for &id in list {
                out.ids.insert(id);
                out.stats.exact += 1;
            }
        }
        // 前缀枚举：完整扩展（上限足够大），不依赖其他通道碰巧补回
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
}

fn collect_compact(index: &RetrievalIndex, q: &ParsedQuery, out: &mut Candidates) {
    // compact 精确/前缀（todo ↔ To Do）
    if q.compact.len() >= 2 {
        if let Some(list) = index.compact_postings(&q.compact) {
            for &id in list {
                out.ids.insert(id);
                out.stats.compact += 1;
            }
        }
        // compact 的前缀也走 gram 连续验证
        add_gram_candidates(index, &q.compact, out);
    }
}

fn collect_multi_token(index: &RetrievalIndex, q: &ParsedQuery, out: &mut Candidates) {
    // 多词：各词 postings 求交（最短优先），不因跨字段过早求交丢失——验证阶段再核
    if q.tokens.len() < 2 {
        return;
    }
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

fn collect_char_skip(index: &RetrievalIndex, q: &ParsedQuery, out: &mut Candidates) {
    // 字符跳字：仅对较长 Query 启用；用最稀有 Query 字符的倒排作锚
    // （3 字以下选择性差，交给精确/前缀/gram）
    if q.chars.len() < 4 {
        return;
    }
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

fn collect_mixed(index: &RetrievalIndex, q: &ParsedQuery, out: &mut Candidates) {
    // 混输：CJK 段按名称精确/包含锚定，拉丁段按拼音词元锚定
    if !q.mixed {
        return;
    }
    for part in &q.cjk_parts {
        if let Some(list) = index.postings(part) {
            for &id in list {
                out.ids.insert(id);
                out.stats.exact += 1;
            }
        }
        add_gram_candidates(index, part, out);
    }
    for part in &q.latin_parts {
        if let Some(list) = index.postings(part) {
            for &id in list {
                out.ids.insert(id);
                out.stats.pinyin += 1;
            }
        }
        for t in index.prefix_terms(part, MAX_PREFIX_TERMS) {
            if let Some(list) = index.postings(&t) {
                for &id in list {
                    out.ids.insert(id);
                    out.stats.pinyin += 1;
                }
            }
        }
    }
}

fn collect_symspell(
    index: &RetrievalIndex,
    q: &ParsedQuery,
    seed_terms: &[String],
    out: &mut Candidates,
) {
    // SymSpell 纠错：独立通道，不被其他必要条件删除；1 字符不纠错。
    // expand 返回的 dist 是「查询侧再删除的次数」，不是与原词的真实编辑距离；
    // 输入直接命中词典词的删除变体时 dist=0，必须保留（crome → chrome）。
    if q.chars.len() < 3 {
        return;
    }
    let max_d = crate::search::fuzzy::max_distance(q.raw_norm.len());
    if max_d == 0 {
        return;
    }
    for term in seed_terms {
        for (orig, _dist) in index.deletes().expand(term, max_d) {
            if let Some(list) = index.postings(&orig) {
                for &id in list {
                    out.ids.insert(id);
                    out.stats.symspell += 1;
                }
            }
        }
    }
    // 整串也查删除索引（chorme → chrome）
    for (orig, _dist) in index.deletes().expand(&q.raw_norm, max_d) {
        if let Some(list) = index.postings(&orig) {
            for &id in list {
                out.ids.insert(id);
                out.stats.symspell += 1;
            }
        }
    }
}

fn collect_short_fallback(index: &RetrievalIndex, q: &ParsedQuery, out: &mut Candidates) {
    // 成本模型：极短 Query 用字符倒排兜底（名称内部 / 词首）
    if q.chars.len() > 2 {
        return;
    }
    if let Some(&first) = q.chars.first() {
        // 1 字符：名称内部任意位置；2 字符：仍要内部片段，first_char 单独再补
        if let Some(list) = index.char_postings(first) {
            for &id in list {
                out.ids.insert(id);
                out.stats.scanned_fallback += 1;
            }
        }
        if q.chars.len() == 2 {
            if let Some(list) = index.first_char_postings(first) {
                for &id in list {
                    out.ids.insert(id);
                    out.stats.scanned_fallback += 1;
                }
            }
        }
    }
    // 拼音首字母任意位置：k → 控制面板（kzmb）；名称首字符是汉字时 first_char 盖不到
    if q.has_ascii_alnum {
        for doc in &index.docs {
            if doc.pinyin_initials.contains(&q.raw_norm) {
                out.ids.insert(doc.id);
                out.stats.pinyin += 1;
            }
        }
    }
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
    // 1 字符也走：拼音首字母任意位置需要中文文档进入候选。
    for &ch in &q.chars {
        if let Some(list) = index.pinyin_char_postings(ch) {
            for &id in list {
                out.ids.insert(id);
                out.stats.pinyin += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppItem;

    fn chrome_item() -> AppItem {
        let mut item = AppItem::scanned(
            "chrome".into(),
            "Google Chrome".into(),
            "C:\\fake\\chrome.exe".into(),
            None,
            None,
            "start-menu",
        );
        item.attach_search_fields();
        item
    }

    #[test]
    fn direct_delete_variant_is_not_dropped_by_symspell_channel() {
        // crome 是 chrome 的删除变体：查询侧删除深度为 0，不得被当成「非纠错」丢掉。
        let index = RetrievalIndex::build(&[chrome_item()], &[]);
        let q = super::super::query::parse("crome");
        let cand = collect(&index, &q, &[]);
        assert!(
            cand.stats.symspell > 0,
            "crome 应通过删除索引召回 chrome，stats={:?}",
            cand.stats
        );
        assert!(!cand.ids.is_empty());
    }
}
