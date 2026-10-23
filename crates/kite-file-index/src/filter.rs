use serde::{Deserialize, Serialize};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileFilter {
    #[default]
    All,
    Images,
    Documents,
    Videos,
    Audio,
    Archives,
    Folders,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Query {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub filter: FileFilter,
    #[serde(default)]
    pub excluded_folders: Vec<PathBuf>,
}
fn default_limit() -> usize {
    20
}
#[link(name = "Kernel32")]
extern "system" {
    fn CompareStringOrdinal(
        a: *const u16,
        alen: i32,
        b: *const u16,
        blen: i32,
        ignore_case: i32,
    ) -> i32;
}
pub(crate) struct Filter {
    kind: FileFilter,
    roots: Vec<Vec<u16>>,
}
impl Filter {
    pub fn new(query: &Query) -> crate::Result<Self> {
        if query.limit > 1000 {
            return Err(crate::invalid("limit must be 0..1000").into());
        }
        let mut roots = Vec::new();
        for root in &query.excluded_folders {
            let mut units: Vec<u16> = root
                .as_os_str()
                .encode_wide()
                .map(|u| if u == 47 { 92 } else { u })
                .collect();
            if units.len() < 3
                || !(units[0] <= 127 && (units[0] as u8).is_ascii_alphabetic())
                || units[1] != 58
                || units[2] != 92
                || units.iter().any(|&u| u == 0)
            {
                return Err(
                    crate::invalid("excluded folder must be an absolute drive path").into(),
                );
            }
            // Lexical dot components cannot safely describe a directory boundary.
            if units[3..]
                .split(|&u| u == 92)
                .any(|part| part == [46] || part == [46, 46])
            {
                return Err(crate::invalid("excluded folder contains dot components").into());
            }
            while units.last() == Some(&92) {
                units.pop();
            }
            roots.push(units);
        }
        Ok(Self {
            kind: query.filter,
            roots,
        })
    }
    pub fn active(&self) -> bool {
        self.kind != FileFilter::All || !self.roots.is_empty()
    }
    pub fn accepts(&self, path: &[u16], is_dir: bool) -> bool {
        for root in &self.roots {
            if path.len() >= root.len()
                && (path.len() == root.len() || matches!(path[root.len()], 47 | 92))
                && unsafe {
                    CompareStringOrdinal(
                        path.as_ptr(),
                        root.len() as i32,
                        root.as_ptr(),
                        root.len() as i32,
                        1,
                    )
                } == 2
            {
                return false;
            }
        }
        if self.kind == FileFilter::All {
            return true;
        }
        if self.kind == FileFilter::Folders {
            return is_dir;
        }
        if is_dir {
            return false;
        }
        let name = path.rsplit(|&u| u == 47 || u == 92).next().unwrap_or(path);
        let Some(dot) = name.iter().rposition(|&u| u == 46) else {
            return false;
        };
        let ext = String::from_utf16_lossy(&name[dot + 1..]).to_ascii_lowercase();
        let list = match self.kind {
            FileFilter::Images => "jpg;jpeg;png;gif;webp;bmp;ico;svg;heic;heif;avif;tif;tiff",
            FileFilter::Documents => "pdf;doc;docx;xls;xlsx;ppt;pptx;txt;md;rtf;odt;ods;odp;csv",
            FileFilter::Videos => "mp4;mkv;mov;avi;wmv;webm;flv;m4v;mpeg;mpg",
            FileFilter::Audio => "mp3;wav;flac;aac;m4a;ogg;wma;opus",
            FileFilter::Archives => "zip;7z;rar;tar;gz;bz2;xz;iso",
            _ => "",
        };
        list.split(';').any(|candidate| candidate == ext)
    }
}
