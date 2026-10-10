//! YouTube access. Video lists come from `yt-dlp`, which already tracks
//! YouTube's changes; my channels and (un)subscribing use YouTube's internal
//! API ("InnerTube") signed with the session cookies, like the website does.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::domain::{Channel, Live, Video};
use crate::session;

const ORIGIN: &str = "https://www.youtube.com";
const CLIENT_VERSION: &str = "2.20251001.00.00";
const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

#[derive(Debug, thiserror::Error)]
pub enum YoutubeError {
    #[error("no se pudo ejecutar yt-dlp: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("yt-dlp falló: {0}")]
    Failed(String),
    #[error("respuesta inesperada: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("sin sesión: configurala en ⚙ Configuración")]
    NoSession,
    #[error("YouTube: {0}")]
    Http(String),
}

/// Where a list comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The home feed ("Inicio").
    Recommended,
    /// New videos from my channels.
    Subscriptions,
    MyChannels,
    Search(String),
    ChannelSearch(String),
    /// A channel's videos, by channel URL.
    Channel(String),
    /// Videos of one channel matching a query.
    SearchInChannel {
        url: String,
        query: String,
    },
}

impl Source {
    pub fn lists_channels(&self) -> bool {
        matches!(self, Self::MyChannels | Self::ChannelSearch(_))
    }

    /// Feeds read best newest first; searches keep YouTube's relevance order.
    fn newest_first(&self) -> bool {
        matches!(self, Self::Recommended | Self::Subscriptions | Self::Channel(_))
    }

    fn target(&self, limit: u32) -> String {
        match self {
            Self::Recommended => ":ytrec".into(),
            Self::Subscriptions => ":ytsubs".into(),
            Self::MyChannels => format!("{ORIGIN}/feed/channels"),
            Self::Search(query) => format!("ytsearch{limit}:{query}"),
            // `sp=EgIQAg==` is the website's "channels only" filter.
            Self::ChannelSearch(query) => {
                format!("{ORIGIN}/results?search_query={}&sp=EgIQAg%3D%3D", percent_encode(query))
            }
            Self::Channel(url) => format!("{}/videos", url.trim_end_matches('/')),
            Self::SearchInChannel { url, query } => {
                format!("{}/search?query={}", url.trim_end_matches('/'), percent_encode(query))
            }
        }
    }
}

/// What a list holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Items {
    Videos(Vec<Video>),
    Channels(Vec<Channel>),
    /// Browser profiles to log in with (`--cookies-from-browser` specs).
    Profiles(Vec<String>),
    /// The settings screen; rows come from `app::Setting::ALL`.
    Settings,
}

impl Items {
    pub fn len(&self) -> usize {
        match self {
            Self::Videos(v) => v.len(),
            Self::Channels(c) => c.len(),
            Self::Profiles(p) => p.len(),
            Self::Settings => crate::app::Setting::ALL.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub fn fetch_args(source: &Source, limit: u32, cookies: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> =
        ["--flat-playlist", "--dump-single-json", "--no-warnings", "--ignore-errors"].map(String::from).into();
    args.extend(["-I".into(), format!("1:{limit}")]);
    if !source.lists_channels() {
        // Dates like "hace 3 días" without opening every video.
        for extractor in ["youtubetab", "youtube"] {
            args.extend(["--extractor-args".into(), format!("{extractor}:approximate_date")]);
        }
    }
    if let Some(cookies) = cookies {
        args.extend(["--cookies".into(), cookies.display().to_string()]);
    }
    args.push(source.target(limit));
    args
}

/// Fetches a list; blocking, run it off the UI thread.
pub fn fetch(source: &Source, limit: u32, cookies: Option<&Path>) -> Result<Items, YoutubeError> {
    if source == &Source::MyChannels {
        return my_channels(cookies.ok_or(YoutubeError::NoSession)?).map(Items::Channels);
    }
    if cookies.is_none() && source == &Source::Subscriptions {
        return Err(YoutubeError::NoSession);
    }
    let json = run_yt_dlp(&fetch_args(source, limit, cookies))?;
    if source.lists_channels() {
        Ok(Items::Channels(parse_channels(&json)?))
    } else {
        let mut videos = parse_videos(&json)?;
        if source.newest_first() {
            videos.sort_by_key(|v| std::cmp::Reverse(v.recency()));
        }
        Ok(Items::Videos(videos))
    }
}

/// My channels. YouTube caps every listing at about a thousand, so this
/// merges the channels page in each of its sort orders with the sidebar.
pub fn my_channels(cookies: &Path) -> Result<Vec<Channel>, YoutubeError> {
    let client = InnerTube::new(cookies)?;
    let first = client.call("browse", json!({ "browseId": "FEchannels" }))?;
    let mut pages = vec![collect_channels(&first)];
    for token in collect_continuations(&first) {
        // A sort order that fails just contributes nothing.
        if let Ok(page) = client.call("browse", json!({ "continuation": token })) {
            pages.push(collect_channels(&page));
        }
    }
    if let Ok(guide) = client.call("guide", json!({})) {
        pages.push(collect_guide_channels(&guide));
    }
    let mut seen = HashSet::new();
    Ok(pages.into_iter().flatten().filter(|c| seen.insert(c.id.clone())).collect())
}

/// Ids of the channels I am subscribed to.
pub fn subscribed_channels(cookies: &Path) -> Result<HashSet<String>, YoutubeError> {
    Ok(my_channels(cookies)?.into_iter().map(|c| c.id).collect())
}

/// Subscribes (or unsubscribes); returns whether I am subscribed afterwards.
pub fn set_subscription(cookies: &Path, channel_id: &str, subscribe: bool) -> Result<bool, YoutubeError> {
    let action = if subscribe { "subscription/subscribe" } else { "subscription/unsubscribe" };
    let reply = InnerTube::new(cookies)?.call(action, json!({ "channelIds": [channel_id] }))?;
    parse_subscription_reply(&reply).ok_or_else(|| YoutubeError::Http("la respuesta no confirma el cambio".into()))
}

/// Authenticated client for `youtubei/v1`.
struct InnerTube {
    agent: ureq::Agent,
    cookie_header: String,
    jar: std::collections::HashMap<String, String>,
}

impl InnerTube {
    fn new(cookies: &Path) -> Result<Self, YoutubeError> {
        let text = std::fs::read_to_string(cookies).map_err(|_| YoutubeError::NoSession)?;
        let jar = session::parse_cookies(&text);
        session::authorization(&jar, 0).ok_or(YoutubeError::NoSession)?;
        let cookie_header = jar.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ");
        let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
        Ok(Self { agent, cookie_header, jar })
    }

    fn call(&self, endpoint: &str, mut body: Value) -> Result<Value, YoutubeError> {
        body["context"] = json!({ "client": { "clientName": "WEB", "clientVersion": CLIENT_VERSION, "hl": "es" } });
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let auth = session::authorization(&self.jar, now).ok_or(YoutubeError::NoSession)?;
        let text = self
            .agent
            .post(&format!("{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false"))
            .header("Content-Type", "application/json")
            .header("Authorization", &auth)
            .header("Origin", ORIGIN)
            .header("X-Origin", ORIGIN)
            .header("X-Goog-AuthUser", "0")
            .header("Cookie", &self.cookie_header)
            .header("User-Agent", USER_AGENT)
            .send(body.to_string())
            .and_then(|mut reply| reply.body_mut().read_to_string())
            .map_err(|e| YoutubeError::Http(e.to_string()))?;
        Ok(serde_json::from_str(&text)?)
    }
}

/// Calls `visit` on every object in a JSON tree.
fn walk<'a>(value: &'a Value, visit: &mut impl FnMut(&'a serde_json::Map<String, Value>)) {
    match value {
        Value::Object(map) => {
            visit(map);
            map.values().for_each(|v| walk(v, visit));
        }
        Value::Array(items) => items.iter().for_each(|v| walk(v, visit)),
        _ => {}
    }
}

fn text(value: &Value) -> Option<String> {
    value["simpleText"].as_str().map(str::to_owned).or_else(|| {
        let runs = value["runs"].as_array()?;
        Some(runs.iter().filter_map(|r| r["text"].as_str()).collect())
    })
}

/// `channelRenderer`s of a channels page.
pub fn collect_channels(page: &Value) -> Vec<Channel> {
    let mut channels = Vec::new();
    walk(page, &mut |map| {
        let Some(r) = map.get("channelRenderer") else { return };
        let Some(id) = r["channelId"].as_str() else { return };
        // YouTube puts the subscriber count in either field depending on the page.
        let followers = [&r["videoCountText"], &r["subscriberCountText"]]
            .into_iter()
            .filter_map(text)
            .find(|t| t.contains("suscript") || t.contains("subscri"));
        channels.push(Channel {
            id: id.to_owned(),
            name: text(&r["title"]).unwrap_or_else(|| id.to_owned()),
            url: format!("{ORIGIN}/channel/{id}"),
            followers,
        });
    });
    channels
}

/// Continuation tokens of a channels page (one per sort order).
pub fn collect_continuations(page: &Value) -> Vec<String> {
    let mut tokens = Vec::new();
    walk(page, &mut |map| {
        if let Some(token) = map.get("continuationCommand").and_then(|c| c["token"].as_str()) {
            tokens.push(token.to_owned());
        }
    });
    tokens
}

/// Channels listed in the sidebar ("guide").
pub fn collect_guide_channels(guide: &Value) -> Vec<Channel> {
    let mut channels = Vec::new();
    walk(guide, &mut |map| {
        let Some(entry) = map.get("guideEntryRenderer") else { return };
        let Some(id) = entry["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str() else { return };
        if id.starts_with("UC") {
            channels.push(Channel {
                id: id.to_owned(),
                name: text(&entry["formattedTitle"]).unwrap_or_else(|| id.to_owned()),
                url: format!("{ORIGIN}/channel/{id}"),
                followers: None,
            });
        }
    });
    channels
}

/// Reads `updateSubscribeButtonAction.subscribed` from the API reply.
pub fn parse_subscription_reply(reply: &Value) -> Option<bool> {
    reply["actions"].as_array()?.iter().find_map(|a| a["updateSubscribeButtonAction"]["subscribed"].as_bool())
}

fn run_yt_dlp(args: &[String]) -> Result<String, YoutubeError> {
    let output = Command::new("yt-dlp").args(args).output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // With --ignore-errors a partial list still comes back on stdout.
    if !output.status.success() && stdout.trim().is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("sin detalle");
        return Err(YoutubeError::Failed(reason.trim().to_owned()));
    }
    Ok(stdout)
}

#[derive(Deserialize)]
struct Playlist<T> {
    #[serde(default = "Vec::new")]
    entries: Vec<Option<T>>,
}

#[derive(Deserialize)]
struct VideoEntry {
    id: String,
    title: Option<String>,
    channel: Option<String>,
    uploader: Option<String>,
    channel_id: Option<String>,
    channel_url: Option<String>,
    uploader_url: Option<String>,
    duration: Option<f64>,
    timestamp: Option<i64>,
    live_status: Option<String>,
}

#[derive(Deserialize)]
struct ChannelEntry {
    id: String,
    title: Option<String>,
    url: Option<String>,
    channel_follower_count: Option<u64>,
}

/// Parses `yt-dlp --flat-playlist -J` video lists. Entries without a title
/// (private or deleted videos) are skipped.
pub fn parse_videos(json: &str) -> Result<Vec<Video>, serde_json::Error> {
    let playlist: Playlist<VideoEntry> = serde_json::from_str(json)?;
    Ok(playlist
        .entries
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some(Video {
                title: e.title?,
                channel: e.channel.or(e.uploader).unwrap_or_default(),
                channel_id: e.channel_id,
                channel_url: e.channel_url.or(e.uploader_url),
                duration: e.duration.map(|d| d.round() as u32),
                published: e.timestamp,
                live: match e.live_status.as_deref() {
                    Some("is_live") => Live::Now,
                    Some("is_upcoming") => Live::Upcoming,
                    _ => Live::No,
                },
                id: e.id,
            })
        })
        .collect())
}

/// Parses yt-dlp channel lists (channel search).
pub fn parse_channels(json: &str) -> Result<Vec<Channel>, serde_json::Error> {
    let playlist: Playlist<ChannelEntry> = serde_json::from_str(json)?;
    Ok(playlist
        .entries
        .into_iter()
        .flatten()
        .filter(|e| e.id.starts_with("UC"))
        .map(|e| Channel {
            url: e.url.unwrap_or_else(|| format!("{ORIGIN}/channel/{}", e.id)),
            name: e.title.unwrap_or_else(|| e.id.clone()),
            followers: e.channel_follower_count.map(|n| format!("{} suscriptores", compact(n))),
            id: e.id,
        })
        .collect())
}

/// `950`, `12 k`, `1,2 M`.
fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{} k", n / 1_000),
        _ => format!("{},{} M", n / 1_000_000, n % 1_000_000 / 100_000),
    }
}

pub fn download_args(url: &str, format: &str, sub_lang: &str, dir: &Path, cookies: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-f".into(), format.into(), "--no-playlist".into()];
    args.extend(["--write-subs", "--write-auto-subs", "--sub-langs"].map(String::from));
    args.push(sub_lang.into());
    args.extend(["-o".into(), dir.join("%(title)s.%(ext)s").display().to_string()]);
    if let Some(cookies) = cookies {
        args.extend(["--cookies".into(), cookies.display().to_string()]);
    }
    args.push(url.into());
    args
}

/// Downloads with subtitles; blocking.
pub fn download(args: &[String]) -> Result<(), YoutubeError> {
    let output = Command::new("yt-dlp").args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(YoutubeError::Failed(stderr.lines().last().unwrap_or("sin detalle").to_owned()))
    }
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH: &str = include_str!("../tests/fixtures/search.json");

    #[test]
    fn parses_real_search_output() {
        let videos = parse_videos(SEARCH).unwrap();
        assert_eq!(videos.len(), 3);
        assert_eq!(videos[0].id, "f7NwyBnIRTE");
        assert_eq!(videos[0].channel, "AnimalsReview");
        assert_eq!(videos[0].channel_id.as_deref(), Some("UCYDivYxr2sFafehywijfzuA"));
        assert_eq!(videos[0].duration, Some(635));
        assert!(videos[0].title.contains("Big Buck Bunny"));
    }

    #[test]
    fn missing_channel_falls_back_to_uploader() {
        let videos = parse_videos(SEARCH).unwrap();
        assert_eq!(videos[2].channel, "Benny Wollin");
        assert_eq!(videos[2].duration, None);
    }

    #[test]
    fn video_entries_carry_dates_live_status_and_urls() {
        let json = r#"{"entries":[
            {"id":"a","title":"A","timestamp":100,"uploader_url":"https://www.youtube.com/@a"},
            {"id":"b","title":"B","live_status":"is_live"},
            {"id":"c","title":"C","live_status":"is_upcoming"},
            {"id":"d","title":null},
            null,
            {"id":"e","title":"E","duration":59.6}]}"#;
        let videos = parse_videos(json).unwrap();
        assert_eq!(videos.len(), 4);
        assert_eq!(videos[0].published, Some(100));
        assert_eq!(videos[0].channel_url.as_deref(), Some("https://www.youtube.com/@a"));
        assert_eq!(videos[1].live, Live::Now);
        assert_eq!(videos[2].live, Live::Upcoming);
        assert_eq!(videos[3].duration, Some(60));
    }

    #[test]
    fn parses_channel_search_and_skips_non_channels() {
        let json = r#"{"entries":[
            {"id":"UCabc","title":"Canal","url":"https://www.youtube.com/channel/UCabc","channel_follower_count":1234567},
            {"id":"UCdef"},
            {"id":"PLxyz","title":"Una playlist"}]}"#;
        let channels = parse_channels(json).unwrap();
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].followers.as_deref(), Some("1,2 M suscriptores"));
        assert_eq!(channels[1].name, "UCdef");
        assert_eq!(channels[1].url, "https://www.youtube.com/channel/UCdef");
    }

    #[test]
    fn empty_playlist_is_ok() {
        assert!(parse_videos(r#"{"id":"x"}"#).unwrap().is_empty());
        assert!(parse_videos("not json").is_err());
    }

    /// Same shape as the real `browse FEchannels` reply (trimmed).
    fn channels_page() -> Value {
        json!({ "contents": { "twoColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
            "sectionListRenderer": {
                "header": { "chipBarViewModel": { "chips": [
                    { "chipViewModel": { "tapCommand": { "innertubeCommand": { "continuationCommand": { "token": "AZ" } } } } },
                    { "chipViewModel": { "tapCommand": { "innertubeCommand": { "continuationCommand": { "token": "NEW" } } } } }
                ] } },
                "contents": [{ "itemSectionRenderer": { "contents": [{ "shelfRenderer": { "content": {
                    "expandedShelfContentsRenderer": { "items": [
                        { "channelRenderer": {
                            "channelId": "UCa",
                            "title": { "simpleText": "María" },
                            "videoCountText": { "simpleText": "358 mil suscriptores" },
                            "subscriberCountText": { "simpleText": "@maria" } } },
                        { "channelRenderer": { "channelId": "UCb", "title": { "runs": [{ "text": "Ca" }, { "text": "nal" }] } } }
                    ] } } } }] } }]
            } } } }] } }
        })
    }

    #[test]
    fn collects_channels_and_sort_order_tokens() {
        let page = channels_page();
        let channels = collect_channels(&page);
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].name, "María");
        assert_eq!(channels[0].followers.as_deref(), Some("358 mil suscriptores"));
        assert_eq!(channels[0].url, "https://www.youtube.com/channel/UCa");
        assert_eq!(channels[1].name, "Canal");
        assert_eq!(channels[1].followers, None);
        assert_eq!(collect_continuations(&page), ["AZ", "NEW"]);
    }

    #[test]
    fn collects_channels_from_the_sidebar_only() {
        let guide = json!({ "items": [
            { "guideEntryRenderer": { "formattedTitle": { "simpleText": "Inicio" },
                "navigationEndpoint": { "browseEndpoint": { "browseId": "FEwhat_to_watch" } } } },
            { "guideEntryRenderer": { "formattedTitle": { "simpleText": "Canal X" },
                "navigationEndpoint": { "browseEndpoint": { "browseId": "UCx" } } } }
        ] });
        let channels = collect_guide_channels(&guide);
        assert_eq!(channels.len(), 1);
        assert_eq!((channels[0].id.as_str(), channels[0].name.as_str()), ("UCx", "Canal X"));
    }

    #[test]
    fn targets() {
        assert_eq!(Source::Recommended.target(60), ":ytrec");
        assert_eq!(Source::Subscriptions.target(60), ":ytsubs");
        assert_eq!(Source::Search("lofi".into()).target(20), "ytsearch20:lofi");
        assert_eq!(
            Source::ChannelSearch("café & tango".into()).target(20),
            "https://www.youtube.com/results?search_query=caf%C3%A9%20%26%20tango&sp=EgIQAg%3D%3D"
        );
        assert_eq!(
            Source::Channel("https://www.youtube.com/@a/".into()).target(1),
            "https://www.youtube.com/@a/videos"
        );
        let inside = Source::SearchInChannel { url: "https://www.youtube.com/@a".into(), query: "rust tui".into() };
        assert_eq!(inside.target(1), "https://www.youtube.com/@a/search?query=rust%20tui");
    }

    #[test]
    fn fetch_args_limit_dates_and_cookies() {
        let args = fetch_args(&Source::Subscriptions, 60, Some(Path::new("/c.txt")));
        let joined = args.join(" ");
        assert!(joined.contains("-I 1:60"), "{joined}");
        assert!(joined.contains("youtubetab:approximate_date"));
        assert!(joined.contains("--cookies /c.txt"));
        assert_eq!(args.last().unwrap(), ":ytsubs");
        let channels = fetch_args(&Source::ChannelSearch("x".into()), 10, None).join(" ");
        assert!(!channels.contains("approximate_date"));
        assert!(!channels.contains("--cookies"));
    }

    #[test]
    fn personal_lists_need_a_session() {
        assert!(matches!(fetch(&Source::Subscriptions, 1, None), Err(YoutubeError::NoSession)));
        assert!(matches!(fetch(&Source::MyChannels, 1, None), Err(YoutubeError::NoSession)));
        assert!(matches!(my_channels(Path::new("/no/such/cookies.txt")), Err(YoutubeError::NoSession)));
    }

    #[test]
    fn subscription_reply() {
        let yes = json!({"actions":[{"other":{}},{"updateSubscribeButtonAction":{"subscribed":true}}]});
        assert_eq!(parse_subscription_reply(&yes), Some(true));
        let no = json!({"actions":[{"updateSubscribeButtonAction":{"subscribed":false}}]});
        assert_eq!(parse_subscription_reply(&no), Some(false));
        assert_eq!(parse_subscription_reply(&json!({"error":{}})), None);
    }

    #[test]
    fn download_args_include_subs_and_output_dir() {
        let args = download_args("URL", "best", "es", Path::new("/v"), None);
        assert_eq!(args.last().unwrap(), "URL");
        let joined = args.join(" ");
        assert!(joined.contains("-f best"));
        assert!(joined.contains("--sub-langs es"));
        assert!(joined.contains("-o /v/%(title)s.%(ext)s"));
    }

    #[test]
    fn compact_numbers() {
        assert_eq!(compact(950), "950");
        assert_eq!(compact(12_345), "12 k");
        assert_eq!(compact(1_234_567), "1,2 M");
    }
}
