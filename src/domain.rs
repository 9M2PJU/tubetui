//! Plain data shared by every layer.

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

const DAY: i64 = 86_400;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Live {
    #[default]
    No,
    Now,
    Upcoming,
}

/// A YouTube video as listed in feeds, searches and "Ver después".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub channel_url: Option<String>,
    /// Seconds; `None` for live streams or when YouTube does not report it.
    #[serde(default)]
    pub duration: Option<u32>,
    /// Unix time, often approximate ("hace 3 días").
    #[serde(default)]
    pub published: Option<i64>,
    #[serde(default)]
    pub live: Live,
    /// A YouTube Short (vertical, under three minutes).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub short: bool,
}

impl Video {
    /// Mixes from the home feed come as `RD<first video id>` playlists.
    pub fn url(&self) -> String {
        match self.id.strip_prefix("RD") {
            Some(rest) if rest.len() >= 11 => {
                format!("https://www.youtube.com/watch?v={}&list={}", &rest[..11], self.id)
            }
            _ => format!("https://www.youtube.com/watch?v={}", self.id),
        }
    }

    /// `m:ss` or `h:mm:ss`, empty when unknown.
    pub fn duration_label(&self) -> String {
        self.duration.map(clock).unwrap_or_default()
    }

    /// Newest first; live streams, then upcoming ones, then by date, unknown dates last.
    pub fn recency(&self) -> i64 {
        match (self.live, self.published) {
            (Live::Now, _) => i64::MAX,
            (Live::Upcoming, _) => i64::MAX - 1,
            (Live::No, Some(ts)) => ts,
            (Live::No, None) => i64::MIN,
        }
    }
}

/// A video I opened in tubetui, for "Historial" and to resume it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub video: Video,
    /// Unix time I last opened it.
    pub at: i64,
    /// Where I left it, in seconds; `None` when finished or barely started.
    #[serde(default)]
    pub position: Option<u32>,
}

/// A video downloaded (or being downloaded) with `d`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Download {
    pub id: u64,
    pub video: Video,
    pub state: DownloadState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadState {
    /// Tenths of a percent of the part being downloaded (video, then audio),
    /// and yt-dlp's speed and time left.
    Running {
        permille: u16,
        speed: String,
        eta: String,
    },
    Done(std::path::PathBuf),
    Failed(String),
    Cancelled,
}

/// `"  45.3%| 1.12MiB/s|00:12"` (our `--progress-template`) → (453, "1.12MiB/s", "00:12").
pub fn parse_progress(line: &str) -> Option<(u16, String, String)> {
    let mut parts = line.split('|').map(str::trim);
    let percent: f64 = parts.next()?.trim_end_matches('%').trim().parse().ok()?;
    let field =
        |s: Option<&str>| s.filter(|s| !s.is_empty() && *s != "NA" && !s.contains("Unknown")).unwrap_or("").to_owned();
    let speed = field(parts.next());
    let eta = field(parts.next());
    Some(((percent.clamp(0.0, 100.0) * 10.0).round() as u16, speed, eta))
}

/// What a video weighs at the chosen quality, as yt-dlp sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub bytes: Option<u64>,
    /// Height of the video part: 720 for 720p.
    pub height: Option<u32>,
    /// Seconds.
    pub duration: Option<u32>,
}

impl Size {
    /// `"≈ 28.5 MB (720p)"`; empty when yt-dlp did not know.
    pub fn label(&self) -> String {
        let Some(bytes) = self.bytes else { return String::new() };
        match self.height {
            Some(h) => format!("≈ {} ({h}p)", bytes_label(bytes)),
            None => format!("≈ {}", bytes_label(bytes)),
        }
    }
}

/// `"29889635|213.5|720"` (`--print "%(filesize,filesize_approx)s|%(duration)s|%(height)s"`).
pub fn parse_size(line: &str) -> Size {
    let mut parts = line.trim().split('|').map(|p| p.trim().parse::<f64>().ok().filter(|n| *n > 0.0));
    let mut next = || parts.next().flatten();
    let bytes = next().map(|n| n as u64);
    let duration = next().map(|n| n.round() as u32);
    Size { bytes, height: next().map(|n| n as u32), duration }
}

/// `"850 KB"`, `"28.5 MB"`, `"1.2 GB"`.
pub fn bytes_label(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e7 {
        format!("{:.0} MB", b / 1e6)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

/// How many videos of each channel I opened since `since` (unix time), by channel id.
pub fn channel_views(history: &[HistoryEntry], since: i64) -> std::collections::HashMap<String, usize> {
    let mut views = std::collections::HashMap::new();
    for entry in history.iter().filter(|e| e.at >= since) {
        if let Some(channel) = &entry.video.channel_id {
            *views.entry(channel.clone()).or_default() += 1;
        }
    }
    views
}

/// The channels I watch most: at least `min` videos, the `max` with most.
pub fn favorite_channels(
    views: &std::collections::HashMap<String, usize>,
    min: usize,
    max: usize,
) -> std::collections::HashSet<String> {
    let mut ranked: Vec<(&String, &usize)> = views.iter().filter(|(_, n)| **n >= min).collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    ranked.into_iter().take(max).map(|(id, _)| id.clone()).collect()
}

/// Seconds worth resuming from: not the first half minute, not the end.
pub fn resume_point(position: f64, duration: Option<f64>) -> Option<u32> {
    const MARGIN: f64 = 30.0;
    if !position.is_finite() || position < MARGIN {
        return None;
    }
    match duration {
        Some(d) if d - position < MARGIN || position / d > 0.95 => None,
        _ => Some(position as u32),
    }
}

/// Spanish `"09/10/26"` and `"hoy" | "ayer" | "5d" | "3m" | "2a"`;
/// English `"10/09/26"` and `"today" | "1d" | "5d" | "3mo" | "2y"`.
pub fn date_labels(timestamp: i64, now: i64, lang: Lang) -> (String, String) {
    let (y, m, d) = civil_from_days(timestamp.div_euclid(DAY));
    let y = y.rem_euclid(100);
    let days = (now - timestamp).max(0) / DAY;
    let age = match days {
        0 => lang.pick("hoy", "today").into(),
        1 => lang.pick("ayer", "1d").into(),
        2..30 => format!("{days}d"),
        30..365 => format!("{}{}", days / 30, lang.pick("m", "mo")),
        _ => format!("{}{}", days / 365, lang.pick("a", "y")),
    };
    let date = match lang {
        Lang::Es => format!("{d:02}/{m:02}/{y:02}"),
        Lang::En => format!("{m:02}/{d:02}/{y:02}"),
    };
    (date, age)
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// `0:05`, `10:35`, `1:02:03`.
pub fn clock(s: u32) -> String {
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub url: String,
    /// Already formatted, e.g. "358 mil suscriptores".
    pub followers: Option<String>,
    /// `@name`, the channel's address on YouTube.
    pub handle: Option<String>,
    /// What the channel says about itself, as YouTube shortens it.
    pub description: Option<String>,
}

/// One of my YouTube playlists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    /// As YouTube says it: `"12 videos"`.
    pub count: Option<String>,
}

impl Playlist {
    /// Its title; YouTube's own lists in the interface language.
    pub fn name(&self, lang: Lang) -> &str {
        match self.id.as_str() {
            "LL" => lang.pick("👍 Videos que me gustan", "👍 Liked videos"),
            "WL" => lang.pick("🕒 Ver más tarde (YouTube)", "🕒 Watch later (YouTube)"),
            _ => &self.title,
        }
    }
}

/// Stable identity of an open tab; survives reordering and closing others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

#[cfg(test)]
pub(crate) fn video(id: &str) -> Video {
    Video {
        id: id.into(),
        title: format!("title {id}"),
        channel: format!("chan {id}"),
        channel_id: Some(format!("UC{id}")),
        channel_url: Some(format!("https://www.youtube.com/channel/UC{id}")),
        duration: Some(60),
        published: None,
        live: Live::No,
        short: false,
    }
}

#[cfg(test)]
pub(crate) fn channel(id: &str) -> Channel {
    Channel {
        id: id.into(),
        name: format!("name {id}"),
        url: format!("https://www.youtube.com/channel/{id}"),
        followers: None,
        handle: None,
        description: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_points_to_watch_page() {
        assert_eq!(video("abc").url(), "https://www.youtube.com/watch?v=abc");
    }

    #[test]
    fn mix_url_plays_first_video_inside_the_list() {
        let mix = Video { id: "RDdQw4w9WgXcQ".into(), ..video("x") };
        assert_eq!(mix.url(), "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=RDdQw4w9WgXcQ");
    }

    #[test]
    fn sizes_parse_and_read() {
        let size = parse_size("29889635|213.4|720\n");
        assert_eq!(size, Size { bytes: Some(29_889_635), height: Some(720), duration: Some(213) });
        assert_eq!(size.label(), "≈ 30 MB (720p)");
        assert_eq!(parse_size("NA|NA|NA"), Size::default());
        assert_eq!(Size::default().label(), "");
        assert_eq!(parse_size("4200000|60|NA").label(), "≈ 4.2 MB");
        assert_eq!(bytes_label(850_000), "850 KB");
        assert_eq!(bytes_label(1_234_000_000), "1.2 GB");
    }

    #[test]
    fn duration_label_formats() {
        let with = |d| Video { duration: d, ..video("x") };
        assert_eq!(with(None).duration_label(), "");
        assert_eq!(with(Some(5)).duration_label(), "0:05");
        assert_eq!(with(Some(635)).duration_label(), "10:35");
        assert_eq!(with(Some(3723)).duration_label(), "1:02:03");
    }

    #[test]
    fn recency_orders_live_upcoming_dated_unknown() {
        let v = |live, published| Video { live, published, ..video("x") };
        let mut list = [v(Live::No, None), v(Live::No, Some(10)), v(Live::Upcoming, None), v(Live::Now, None)];
        list.sort_by_key(|v| std::cmp::Reverse(v.recency()));
        let order: Vec<_> = list.iter().map(|v| (v.live, v.published)).collect();
        assert_eq!(order, [(Live::Now, None), (Live::Upcoming, None), (Live::No, Some(10)), (Live::No, None)]);
    }

    #[test]
    fn date_labels_show_date_and_age() {
        let ts = 1_791_590_400; // 2026-10-10 00:00 UTC
        let es = |ts, now| date_labels(ts, now, Lang::Es);
        assert_eq!(es(ts, ts + 3600), ("10/10/26".into(), "hoy".into()));
        assert_eq!(es(ts, ts + DAY), ("10/10/26".into(), "ayer".into()));
        assert_eq!(es(ts, ts + 5 * DAY).1, "5d");
        assert_eq!(es(ts, ts + 95 * DAY).1, "3m");
        assert_eq!(es(ts, ts + 800 * DAY).1, "2a");
        assert_eq!(es(0, 0).0, "01/01/70");
        assert_eq!(es(951_782_400, 0).0, "29/02/00"); // leap day
        let en = |ts, now| date_labels(ts, now, Lang::En);
        assert_eq!(en(951_782_400, 951_782_400), ("02/29/00".into(), "today".into()));
        assert_eq!(en(ts, ts + 95 * DAY).1, "3mo");
        assert_eq!(en(ts, ts + 800 * DAY).1, "2y");
    }

    #[test]
    fn resume_points_skip_the_start_and_the_end() {
        assert_eq!(resume_point(10.0, Some(600.0)), None, "barely started");
        assert_eq!(resume_point(125.7, Some(600.0)), Some(125));
        assert_eq!(resume_point(580.0, Some(600.0)), None, "the last half minute");
        assert_eq!(resume_point(5800.0, Some(6000.0)), None, "past 95 %");
        assert_eq!(resume_point(90.0, None), Some(90), "unknown length");
        assert_eq!(resume_point(f64::NAN, None), None);
    }

    #[test]
    fn reads_download_progress_lines() {
        assert_eq!(parse_progress("  45.3%| 1.12MiB/s|00:12"), Some((453, "1.12MiB/s".into(), "00:12".into())));
        assert_eq!(parse_progress("100.0%|3.11MiB/s|NA"), Some((1000, "3.11MiB/s".into(), String::new())));
        assert_eq!(parse_progress("  0.0%|Unknown B/s|Unknown"), Some((0, String::new(), String::new())));
        assert_eq!(parse_progress("/home/me/Videos/x.mp4"), None);
    }

    #[test]
    fn later_list_format_is_stable() {
        let json = serde_json::to_string(&video("a")).unwrap();
        assert_eq!(serde_json::from_str::<Video>(&json).unwrap(), video("a"));
        let old: Video = serde_json::from_str(r#"{"id":"a","title":"t","channel":"c"}"#).unwrap();
        assert_eq!(old.live, Live::No);
    }
}
