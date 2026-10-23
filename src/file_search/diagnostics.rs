//! Line-oriented diagnostics for the production file-search controller.
//!
//! This entry point uses only the explicitly supplied data directory. It never
//! installs components, opens a result, or creates a desktop window.

use super::{Controller, FileFilter, Phase};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

const SESSION_ARG: &str = "--kite-file-index-session";
const MAX_REQUEST_BYTES: usize = 64 * 1024;

pub(super) fn dispatch_entry() -> bool {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_none_or(|arg| arg != SESSION_ARG) {
        return false;
    }
    let result = (|| -> Result<(), String> {
        if args.len() != 2 || args[1].is_empty() {
            return Err("usage: kite.exe --kite-file-index-session <data-directory>".into());
        }
        let mut data_dir = PathBuf::from(&args[1]);
        if !data_dir.is_absolute() {
            data_dir = std::env::current_dir()
                .map_err(|error| error.to_string())?
                .join(data_dir);
        }
        std::fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
        crate::log::init(crate::log::default_path_under(&data_dir));
        session(Controller::new(data_dir))
    })();
    if let Err(error) = result {
        let _ = respond(&mut io::stdout().lock(), &json!({"ok":false,"error":error}));
        std::process::exit(1);
    }
    true
}

fn session(controller: Arc<Controller>) -> Result<(), String> {
    controller.start(false);
    let result = (|| -> Result<(), String> {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        let stdout = io::stdout();
        let mut output = stdout.lock();
        while let Some(request) = read_request(&mut input).map_err(|error| error.to_string())? {
            let (response, stop) = match request.and_then(|bytes| {
                serde_json::from_slice::<Value>(&bytes).map_err(|error| error.to_string())
            }) {
                Ok(value) => handle(&controller, &value),
                Err(error) => (json!({"ok":false,"error":error}), false),
            };
            // A stop response confirms the worker and metadata session have
            // been released, rather than merely queued for cancellation.
            if stop {
                stop_and_wait(&controller);
            }
            respond(&mut output, &response).map_err(|error| error.to_string())?;
            if stop {
                break;
            }
        }
        Ok(())
    })();
    stop_and_wait(&controller);
    result
}

/// Drain overlong requests through their newline without retaining the body.
fn read_request(input: &mut impl BufRead) -> io::Result<Option<Result<Vec<u8>, String>>> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            if bytes.is_empty() && !oversized {
                return Ok(None);
            }
            break;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |index| index + 1);
        if !oversized {
            if bytes.len().saturating_add(count) > MAX_REQUEST_BYTES {
                oversized = true;
                bytes.clear();
            } else {
                bytes.extend_from_slice(&available[..count]);
            }
        }
        input.consume(count);
        if newline.is_some() {
            break;
        }
    }
    Ok(Some(if oversized {
        Err("diagnostic request exceeds 64 KiB".into())
    } else {
        Ok(bytes)
    }))
}

fn handle(controller: &Arc<Controller>, value: &Value) -> (Value, bool) {
    let result = (|| -> Result<(Value, bool), String> {
        let command = value
            .as_object()
            .and_then(|object| object.get("command"))
            .and_then(Value::as_str)
            .ok_or("command must be a string")?;
        match command {
            "status" => {
                let snapshot = controller.snapshot();
                let mut state = controller
                    .shared
                    .diagnostics
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                state["phase"] = json!(phase_name(snapshot.phase));
                state["detail"] = json!(snapshot.detail);
                state["entries"] = json!(snapshot.entries);
                state["revision"] = json!(snapshot.revision);
                Ok((json!({"ok":true,"state":state}), false))
            }
            "query" => {
                let query = value["query"].as_str().ok_or("query must be a string")?;
                let limit = match value.get("limit") {
                    None => 20,
                    Some(value) => value
                        .as_u64()
                        .filter(|limit| *limit <= 1000)
                        .ok_or("limit must be an integer between 0 and 1000")?
                        as usize,
                };
                let filter = match value.get("filter").and_then(Value::as_str) {
                    None if value.get("filter").is_none() => FileFilter::All,
                    Some("all") => FileFilter::All,
                    Some("images") => FileFilter::Images,
                    Some("documents") => FileFilter::Documents,
                    Some("videos") => FileFilter::Videos,
                    Some("audio") => FileFilter::Audio,
                    Some("archives") => FileFilter::Archives,
                    Some("folders") => FileFilter::Folders,
                    _ => return Err("unknown file filter".into()),
                };
                let snapshot = controller.snapshot();
                if snapshot.phase != Phase::Ready {
                    return Err(format!(
                        "file search is {}: {}",
                        phase_name(snapshot.phase),
                        snapshot.detail
                    ));
                }
                // Controller uses its production 15-second response timeout.
                let result = controller.query(query, limit, filter);
                controller.cancel_query();
                let hits: Vec<_> = result?
                    .into_iter()
                    .map(|hit| json!({"path":hit.path,"name":hit.name,"is_folder":hit.is_folder,"raw_path":hit.raw_path}))
                    .collect();
                Ok((json!({"ok":true,"hits":hits}), false))
            }
            "rebuild" => {
                controller.rebuild(false);
                Ok((json!({"ok":true}), false))
            }
            "stop" => Ok((json!({"ok":true}), true)),
            _ => Err("unknown diagnostic command".into()),
        }
    })();
    result.unwrap_or_else(|error| (json!({"ok":false,"error":error}), false))
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Stopped => "stopped",
        Phase::Starting => "starting",
        Phase::Indexing => "indexing",
        Phase::CatchingUp => "catching_up",
        Phase::Ready => "ready",
        Phase::NeedsAuthorization => "needs_authorization",
        Phase::NeedsRebuild => "needs_rebuild",
        Phase::Error => "error",
    }
}

fn stop_and_wait(controller: &Controller) {
    controller.stop();
    let running = controller
        .shared
        .running
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some(running) = running {
        let _ = running.thread.join();
    }
}

fn respond(output: &mut impl Write, response: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *output, response)?;
    output.write_all(b"\n")?;
    output.flush()
}
