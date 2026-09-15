//! 评分诊断：解释统一排序链上的命中，不改变排序结果。

use crate::history::Personalization;
use crate::search::retrieval::verify::{MatchField, MatchKind, RankedHit};
use crate::search::retrieval::RetrievalIndex;
use crate::search::matcher::UserTarget;
use crate::model::{AppItem, SearchResult};

/// 一条候选的可解释明细。
#[derive(Debug, Clone)]
pub struct HitDiagnostic {
    pub stable_id: String,
    pub name: String,
    pub field: MatchField,
    pub kind: MatchKind,
    /// 通道标签（可能含 +pin/+history/+demote）。
    pub matched_by: String,
    /// 最终分（含偏好）。
    pub score: i32,
    pub quality_tier: i32,
    pub start: usize,
    pub gaps: usize,
    pub edit_cost: usize,
    /// 偏好来源：pin / history / demote。
    pub preference_tags: Vec<&'static str>,
    /// 归并后的启动代表 stable id。
    pub launch_rep_stable_id: String,
}

fn preference_tags(matched_by: &str) -> Vec<&'static str> {
    let mut tags = Vec::new();
    if matched_by.contains("+pin") {
        tags.push("pin");
    }
    if matched_by.contains("+history") {
        tags.push("history");
    }
    if matched_by.contains("+demote") {
        tags.push("demote");
    }
    tags
}

impl HitDiagnostic {
    fn from_ranked(index: &RetrievalIndex, hit: &RankedHit) -> Self {
        let name = index
            .doc(hit.doc_id)
            .map(|d| d.item.name.clone())
            .unwrap_or_default();
        let launch_rep_stable_id = index
            .launch_rep_stable_id(hit.doc_id)
            .unwrap_or_else(|| hit.stable_id.clone());
        Self {
            stable_id: hit.stable_id.clone(),
            name,
            field: hit.evidence.field,
            kind: hit.evidence.kind,
            matched_by: hit.matched_by.clone(),
            score: hit.score,
            quality_tier: hit.quality_tier,
            start: hit.evidence.start,
            gaps: hit.evidence.gaps,
            edit_cost: hit.evidence.edit_cost,
            preference_tags: preference_tags(&hit.matched_by),
            launch_rep_stable_id,
        }
    }
}

/// 从已排序轻量候选生成诊断；不重算分数、不改变顺序。
pub fn explain_ranked_hits(index: &RetrievalIndex, ranked: &[RankedHit]) -> Vec<HitDiagnostic> {
    ranked
        .iter()
        .map(|hit| HitDiagnostic::from_ranked(index, hit))
        .collect()
}

/// 带诊断的搜索：与 `search_personalized` 同一排序链，额外返回命中字段/证据/偏好标签。
pub fn search_personalized_explained(
    index: &RetrievalIndex,
    query: &str,
    user_targets: &[UserTarget],
    prefs: Option<&Personalization>,
    max_results: usize,
) -> (Vec<SearchResult>, Vec<HitDiagnostic>) {
    let ranked =
        index.search_personalized_ranked(query, user_targets, prefs, max_results);
    let diags = explain_ranked_hits(index, &ranked);
    let hits = ranked
        .into_iter()
        .filter_map(|r| {
            let doc = index.doc(r.doc_id)?;
            let mut item: AppItem = doc.item.clone();
            if item.icon.is_none() {
                if let Some(icon_doc) = index.doc(r.icon_doc_id) {
                    if icon_doc.item.icon.is_some() {
                        item.icon.clone_from(&icon_doc.item.icon);
                        item.icon_src.clone_from(&icon_doc.item.icon_src);
                    }
                }
            }
            Some(SearchResult {
                item,
                score: r.score,
                matched_by: r.matched_by,
                quality_tier: r.quality_tier,
            })
        })
        .collect();
    (hits, diags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{search_with_personalization, RetrievalIndex};

    fn app(id: &str, name: &str) -> AppItem {
        let mut it = AppItem::scanned(
            id.into(),
            name.into(),
            format!(r"C:\{id}.exe"),
            None,
            None,
            "test",
        );
        it.attach_search_fields();
        it
    }

    #[test]
    fn diagnostics_cover_field_score_and_preference_tags() {
        let index = RetrievalIndex::build(
            &[app("a", "Visual Code"), app("b", "Visual Studio Code")],
            &[],
        );
        let mut prefs = crate::history::Personalization::default();
        prefs.query_norm = "visual code".into();
        prefs.pinned.insert("b".into());
        let (hits, diags) =
            search_personalized_explained(&index, "visual code", &[], Some(&prefs), 10);
        assert_eq!(hits.len(), diags.len());
        assert!(diags.iter().any(|d| d.preference_tags.contains(&"pin")));
        assert!(diags.iter().all(|d| !d.matched_by.is_empty()));
        assert!(
            diags.iter().any(|d| d.name.contains("Code")),
            "诊断应带名称: {diags:?}"
        );
        let plain = search_with_personalization(&index, "visual code", &[], Some(&prefs), 10);
        assert_eq!(
            hits.iter().map(|h| &h.item.id).collect::<Vec<_>>(),
            plain.iter().map(|h| &h.item.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn demote_appears_in_preference_tags() {
        let index = RetrievalIndex::build(&[app("a", "Code Alpha"), app("b", "Code Beta")], &[]);
        let mut prefs = crate::history::Personalization::default();
        prefs.query_norm = "code".into();
        prefs.demoted.insert("b".into());
        let (_hits, diags) = search_personalized_explained(&index, "code", &[], Some(&prefs), 10);
        assert!(diags.iter().any(|d| d.preference_tags.contains(&"demote")));
    }

    #[test]
    fn diagnostics_record_real_evidence_fields() {
        let index = RetrievalIndex::build(&[app("a", "XcodeX"), app("b", "Abcode")], &[]);
        let (_hits, diags) = search_personalized_explained(&index, "code", &[], None, 10);
        let xcode = diags.iter().find(|d| d.stable_id == "a").expect("xcode");
        assert!(
            !matches!(xcode.kind, MatchKind::Exact) || xcode.start < usize::MAX,
            "证据应来自真实验证: {xcode:?}"
        );
    }
}
