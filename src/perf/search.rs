//! 场景：搜索热路径与索引构建（反复唤起/长时间运行的代码代理）。
//! 职责：只压 RetrievalIndex / 空 Query 排序，不碰 UI 与插件。

use super::report::{print_latency, print_section, synthetic_apps, time_iters, ITERS};
use crate::search::{order_by_recent, search_with_index, MAX_RESULTS, RetrievalIndex};
use std::hint::black_box;
use std::time::Instant;

pub fn run() {
    print_section("搜索 / 索引（反复查询代理，无 UI）");

    for n in [80usize, 800] {
        let apps = synthetic_apps(n);
        let t_build = Instant::now();
        let index = RetrievalIndex::build(&apps, &[]);
        let build = t_build.elapsed();

        for q in ["chrome", "vis", "微信", "weixin", "zzzz", "v"] {
            let (p50, p95, max) = time_iters(ITERS, || {
                black_box(search_with_index(&index, q, &[], MAX_RESULTS));
            });
            print_latency(&format!("search/{n}/`{q}`"), p50, p95, max, "hits ok");
        }

        let recent: Vec<String> = apps.iter().take(30).map(|a| a.id.clone()).collect();
        let pinned: Vec<String> = apps.iter().take(5).map(|a| a.id.clone()).collect();
        let (p50, p95, max) = time_iters(ITERS, || {
            black_box(order_by_recent(&apps, &recent, &pinned, MAX_RESULTS));
        });
        print_latency(
            &format!("empty-query/{n}"),
            p50,
            p95,
            max,
            &format!("index-build {}µs", build.as_micros()),
        );
    }
}

pub fn smoke_hits_nonempty() {
    let apps = synthetic_apps(30);
    let index = RetrievalIndex::build(&apps, &[]);
    let hits = search_with_index(&index, "chrome", &[], MAX_RESULTS);
    assert!(!hits.is_empty());
}
