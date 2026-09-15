//! Internet shortcuts for installed apps that register a launch protocol.

use std::path::Path;

pub(super) fn is_app_protocol_shortcut(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    // Shortcuts are small INI files. Ignore malformed/oversized files rather
    // than making the Start Menu scan process arbitrary data.
    if bytes.len() > 64 * 1024 {
        return false;
    }
    let content = if bytes.starts_with(&[0xff, 0xfe]) {
        let chars = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&chars)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };

    let mut in_shortcut = false;
    for line in content.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_shortcut = line.eq_ignore_ascii_case("[InternetShortcut]");
        } else if in_shortcut {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim().eq_ignore_ascii_case("URL") {
                    return is_app_protocol(value.trim());
                }
            }
        }
    }
    false
}

fn is_app_protocol(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return false;
    }
    ![
        "http",
        "https",
        "ftp",
        "file",
        "mailto",
        "data",
        "javascript",
    ]
    .iter()
    .any(|web| scheme.eq_ignore_ascii_case(web))
}
