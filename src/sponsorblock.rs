//! SponsorBlock: parts of a video its viewers marked as sponsors, self
//! promotion or "subscribe!" reminders, skipped while playing.
//! <https://wiki.sponsor.ajay.app/w/API_Docs>

use std::collections::HashSet;
use std::time::Duration;

use serde::Deserialize;

use crate::i18n::Lang;

const API: &str = "https://sponsor.ajay.app/api/skipSegments";
/// What gets skipped. Intros and outros are left alone: they can be the video.
const CATEGORIES: [&str; 4] = ["sponsor", "selfpromo", "interaction", "music_offtopic"];

/// A part to skip, in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub category: String,
}

impl Segment {
    pub fn start_secs(&self) -> f64 {
        self.start as f64 / 1000.0
    }

    pub fn end_secs(&self) -> f64 {
        self.end as f64 / 1000.0
    }

    /// `"patrocinio"`, `"autopromoción"` …
    pub fn label(&self, lang: Lang) -> &'static str {
        let (es, en) = match self.category.as_str() {
            "sponsor" => ("patrocinio", "sponsor"),
            "selfpromo" => ("autopromoción", "self promotion"),
            "interaction" => ("pedido de suscribirse", "subscribe reminder"),
            "music_offtopic" => ("parte sin música", "non-music part"),
            _ => ("segmento", "segment"),
        };
        lang.pick(es, en)
    }
}

/// The segments of a video; none when nobody marked any. Blocking.
pub fn fetch(video_id: &str) -> Result<Vec<Segment>, String> {
    let categories = format!("[{}]", CATEGORIES.map(|c| format!("\"{c}\"")).join(","));
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into();
    let reply = agent.get(API).query("videoID", video_id).query("categories", &categories).call();
    match reply {
        Ok(mut reply) => parse(&reply.body_mut().read_to_string().map_err(|e| e.to_string())?),
        // "Not Found": no segments for this video.
        Err(ureq::Error::StatusCode(404)) => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn parse(json: &str) -> Result<Vec<Segment>, String> {
    #[derive(Deserialize)]
    struct Entry {
        segment: (f64, f64),
        category: String,
        #[serde(rename = "actionType", default)]
        action: String,
    }
    let entries: Vec<Entry> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let ms = |s: f64| (s.max(0.0) * 1000.0).round() as u64;
    let mut segments: Vec<Segment> = entries
        .into_iter()
        .filter(|e| e.action.is_empty() || e.action == "skip")
        .filter(|e| e.segment.1 > e.segment.0)
        .map(|e| Segment { start: ms(e.segment.0), end: ms(e.segment.1), category: e.category })
        .collect();
    segments.sort_by_key(|s| s.start);
    Ok(segments)
}

/// The segment playing at `position` (seconds) that was not skipped yet.
/// The last half second is left: seeking there gains nothing.
pub fn to_skip(segments: &[Segment], position: f64, skipped: &HashSet<usize>) -> Option<usize> {
    let ms = (position * 1000.0) as u64;
    segments.iter().position(|s| s.start <= ms && ms + 500 < s.end).filter(|i| !skipped.contains(i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_skip_segments_in_order() {
        let json = r#"[
            {"category":"selfpromo","actionType":"skip","segment":[300.5,320],"UUID":"b"},
            {"category":"sponsor","actionType":"skip","segment":[10,42.25],"UUID":"a"},
            {"category":"sponsor","actionType":"mute","segment":[50,60]},
            {"category":"sponsor","actionType":"skip","segment":[70,70]}]"#;
        let segments = parse(json).unwrap();
        assert_eq!(
            segments,
            [
                Segment { start: 10_000, end: 42_250, category: "sponsor".into() },
                Segment { start: 300_500, end: 320_000, category: "selfpromo".into() },
            ]
        );
        assert_eq!(segments[1].label(Lang::Es), "autopromoción");
        assert!(parse("Not Found").is_err());
    }

    #[test]
    fn skips_each_segment_once_while_inside_it() {
        let segments = parse(r#"[{"category":"sponsor","segment":[10,20]}]"#).unwrap();
        let none = HashSet::new();
        assert_eq!(to_skip(&segments, 9.9, &none), None);
        assert_eq!(to_skip(&segments, 10.0, &none), Some(0));
        assert_eq!(to_skip(&segments, 19.0, &none), Some(0));
        assert_eq!(to_skip(&segments, 19.6, &none), None, "almost over");
        assert_eq!(to_skip(&segments, 15.0, &HashSet::from([0])), None, "already skipped: I went back to it");
    }
}
