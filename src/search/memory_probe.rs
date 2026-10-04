//! 驻留内存分项探针（手动运行，不进常规 cargo test）。
//!
//! ```text
//! cargo test --release memory_probe -- --ignored --nocapture
//! ```
//!
//! 估算当前机器上系统词、合成/真实规模索引的字符串堆占用，用于 docs/PERFORMANCE 归因。

use crate::app::builtin;
use crate::model::AppItem;
use crate::search::RetrievalIndex;

fn app_item_str_bytes(item: &AppItem) -> usize {
    let mut n = 0usize;
    n += item.id.len()
        + item.name.len()
        + item.display_name.len()
        + item.target.len()
        + item.source.len()
        + item.normalized_name.len()
        + item.normalized_display.len()
        + item.pinyin.len()
        + item.pinyin_initials.len();
    if let Some(a) = &item.args {
        n += a.len();
    }
    if let Some(w) = &item.working_dir {
        n += w.len();
    }
    if let Some(i) = &item.icon {
        n += i.len();
    }
    if let Some(i) = &item.icon_src {
        n += i.len();
    }
    for k in &item.search_keywords {
        n += k.len();
    }
    for c in &item.search_context {
        n += c.len();
    }
    n
}

fn doc_str_bytes(doc: &crate::search::retrieval::IndexedDoc) -> usize {
    let mut n = app_item_str_bytes(&doc.item);
    n += doc.name.len()
        + doc.display.len()
        + doc.compact_name.len()
        + doc.compact_display.len()
        + doc.acronym.len()
        + doc.pinyin.len()
        + doc.pinyin_initials.len();
    for t in &doc.tokens {
        n += t.len();
    }
    for t in &doc.pinyin_syllables {
        n += t.len();
    }
    for t in &doc.keywords {
        n += t.len();
    }
    for t in &doc.keyword_compacts {
        n += t.len();
    }
    for t in &doc.keyword_full_pinyin {
        n += t.len();
    }
    for t in &doc.keyword_initials {
        n += t.len();
    }
    for t in &doc.context_fields {
        n += t.len();
    }
    for t in &doc.context_terms {
        n += t.len();
    }
    for t in &doc.context_compacts {
        n += t.len();
    }
    for t in &doc.context_full_pinyin {
        n += t.len();
    }
    for t in &doc.context_initials {
        n += t.len();
    }
    n
}

#[test]
#[ignore = "内存分项探针，手动运行：cargo test --release memory_probe -- --ignored --nocapture"]
fn report_memory_breakdown() {
    println!("\nKite 内存分项探针（字符串堆粗估；不含 HashMap 节点/FST/线程栈/渲染面）\n");

    // 系统入口（本机 SearchResources）
    let entries = crate::app::builtin::materialize_system_entries(None);
    let win: Vec<_> = entries
        .iter()
        .filter(|e| e.id.starts_with("winsettings:"))
        .collect();
    let mut ctx_terms = 0usize;
    let mut ctx_chars = 0usize;
    for e in &win {
        ctx_terms += e.search_context.len();
        for t in &e.search_context {
            ctx_chars += t.len();
        }
    }
    println!(
        "系统入口 total={} winsettings={} · search_context 词条={} 字符≈{:.1}KB",
        entries.len(),
        win.len(),
        ctx_terms,
        ctx_chars as f64 / 1024.0
    );

    // 合成应用，对齐本机规模 297
    let apps = synthetic_apps(297);
    let apps_ctx_bytes: usize = apps.iter().map(app_item_str_bytes).sum();
    let ent_ctx_bytes: usize = entries.iter().map(app_item_str_bytes).sum();
    println!(
        "AppItem 字符串粗估：apps({})={:.1}KB · system_entries({})={:.1}KB · 合计={:.1}KB",
        apps.len(),
        apps_ctx_bytes as f64 / 1024.0,
        entries.len(),
        ent_ctx_bytes as f64 / 1024.0,
        (apps_ctx_bytes + ent_ctx_bytes) as f64 / 1024.0
    );

    let index = RetrievalIndex::build(&apps, &entries);
    let docs = &index.docs;
    let mut doc_bytes = 0usize;
    let mut item_bytes = 0usize;
    let mut derived_bytes = 0usize;
    for d in docs {
        let total = doc_str_bytes(d);
        let item = app_item_str_bytes(&d.item);
        doc_bytes += total;
        item_bytes += item;
        derived_bytes += total - item;
    }
    println!(
        "RetrievalIndex.docs({}) 字符串粗估合计={:.1}KB（其中内嵌 AppItem≈{:.1}KB，派生字段≈{:.1}KB）",
        docs.len(),
        doc_bytes as f64 / 1024.0,
        item_bytes as f64 / 1024.0,
        derived_bytes as f64 / 1024.0
    );

    // 对照：清空 search_context 后再建索引，看派生字段与倒排键是否明显缩水
    let mut apps_nc = apps.clone();
    for a in &mut apps_nc {
        a.search_context.clear();
        a.attach_search_fields();
    }
    let mut entries_nc = entries.clone();
    for e in &mut entries_nc {
        e.search_context.clear();
        e.attach_search_fields();
    }
    let index_nc = RetrievalIndex::build(&apps_nc, &entries_nc);
    let mut doc_nc = 0usize;
    for d in &index_nc.docs {
        doc_nc += doc_str_bytes(d);
    }
    println!(
        "对照：清空 search_context 后 docs 字符串={:.1}KB（差值={:.1}KB，主要来自设置页标准词）",
        doc_nc as f64 / 1024.0,
        (doc_bytes as isize - doc_nc as isize) as f64 / 1024.0
    );

    // 结构规模（公开可访问部分）
    println!("索引规模：docs={}（term_postings/gram 等为私有，未计入粗估）", docs.len());
    let big = RetrievalIndex::build(&synthetic_apps(2000), &entries);
    let mut big_bytes = 0usize;
    for d in &big.docs {
        big_bytes += doc_str_bytes(d);
    }
    println!(
        "压力档 2000+system：docs={} 字符串粗估={:.1}KB",
        big.docs.len(),
        big_bytes as f64 / 1024.0
    );

    // 进程侧（若本测试进程与 kite 无关，仅作占位说明）
    println!(
        "\n注意：字符串粗估不含 HashMap/Vec 容器头、FST、gram2/gram3、SymSpell deletes、\n\
         CharBits、线程栈、iced/tiny-skia 帧缓冲与字体。真实驻留见 measure-performance.ps1。"
    );
}

fn synthetic_apps(n: usize) -> Vec<AppItem> {
    const EN: &[&str] = &[
        "Google Chrome",
        "Visual Studio Code",
        "Windows Terminal",
        "Microsoft Edge",
        "Notepad++",
        "7-Zip File Manager",
        "Everything",
        "PowerToys",
    ];
    const CN: &[&str] = &["微信", "企业微信", "网易云音乐", "钉钉", "腾讯会议", "向日葵"];
    let mut apps = Vec::with_capacity(n);
    for i in 0..n {
        let name = if i < EN.len() {
            EN[i].to_string()
        } else if i < EN.len() + CN.len() {
            CN[i - EN.len()].to_string()
        } else {
            format!("Sample Application {:04}", i)
        };
        let mut item = AppItem::scanned(
            format!("memprobe-{i}"),
            name.clone(),
            format!(r"C:\Program Files\Fake\app{i}.exe"),
            None,
            None,
            "probe",
        );
        item.search_keywords = vec![format!("keyword{i}"), format!("别名{}", i % 17)];
        if i % 5 == 0 {
            item.search_context = vec![
                format!("context term {i}"),
                format!("上下文词{}", i),
            ];
        }
        item.attach_search_fields();
        apps.push(item);
    }
    apps
}

#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

#[link(name = "psapi")]
extern "system" {
    fn GetProcessMemoryInfo(
        process: isize,
        counters: *mut ProcessMemoryCounters,
        cb: u32,
    ) -> i32;
}

fn private_mb() -> f64 {
    unsafe {
        let process = windows::Win32::System::Threading::GetCurrentProcess();
        let mut counters = ProcessMemoryCounters {
            cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
            ..Default::default()
        };
        let ok = GetProcessMemoryInfo(process.0 as isize, &mut counters, counters.cb);
        debug_assert_ne!(ok, 0);
        counters.pagefile_usage as f64 / (1024.0 * 1024.0)
    }
}

#[test]
#[ignore = "内存分项探针，手动运行：cargo test --release memory_probe -- --ignored --nocapture"]
fn report_heap_deltas() {
    println!("\nKite 堆增量探针（本测试进程 Private/Pagefile 近似，单位 MB）\n");
    let base = private_mb();
    println!("baseline private≈{base:.2} MB");

    // fontdb 系统字体扫描（ui::font 路径同款）
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let after_fonts = private_mb();
    println!(
        "after fontdb::load_system_fonts faces={} private≈{:.2} MB (+{:.2})",
        db.len(),
        after_fonts,
        after_fonts - base
    );
    drop(db);
    let after_drop = private_mb();
    println!("after drop(db) private≈{after_drop:.2} MB（分配器可能不归还）");

    // 索引构建（297 + 系统入口）—— 首次在干净堆上构建，最能代表驻留
    let apps = synthetic_apps(297);
    let entries = crate::app::builtin::materialize_system_entries(None);
    let before_idx = private_mb();
    let index = RetrievalIndex::build(&apps, &entries);
    let after_idx = private_mb();
    let stats = index.structure_stats();
    println!(
        "after RetrievalIndex::build docs={} private≈{:.2} MB (+{:.2} vs before)",
        stats.docs,
        after_idx,
        after_idx - before_idx
    );
    println!(
        "structure: term_postings keys={} ids={} · compact keys={} ids={} · gram2 keys={} ids={} · gram3 keys={} ids={} · deletes keys={} vals={} · char_postings keys={} · pinyin_char keys={}",
        stats.term_postings_keys,
        stats.term_postings_ids,
        stats.compact_postings_keys,
        stats.compact_postings_ids,
        stats.gram2_keys,
        stats.gram2_ids,
        stats.gram3_keys,
        stats.gram3_ids,
        stats.deletes_keys,
        stats.deletes_values,
        stats.char_postings_keys,
        stats.pinyin_char_postings_keys
    );
    // 粗算：HashMap 每键约 80–120B + 每 DocId 4B（不含 Vec 分配器放大）
    let gram_est = (stats.gram2_keys + stats.gram3_keys) as f64 * 100.0
        + (stats.gram2_ids + stats.gram3_ids) as f64 * 4.0;
    let term_est = (stats.term_postings_keys + stats.compact_postings_keys) as f64 * 80.0
        + (stats.term_postings_ids + stats.compact_postings_ids) as f64 * 4.0;
    let del_est = stats.deletes_keys as f64 * 80.0 + stats.deletes_values as f64 * 24.0;
    println!(
        "粗算（偏低）：gram≈{:.1}MB · term/compact≈{:.1}MB · deletes≈{:.1}MB",
        gram_est / (1024.0 * 1024.0),
        term_est / (1024.0 * 1024.0),
        del_est / (1024.0 * 1024.0)
    );
    drop(index);
    let after_idx_drop = private_mb();
    println!("after drop(index) private≈{after_idx_drop:.2} MB");
}

#[test]
#[ignore = "内存分项探针，手动运行：cargo test --release memory_probe -- --ignored --nocapture"]
fn report_index_structure_cost() {
    println!("\nRetrievalIndex 结构成本拆分（测试进程 private 增量，MB）\n");
    let apps = synthetic_apps(297);
    let entries = crate::app::builtin::materialize_system_entries(None);

    fn delta(label: &str, f: impl FnOnce()) {
        let before = private_mb();
        f();
        let after = private_mb();
        println!("{label}: +{:.2} MB (private {:.2} → {:.2})", after - before, before, after);
    }

    delta("ib_pinyin::PinyinData::new", || {
        let data = ib_pinyin::pinyin::PinyinData::new(ib_pinyin::pinyin::PinyinNotation::Ascii);
        std::hint::black_box(data);
    });

    // apps only
    delta("build(apps, [])", || {
        let index = RetrievalIndex::build(&apps, &[]);
        std::hint::black_box(&index);
        println!("  docs={}", index.docs.len());
    });

    delta("build([], system_entries)", || {
        let index = RetrievalIndex::build(&[], &entries);
        std::hint::black_box(&index);
        println!("  docs={}", index.docs.len());
    });

    delta("build(apps, system_entries)", || {
        let index = RetrievalIndex::build(&apps, &entries);
        std::hint::black_box(&index);
        println!("  docs={}", index.docs.len());
    });

    let mut apps_nc = apps.clone();
    for a in &mut apps_nc {
        a.search_context.clear();
        a.attach_search_fields();
    }
    let mut entries_nc = entries.clone();
    for e in &mut entries_nc {
        e.search_context.clear();
        e.attach_search_fields();
    }
    delta("build(apps, system) 无 search_context", || {
        let index = RetrievalIndex::build(&apps_nc, &entries_nc);
        std::hint::black_box(&index);
        println!("  docs={}", index.docs.len());
    });
}

// 仅为避免 unused import 警告（materialize 在 lib 其它路径）。
#[allow(dead_code)]
fn _touch_builtin() {
    let _ = builtin::materialize_system_entries(None);
}
