//! Registered application URI protocols.
//!
//! Windows exposes protocol handlers through the merged `HKCR` view.  The
//! scanner reads the two backing locations explicitly (`HKCU` and `HKLM`) so
//! per-user registrations can take precedence over machine-wide ones and both
//! registry views can be inspected.  Only handlers whose `shell\\open\\command`
//! resolves to an existing, non-system `.exe` are returned.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, REG_EXPAND_SZ, REG_SZ,
};

use crate::model::AppItem;

use super::util::{stable_item_id, wide};

/// Source label used by materialized protocol-only entries.
pub const SOURCE: &str = "protocols";
const CLASSES_PATH: &str = r"Software\Classes";

/// A verified application protocol registration.
///
/// `target` is an expanded path to an existing executable.  `args` contains
/// only static command arguments; registry placeholders such as `%1`, `%L`,
/// and `%*` are deliberately omitted because a normal `AppItem` launch must
/// never receive user-controlled protocol data implicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolEntry {
    pub protocol: String,
    pub target: String,
    pub args: Option<String>,
}

impl ProtocolEntry {
    pub fn new(
        protocol: impl Into<String>,
        target: impl Into<String>,
        args: Option<String>,
    ) -> Self {
        Self {
            protocol: protocol.into().trim().to_ascii_lowercase(),
            target: target.into().trim().to_string(),
            args: args
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        }
    }
}

/// The executable and safe-to-forward static arguments parsed from a registry
/// `shell\\open\\command` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub target: String,
    pub args: Option<String>,
}

/// Enumerate verified application protocol registrations from HKCU/HKLM.
///
/// Registry access is best-effort: a missing hive, malformed value, inaccessible
/// key, or one broken registration only removes that registration from the
/// result.  The effective HKCU registration wins when the same protocol is
/// present in both hives, matching the normal merged Classes view.
pub fn collect_protocols() -> Vec<ProtocolEntry> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(collect_from_registry));
    result.unwrap_or_default()
}

/// Parse the first executable in a registered open command.
///
/// Quoted targets are read up to the closing quote.  For an unquoted target we
/// locate the first `.exe` suffix, which also supports the common but technically
/// ambiguous `C:\\Program Files\\App\\app.exe "%1"` form.  Any percent sign in
/// the argument tail causes that tail to be dropped; this covers protocol
/// placeholders and keeps dynamic registry input out of an `AppItem` launch.
pub fn parse_open_command(command: &str) -> Option<ParsedCommand> {
    let expanded = crate::system::env::expand_env(command.trim());
    let command = expanded.trim();
    if command.is_empty() {
        return None;
    }

    let (target, raw_args) = split_command_target(command)?;
    let target = target.trim();
    if target.is_empty()
        || target.contains('%')
        || target
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '"' | '&' | '|' | '<' | '>'))
        || !is_executable_path(target)
    {
        return None;
    }

    let raw_args = raw_args.trim();
    let args = if raw_args.is_empty()
        || raw_args.chars().any(|ch| ch.is_control())
        || raw_args.contains('%')
    {
        None
    } else {
        Some(raw_args.to_string())
    };
    Some(ParsedCommand {
        target: target.to_string(),
        args,
    })
}

/// Resolve a parsed command target to an existing executable path.
///
/// Full and relative paths are checked directly.  A bare executable name is
/// resolved against the current PATH and `%SystemRoot%\\System32`; all
/// candidates still pass the system-host filter below.
pub fn resolve_executable_target(raw_target: &str) -> Option<String> {
    let expanded = crate::system::env::expand_env(raw_target.trim().trim_matches('"'));
    if expanded.is_empty() || expanded.contains('%') {
        return None;
    }

    let direct = PathBuf::from(&expanded);
    let mut candidates = Vec::new();
    candidates.push(direct.clone());

    if is_bare_path(&direct) {
        if let Some(path) = std::env::var_os("PATH") {
            for segment in
                std::env::split_paths(&path).filter(|segment| !segment.as_os_str().is_empty())
            {
                candidates.push(segment.join(&direct));
            }
        }
        let system_root = system_root();
        candidates.push(system_root.join("System32").join(&direct));
    }

    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(path_key(candidate)))
        .find(|candidate| candidate.is_file() && is_allowed_target_path(candidate))
        .map(|candidate| candidate.to_string_lossy().to_string())
}

/// Return whether a URI scheme can represent a user-facing application.
///
/// Website schemes and Windows/Shell namespace schemes are intentionally
/// excluded even when they happen to point at an executable host.  The target
/// path is filtered separately, so this function remains a cheap pure policy
/// boundary for callers and tests.
pub fn is_allowed_protocol(protocol: &str) -> bool {
    let protocol = protocol.trim();
    let mut chars = protocol.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic()
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
        || protocol.len() > 64
    {
        return false;
    }

    let lower = protocol.to_ascii_lowercase();
    const WEB_SCHEMES: &[&str] = &[
        "http",
        "https",
        "ftp",
        "file",
        "mailto",
        "data",
        "javascript",
        "vbscript",
        "about",
        "blob",
        "ws",
        "wss",
        "gopher",
        "gopher+",
        "news",
        "nntp",
        "telnet",
        "wais",
        "irc",
        "ircs",
        "webcal",
        "webcals",
        "tel",
        "callto",
        "sms",
        "sip",
        "sips",
        "geo",
        "maps",
        "market",
        "itms-services",
        "intent",
        "chrome",
        "edge",
        "view-source",
        "ms-browser-extension",
    ];
    if WEB_SCHEMES.iter().any(|scheme| *scheme == lower) {
        return false;
    }

    const INTERNAL_SCHEMES: &[&str] = &[
        "shell",
        "res",
        "resource",
        "search",
        "query",
        "run",
        "appx",
        "ms-appx",
        "ms-appx-web",
        "ms-resource",
        "ms-settings",
        "ms-settingsfile",
        "ms-uup",
        "ms-windows-store",
        "ms-gamebarservices",
        "ms-gamingoverlay",
        "ms-xbox",
        "ms-screenclip",
    ];
    if INTERNAL_SCHEMES.iter().any(|scheme| *scheme == lower) {
        return false;
    }

    // New Windows namespace schemes are generally introduced with an `ms-`
    // prefix.  Treat that namespace as internal by default, while allowing
    // ordinary third-party names such as `steam`, `zoommtg`, and `vscode`.
    !lower.starts_with("ms-") && !lower.starts_with("windows-") && !lower.starts_with("windows.")
}

/// Return whether a path has an executable extension and is safe to expose as
/// a protocol application's target.  Existence is checked by
/// [`resolve_executable_target`] because this policy helper is also useful for
/// deterministic unit tests and for callers that already performed the check.
pub fn is_allowed_target_path(path: &Path) -> bool {
    if !is_executable_path(path.to_string_lossy().as_ref()) {
        return false;
    }

    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    const SYSTEM_HOSTS: &[&str] = &[
        "applicationframehost.exe",
        "cmd.exe",
        "command.com",
        "conhost.exe",
        "control.exe",
        "cscript.exe",
        "dllhost.exe",
        "explorer.exe",
        "mmc.exe",
        "mshta.exe",
        "msiexec.exe",
        "powershell.exe",
        "pwsh.exe",
        "regsvr32.exe",
        "rundll32.exe",
        "runtimebroker.exe",
        "searchapp.exe",
        "sihost.exe",
        "start.exe",
        "svchost.exe",
        "taskhostw.exe",
        "wscript.exe",
    ];
    if SYSTEM_HOSTS.iter().any(|name| *name == file_name) {
        return false;
    }

    let candidate = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let candidate_key = path_key(&candidate);
    let root_key = path_key(&system_root());
    if candidate_key == root_key || candidate_key.starts_with(&(root_key.clone() + "\\")) {
        return false;
    }

    // Package aliases and UWP launchers under WindowsApps are not ordinary
    // desktop targets; activating them requires the package identity/AUMID.
    !candidate_key
        .split('\\')
        .any(|component| component.eq_ignore_ascii_case("windowsapps"))
}

/// Add protocol names to existing apps by target, then create one searchable
/// `AppItem` for each target that has no existing application entry.
///
/// Matching intentionally ignores the command's static arguments: a protocol
/// is an alias for the installed application, and an existing Start Menu item
/// should gain that alias even if its shortcut has a different launch mode.
pub fn merge_or_materialize(items: &mut Vec<AppItem>, protocols: &[ProtocolEntry]) {
    for entry in protocols {
        if !is_allowed_protocol(&entry.protocol)
            || entry.target.is_empty()
            || !is_allowed_target_path(Path::new(&entry.target))
        {
            continue;
        }
        let target_key = path_key(Path::new(&entry.target));
        let mut matched = false;
        for item in items.iter_mut() {
            if path_key(Path::new(&item.target)) == target_key {
                push_keyword(item, &entry.protocol);
                matched = true;
            }
        }
        if !matched && should_materialize_entry(entry) {
            items.push(materialize_entry(entry));
        }
    }
}

fn should_materialize_entry(entry: &ProtocolEntry) -> bool {
    let path = Path::new(&entry.target);
    const AUXILIARY_MARKERS: &[&str] = &[
        "update",
        "helper",
        "daemon",
        "handler",
        "selector",
        "server",
        "host",
        "service",
        "runtime",
        "webview",
        "bootstrap",
    ];
    let stem_text = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if AUXILIARY_MARKERS
        .iter()
        .any(|marker| stem_text.contains(marker))
    {
        return false;
    }

    let compact = |value: &str| {
        value
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let protocol = compact(&entry.protocol);
    let stem = path
        .file_stem()
        .map(|value| compact(&value.to_string_lossy()))
        .unwrap_or_default();
    if path.components().any(|component| {
        let component = compact(&component.as_os_str().to_string_lossy());
        AUXILIARY_MARKERS.iter().any(|marker| component == *marker)
    }) {
        return false;
    }
    protocol.len() >= 3
        && stem.len() >= 3
        && (protocol == stem || protocol.starts_with(&stem) || stem.starts_with(&protocol))
}

fn materialize_entry(entry: &ProtocolEntry) -> AppItem {
    let working_dir = Path::new(&entry.target)
        .parent()
        .filter(|path| path.is_dir())
        .map(|path| path.to_string_lossy().to_string());
    let mut item = AppItem::scanned(
        stable_item_id(&entry.target, entry.args.as_deref()),
        entry.protocol.clone(),
        entry.target.clone(),
        entry.args.clone(),
        working_dir,
        SOURCE,
    );
    item.icon_src = Some(entry.target.clone());
    item.search_keywords.push(entry.protocol.clone());
    item.attach_search_fields();
    item
}

fn push_keyword(item: &mut AppItem, keyword: &str) {
    if !item
        .search_keywords
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(keyword))
    {
        item.search_keywords.push(keyword.to_string());
    }
}

fn split_command_target(command: &str) -> Option<(&str, &str)> {
    if let Some(rest) = command.strip_prefix('"') {
        let end = rest.find('"')?;
        let target = &rest[..end];
        let tail = &rest[end + 1..];
        if !tail.is_empty() && !tail.chars().next().is_some_and(char::is_whitespace) {
            return None;
        }
        return Some((target, tail));
    }

    let lower = command.to_ascii_lowercase();
    let mut start = 0usize;
    while let Some(offset) = lower[start..].find(".exe") {
        let end = start + offset + ".exe".len();
        let tail = &command[end..];
        if tail.is_empty() || tail.chars().next().is_some_and(char::is_whitespace) {
            let target = command[..end].trim();
            if !target.is_empty()
                && !target
                    .chars()
                    .any(|ch| ch.is_control() || matches!(ch, '"' | '&' | '|' | '<' | '>'))
            {
                return Some((target, tail));
            }
        }
        start = end;
        if start >= lower.len() {
            break;
        }
    }
    None
}

fn is_executable_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .map(|extension| extension.to_string_lossy().eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

fn is_bare_path(path: &Path) -> bool {
    let value = path.as_os_str().to_string_lossy();
    !value.is_empty()
        && !value.contains(['\\', '/', ':'])
        && path
            .file_name()
            .map(|name| name == path.as_os_str())
            .unwrap_or(false)
}

fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

fn path_key(path: &Path) -> String {
    let mut value = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    if let Some(stripped) = value.strip_prefix("\\\\?\\") {
        value = stripped.to_string();
    }
    while value.ends_with('\\') {
        value.pop();
    }
    value
}

fn collect_from_registry() -> Vec<ProtocolEntry> {
    let mut selected = HashMap::<String, ProtocolEntry>::new();
    let views = [None, Some(KEY_WOW64_32KEY.0)];

    // HKCU precedes HKLM to mirror HKCR's per-user override behavior.
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in views.iter().copied() {
            collect_hive(hive, view, &mut selected);
        }
    }

    let mut entries: Vec<_> = selected.into_values().collect();
    entries.sort_by(|a, b| a.protocol.cmp(&b.protocol));
    entries
}

fn collect_hive(hive: HKEY, view: Option<u32>, selected: &mut HashMap<String, ProtocolEntry>) {
    let Some(root) = open_registry_key(hive, CLASSES_PATH, view) else {
        return;
    };

    let mut index = 0u32;
    loop {
        let Some(protocol) = enum_subkey(root, index) else {
            break;
        };
        index += 1;

        if !is_allowed_protocol(&protocol) {
            continue;
        }
        let Some(protocol_key) = open_subkey(root, &protocol) else {
            continue;
        };
        let has_marker = read_registry_string(protocol_key, "URL Protocol").is_some();
        let candidate = if has_marker {
            open_subkey(protocol_key, r"shell\open\command")
                .and_then(|command_key| {
                    let command = read_registry_string(command_key, "");
                    close_key(command_key);
                    command
                })
                .and_then(|command| parse_open_command(&command))
                .and_then(|parsed| {
                    let target = resolve_executable_target(&parsed.target)?;
                    Some(ProtocolEntry::new(protocol.clone(), target, parsed.args))
                })
                .filter(|entry| is_allowed_protocol(&entry.protocol))
        } else {
            None
        };
        close_key(protocol_key);

        if let Some(entry) = candidate {
            selected.entry(entry.protocol.clone()).or_insert(entry);
        }
    }
    close_key(root);
}

fn open_registry_key(hive: HKEY, path: &str, view: Option<u32>) -> Option<HKEY> {
    let path = wide(path);
    let mut key = HKEY::default();
    let result = unsafe { RegOpenKeyExW(hive, PCWSTR(path.as_ptr()), view, KEY_READ, &mut key) };
    (result == ERROR_SUCCESS).then_some(key)
}

fn open_subkey(parent: HKEY, path: &str) -> Option<HKEY> {
    let path = wide(path);
    let mut key = HKEY::default();
    let result = unsafe { RegOpenKeyExW(parent, PCWSTR(path.as_ptr()), None, KEY_READ, &mut key) };
    (result == ERROR_SUCCESS).then_some(key)
}

fn close_key(key: HKEY) {
    unsafe {
        let _ = RegCloseKey(key);
    }
}

fn enum_subkey(key: HKEY, index: u32) -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut length = buffer.len() as u32;
    let result = unsafe {
        RegEnumKeyExW(
            key,
            index,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut length,
            None,
            None,
            None,
            None,
        )
    };
    result
        .is_ok()
        .then(|| String::from_utf16_lossy(&buffer[..length as usize]))
}

fn read_registry_string(key: HKEY, value: &str) -> Option<String> {
    let value = wide(value);
    let mut data = vec![0u8; 1024];
    let mut data_len = data.len() as u32;
    let mut ty = REG_SZ;
    let mut result = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(value.as_ptr()),
            None,
            Some(&mut ty),
            Some(data.as_mut_ptr()),
            Some(&mut data_len),
        )
    };
    if result == ERROR_MORE_DATA {
        if data_len == 0 {
            return None;
        }
        data.resize(data_len as usize, 0);
        result = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(value.as_ptr()),
                None,
                Some(&mut ty),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
    }
    if result != ERROR_SUCCESS || (ty != REG_SZ && ty != REG_EXPAND_SZ) {
        return None;
    }
    let len = usize::try_from(data_len).ok()?;
    if len > data.len() {
        return None;
    }
    let mut units = data[..len]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    while units.last() == Some(&0) {
        units.pop();
    }
    let text = String::from_utf16_lossy(&units);
    if ty == REG_EXPAND_SZ {
        Some(crate::system::env::expand_env(&text))
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_command_returns_exe_and_discards_protocol_placeholder() {
        let command = r#""C:\Program Files\Example App\example.exe" "%1""#;
        let parsed = parse_open_command(command).expect("quoted exe command should parse");
        assert_eq!(parsed.target, r"C:\Program Files\Example App\example.exe");
        assert_eq!(
            parsed.args, None,
            "%1 must never be forwarded as an argument"
        );
    }

    #[test]
    fn unquoted_path_with_spaces_uses_first_exe_suffix() {
        let command = r#"C:\Program Files\Example App\example.exe "%L""#;
        let parsed = parse_open_command(command).expect("unquoted exe command should parse");
        assert_eq!(parsed.target, r"C:\Program Files\Example App\example.exe");
        assert_eq!(parsed.args, None);
    }

    #[test]
    fn static_arguments_survive_but_placeholder_arguments_do_not() {
        let static_command = r#""C:\Apps\Example\example.exe" --background"#;
        assert_eq!(
            parse_open_command(static_command)
                .expect("static command should parse")
                .args
                .as_deref(),
            Some("--background")
        );

        let mixed_command = r#""C:\Apps\Example\example.exe" --open "%1""#;
        assert_eq!(
            parse_open_command(mixed_command)
                .expect("mixed command should parse")
                .args,
            None,
            "dynamic protocol parameters must be omitted from a launch record"
        );
    }

    #[test]
    fn protocol_filter_accepts_app_scheme_and_rejects_web_or_internal_scheme() {
        assert!(is_allowed_protocol("steam"));
        assert!(is_allowed_protocol("zoommtg"));
        for protocol in [
            "http",
            "https",
            "file",
            "mailto",
            "javascript",
            "shell",
            "ms-settings",
            "ms-appx",
        ] {
            assert!(
                !is_allowed_protocol(protocol),
                "protocol must be filtered: {protocol}"
            );
        }
        assert!(!is_allowed_protocol("9starts-with-a-digit"));
        assert!(!is_allowed_protocol("has/slash"));
    }

    #[test]
    fn target_filter_rejects_system_hosts_and_non_executables() {
        assert!(!is_allowed_target_path(Path::new(
            r"C:\Windows\System32\explorer.exe"
        )));
        assert!(!is_allowed_target_path(Path::new(
            r"C:\Users\admin\AppData\Local\WindowsApps\example.exe"
        )));
        assert!(!is_allowed_target_path(Path::new(
            r"C:\Apps\Example\example.dll"
        )));
        assert!(!is_allowed_target_path(Path::new(
            r"C:\Apps\Example\cmd.exe"
        )));
    }

    #[test]
    fn protocol_keywords_merge_by_target_and_materialize_unmatched_apps() {
        let existing_target = r"C:\Apps\Example\example.exe";
        let mut apps = vec![AppItem::scanned(
            "existing".into(),
            "Example App".into(),
            existing_target.into(),
            None,
            None,
            "start-menu",
        )];
        let protocols = vec![
            ProtocolEntry::new("example", existing_target.to_string(), None),
            ProtocolEntry::new(
                "new-app",
                r"C:\Apps\New\new.exe".to_string(),
                Some("--safe".into()),
            ),
        ];

        merge_or_materialize(&mut apps, &protocols);

        assert_eq!(apps.len(), 2);
        assert!(apps[0]
            .search_keywords
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case("example")));
        let generated = apps
            .iter()
            .find(|item| item.target.ends_with(r"new.exe"))
            .expect("unmatched protocol should create a searchable app");
        assert_eq!(generated.name, "new-app");
        assert_eq!(generated.args.as_deref(), Some("--safe"));
        assert!(generated
            .search_keywords
            .iter()
            .any(|keyword| keyword == "new-app"));
    }

    #[test]
    fn unmatched_helper_protocols_do_not_create_launcher_rows() {
        let mut apps = Vec::new();
        merge_or_materialize(
            &mut apps,
            &[
                ProtocolEntry::new(
                    "auphd",
                    r"C:\Adobe\Update Helper\Adobe Update Helper.exe",
                    None,
                ),
                ProtocolEntry::new(
                    "vsweb+diag",
                    r"C:\VisualStudio\VsWebProtocolSelector.exe",
                    None,
                ),
            ],
        );
        assert!(apps.is_empty());
    }
}
