use std::path::Path;

/// Coarse file-type buckets used to color the treemap, mirroring the
/// category grouping WinDirStat/TreeSize use so files of a kind cluster
/// visually even before you read a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Directory,
    Archive,
    Audio,
    Video,
    Image,
    Document,
    Code,
    Executable,
    System,
    Other,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Directory => "directory",
            Category::Archive => "archive",
            Category::Audio => "audio",
            Category::Video => "video",
            Category::Image => "image",
            Category::Document => "document",
            Category::Code => "code",
            Category::Executable => "executable",
            Category::System => "system",
            Category::Other => "other",
        }
    }
}

pub fn categorize(path: &Path, is_dir: bool) -> Category {
    if is_dir {
        return Category::Directory;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" => Category::Archive,
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" => Category::Audio,
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "wmv" | "m4v" => Category::Video,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "heic" | "tiff" => {
            Category::Image
        }
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "rtf" | "odt" => {
            Category::Document
        }
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp" | "h" | "hpp"
        | "swift" | "rb" | "cs" | "json" | "yaml" | "yml" | "toml" | "html" | "css" => {
            Category::Code
        }
        "exe" | "msi" | "app" | "dmg" | "deb" | "rpm" | "appimage" => Category::Executable,
        "dll" | "so" | "dylib" | "sys" | "log" | "tmp" | "cache" => Category::System,
        _ => Category::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn directories_are_always_directory_category() {
        assert_eq!(
            categorize(&PathBuf::from("anything.mp3"), true),
            Category::Directory
        );
    }

    #[test]
    fn known_extensions_map_correctly() {
        assert_eq!(
            categorize(&PathBuf::from("song.mp3"), false),
            Category::Audio
        );
        assert_eq!(
            categorize(&PathBuf::from("movie.mkv"), false),
            Category::Video
        );
        assert_eq!(categorize(&PathBuf::from("main.rs"), false), Category::Code);
        assert_eq!(
            categorize(&PathBuf::from("setup.exe"), false),
            Category::Executable
        );
    }

    #[test]
    fn unknown_extension_falls_back_to_other() {
        assert_eq!(
            categorize(&PathBuf::from("data.xyz123"), false),
            Category::Other
        );
    }
}
