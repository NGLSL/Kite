//! 文件模式类别：按目录身份与扩展名筛选文件搜索结果。

use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

impl FileFilter {
    pub const ALL: [Self; 7] = [
        Self::All,
        Self::Images,
        Self::Documents,
        Self::Videos,
        Self::Audio,
        Self::Archives,
        Self::Folders,
    ];

    pub fn matches(self, path: impl AsRef<Path>, is_folder: bool) -> bool {
        if self == Self::All {
            return true;
        }
        if self == Self::Folders {
            return is_folder;
        }
        if is_folder {
            return false;
        }
        let extensions: &[&str] = match self {
            Self::Images => &[
                "jpg", "jpeg", "png", "gif", "webp", "bmp", "ico", "svg", "heic", "heif", "avif",
                "tif", "tiff",
            ],
            Self::Documents => &[
                "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "odt",
                "ods", "odp", "csv",
            ],
            Self::Videos => &[
                "mp4", "mkv", "mov", "avi", "wmv", "webm", "flv", "m4v", "mpeg", "mpg",
            ],
            Self::Audio => &["mp3", "wav", "flac", "aac", "m4a", "ogg", "wma", "opus"],
            Self::Archives => &["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "iso"],
            Self::All | Self::Folders => unreachable!(),
        };
        path.as_ref()
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                extensions
                    .iter()
                    .any(|expected| ext.eq_ignore_ascii_case(expected))
            })
    }
}

impl fmt::Display for FileFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::All => "全部",
            Self::Images => "图片",
            Self::Documents => "文档",
            Self::Videos => "视频",
            Self::Audio => "音频",
            Self::Archives => "压缩包",
            Self::Folders => "文件夹",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_extensions_match_without_case_sensitivity() {
        for (filter, extensions) in [
            (
                FileFilter::Images,
                "jpg jpeg png gif webp bmp ico svg heic heif avif tif tiff",
            ),
            (
                FileFilter::Documents,
                "pdf doc docx xls xlsx ppt pptx txt md rtf odt ods odp csv",
            ),
            (
                FileFilter::Videos,
                "mp4 mkv mov avi wmv webm flv m4v mpeg mpg",
            ),
            (FileFilter::Audio, "mp3 wav flac aac m4a ogg wma opus"),
            (FileFilter::Archives, "zip 7z rar tar gz bz2 xz iso"),
        ] {
            for ext in extensions.split_whitespace() {
                let path = format!(r"C:\files\item.{}", ext.to_uppercase());
                assert!(filter.matches(&path, false), "{filter}: {path}");
                assert!(!filter.matches(&path, true), "directory: {path}");
                for other in FileFilter::ALL {
                    if other != filter && other != FileFilter::All {
                        assert!(!other.matches(&path, false), "{other}: {path}");
                    }
                }
            }
        }
    }

    #[test]
    fn folders_unknown_extensions_and_extensionless_paths_are_distinct() {
        for path in [
            r"C:\files\folder.txt",
            r"C:\files\README",
            r"C:\files\item.unknown",
        ] {
            assert!(FileFilter::All.matches(path, false));
            assert!(FileFilter::All.matches(path, true));
            assert!(FileFilter::Folders.matches(path, true));
            assert!(!FileFilter::Folders.matches(path, false));
        }
        for filter in [
            FileFilter::Images,
            FileFilter::Documents,
            FileFilter::Videos,
            FileFilter::Audio,
            FileFilter::Archives,
        ] {
            assert!(!filter.matches(r"C:\files\README", false));
            assert!(!filter.matches(r"C:\files\item.unknown", false));
        }
        assert!(FileFilter::Archives.matches(r"C:\files\backup.tar.gz", false));
    }

    #[test]
    fn filter_labels_stay_in_existing_order() {
        assert_eq!(
            FileFilter::ALL.map(|filter| filter.to_string()),
            ["全部", "图片", "文档", "视频", "音频", "压缩包", "文件夹"]
        );
    }
}
