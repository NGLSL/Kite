//! 基线报告工具：延迟分位与 markdown 表输出。
//! 只服务 `perf` 各场景，不承载业务。

use std::time::{Duration, Instant};

pub const WARMUP: usize = 20;
pub const ITERS: usize = 200;

pub fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

/// 预热 + iters 次采样，返回 (p50, p95, max)。
pub fn time_iters<F: FnMut()>(iters: usize, mut f: F) -> (Duration, Duration, Duration) {
    for _ in 0..WARMUP.min(iters) {
        f();
    }
    let mut durs = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        f();
        durs.push(t.elapsed());
    }
    durs.sort();
    (
        percentile(&durs, 0.5),
        percentile(&durs, 0.95),
        *durs.last().unwrap_or(&Duration::ZERO),
    )
}

pub fn print_section(title: &str) {
    println!("\n## {title}\n");
    println!("| 场景 | p50 µs | p95 µs | max µs | 不变量 |");
    println!("|---|---:|---:|---:|---|");
}

pub fn print_latency(scenario: &str, p50: Duration, p95: Duration, max: Duration, fact: &str) {
    println!(
        "| {scenario} | {:.1} | {:.1} | {:.1} | {fact} |",
        p50.as_micros(),
        p95.as_micros(),
        max.as_micros()
    );
}

pub fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("kite-perf-{tag}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d
}

/// 合成 AppItem 语料（搜索/索引/快照共用）。
pub fn synthetic_apps(total: usize) -> Vec<crate::model::AppItem> {
    let names = [
        "Google Chrome",
        "Visual Studio Code",
        "微信",
        "Windows Terminal",
        "计算器",
        "Notepad++",
        "企业微信",
        "Firefox",
    ];
    (0..total)
        .map(|i| {
            let name = if i < names.len() {
                names[i].to_string()
            } else {
                format!("Sample App {i:04}")
            };
            let mut item = crate::model::AppItem::scanned(
                format!("perf-{i}"),
                name,
                format!(r"C:\fake\app{i}.exe"),
                None,
                None,
                "perf",
            );
            item.attach_search_fields();
            item
        })
        .collect()
}
