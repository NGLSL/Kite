//! 搜索热路径基准。不进常规 `cargo test`（已 #[ignore]），手动运行：
//!
//! ```text
//! cd 项目根目录 && cargo test --release bench -- --ignored --nocapture
//! ```
//!
//! 输出各查询类型的延迟分位与 QPS，数字用于 docs/PERFORMANCE.md。
//! 索引为合成数据（真实中英文混合 + 填充项），规模含本机实测（80）与压力档（2000）。

use std::time::{Duration, Instant};

use super::{order_by_recent, MAX_RESULTS};
use crate::model::AppItem;

const EN_NAMES: &[&str] = &[
    "Google Chrome",
    "Visual Studio Code",
    "Visual Studio",
    "IntelliJ IDEA",
    "Windows Terminal",
    "Notepad++",
    "Microsoft Edge",
    "Firefox",
    "Spotify",
    "Discord",
    "Postman",
    "Git Bash",
    "Obsidian",
    "Typora",
    "Snipaste",
];

const CN_NAMES: &[&str] = &[
    "微信",
    "微信开发者工具",
    "企业微信",
    "网易云音乐",
    "计算器",
    "记事本",
    "画图",
    "任务管理器",
    "资源监视器",
    "字符映射表",
];

/// (查询类型, query)。覆盖 Exact/Prefix/Substring/Fuzzy、中文、全拼、拼音首字母。
const QUERIES: &[(&str, &str)] = &[
    ("exact-en", "chrome"),
    ("prefix-en", "vis"),
    ("fuzzy-en", "chorme"),
    ("substring-en", "studio"),
    ("exact-zh", "微信"),
    ("pinyin-full", "weixin"),
    ("pinyin-initial", "wxkf"),
    ("pinyin-initial-short", "wx"),
    ("one-char", "v"),
    ("no-hit", "zzzzqq"),
];

const WARMUP: usize = 30;
const ITERS: usize = 300;

fn synthetic_index(total: usize) -> Vec<AppItem> {
    let mut apps = Vec::with_capacity(total);
    for i in 0..total {
        let name: String = if i < EN_NAMES.len() {
            EN_NAMES[i].to_string()
        } else if i < EN_NAMES.len() + CN_NAMES.len() {
            CN_NAMES[i - EN_NAMES.len()].to_string()
        } else {
            format!("Sample App {:04}", i)
        };
        let mut item = AppItem::scanned(
            format!("bench-{i}"),
            name,
            format!(r"C:\fake\app{i}.exe"),
            None,
            None,
            "bench",
        );
        item.attach_search_fields();
        apps.push(item);
    }
    apps
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn bench_one<F: FnMut()>(label: &str, query: &str, n: usize, mut f: F) {
    for _ in 0..WARMUP {
        f();
    }
    let mut durs = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let t = Instant::now();
        f();
        durs.push(t.elapsed());
    }
    durs.sort();
    let total: Duration = durs.iter().sum();
    println!(
        "| {n} | {label} | `{query}` | {:.1} | {:.1} | {:.1} | {} |",
        percentile(&durs, 0.5).as_nanos() as f64 / 1000.0,
        percentile(&durs, 0.95).as_nanos() as f64 / 1000.0,
        durs.last().unwrap().as_nanos() as f64 / 1000.0,
        (ITERS as f64 / total.as_secs_f64()).round(),
    );
}

/// 报告入口：两个索引规模 × 全部查询类型。
#[test]
#[ignore = "基准测试，手动运行：cargo test --release bench -- --ignored --nocapture"]
fn report_search_latency() {
    println!("\n搜索延迟（单元：µs；QPS：每秒可完成查询数）\n");
    println!("| 索引条数 | 查询类型 | query | p50 | p95 | max | QPS |");
    println!("|---|---|---|---|---|---|---|");
    for n in [80usize, 2000] {
        let apps = synthetic_index(n);
        // 生产路径：快照同代索引只建一次，测查询热路径
        let index = super::RetrievalIndex::build(&apps, &[]);
        for (label, q) in QUERIES {
            bench_one(label, q, n, || {
                let hits = super::search_with_index(&index, q, &[], MAX_RESULTS);
                std::hint::black_box(&hits);
            });
        }
        // 空 Query 默认列表：打开启动器时的固定项 + 最近使用排序路径
        let recent: Vec<String> = apps.iter().take(50).map(|a| a.id.clone()).collect();
        let pinned: Vec<String> = apps.iter().take(5).map(|a| a.id.clone()).collect();
        bench_one("empty-query", "(打开启动器)", n, || {
            let hits = order_by_recent(&apps, &recent, &pinned, MAX_RESULTS);
            std::hint::black_box(&hits);
        });
        // 索引构建耗时（快照发布路径，非每键）
        let t0 = Instant::now();
        let _ = super::RetrievalIndex::build(&apps, &[]);
        println!(
            "| {n} | index-build | (快照) | {} | | | |",
            t0.elapsed().as_micros()
        );
    }
}

/// Windows 页面级标准词的查询成本。与相同入口、但不含标准词的索引比较。
#[test]
#[ignore = "基准测试，手动运行：cargo test --release system_vocabulary_latency -- --ignored --nocapture"]
fn report_system_vocabulary_latency() {
    let mut entries = crate::app::builtin::materialize_system_entries(None);
    entries.retain(|entry| entry.id.starts_with("winsettings:"));
    let enriched_entries = entries.clone();
    let enriched = super::RetrievalIndex::build(&[], &entries);
    for entry in &mut entries {
        entry.search_context.clear();
    }
    let baseline = super::RetrievalIndex::build(&[], &entries);
    for (label, corpus) in [("baseline", &entries), ("enriched", &enriched_entries)] {
        let mut builds = Vec::new();
        for _ in 0..12 {
            let started = Instant::now();
            std::hint::black_box(super::RetrievalIndex::build(&[], corpus));
            builds.push(started.elapsed());
        }
        builds.sort();
        println!(
            "index-build {label}: p50={}ms p95={}ms",
            percentile(&builds, 0.5).as_millis(),
            percentile(&builds, 0.95).as_millis()
        );
    }
    println!(
        "\nWindows 设置页查询延迟（µs），同一批 {} 个入口\n",
        entries.len()
    );
    println!("| 入口数 | 词汇 | query | p50 | p95 | max | QPS |");
    println!("|---|---|---|---|---|---|---|");
    for (label, index) in [("baseline", &baseline), ("enriched", &enriched)] {
        for query in ["kj", "启动任务", "文件", "activation", "zzzzqq"] {
            bench_one(label, query, entries.len(), || {
                let hits = super::search_with_index(index, query, &[], MAX_RESULTS);
                std::hint::black_box(hits);
            });
        }
    }
}
