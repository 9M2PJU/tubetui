//! Local files: the "Ver después" list and the cache of my channel ids.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use crate::domain::Video;

#[derive(Debug, Clone)]
pub struct Paths {
    /// `later.json`, newest first.
    pub later: PathBuf,
    /// One channel id per line.
    pub subs: PathBuf,
    /// Cookie exports.
    pub cache: PathBuf,
    /// Videos watched in tubetui, one id per line, oldest first.
    pub watched: PathBuf,
}

impl Paths {
    /// Platform data and cache dirs (`~/.local/share/tubetui`, `~/.cache/tubetui`).
    pub fn system() -> Option<Self> {
        let dirs = directories::ProjectDirs::from("", "", "tubetui")?;
        Some(Self::new(dirs.data_dir(), dirs.cache_dir()))
    }

    pub fn new(data: &Path, cache: &Path) -> Self {
        Self {
            later: data.join("later.json"),
            subs: cache.join("subs.txt"),
            cache: cache.into(),
            watched: data.join("watched.txt"),
        }
    }
}

pub fn load_later(path: &Path) -> io::Result<Vec<Video>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

pub fn save_later(path: &Path, videos: &[Video]) -> io::Result<()> {
    write_atomic(path, &serde_json::to_string_pretty(videos).map_err(io::Error::other)?)
}

pub fn load_subs(path: &Path) -> HashSet<String> {
    std::fs::read_to_string(path).map(|text| text.lines().map(str::to_owned).collect()).unwrap_or_default()
}

pub fn save_subs(path: &Path, subs: &HashSet<String>) -> io::Result<()> {
    let mut ids: Vec<&str> = subs.iter().map(String::as_str).collect();
    ids.sort_unstable();
    write_atomic(path, &(ids.join("\n") + "\n"))
}

/// Ids kept in `watched.txt`; older ones are dropped beyond this.
const WATCHED_MAX: usize = 5000;

pub fn load_watched(path: &Path) -> HashSet<String> {
    load_subs(path)
}

/// Adds `id` to `watched.txt`, keeping the newest [`WATCHED_MAX`].
pub fn add_watched(path: &Path, id: &str) -> io::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut ids: Vec<&str> = text.lines().filter(|line| !line.is_empty() && *line != id).collect();
    ids.push(id);
    let keep = &ids[ids.len().saturating_sub(WATCHED_MAX)..];
    write_atomic(path, &(keep.join("\n") + "\n"))
}

/// Reads the "Ver después" list of the old `ytv` script
/// (`id, canal, título, duración, url del canal, fecha, id del canal` per
/// line, oldest first) and returns it newest first.
pub fn parse_ytv_later(text: &str) -> Vec<Video> {
    let clean = |s: &str| strip_ansi(s).trim().to_owned();
    let mut videos: Vec<Video> = text
        .lines()
        .filter_map(|line| {
            let fields: Vec<String> = line.split('\t').map(clean).collect();
            let field = |i: usize| fields.get(i).filter(|f| !f.is_empty() && *f != "-").cloned();
            Some(Video {
                id: field(0)?,
                title: field(2)?,
                channel: field(1).unwrap_or_default(),
                channel_id: field(6),
                channel_url: field(4),
                duration: field(3).as_deref().and_then(parse_duration),
                published: None,
                live: Default::default(),
            })
        })
        .collect();
    videos.reverse();
    videos
}

/// `"1:02:03"` / `"23:33"` → seconds.
fn parse_duration(text: &str) -> Option<u32> {
    text.split(':').try_fold(0u32, |total, part| Some(total * 60 + part.parse::<u32>().ok()?))
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip `ESC [ ... letter`.
            chars.by_ref().take_while(|c| !c.is_ascii_alphabetic()).for_each(drop);
        } else {
            out.push(c);
        }
    }
    out
}

/// Write to a sibling temp file and rename, so a crash never leaves half a file.
fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::temp_dir;
    use crate::domain::video;

    #[test]
    fn watched_ids_are_remembered_once() {
        let path = temp_dir("watched").join("data/watched.txt");
        assert!(load_watched(&path).is_empty());
        add_watched(&path, "a").unwrap();
        add_watched(&path, "b").unwrap();
        add_watched(&path, "a").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "b\na\n", "no repeats, newest last");
        assert_eq!(load_watched(&path), HashSet::from(["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn later_round_trips_and_missing_is_empty() {
        let path = temp_dir("later").join("data/later.json");
        assert!(load_later(&path).unwrap().is_empty());
        save_later(&path, &[video("a"), video("b")]).unwrap();
        assert_eq!(load_later(&path).unwrap(), vec![video("a"), video("b")]);
    }

    #[test]
    fn corrupt_later_is_an_error_not_an_empty_list() {
        // Silently treating it as empty would wipe the list on the next save.
        let path = temp_dir("corrupt").join("later.json");
        std::fs::write(&path, "{oops").unwrap();
        assert!(load_later(&path).is_err());
    }

    #[test]
    fn imports_ytv_later_list_newest_first() {
        let text = "old1\t\u{1b}[35mCanal A   \u{1b}[0m\tViejo\t\u{1b}[90m1:02:03\u{1b}[0m\thttps://www.youtube.com/@a\t06/10/26 3d\tUCa\n\
                    new2\t-                   \tNuevo\t23:33\t\t\t\n\
                    broken line\n";
        let videos = parse_ytv_later(text);
        assert_eq!(videos.len(), 2);
        assert_eq!(videos[0].id, "new2");
        assert_eq!(videos[0].channel, "");
        assert_eq!(videos[0].channel_id, None);
        assert_eq!(videos[0].duration, Some(23 * 60 + 33));
        assert_eq!(videos[1].channel, "Canal A");
        assert_eq!(videos[1].channel_id.as_deref(), Some("UCa"));
        assert_eq!(videos[1].channel_url.as_deref(), Some("https://www.youtube.com/@a"));
        assert_eq!(videos[1].duration, Some(3723));
    }

    #[test]
    fn subs_round_trip() {
        let path = temp_dir("subs").join("subs.txt");
        assert!(load_subs(&path).is_empty());
        let subs = HashSet::from(["UCb".to_string(), "UCa".to_string()]);
        save_subs(&path, &subs).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "UCa\nUCb\n");
        assert_eq!(load_subs(&path), subs);
    }
}
