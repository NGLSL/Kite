//! Scoop shims 目录收集。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::cache::ScanCache;
use super::util::normalize_path_key;
use super::walk::collect_from_dir;
use super::RawItem;

pub(crate) fn collect_scoop_shims(
    home: Option<&Path>,
    program_data: Option<&Path>,
    scoop_root: Option<&Path>,
    scoop_global_root: Option<&Path>,
    extra_shim_dirs: &[PathBuf],
    budget: Option<Duration>,
    t0: Instant,
    max_depth: usize,
    max_per_dir: usize,
    max_total: usize,
    out: &mut Vec<RawItem>,
    cache: &mut ScanCache,
) {
    let mut roots = extra_shim_dirs.to_vec();
    if let Some(home) = home {
        roots.push(home.join("scoop").join("shims"));
    }
    if let Some(root) = scoop_root {
        roots.push(root.join("shims"));
    }
    if let Some(root) = scoop_global_root {
        roots.push(root.join("shims"));
    }
    if let Some(program_data) = program_data {
        roots.push(program_data.join("scoop").join("shims"));
    }

    let mut seen = HashSet::new();
    for root in roots {
        if !seen.insert(normalize_path_key(&root.to_string_lossy())) {
            continue;
        }
        collect_from_dir(
            &root,
            "scoop",
            max_depth,
            budget,
            t0,
            max_per_dir,
            max_total,
            out,
            cache,
        );
    }
}
