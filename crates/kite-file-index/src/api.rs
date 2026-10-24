use crate::{CancelToken, Progress, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapturedVolume {
    pub volume: String,
    pub reader: String,
    pub directory: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureManifest {
    pub format: String,
    pub complete: bool,
    pub root: PathBuf,
    pub checkpoint: PathBuf,
    pub volumes: Vec<CapturedVolume>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Generation {
    pub root: PathBuf,
    pub snapshot: PathBuf,
    pub index: PathBuf,
    pub identities: PathBuf,
    pub checkpoint: PathBuf,
    pub entries: u64,
}
/// New service-owned directory only; volumes are drive letters, never arbitrary paths.
pub fn capture(
    root: &Path,
    volumes: &[String],
    cancel: CancelToken,
    progress: Progress<'_>,
) -> Result<CaptureManifest> {
    capture_impl(root, volumes, cancel, progress)
}
/// Read the immutable base cursor until cancelled. Never mutates the USN journal.
pub fn watch(
    checkpoint: &Path,
    feed: &Path,
    cancel: CancelToken,
    progress: Progress<'_>,
) -> Result<()> {
    watch_resumable(
        checkpoint,
        feed,
        cancel,
        progress,
        None,
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    )
}
/// Continue from a durable applied cursor. Acknowledgements reclaim only
/// transport segments already consumed by this authenticated session.
pub fn watch_resumable(
    checkpoint: &Path,
    feed: &Path,
    cancel: CancelToken,
    progress: Progress<'_>,
    resume: Option<Value>,
    acknowledged: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> Result<()> {
    let _guard = crate::cancel::enter(cancel);
    progress(serde_json::json!({"stage":"watching"}));
    let result = crate::mft::journal_watch_resumable(checkpoint, feed, resume, acknowledged);
    if crate::cancel::cancelled() {
        Ok(())
    } else {
        result
    }
}
/// Ordinary-rights build into a new generation; caller publishes only on success.
/// The destination may be absent or an empty ordinary directory owned by the caller.
pub fn build(
    capture_root: &Path,
    destination: &Path,
    cancel: CancelToken,
    progress: Progress<'_>,
) -> Result<Generation> {
    build_impl(capture_root, destination, cancel, progress)
}

fn load_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(path)?,
    ))?)
}
fn save_new(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut out = std::io::BufWriter::with_capacity(1024 * 1024, file);
    serde_json::to_writer(&mut out, value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    out.get_ref().sync_all()?;
    Ok(())
}
impl CaptureManifest {
    pub fn load(root: &Path) -> Result<Self> {
        let manifest: Self = serde_json::from_value(load_json(&root.join("capture.json"))?)?;
        if manifest.format != "KITEFILECAP1"
            || !manifest.complete
            || manifest.root != root.canonicalize()?
            || manifest.checkpoint != manifest.root.join("journal-start.json")
        {
            return Err(crate::invalid("capture manifest identity/completeness mismatch").into());
        }
        if manifest.volumes.is_empty() {
            return Err(crate::invalid("capture has no volumes").into());
        }
        let mut seen = std::collections::HashSet::new();
        for volume in &manifest.volumes {
            if normalize(&volume.volume)? != volume.volume
                || volume.directory != volume.volume[..1]
                || !matches!(volume.reader.as_str(), "layout" | "fast")
                || !seen.insert(&volume.volume)
            {
                return Err(
                    crate::invalid("capture contains invalid/duplicate volume plan").into(),
                );
            }
        }
        Ok(manifest)
    }
}
impl Generation {
    pub fn load(root: &Path) -> Result<Self> {
        let generation: Self = serde_json::from_value(load_json(&root.join("generation.json"))?)?;
        let root = root.canonicalize()?;
        if generation.root != root
            || generation.snapshot != root.join("corpus.bin")
            || generation.index != root.join("names.index")
            || generation.identities != root.join("identities.sorted")
            || generation.checkpoint != root.join("journal-start.json")
        {
            return Err(
                crate::invalid("generation paths differ from fixed generation layout").into(),
            );
        }
        Ok(generation)
    }
}
fn normalize(value: &str) -> Result<String> {
    let value = value.trim_end_matches('\\').to_ascii_uppercase();
    if value.len() != 2 || !value.as_bytes()[0].is_ascii_uppercase() || value.as_bytes()[1] != b':'
    {
        return Err(crate::invalid("volume must be a drive letter such as C:").into());
    }
    Ok(value)
}
fn capture_volume(
    root: &Path,
    volume: &str,
    cancel: CancelToken,
    progress: Progress<'_>,
) -> Result<CapturedVolume> {
    let _guard = crate::cancel::enter(cancel);
    crate::cancel::check()?;
    let directory = volume[..1].to_owned();
    let dir = root.join(&directory);
    std::fs::create_dir(&dir)?;
    progress(serde_json::json!({"stage":"capturing","volume":volume}));
    let layout = dir.join("layout");
    let layout_result = crate::mft::capture_layout(&layout, &[volume.to_owned()]);
    crate::cancel::check()?;
    if layout_result.is_ok() {
        crate::layout_project::verify_zero_links(&layout)?;
        // Scope is an intentional product contract, independent of API completeness.
        let path = layout.join("manifest.json");
        let mut m = load_json(&path)?;
        m["file_search_scope"] = serde_json::json!(crate::mft::FILE_SEARCH_SCOPE);
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        serde_json::to_writer(&mut out, &m)?;
        std::io::Write::flush(&mut out)?;
        out.get_ref().sync_all()?;
        return Ok(CapturedVolume {
            volume: volume.to_owned(),
            reader: "layout".into(),
            directory,
        });
    }
    let m = load_json(&layout.join("manifest.json"))?;
    if m["volumes"]
        .as_array()
        .and_then(|v| v.first())
        .and_then(|v| v["win32_error"].as_i64())
        != Some(50)
    {
        return Err(layout_result.err().unwrap());
    }
    progress(
        serde_json::json!({"stage":"capturing-fast-records","volume":volume,"layout_win32":50}),
    );
    let output = dir.join("collected.bin");
    let ids = dir.join("collected.raw");
    let result = crate::mft::run_fast_mapped(&output, &[std::ffi::OsString::from(volume)], &ids);
    crate::cancel::check()?;
    let capture = load_json(&dir.join("collected.bin.capture/manifest.json"))?;
    if capture["complete"] != true {
        result?;
        return Err(crate::invalid("metadata capture incomplete").into());
    }
    // A complete metadata capture may need its recorded identity repairs finalized
    // by the ordinary builder. Never publish the collector's partial snapshot.
    Ok(CapturedVolume {
        volume: volume.to_owned(),
        reader: "fast".into(),
        directory,
    })
}
fn capture_impl(
    root: &Path,
    volumes: &[String],
    cancel: CancelToken,
    progress: Progress<'_>,
) -> Result<CaptureManifest> {
    let _guard = crate::cancel::enter(cancel.clone());
    crate::cancel::check()?;
    if volumes.is_empty() || volumes.len() > 26 {
        return Err(crate::invalid("capture needs 1..26 volumes").into());
    }
    let volumes = volumes
        .iter()
        .map(|v| normalize(v))
        .collect::<Result<Vec<_>>>()?;
    if volumes
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        != volumes.len()
    {
        return Err(crate::invalid("duplicate capture volumes").into());
    }
    std::fs::create_dir(root)?;
    let root = root.canonicalize()?;
    let checkpoint = root.join("journal-start.json");
    let roots = volumes
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    crate::mft::journal_begin(&checkpoint, &roots)?;
    let mut captured = Vec::new();
    for chunk in volumes.chunks(2) {
        let result = std::thread::scope(|scope| {
            let jobs = chunk
                .iter()
                .map(|volume| {
                    let root = &root;
                    let token = cancel.clone();
                    scope.spawn(move || capture_volume(root, volume, token, progress))
                })
                .collect::<Vec<_>>();
            jobs.into_iter()
                .map(|job| {
                    job.join()
                        .map_err(|_| crate::invalid("metadata capture thread panicked").into())
                        .and_then(|r| r)
                })
                .collect::<Result<Vec<_>>>()
        });
        match result {
            Ok(items) => captured.extend(items),
            Err(error) => {
                save_new(
                    &root.join("capture-error.json"),
                    &serde_json::json!({"complete":false,"error":error.to_string()}),
                )?;
                return Err(error);
            }
        }
    }
    let manifest = CaptureManifest {
        format: "KITEFILECAP1".into(),
        complete: true,
        root: root.clone(),
        checkpoint,
        volumes: captured,
    };
    save_new(&root.join("capture.json"), &manifest)?;
    progress(serde_json::json!({"stage":"captured","volumes":manifest.volumes.len()}));
    Ok(manifest)
}
fn build_impl(
    capture_root: &Path,
    destination: &Path,
    cancel: CancelToken,
    progress: Progress<'_>,
) -> Result<Generation> {
    let _guard = crate::cancel::enter(cancel.clone());
    crate::cancel::check()?;
    let capture = CaptureManifest::load(capture_root)?;
    match std::fs::create_dir(destination) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            use std::os::windows::fs::MetadataExt;
            // The controller atomically claims a new empty directory before
            // spawning its builder, so it can clean up interrupted builds.
            let metadata = std::fs::symlink_metadata(destination)?;
            if !metadata.is_dir()
                || metadata.file_attributes() & 0x400 != 0
                || std::fs::read_dir(destination)?
                    .next()
                    .transpose()?
                    .is_some()
            {
                return Err(crate::invalid(
                    "build destination must be an empty ordinary directory",
                )
                .into());
            }
        }
        Err(error) => return Err(error.into()),
    }
    let root = destination.canonicalize()?;
    let checkpoint = root.join("journal-start.json");
    std::fs::copy(&capture.checkpoint, &checkpoint)?;
    let begin = load_json(&checkpoint)?;
    let mut sources = Vec::new();
    for volume in &capture.volumes {
        crate::cancel::check()?;
        progress(serde_json::json!({"stage":"projecting","volume":volume.volume}));
        let source = capture.root.join(&volume.directory);
        let target = root.join(&volume.directory);
        std::fs::create_dir(&target)?;
        let snapshot = target.join("snapshot.bin");
        let ids = target.join("identities.raw");
        let observed = if volume.reader == "layout" {
            crate::layout_project::run_mapped(&source.join("layout"), &snapshot, &ids)?;
            load_json(&source.join("layout/manifest.json"))?["volumes"]
                .as_array()
                .and_then(|v| v.first())
                .cloned()
                .ok_or_else(|| crate::invalid("missing captured layout volume"))?
        } else {
            let report = load_json(&source.join("collected.bin.json"))?;
            if report["status"] == "complete" {
                std::fs::copy(source.join("collected.bin"), &snapshot)?;
                std::fs::copy(source.join("collected.raw"), &ids)?;
            } else {
                crate::mft::replay_mapped(&source.join("collected.bin.capture"), &snapshot, &ids)?;
            }
            load_json(&source.join("collected.bin.capture/manifest.json"))?["volumes"]
                .as_array()
                .and_then(|v| v.first())
                .cloned()
                .ok_or_else(|| crate::invalid("missing captured fast volume"))?
        };
        let expected = begin["volumes"]
            .as_array()
            .and_then(|v| v.iter().find(|v| v["volume"] == volume.volume))
            .ok_or_else(|| crate::invalid("captured volume absent from checkpoint"))?;
        for field in ["volume_serial", "root_frn"] {
            if observed[field] != expected[field] {
                return Err(crate::invalid(
                    "captured volume identity differs from journal checkpoint",
                )
                .into());
            }
        }
        sources.push(snapshot.into_os_string());
        sources.push(ids.into_os_string());
    }
    let snapshot = root.join("corpus.bin");
    let raw = root.join("identities.raw");
    crate::snapshot_merge::run(&snapshot, &raw, &sources)?;
    let index = root.join("names.index");
    let identities = root.join("identities.sorted");
    progress(serde_json::json!({"stage":"building"}));
    std::thread::scope(|scope| -> Result<()> {
        let token = cancel.clone();
        let names = scope.spawn(|| {
            let _guard = crate::cancel::enter(token);
            crate::paged::build_in_memory(&snapshot, &index)
        });
        crate::live::build_identities(&snapshot, &raw, &identities)?;
        names
            .join()
            .map_err(|_| crate::invalid("name builder thread panicked"))??;
        Ok(())
    })?;
    crate::cancel::check()?;
    let mut file = std::fs::File::open(&snapshot)?;
    let entries = crate::snapshot_header(&mut file)?;
    save_new(
        &root.join("identities.sorted.provenance.json"),
        &serde_json::json!({"begin":begin,"snapshot_entries":entries,"snapshot_bytes":file.metadata()?.len()}),
    )?;
    let generation = Generation {
        root: root.clone(),
        snapshot,
        index,
        identities,
        checkpoint,
        entries,
    };
    save_new(&root.join("generation.json"), &generation)?;
    // Only files created by this successful build in its new destination.
    // Source service capture artifacts are never removed by ordinary code.
    let mut cleanup_warnings = Vec::new();
    for volume in &capture.volumes {
        let temporary = root.join(&volume.directory);
        for name in ["snapshot.bin", "snapshot.bin.json", "identities.raw"] {
            let path = temporary.join(name);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => cleanup_warnings
                    .push(serde_json::json!({"path":path,"error":error.to_string()})),
            }
        }
        if let Err(error) = std::fs::remove_dir(&temporary) {
            cleanup_warnings.push(serde_json::json!({"path":temporary,"error":error.to_string()}));
        }
    }
    if let Err(error) = std::fs::remove_file(&raw) {
        cleanup_warnings.push(serde_json::json!({"path":raw,"error":error.to_string()}));
    }
    if !cleanup_warnings.is_empty() {
        progress(serde_json::json!({"stage":"cleanup-warning","files":cleanup_warnings}));
    }

    progress(serde_json::json!({"stage":"built","entries":entries}));
    Ok(generation)
}
