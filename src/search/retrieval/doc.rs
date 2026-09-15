//! 索引文档与倒排结构。快照构建时一次性生成，查询只读。

use std::collections::{HashMap, HashSet};

use fst::{IntoStreamer, Map as FstMap, MapBuilder, Streamer};

use crate::model::AppItem;
use crate::search::matcher::UserTarget;
use crate::search::normalizer::{compact, split_camel, tokens};
use crate::search::pinyin_of;

use super::symspell::{self, DeleteIndex};
use super::verify::{CharBits, QueryContext};

/// 内部文档 ID（快照局部，不持久化）。
pub type DocId = u32;

/// 一条可搜索文档：应用或系统入口。
#[derive(Debug, Clone)]
pub struct IndexedDoc {
    pub id: DocId,
    pub item: AppItem,
    pub name: String,
    pub display: String,
    pub compact_name: String,
    pub compact_display: String,
    pub tokens: Vec<String>,
    pub acronym: String,
    pub pinyin: String,
    pub pinyin_initials: String,
    pub pinyin_syllables: Vec<String>,
    pub keywords: Vec<String>,
    pub keyword_compacts: Vec<String>,
    pub keyword_bits: Vec<CharBits>,
    pub keyword_full_pinyin: Vec<String>,
    pub keyword_initials: Vec<String>,
    pub context_fields: Vec<String>,
    pub context_terms: Vec<String>,
    pub context_compacts: Vec<String>,
    pub context_full_pinyin: Vec<String>,
    pub context_initials: Vec<String>,
    pub name_bits: CharBits,
    pub display_bits: CharBits,
}

impl IndexedDoc {
    fn from_item(id: DocId, item: AppItem, extra_keywords: Vec<String>) -> Self {
        let name = if item.normalized_name.is_empty() {
            crate::search::normalizer::normalize_name(&item.name)
        } else {
            item.normalized_name.clone()
        };
        let display = if item.normalized_display.is_empty() {
            crate::search::normalizer::normalize_name(&item.display_name)
        } else {
            item.normalized_display.clone()
        };
        let mut token_set: Vec<String> = tokens(&name).into_iter().map(str::to_string).collect();
        for raw in [&item.name, &item.display_name] {
            for part in split_camel(raw) {
                if !token_set.contains(&part) {
                    token_set.push(part);
                }
            }
        }
        let mut keywords = Vec::new();
        let mut keyword_full_pinyin = Vec::new();
        let mut keyword_initials = Vec::new();
        for raw in &extra_keywords {
            let keyword = crate::search::normalizer::normalize_name(raw);
            if keyword.is_empty() {
                continue;
            }
            if !keywords.contains(&keyword) {
                keywords.push(keyword.clone());
            }
            for token in tokens(&keyword) {
                push_unique(&mut token_set, token.to_string());
            }
            for part in split_camel(raw) {
                push_unique(&mut token_set, part);
            }
            let (full, initials) = pinyin_of(raw);
            push_unique_nonempty(&mut keyword_full_pinyin, full.clone());
            push_unique_nonempty(&mut keyword_initials, initials.clone());
            push_unique_nonempty(&mut token_set, full);
            push_unique_nonempty(&mut token_set, initials);
        }
        let mut context_fields = Vec::new();
        let mut context_terms = Vec::new();
        let mut context_compacts = Vec::new();
        let mut context_full_pinyin = Vec::new();
        let mut context_initials = Vec::new();
        for raw in &item.search_context {
            let field = crate::search::normalizer::normalize_name(raw);
            if field.is_empty() {
                continue;
            }
            push_unique(&mut context_fields, field.clone());
            push_unique(&mut context_terms, field.clone());
            push_unique_nonempty(&mut context_compacts, compact(&field));
            for token in tokens(&field) {
                push_unique(&mut context_terms, token.to_string());
            }
            for part in split_camel(raw) {
                push_unique(&mut context_terms, part);
            }
            let (full, initials) = pinyin_of(raw);
            push_unique_nonempty(&mut context_full_pinyin, full);
            push_unique_nonempty(&mut context_initials, initials);
        }
        let acronym = word_acronym(&name);
        let pinyin = if item.pinyin.is_empty() {
            pinyin_of(&item.display_name).0
        } else {
            item.pinyin.clone()
        };
        let pinyin_initials = if item.pinyin_initials.is_empty() {
            pinyin_of(&item.display_name).1
        } else {
            item.pinyin_initials.clone()
        };
        let pinyin_syllables = crate::search::pinyin::syllables(&item.display_name);
        Self {
            name_bits: CharBits::from_str(&name),
            display_bits: CharBits::from_str(&display),
            compact_name: compact(&name),
            compact_display: compact(&display),
            id,
            item,
            name,
            display,
            tokens: token_set,
            acronym,
            pinyin,
            pinyin_initials,
            pinyin_syllables,
            keyword_compacts: keywords.iter().map(|k| compact(k)).collect(),
            keyword_bits: keywords.iter().map(|k| CharBits::from_str(k)).collect(),
            keyword_full_pinyin,
            keyword_initials,
            keywords,
            context_fields,
            context_terms,
            context_compacts,
            context_full_pinyin,
            context_initials,
        }
    }

    /// 参与倒排的全部规范化词元（名称、显示名、token、关键词、拼音、缩写）。
    fn all_terms(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        if !self.name.is_empty() {
            out.push(&self.name);
        }
        if !self.display.is_empty() {
            out.push(&self.display);
        }
        out.extend(
            self.tokens
                .iter()
                .filter(|t| !t.is_empty())
                .map(|s| s.as_str()),
        );
        out.extend(
            self.keywords
                .iter()
                .filter(|k| !k.is_empty())
                .map(|s| s.as_str()),
        );
        out.extend(
            self.keyword_full_pinyin
                .iter()
                .chain(self.keyword_initials.iter())
                .filter(|term| !term.is_empty())
                .map(String::as_str),
        );
        out.extend(
            self.context_terms
                .iter()
                .chain(self.context_compacts.iter())
                .chain(self.context_full_pinyin.iter())
                .chain(self.context_initials.iter())
                .filter(|term| !term.is_empty())
                .map(String::as_str),
        );
        if !self.pinyin.is_empty() {
            out.push(&self.pinyin);
        }
        if !self.pinyin_initials.is_empty() {
            out.push(&self.pinyin_initials);
        }
        if !self.acronym.is_empty() {
            out.push(&self.acronym);
        }
        out
    }
}

/// 同代只读检索索引。
#[derive(Debug)]
pub struct RetrievalIndex {
    pub docs: Vec<IndexedDoc>,
    /// 词元 → 文档 ID（排序去重）。
    term_postings: HashMap<String, Vec<DocId>>,
    /// FST 词典：规范化词元 → 词元序号（快照构建时整体生成，支持精确/前缀枚举）。
    term_fst: FstMap<Vec<u8>>,
    /// compact 精确/前缀。
    compact_postings: HashMap<String, Vec<DocId>>,
    /// Unicode 字符 2-gram → 文档。
    gram2: HashMap<(char, char), Vec<DocId>>,
    /// Unicode 字符 3-gram → 文档。
    gram3: HashMap<(char, char, char), Vec<DocId>>,
    /// SymSpell 删除索引：删除变体 → 原词元。
    deletes: DeleteIndex,
    /// 稳定 ID → 文档 ID。
    by_stable_id: HashMap<String, DocId>,
    /// 名称/显示名首字符 → 文档（短 Query 前缀锚点）。
    first_char: HashMap<char, Vec<DocId>>,
    /// 名称/显示名任意字符 → 文档（跳字稀有字符锚点）。
    char_postings: HashMap<char, Vec<DocId>>,
    /// All readings of CJK characters (and literal ASCII) in mixed names.
    pinyin_char_postings: HashMap<char, Vec<DocId>>,
}

impl RetrievalIndex {
    /// 从应用 + 系统入口快照构建。`system_entries` 中与应用同 ID 的条目会被跳过。
    pub fn build(apps: &[AppItem], system_entries: &[AppItem]) -> Self {
        let mut docs = Vec::with_capacity(apps.len() + system_entries.len());
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut next_id = 0u32;

        let mut push_item = |item: &AppItem, keywords: Vec<String>, docs: &mut Vec<IndexedDoc>| {
            if !seen_ids.insert(item.id.clone()) {
                return;
            }
            let doc = IndexedDoc::from_item(next_id, item.clone(), keywords);
            next_id += 1;
            docs.push(doc);
        };

        for item in apps {
            push_item(item, item_extra_keywords(item), &mut docs);
        }
        for item in system_entries {
            push_item(item, item_extra_keywords(item), &mut docs);
        }

        let mut term_postings: HashMap<String, Vec<DocId>> = HashMap::new();
        let mut compact_postings: HashMap<String, Vec<DocId>> = HashMap::new();
        let mut gram2: HashMap<(char, char), Vec<DocId>> = HashMap::new();
        let mut gram3: HashMap<(char, char, char), Vec<DocId>> = HashMap::new();
        let mut by_stable_id: HashMap<String, DocId> = HashMap::new();
        let mut term_set: HashSet<String> = HashSet::new();
        let mut first_char: HashMap<char, Vec<DocId>> = HashMap::new();
        let mut char_postings: HashMap<char, Vec<DocId>> = HashMap::new();
        let mut pinyin_char_postings: HashMap<char, Vec<DocId>> = HashMap::new();
        let pinyin_data =
            ib_pinyin::pinyin::PinyinData::new(ib_pinyin::pinyin::PinyinNotation::Ascii);

        for doc in &docs {
            by_stable_id.insert(doc.item.id.clone(), doc.id);
            for term in doc.all_terms() {
                term_set.insert(term.to_string());
                term_postings
                    .entry(term.to_string())
                    .or_default()
                    .push(doc.id);
            }
            for c in std::iter::once(&doc.compact_name)
                .chain(std::iter::once(&doc.compact_display))
                .chain(doc.keyword_compacts.iter())
                .chain(doc.context_compacts.iter())
            {
                if !c.is_empty() {
                    compact_postings.entry(c.clone()).or_default().push(doc.id);
                    term_set.insert(c.clone());
                    term_postings.entry(c.clone()).or_default().push(doc.id);
                }
            }
            for field in std::iter::once(&doc.name)
                .chain(std::iter::once(&doc.display))
                .chain(doc.keywords.iter())
                .chain(doc.keyword_full_pinyin.iter())
                .chain(doc.keyword_initials.iter())
                .chain(doc.context_terms.iter())
                .chain(doc.context_full_pinyin.iter())
                .chain(doc.context_initials.iter())
            {
                index_grams(field, doc.id, &mut gram2, &mut gram3);
                let mut seen_chars: HashSet<char> = HashSet::new();
                for (i, ch) in field.chars().enumerate() {
                    if i == 0 {
                        first_char.entry(ch).or_default().push(doc.id);
                    }
                    if seen_chars.insert(ch) {
                        char_postings.entry(ch).or_default().push(doc.id);
                    }
                }
            }
            if doc.item.display_name.chars().any(is_cjk) || doc.item.name.chars().any(is_cjk) {
                let mut reading_chars = HashSet::new();
                for ch in doc.item.display_name.chars().chain(doc.item.name.chars()) {
                    if ch.is_ascii_alphabetic() {
                        reading_chars.insert(ch.to_ascii_lowercase());
                    }
                    pinyin_data.get_pinyins_and_for_each(ch, |reading| {
                        if let Some(ascii) =
                            reading.notation(ib_pinyin::pinyin::PinyinNotation::Ascii)
                        {
                            reading_chars.extend(ascii.chars());
                        }
                    });
                }
                for ch in reading_chars {
                    pinyin_char_postings.entry(ch).or_default().push(doc.id);
                }
            }
        }

        for list in term_postings.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        for list in compact_postings.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        for list in gram2.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        for list in gram3.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        for list in first_char.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        for list in char_postings.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        for list in pinyin_char_postings.values_mut() {
            list.sort_unstable();
            list.dedup();
        }

        let deletes = symspell::build(&term_set, 2);

        // FST 词典：字典序插入（值暂存序号，便于日后挂倒排地址）
        let mut term_list: Vec<String> = term_set.into_iter().collect();
        term_list.sort_unstable();
        let mut builder = MapBuilder::memory();
        for (i, term) in term_list.iter().enumerate() {
            builder
                .insert(term.as_bytes(), i as u64)
                .expect("fst insert sorted term");
        }
        let term_fst = builder.into_map();

        Self {
            docs,
            term_postings,
            term_fst,
            compact_postings,
            gram2,
            gram3,
            deletes,
            by_stable_id,
            first_char,
            char_postings,
            pinyin_char_postings,
        }
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    pub fn doc(&self, id: DocId) -> Option<&IndexedDoc> {
        self.docs.get(id as usize)
    }

    pub fn lookup_stable(&self, stable_id: &str) -> Option<DocId> {
        self.by_stable_id.get(stable_id).copied()
    }

    pub fn postings(&self, term: &str) -> Option<&[DocId]> {
        self.term_postings.get(term).map(|v| v.as_slice())
    }

    pub fn compact_postings(&self, term: &str) -> Option<&[DocId]> {
        self.compact_postings.get(term).map(|v| v.as_slice())
    }

    /// FST 前缀枚举：返回前缀命中的词元（有序，最多 limit 条）。
    pub fn prefix_terms(&self, prefix: &str, limit: usize) -> Vec<String> {
        if prefix.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut stream = self.term_fst.range().ge(prefix.as_bytes()).into_stream();
        while let Some((key, _idx)) = stream.next() {
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            if let Ok(s) = std::str::from_utf8(key) {
                out.push(s.to_string());
            }
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    pub fn gram2_postings(&self, g: (char, char)) -> Option<&[DocId]> {
        self.gram2.get(&g).map(|v| v.as_slice())
    }

    pub fn gram3_postings(&self, g: (char, char, char)) -> Option<&[DocId]> {
        self.gram3.get(&g).map(|v| v.as_slice())
    }

    pub fn deletes(&self) -> &DeleteIndex {
        &self.deletes
    }

    pub fn first_char_postings(&self, c: char) -> Option<&[DocId]> {
        self.first_char.get(&c).map(|v| v.as_slice())
    }

    pub fn char_postings(&self, c: char) -> Option<&[DocId]> {
        self.char_postings.get(&c).map(|v| v.as_slice())
    }

    pub fn pinyin_char_postings(&self, c: char) -> Option<&[DocId]> {
        self.pinyin_char_postings.get(&c).map(|v| v.as_slice())
    }

    /// 非空 Query 搜索：多路召回 → 验证 → 统一排序 → 物化 Top K。
    /// 友好入口折扣可能下调 app-paths，因此物化窗口为 cutoff 分以上（含折扣余量）。
    pub fn search(
        &self,
        query: &str,
        user_targets: &[UserTarget],
        max_results: usize,
    ) -> Vec<crate::model::SearchResult> {
        let q = super::query::parse(query);
        if q.is_empty() {
            return Vec::new();
        }
        let mut ctx = QueryContext::build(&q, self);
        let candidates = super::channels::collect(self, &q, user_targets);
        let mut scored = super::verify::verify_all(self, &candidates, &mut ctx, user_targets);
        scored.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.doc_id.cmp(&b.doc_id)));
        // 排序前不物化全部候选：只克隆可能进入 Top K 的窗口
        const FRIENDLY_DISCOUNT_SLACK: i32 = 220;
        let threshold = scored
            .get(max_results.saturating_sub(1))
            .map(|s| s.score)
            .unwrap_or(i32::MIN);
        let mut results: Vec<crate::model::SearchResult> = scored
            .into_iter()
            .filter(|s| s.score >= threshold.saturating_sub(FRIENDLY_DISCOUNT_SLACK))
            .filter_map(|s| {
                let doc = self.doc(s.doc_id)?;
                Some(crate::model::SearchResult {
                    item: doc.item.clone(),
                    score: s.score,
                    matched_by: s.matched_by,
                })
            })
            .collect();
        crate::search::ranker::prefer_friendly_install_entries(&mut results);
        crate::search::ranker::rank_and_truncate(results, max_results)
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn push_unique_nonempty(values: &mut Vec<String>, value: String) {
    if !value.is_empty() {
        push_unique(values, value);
    }
}

fn item_extra_keywords(item: &AppItem) -> Vec<String> {
    let mut kws = item.search_keywords.clone();
    if item.source == "win-settings" || item.source == "builtin-system" {
        if let Some(rest) = item.target.strip_prefix("ms-settings:") {
            kws.push(rest.replace('-', " "));
        }
        if item.id.starts_with("system-tool:") {
            kws.push(item.id.trim_start_matches("system-tool:").replace('-', " "));
        }
    }
    kws
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x4e00..=0x9fff | 0x3400..=0x4dbf)
}

fn index_grams(
    field: &str,
    id: DocId,
    gram2: &mut HashMap<(char, char), Vec<DocId>>,
    gram3: &mut HashMap<(char, char, char), Vec<DocId>>,
) {
    let chars: Vec<char> = field.chars().collect();
    if chars.len() >= 2 {
        for w in chars.windows(2) {
            gram2.entry((w[0], w[1])).or_default().push(id);
        }
    }
    if chars.len() >= 3 {
        for w in chars.windows(3) {
            gram3.entry((w[0], w[1], w[2])).or_default().push(id);
        }
    }
}

fn word_acronym(name: &str) -> String {
    let toks = tokens(name);
    if toks.len() < 2 {
        return String::new();
    }
    toks.iter().filter_map(|t| t.chars().next()).collect()
}
