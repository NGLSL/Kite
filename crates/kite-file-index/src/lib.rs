//! Kite internal Windows filename index. Metadata readers never read file contents.
#![cfg(windows)]
macro_rules! diagnostic { ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }}; }
pub(crate) use diagnostic;
include!("format.rs");
mod api;
mod cancel;
#[allow(dead_code)]
mod compact;
mod filter;
#[allow(dead_code)]
mod layout_project;
mod live;
#[allow(dead_code)]
mod mft;
#[allow(dead_code)]
mod paged;
mod snapshot_merge;
pub use api::{build, capture, watch, CaptureManifest, CapturedVolume, Generation};
pub use filter::{FileFilter, Query};
pub use live::LiveSearch;
/// Share with the service/controller; true requests cooperative cancellation.
pub type CancelToken = std::sync::Arc<std::sync::atomic::AtomicBool>;
pub type Progress<'a> = &'a (dyn Fn(serde_json::Value) + Send + Sync);

/// A saved base cannot be continued safely; the UI should offer Rebuild.
#[derive(Debug)]
pub struct NeedsRebuild(pub String);
impl std::fmt::Display for NeedsRebuild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NeedsRebuild: {}", self.0)
    }
}
impl std::error::Error for NeedsRebuild {}
