//! Search aliases from executable version resources.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};

const MAX_VERSION_RESOURCE: u32 = 16 * 1024 * 1024;
const FIELDS: [&str; 4] = [
    "FileDescription",
    "ProductName",
    "InternalName",
    "OriginalFilename",
];

pub(super) fn executable_keywords(path: &Path) -> Vec<String> {
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return Vec::new();
    }
    let wide_path = wide_os(path.as_os_str());
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(wide_path.as_ptr()), None) };
    if size == 0 || size > MAX_VERSION_RESOURCE {
        return Vec::new();
    }
    let mut block = vec![0u8; size as usize];
    if unsafe {
        GetFileVersionInfoW(
            PCWSTR(wide_path.as_ptr()),
            None,
            size,
            block.as_mut_ptr().cast(),
        )
    }
    .is_err()
    {
        return Vec::new();
    }

    let mut translations = query_translations(&block);
    if !translations.contains(&(0x0409, 0x04b0)) {
        translations.push((0x0409, 0x04b0));
    }
    let mut values = Vec::new();
    for field in FIELDS {
        for &(language, code_page) in &translations {
            let key = format!(r"\StringFileInfo\{language:04x}{code_page:04x}\{field}");
            if let Some(value) = query_string(&block, &key) {
                if value.len() <= 512
                    && !values
                        .iter()
                        .any(|known: &String| known.eq_ignore_ascii_case(&value))
                {
                    values.push(value);
                }
                break;
            }
        }
    }
    values
}

fn query_translations(block: &[u8]) -> Vec<(u16, u16)> {
    let Some((ptr, len)) = query_value(block, r"\VarFileInfo\Translation") else {
        return Vec::new();
    };
    let count = len as usize / 4;
    let words = (0..count * 2)
        .map(|index| unsafe { ptr.cast::<u16>().add(index).read_unaligned() })
        .collect::<Vec<_>>();
    words
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

fn query_string(block: &[u8], key: &str) -> Option<String> {
    let (ptr, len) = query_value(block, key)?;
    let units = (0..len as usize)
        .map(|index| unsafe { ptr.cast::<u16>().add(index).read_unaligned() })
        .collect::<Vec<_>>();
    let value = String::from_utf16_lossy(&units)
        .trim_end_matches('\0')
        .trim()
        .to_string();
    (!value.is_empty()).then_some(value)
}

fn query_value(block: &[u8], key: &str) -> Option<(*mut c_void, u32)> {
    let wide_key = wide(key);
    let mut ptr = std::ptr::null_mut();
    let mut len = 0u32;
    let ok = unsafe {
        VerQueryValueW(
            block.as_ptr().cast(),
            PCWSTR(wide_key.as_ptr()),
            &mut ptr,
            &mut len,
        )
    };
    (ok.as_bool() && !ptr.is_null() && len > 0).then_some((ptr, len))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_os(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_searchable_metadata_from_windows_executable() {
        let windows = std::env::var_os("WINDIR").unwrap();
        let fields = executable_keywords(&Path::new(&windows).join("System32/notepad.exe"));
        assert!(
            !fields.is_empty(),
            "Notepad should expose version search metadata"
        );
    }

    #[test]
    fn invalid_executable_has_no_metadata() {
        let file = std::env::temp_dir().join(format!("kite-metadata-{}.exe", std::process::id()));
        std::fs::write(&file, b"fixture").unwrap();
        assert!(executable_keywords(&file).is_empty());
        let _ = std::fs::remove_file(file);
    }
}
