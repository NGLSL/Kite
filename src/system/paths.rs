//! User data locations shared by the entry point and the native UI.

use std::path::PathBuf;

/// Debug and explicitly isolated measurement builds can use a separate profile.
/// Normal Release builds keep the established per-user location.
pub fn data_dir() -> PathBuf {
    #[cfg(any(debug_assertions, feature = "isolated-profile"))]
    if let Some(path) = std::env::var_os("KITE_DEV_DATA_DIR") {
        return PathBuf::from(path);
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("com.kite.launcher")
}

pub fn cache_dir() -> PathBuf {
    #[cfg(any(debug_assertions, feature = "isolated-profile"))]
    if std::env::var_os("KITE_DEV_DATA_DIR").is_some() {
        return data_dir().join("cache");
    }
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("com.kite.launcher")
}
