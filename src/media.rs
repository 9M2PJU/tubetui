//! Local media: video and music files found under the configured folders.

use std::path::{Path, PathBuf};

/// Deep enough for `Música/Artista/Disco/01.flac`, shallow enough to stay fast.
const MAX_DEPTH: usize = 6;

const VIDEO: &[&str] = &["mp4", "mkv", "webm", "avi", "mov", "m4v", "mpg", "mpeg", "wmv", "flv", "ts"];
const MUSIC: &[&str] = &["mp3", "flac", "ogg", "opus", "m4a", "wav", "aac", "wma", "alac", "aiff"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Music,
}

impl MediaKind {
    fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Video => VIDEO,
            Self::Music => MUSIC,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub path: PathBuf,
    /// File name without extension.
    pub name: String,
    /// Path of the containing folder relative to the scanned root, e.g. `Artista/Disco`.
    pub folder: String,
}

/// Media files of `kind` under `roots`, sorted by folder and name. Hidden
/// entries are skipped and unreadable folders ignored.
pub fn scan(roots: &[PathBuf], kind: MediaKind) -> Vec<LocalFile> {
    let mut files = Vec::new();
    for root in roots {
        walk(root, root, kind, 0, &mut files);
    }
    files.sort_by(|a, b| {
        (a.folder.to_lowercase(), a.name.to_lowercase()).cmp(&(b.folder.to_lowercase(), b.name.to_lowercase()))
    });
    files.dedup_by(|a, b| a.path == b.path);
    files
}

fn walk(root: &Path, dir: &Path, kind: MediaKind, depth: usize, files: &mut Vec<LocalFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let hidden = entry.file_name().to_string_lossy().starts_with('.');
        if hidden {
            continue;
        }
        if path.is_dir() {
            if depth < MAX_DEPTH {
                walk(root, &path, kind, depth + 1, files);
            }
        } else if has_extension(&path, kind.extensions()) {
            let folder = dir.strip_prefix(root).unwrap_or(dir).display().to_string();
            let root_name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            files.push(LocalFile {
                name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                folder: if folder.is_empty() { root_name } else { folder },
                path,
            });
        }
    }
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// `"~/Videos, /mnt/pelis"` → paths, ignoring blanks.
pub fn parse_dirs(text: &str) -> Vec<String> {
    text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::temp_dir;

    fn touch(root: &Path, file: &str) {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn finds_media_by_kind_recursively_and_sorted() {
        let root = temp_dir("media");
        for file in ["b.MKV", "a.mp4", "Serie/T1/e01.webm", "notas.txt", ".oculto/x.mp4", "Disco/01 Tema.flac"] {
            touch(&root, file);
        }
        let videos = scan(std::slice::from_ref(&root), MediaKind::Video);
        let names: Vec<_> = videos.iter().map(|f| (f.folder.as_str(), f.name.as_str())).collect();
        let root_name = root.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(names, [("Serie/T1", "e01"), (root_name.as_str(), "a"), (root_name.as_str(), "b")]);
        let music = scan(&[root], MediaKind::Music);
        assert_eq!(music.len(), 1);
        assert_eq!((music[0].folder.as_str(), music[0].name.as_str()), ("Disco", "01 Tema"));
    }

    #[test]
    fn missing_folders_and_duplicates_are_harmless() {
        let root = temp_dir("dupes");
        touch(&root, "a.mp3");
        let files = scan(&[root.clone(), root, PathBuf::from("/no/such/dir")], MediaKind::Music);
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn dir_lists_parse_from_text() {
        assert_eq!(parse_dirs(" ~/Videos , /mnt/pelis,, "), ["~/Videos", "/mnt/pelis"]);
        assert!(parse_dirs("  ").is_empty());
    }
}
