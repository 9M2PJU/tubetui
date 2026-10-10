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
        match self.duration {
            None => String::new(),
            Some(s) if s >= 3600 => format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60),
            Some(s) => format!("{}:{:02}", s / 60, s % 60),
        }
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
fn civil_from_days(days: i64) -> (i64, u32, u32) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub url: String,
    /// Already formatted, e.g. "358 mil suscriptores".
    pub followers: Option<String>,
    /// What the channel says about itself, as YouTube shortens it.
    pub description: Option<String>,
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
    }
}

#[cfg(test)]
pub(crate) fn channel(id: &str) -> Channel {
    Channel {
        id: id.into(),
        name: format!("name {id}"),
        url: format!("https://www.youtube.com/channel/{id}"),
        followers: None,
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
    fn later_list_format_is_stable() {
        let json = serde_json::to_string(&video("a")).unwrap();
        assert_eq!(serde_json::from_str::<Video>(&json).unwrap(), video("a"));
        let old: Video = serde_json::from_str(r#"{"id":"a","title":"t","channel":"c"}"#).unwrap();
        assert_eq!(old.live, Live::No);
    }
}
