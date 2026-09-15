//! 固定 Query 评估：加载 `tests/search_cases.json`，输出 Top1 / Recall@5 / MRR 与缺口。
//!
//! 运行：`cargo test search_eval -- --nocapture`
//! `known-gap` 样本计入报告但不在 CI 硬失败；票 07/08/09 合入后应改为 `required`。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use serde::Deserialize;

use crate::model::AppItem;
use crate::search::{search, TOP_N};

#[derive(Debug, Deserialize)]
struct CasesFile {
    fixtures: HashMap<String, Fixture>,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    apps: Vec<FixtureApp>,
}

#[derive(Debug, Deserialize, Clone)]
struct FixtureApp {
    id: String,
    name: String,
    target: String,
    #[serde(default)]
    args: Option<String>,
    source: String,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    query: String,
    fixture: String,
    expected_top: Option<String>,
    #[serde(default)]
    expected_in_top5: Vec<String>,
    #[serde(default)]
    forbidden_top: Vec<String>,
    #[serde(default)]
    status: Status,
    #[serde(default)]
    gap_ticket: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Status {
    #[default]
    Required,
    KnownGap,
}

#[derive(Clone)]
struct CaseOutcome {
    id: String,
    query: String,
    top1_hit: bool,
    recall5: f64,
    reciprocal_rank: f64,
    forbidden_violated: bool,
    status: Status,
    gap_ticket: Option<String>,
    top_names: Vec<String>,
    latency_us: u128,
}

fn cases_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/search_cases.json")
}

fn build_index(fixture: &Fixture) -> Vec<AppItem> {
    fixture
        .apps
        .iter()
        .map(|a| {
            let mut item = AppItem::scanned(
                a.id.clone(),
                a.name.clone(),
                a.target.clone(),
                a.args.clone(),
                None,
                a.source.clone(),
            );
            item.attach_search_fields();
            item
        })
        .collect()
}

fn evaluate_case(apps: &[AppItem], case: &Case) -> CaseOutcome {
    let t0 = Instant::now();
    let hits = search(apps, &case.query, &[], TOP_N);
    let latency_us = t0.elapsed().as_micros();
    let ids: Vec<&str> = hits.iter().map(|h| h.item.id.as_str()).collect();
    let top_names: Vec<String> = hits.iter().map(|h| h.item.display_name.clone()).collect();
    let top = ids.first().copied();

    let top1_hit = match &case.expected_top {
        Some(expected) => top == Some(expected.as_str()),
        None => true,
    };

    let top5 = &ids[..5.min(ids.len())];
    let recall5 = if case.expected_in_top5.is_empty() {
        1.0
    } else {
        let hit = case
            .expected_in_top5
            .iter()
            .filter(|id| top5.contains(&id.as_str()))
            .count() as f64;
        hit / case.expected_in_top5.len() as f64
    };

    let reciprocal_rank = match &case.expected_top {
        Some(expected) => ids
            .iter()
            .position(|id| *id == expected.as_str())
            .map(|i| 1.0 / (i as f64 + 1.0))
            .unwrap_or(0.0),
        None => 1.0,
    };

    let forbidden_violated = case.forbidden_top.iter().any(|f| top == Some(f.as_str()));

    CaseOutcome {
        id: case.id.clone(),
        query: case.query.clone(),
        top1_hit,
        recall5,
        reciprocal_rank,
        forbidden_violated,
        status: case.status,
        gap_ticket: case.gap_ticket.clone(),
        top_names,
        latency_us,
    }
}

struct Metrics {
    n: usize,
    top1: f64,
    recall5: f64,
    mrr: f64,
    latency_p50_us: u128,
    latency_p95_us: u128,
    failures: Vec<String>,
}

fn aggregate(outcomes: &[CaseOutcome]) -> Metrics {
    let n = outcomes.len().max(1);
    let top1 = outcomes.iter().filter(|o| o.top1_hit).count() as f64 / n as f64;
    let recall5 = outcomes.iter().map(|o| o.recall5).sum::<f64>() / n as f64;
    let mrr = outcomes.iter().map(|o| o.reciprocal_rank).sum::<f64>() / n as f64;
    let mut lat: Vec<u128> = outcomes.iter().map(|o| o.latency_us).collect();
    lat.sort_unstable();
    let pct = |p: f64| -> u128 {
        if lat.is_empty() {
            return 0;
        }
        let idx = ((lat.len() as f64 - 1.0) * p).round() as usize;
        lat[idx]
    };
    let failures = outcomes
        .iter()
        .filter(|o| !o.top1_hit || o.forbidden_violated || o.recall5 < 1.0)
        .map(|o| {
            format!(
                "{} q={:?} top={:?} top1={} recall5={:.2} forbidden={}",
                o.id,
                o.query,
                o.top_names.first(),
                o.top1_hit,
                o.recall5,
                o.forbidden_violated
            )
        })
        .collect();
    Metrics {
        n: outcomes.len(),
        top1,
        recall5,
        mrr,
        latency_p50_us: pct(0.5),
        latency_p95_us: pct(0.95),
        failures,
    }
}

fn print_report(label: &str, metrics: &Metrics, outcomes: &[CaseOutcome]) {
    println!("\n=== search eval baseline: {label} ===");
    println!(
        "cases={} top1={:.3} recall@5={:.3} mrr={:.3} latency_us p50={} p95={}",
        metrics.n,
        metrics.top1,
        metrics.recall5,
        metrics.mrr,
        metrics.latency_p50_us,
        metrics.latency_p95_us
    );
    for o in outcomes {
        let mark = if o.status == Status::KnownGap {
            "GAP"
        } else if o.top1_hit && !o.forbidden_violated && o.recall5 >= 1.0 {
            "ok "
        } else {
            "FAIL"
        };
        let ticket = o
            .gap_ticket
            .as_deref()
            .map(|t| format!(" ticket#{t}"))
            .unwrap_or_default();
        println!(
            "  [{mark}] {} q={:?} -> {:?}{ticket}",
            o.id,
            o.query,
            o.top_names.iter().take(3).collect::<Vec<_>>()
        );
    }
}

fn load_cases() -> CasesFile {
    let path = cases_path();
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// 可复现评估入口。`required` 样本必须全部达标；`known-gap` 只报告。
#[test]
fn search_eval_baseline() {
    let file = load_cases();
    let fixture = file
        .fixtures
        .get("default")
        .expect("default fixture present");
    let apps = build_index(fixture);

    let mut outcomes = Vec::with_capacity(file.cases.len());
    for case in &file.cases {
        assert_eq!(case.fixture, "default", "case {} fixture", case.id);
        outcomes.push(evaluate_case(&apps, case));
    }

    print_report("all", &aggregate(&outcomes), &outcomes);

    let required: Vec<CaseOutcome> = outcomes
        .iter()
        .filter(|o| o.status == Status::Required)
        .cloned()
        .collect();
    let metrics = aggregate(&required);
    print_report("required", &metrics, &required);

    assert!(
        metrics.failures.is_empty(),
        "required eval failures:\n{}",
        metrics.failures.join("\n")
    );
}
