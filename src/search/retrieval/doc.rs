//! 索引文档与倒排结构。快照构建时一次性生成，查询只读。

use std::collections::{HashMap, HashSet};

use fst::{IntoStreamer, Map as FstMap, MapBuilder, Streamer};

use crate::model::AppItem;
use crate::search::matcher::UserTarget;
use crate::search::normalizer::{compact, split_camel, tokens};
use crate::search::pinyin_of;

use super::symspell::{self, DeleteIndex};
use super::verify::{cmp_ranked_hit, CharBits, QueryContext, RankedHit, ScoredHit};

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
        self.search_personalized(query, user_targets, None, max_results)
    }

    /// 统一最终排序入口：验证后在完整候选上应用个性化，再一次截断。
    /// 证据与稳定 id 保留到截断后再物化展示对象。
    /// `personalization` 为 None 时行为与仅按 MatchScore 排序一致（同分仍比证据）。
    pub fn search_personalized(
        &self,
        query: &str,
        user_targets: &[UserTarget],
        personalization: Option<&crate::history::Personalization>,
        max_results: usize,
    ) -> Vec<crate::model::SearchResult> {
        let q = super::query::parse(query);
        if q.is_empty() {
            return Vec::new();
        }
        let mut ctx = QueryContext::build(&q, self);
        let candidates = super::channels::collect(self, &q, user_targets);
        let scored = super::verify::verify_all(self, &candidates, &mut ctx, user_targets);
        let mut ranked = self.into_ranked(scored);
        if let Some(prefs) = personalization {
            self.apply_personalization_boosts(&mut ranked, prefs);
        }
        let mut ranked = self.group_same_install_family(self.group_by_launch_identity(ranked));
        ranked.sort_by(cmp_ranked_hit);
        ranked.truncate(max_results);
        ranked
            .into_iter()
            .filter_map(|r| {
                let doc = self.doc(r.doc_id)?;
                let mut item = doc.item.clone();
                // 启动身份保持友好入口；图标可从同组另一入口借用
                if item.icon.is_none() {
                    if let Some(icon_doc) = self.doc(r.icon_doc_id) {
                        if icon_doc.item.icon.is_some() || icon_doc.item.icon_src.is_some() {
                            item.icon.clone_from(&icon_doc.item.icon);
                            item.icon_src.clone_from(&icon_doc.item.icon_src);
                        }
                    }
                }
                Some(crate::model::SearchResult {
                    item,
                    score: r.score,
                    matched_by: r.matched_by,
                    quality_tier: r.quality_tier,
                })
            })
            .collect()
    }

    fn into_ranked(&self, scored: Vec<ScoredHit>) -> Vec<RankedHit> {
        scored
            .into_iter()
            .filter_map(|s| {
                let doc = self.doc(s.doc_id)?;
                Some(RankedHit {
                    doc_id: s.doc_id,
                    quality_tier: crate::history::quality_tier(s.score),
                    score: s.score,
                    matched_by: s.matched_by,
                    evidence: s.evidence,
                    name_len: doc.item.name.len(),
                    name_lower: doc.item.name.to_lowercase(),
                    stable_id: doc.item.id.clone(),
                    source: doc.item.source.clone(),
                    has_icon: doc.item.icon.is_some() || doc.item.icon_src.is_some(),
                    icon_doc_id: s.doc_id,
                    has_working_dir: doc.item.working_dir.is_some(),
                })
            })
            .collect()
    }

    /// 只写偏好信息与最终分；排序交给统一比较器，不在个性化阶段分叉。
    fn apply_personalization_boosts(&self, ranked: &mut [RankedHit], prefs: &crate::history::Personalization) {
        use crate::history::{history_boost, PIN_BOOST};
        for hit in ranked.iter_mut() {
            let Some(doc) = self.doc(hit.doc_id) else {
                continue;
            };
            let id = &doc.item.id;
            let base = hit.score;
            let is_pinned = prefs.pinned.contains(id);
            let usage = prefs.usage.get(id).cloned().unwrap_or_default();
            let pair = prefs.pairs.get(id).cloned().unwrap_or_default();
            let history = history_boost(&prefs.query_norm, &usage, &pair, prefs.now);
            let boost = if is_pinned {
                PIN_BOOST.max(history)
            } else {
                history
            };
            hit.score = base + boost;
            if is_pinned {
                hit.matched_by = format!("{}+pin", hit.matched_by);
            } else if boost > 0 {
                hit.matched_by = format!("{}+history", hit.matched_by);
            }
        }
    }

    /// 同一启动身份（规范 exe/AUMID + args）只展示一条：
    /// 保留分最高的匹配证据与最终分，代表入口优先友好来源。
    fn group_by_launch_identity(&self, ranked: Vec<RankedHit>) -> Vec<RankedHit> {
        use crate::app::scanner::util::launch_identity;
        use std::collections::HashMap;

        let mut groups: HashMap<String, RankedHit> = HashMap::new();
        for hit in ranked {
            let Some(doc) = self.doc(hit.doc_id) else {
                continue;
            };
            let key = launch_identity(&doc.item.target, doc.item.args.as_deref());
            match groups.entry(key) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(hit);
                }
                std::collections::hash_map::Entry::Occupied(mut slot) => {
                    let current = slot.get().clone();
                    let (strong, weak) = if hit.score >= current.score {
                        (hit, current)
                    } else {
                        (current, hit)
                    };
                    let rep = merge_display_rep(strong, weak);
                    crate::log::info(&format!(
                        "launch-group: keep={}({}) icon={} score={}",
                        rep.stable_id, rep.source, rep.has_icon, rep.score
                    ));
                    slot.insert(rep);
                }
            }
        }
        groups.into_values().collect()
    }

    /// 同产品多入口只展示一条：同安装根（可不同 args）或 AppsFolder 包 + 桌面/开始菜单同名。
    fn group_same_install_family(&self, ranked: Vec<RankedHit>) -> Vec<RankedHit> {
        use crate::app::scanner::util::{install_root_dir, names_share_install_family};

        fn is_shell_package(target: &str) -> bool {
            let t = target.trim();
            t.len() >= 16
                && t.is_char_boundary(16)
                && t[..16].eq_ignore_ascii_case("shell:appsfolder")
        }

        fn is_friendly(source: &str) -> bool {
            matches!(source, "start-menu" | "desktop" | "portable")
        }

        fn should_merge(a: &AppItem, b: &AppItem) -> bool {
            use crate::app::scanner::util::normalize_path_key;
            let same_name = names_share_install_family(&a.name, &b.name);
            let stem_link = exe_stem_matches_name(a, b) || exe_stem_matches_name(b, a);
            if !same_name && !stem_link {
                return false;
            }
            // 同一 exe 路径：args 可不同（WPS /from=startmenu vs /from=desktop_shortcut）
            let ta = normalize_path_key(a.target.trim());
            let tb = normalize_path_key(b.target.trim());
            if ta == tb && !ta.is_empty() {
                return true;
            }
            let root_a = install_root_dir(&a.target);
            let root_b = install_root_dir(&b.target);
            match (root_a, root_b) {
                (Some(x), Some(y)) => x == y,
                (None, Some(_)) | (Some(_), None) => {
                    // AppsFolder/AUMID 无路径安装根：与友好入口同名则视为同一产品
                    let shell = is_shell_package(&a.target) || is_shell_package(&b.target);
                    let pair_ok = (is_friendly(&a.source) || is_friendly(&b.source))
                        || a.source == "app-paths"
                        || b.source == "app-paths";
                    same_name && shell && pair_ok
                }
                (None, None) => same_name && is_shell_package(&a.target) && is_shell_package(&b.target),
            }
        }

        let mut kept: Vec<RankedHit> = Vec::with_capacity(ranked.len());
        for hit in ranked {
            let Some(doc) = self.doc(hit.doc_id) else {
                continue;
            };
            if doc.item.source == "uninstall" {
                // 与同族主程序归并；启动代表由 merge_display_rep 优先友好来源，不会落到 uninst.exe
                let mut merged = false;
                for slot in kept.iter_mut() {
                    let Some(sdoc) = self.doc(slot.doc_id) else {
                        continue;
                    };
                    if sdoc.item.source == "uninstall" {
                        continue;
                    }
                    if !should_merge(&sdoc.item, &doc.item) {
                        continue;
                    }
                    let (strong, weak) = if hit.score >= slot.score {
                        (hit.clone(), slot.clone())
                    } else {
                        (slot.clone(), hit.clone())
                    };
                    let rep = merge_display_rep(strong, weak);
                    *slot = rep;
                    merged = true;
                    break;
                }
                if !merged {
                    kept.push(hit);
                }
                continue;
            }
            let mut merged = false;
            for slot in kept.iter_mut() {
                let Some(sdoc) = self.doc(slot.doc_id) else {
                    continue;
                };
                if sdoc.item.source == "uninstall" {
                    // 主应用后到：挤掉同族卸载项
                    continue;
                }
                if !should_merge(&sdoc.item, &doc.item) {
                    continue;
                }
                let (strong, weak) = if hit.score >= slot.score {
                    (hit.clone(), slot.clone())
                } else {
                    (slot.clone(), hit.clone())
                };
                let rep = merge_display_rep(strong, weak);
                // 卸载项不得成为启动代表
                let rep = if rep.source == "uninstall" && slot.source != "uninstall" {
                    let mut r = slot.clone();
                    r.score = rep.score;
                    r.quality_tier = rep.quality_tier;
                    r.matched_by = rep.matched_by;
                    r.evidence = rep.evidence;
                    if !r.has_icon && rep.has_icon {
                        r.icon_doc_id = rep.icon_doc_id;
                        r.has_icon = true;
                    }
                    r
                } else {
                    rep
                };
                crate::log::info(&format!(
                    "install-family: keep={}({}) icon={} score={} from={}+{}",
                    rep.stable_id,
                    rep.source,
                    rep.has_icon,
                    rep.score,
                    sdoc.item.source,
                    doc.item.source
                ));
                *slot = rep;
                merged = true;
                break;
            }
            if !merged {
                // 新可启动应用进入时，清掉同名族的卸载占位
                kept.retain(|s| {
                    self.doc(s.doc_id).is_none_or(|d| {
                        !(d.item.source == "uninstall"
                            && names_share_install_family(&d.item.name, &doc.item.name))
                    })
                });
                kept.push(hit);
            }
        }
        kept
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

fn launch_source_priority(source: &str) -> u8 {
    match source {
        "start-menu" => 0,
        "desktop" => 1,
        "portable" => 2,
        "uninstall" => 3,
        "app-paths" => 4,
        "uwp" | "apps-folder" => 5,
        "commands" => 6,
        _ => 7,
    }
}

/// 相关性取 strong；启动代表优先友好来源（保留 lnk 的 target/args/working_dir）；
/// 展示图标可从组内带图标的入口借用。
fn merge_display_rep(strong: RankedHit, weak: RankedHit) -> RankedHit {
    // 启动代表：来源友好 → 带 working_dir 的 lnk → 带图标
    let launch_key = |h: &RankedHit| {
        (
            launch_source_priority(&h.source),
            if h.has_working_dir { 0u8 } else { 1u8 },
            if h.has_icon { 0u8 } else { 1u8 },
        )
    };
    let launch = if launch_key(&weak) < launch_key(&strong) {
        weak.clone()
    } else {
        strong.clone()
    };
    let other = if launch.doc_id == strong.doc_id {
        weak.clone()
    } else {
        strong.clone()
    };
    let mut rep = launch.clone();
    rep.score = strong.score;
    rep.quality_tier = strong.quality_tier;
    rep.matched_by = strong.matched_by;
    rep.evidence = strong.evidence;
    if !rep.has_icon && other.has_icon {
        rep.icon_doc_id = other.doc_id;
        rep.has_icon = true;
    }
    rep
}

/// exe 文件名（去扩展）与另一条展示名一致：app-paths 的 `wps` ↔ 快捷方式 `WPS Office` 用。
fn exe_stem_matches_name(item: &AppItem, other: &AppItem) -> bool {
    use std::path::Path;
    let body = item
        .target
        .trim()
        .strip_prefix("shell:AppsFolder\\")
        .or_else(|| item.target.trim().strip_prefix("shell:appsfolder\\"))
        .unwrap_or(item.target.trim());
    let Some(stem) = Path::new(body)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
    else {
        return false;
    };
    if stem.len() < 3 || stem.contains(' ') {
        return false;
    }
    let family = crate::app::scanner::util::display_family_key(&other.name);
    family == stem || family.starts_with(&format!("{stem} "))
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
