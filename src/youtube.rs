//! YouTube access. Feeds come from `yt-dlp`, which already tracks YouTube's
//! changes. Searches, my channels and (un)subscribing use YouTube's internal
//! API ("InnerTube") like the website does: a search page answers in under a
//! second, where starting yt-dlp takes several.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use crate::session::JarCopy;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::domain::{Channel, Live, Playlist, Video};
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
    /// My playlists (the lists themselves, not their videos).
    MyPlaylists,
    /// A playlist's videos, by playlist id.
    Playlist(String),
    Search(String),
    ChannelSearch(String),
    /// A channel's videos, by channel URL.
    Channel(String),
    /// Videos of one channel matching a query.
    SearchInChannel {
        url: String,
        query: String,
    },
    /// This week's videos on my topics, for discovering ("Descubrir").
    Discover(Vec<String>),
    /// The "up next" of these videos (the last ones I watched), taking turns.
    UpNext(Vec<String>),
}

impl Source {
    /// Feeds that are empty, or refused, without a working login.
    pub fn needs_login(&self) -> bool {
        matches!(self, Self::Recommended | Self::Subscriptions | Self::MyPlaylists | Self::Playlist(_))
    }

    pub fn lists_channels(&self) -> bool {
        matches!(self, Self::MyChannels | Self::ChannelSearch(_))
    }

    /// Subscriptions and channels read best newest first; the home feed and
    /// searches keep YouTube's order, which is what it thinks I want to see.
    fn newest_first(&self) -> bool {
        matches!(self, Self::Subscriptions | Self::Channel(_))
    }

    fn target(&self, limit: u32) -> String {
        match self {
            Self::Recommended => ":ytrec".into(),
            Self::Subscriptions => ":ytsubs".into(),
            Self::MyChannels => format!("{ORIGIN}/feed/channels"),
            Self::MyPlaylists => String::new(), // read by `my_playlists`, not yt-dlp
            Self::Playlist(id) => format!("{ORIGIN}/playlist?list={id}"),
            Self::Search(query) => format!("ytsearch{limit}:{query}"),
            // `sp=EgIQAg==` is the website's "channels only" filter.
            Self::ChannelSearch(query) => {
                format!("{ORIGIN}/results?search_query={}&sp=EgIQAg%3D%3D", percent_encode(query))
            }
            Self::Channel(url) => format!("{}/videos", url.trim_end_matches('/')),
            Self::SearchInChannel { url, query } => {
                format!("{}/search?query={}", url.trim_end_matches('/'), percent_encode(query))
            }
            Self::Discover(_) | Self::UpNext(_) => String::new(), // read through InnerTube, not yt-dlp
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
    Playlists(Vec<Playlist>),
    /// The settings screen; rows come from `app::Setting::ALL`.
    Settings,
    /// The "About" page.
    About,
    /// What the system has and lacks (mpv, yt-dlp…).
    System,
    /// Local media files.
    Files(Vec<crate::media::LocalFile>, crate::media::MediaKind),
    /// "Descargas": running and finished downloads.
    Downloads(Vec<crate::domain::Download>),
}

impl Items {
    pub fn len(&self) -> usize {
        match self {
            Self::Videos(v) => v.len(),
            Self::Channels(c) => c.len(),
            Self::Profiles(p) => p.len(),
            Self::Playlists(p) => p.len(),
            Self::Settings => crate::app::Setting::ALL.len(),
            Self::About | Self::System => 0,
            Self::Files(files, _) => files.len(),
            Self::Downloads(downloads) => downloads.len(),
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

/// What a feed brings when the login copy went stale: nothing, or yt-dlp
/// asking for one.
pub fn looks_logged_out(result: &Result<Items, YoutubeError>) -> bool {
    match result {
        Ok(items) => items.is_empty(),
        // yt-dlp asks for a login; YouTube's API answers 401.
        Err(e) => {
            let e = e.to_string();
            e.contains("Login details are needed") || e.contains("status: 401")
        }
    }
}

/// Fetches a list; blocking, run it off the UI thread.
pub fn fetch(source: &Source, limit: u32, cookies: Option<&Path>) -> Result<Items, YoutubeError> {
    if source == &Source::MyChannels {
        return my_channels(cookies.ok_or(YoutubeError::NoSession)?).map(Items::Channels);
    }
    if let Source::Discover(topics) = source {
        return discover(topics).map(Items::Videos);
    }
    if let Source::UpNext(ids) = source {
        return up_next(ids, cookies).map(Items::Videos);
    }
    if source == &Source::MyPlaylists {
        return my_playlists(cookies.ok_or(YoutubeError::NoSession)?).map(Items::Playlists);
    }
    if cookies.is_none() && source == &Source::Subscriptions {
        return Err(YoutubeError::NoSession);
    }
    let copy = cookies.map(JarCopy::new).transpose()?;
    let json = run_yt_dlp(&fetch_args(source, limit, copy.as_ref().map(JarCopy::path)))?;
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

/// My playlists: "Liked videos" and YouTube's "Watch later" first, then
/// the ones I made or saved.
pub fn my_playlists(cookies: &Path) -> Result<Vec<Playlist>, YoutubeError> {
    let page = InnerTube::new(cookies)?.call("browse", json!({ "browseId": "FEplaylist_aggregation" }))?;
    // Named by `Playlist::name`.
    let fixed = ["LL", "WL"];
    let mut playlists: Vec<Playlist> =
        fixed.iter().map(|id| Playlist { id: (*id).into(), title: (*id).into(), count: None }).collect();
    playlists.extend(collect_playlists(&page).into_iter().filter(|p| !fixed.contains(&p.id.as_str())));
    Ok(playlists)
}

/// Playlists of a page, in YouTube's newer "lockup" format and the older grid.
pub fn collect_playlists(page: &Value) -> Vec<Playlist> {
    let mut playlists = Vec::new();
    let mut seen = HashSet::new();
    walk(page, &mut |map| {
        let found = if let Some(lockup) = map.get("lockupViewModel")
            && lockup["contentType"] == "LOCKUP_CONTENT_TYPE_PLAYLIST"
        {
            let mut count = None;
            walk(&lockup["contentImage"], &mut |o| {
                if let Some(text) = o.get("thumbnailBadgeViewModel").and_then(|b| b["text"].as_str())
                    && count.is_none()
                {
                    count = Some(text.to_owned());
                }
            });
            lockup["contentId"]
                .as_str()
                .zip(lockup["metadata"]["lockupMetadataViewModel"]["title"]["content"].as_str())
                .map(|(id, title)| Playlist { id: id.to_owned(), title: title.to_owned(), count })
        } else if let Some(grid) = map.get("gridPlaylistRenderer") {
            grid["playlistId"].as_str().zip(text(&grid["title"])).map(|(id, title)| Playlist {
                id: id.to_owned(),
                title,
                count: text(&grid["videoCountText"]).or_else(|| text(&grid["videoCountShortText"])),
            })
        } else {
            None
        };
        if let Some(playlist) = found.filter(|p| seen.insert(p.id.clone())) {
            playlists.push(playlist);
        }
    });
    playlists
}

/// Ids of the videos in my YouTube watch history (its first page, the most
/// recent couple of hundred).
pub fn watch_history(cookies: &Path) -> Result<HashSet<String>, YoutubeError> {
    let page = InnerTube::new(cookies)?.call("browse", json!({ "browseId": "FEhistory" }))?;
    Ok(collect_video_ids(&page))
}

/// Video ids in a page, in both of YouTube's formats.
pub fn collect_video_ids(page: &Value) -> HashSet<String> {
    let mut ids = HashSet::new();
    walk(page, &mut |map| {
        if let Some(id) = map.get("videoRenderer").and_then(|r| r["videoId"].as_str()) {
            ids.insert(id.to_owned());
        }
        if let Some(lockup) = map.get("lockupViewModel")
            && lockup["contentType"] == "LOCKUP_CONTENT_TYPE_VIDEO"
            && let Some(id) = lockup["contentId"].as_str()
        {
            ids.insert(id.to_owned());
        }
    });
    ids
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

/// One page of search results and the token that asks for the next one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchPage {
    pub videos: Vec<Video>,
    pub next: Option<String>,
}

/// Searches videos, one page (about 20) at a time; `next` continues a search.
pub fn search(query: &str, next: Option<&str>) -> Result<SearchPage, YoutubeError> {
    let body = match next {
        Some(token) => json!({ "continuation": token }),
        // `EgIQAQ==` is the "Videos" filter: no channels, playlists or shorts shelves.
        None => json!({ "query": query, "params": "EgIQAQ==" }),
    };
    let reply = InnerTube::anonymous().call("search", body)?;
    Ok(parse_search(&reply, unix_now()))
}

/// Search filter "Videos" + "This week".
const THIS_WEEK: &str = "EgQIAxAB";

/// This week's videos on each topic, the most watched of each topic first and
/// the topics taking turns. Anonymous, so neither my history nor my channels
/// shape what comes up.
pub fn discover(topics: &[String]) -> Result<Vec<Video>, YoutubeError> {
    let replies: Vec<Result<Value, YoutubeError>> = std::thread::scope(|scope| {
        let searches: Vec<_> = topics
            .iter()
            .map(|topic| {
                scope.spawn(move || {
                    InnerTube::anonymous().call("search", json!({ "query": topic, "params": THIS_WEEK }))
                })
            })
            .collect();
        searches
            .into_iter()
            .map(|search| search.join().unwrap_or_else(|_| Err(YoutubeError::Http("search failed".into()))))
            .collect()
    });
    let now = unix_now();
    let mut per_topic = Vec::new();
    let mut failure = None;
    for reply in replies {
        match reply {
            Ok(reply) => per_topic.push(parse_discover(&reply, now)),
            Err(e) => failure = Some(e),
        }
    }
    // One topic failing is fine while the others bring something.
    match failure {
        Some(e) if per_topic.iter().all(Vec::is_empty) => Err(e),
        _ => Ok(take_turns(per_topic)),
    }
}

/// The videos of a search reply with their views.
pub fn parse_discover(reply: &Value, now: i64) -> Vec<(Video, u64)> {
    let mut found = Vec::new();
    walk(reply, &mut |object| {
        if let Some(r) = object.get("videoRenderer")
            && let Some(video) = video_renderer(r, now)
        {
            let views = text(&r["viewCountText"]).and_then(|t| first_number(&t)).unwrap_or(0);
            found.push((video, views));
        }
    });
    found
}

/// One video of each topic in turn, each topic's most watched first, so a
/// big topic doesn't bury a small one; without repeats (a video can match two).
/// What YouTube puts next to each of `ids`, one from each in turn, without
/// repeats nor `ids` themselves.
pub fn up_next(ids: &[String], cookies: Option<&Path>) -> Result<Vec<Video>, YoutubeError> {
    let replies: Vec<Result<Watch, YoutubeError>> = std::thread::scope(|scope| {
        let calls: Vec<_> = ids.iter().map(|id| scope.spawn(move || watch(id, cookies))).collect();
        calls.into_iter().map(|c| c.join().unwrap_or_else(|_| Err(YoutubeError::Http("next failed".into())))).collect()
    });
    let mut lists = Vec::new();
    let mut failure = None;
    for reply in replies {
        match reply {
            // In YouTube's order: no views to sort by.
            Ok(watch) => lists.push(watch.related.into_iter().map(|v| (v, 0)).collect()),
            Err(e) => failure = Some(e),
        }
    }
    if let Some(e) = failure.filter(|_| lists.iter().all(Vec::is_empty)) {
        return Err(e);
    }
    let mut videos = take_turns(lists);
    videos.retain(|v| !ids.contains(&v.id));
    Ok(videos)
}

fn take_turns(per_topic: Vec<Vec<(Video, u64)>>) -> Vec<Video> {
    let mut queues: Vec<_> = per_topic
        .into_iter()
        .map(|mut found| {
            found.sort_by_key(|(_, views)| std::cmp::Reverse(*views));
            found.into_iter()
        })
        .collect();
    let mut seen = HashSet::new();
    let mut videos = Vec::new();
    loop {
        let mut any = false;
        for queue in &mut queues {
            if let Some((video, _)) = queue.next() {
                any = true;
                if seen.insert(video.id.clone()) {
                    videos.push(video);
                }
            }
        }
        if !any {
            return videos;
        }
    }
}

/// Videos and the next-page token of a search reply (first page or continuation).
pub fn parse_search(reply: &Value, now: i64) -> SearchPage {
    let mut page = SearchPage::default();
    let mut seen = HashSet::new();
    walk(reply, &mut |object| {
        if let Some(video) = object.get("videoRenderer").and_then(|r| video_renderer(r, now))
            && seen.insert(video.id.clone())
        {
            page.videos.push(video);
        }
        if let Some(token) = object
            .get("continuationItemRenderer")
            .and_then(|c| c["continuationEndpoint"]["continuationCommand"]["token"].as_str())
        {
            page.next = Some(token.to_owned());
        }
    });
    page
}

fn video_renderer(r: &Value, now: i64) -> Option<Video> {
    let owner = &r["ownerText"]["runs"][0];
    let channel_id = owner["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str().map(str::to_owned);
    let badges = r["badges"].to_string() + &r["thumbnailOverlays"].to_string();
    let live = if r.get("upcomingEventData").is_some() {
        Live::Upcoming
    } else if badges.contains("LIVE") {
        Live::Now
    } else {
        Live::No
    };
    Some(Video {
        id: r["videoId"].as_str()?.to_owned(),
        title: text(&r["title"])?,
        channel: owner["text"].as_str().unwrap_or_default().to_owned(),
        channel_url: channel_id.as_ref().map(|id| format!("{ORIGIN}/channel/{id}")),
        channel_id,
        duration: text(&r["lengthText"]).and_then(|t| parse_clock(&t)),
        published: text(&r["publishedTimeText"]).and_then(|t| parse_age(&t, now)),
        live,
        short: r["navigationEndpoint"]["commandMetadata"]["webCommandMetadata"]["url"]
            .as_str()
            .is_some_and(|url| url.starts_with("/shorts/")),
    })
}

/// `"1:52:00"` → 6720 seconds.
/// What the watch page says about a video, and what it suggests next.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Watch {
    pub views: Option<u64>,
    pub likes: Option<u64>,
    /// When it was published, approximately (YouTube says "3 weeks ago").
    pub published: Option<i64>,
    /// The channel's subscribers, shortened by YouTube: `559K`.
    pub subscribers: Option<String>,
    /// "Up next".
    pub related: Vec<Video>,
    /// How I rated it; `None` without a login.
    pub rating: Option<Rating>,
    /// Asks YouTube for the comments (see [`comments`]).
    pub comments_token: Option<String>,
}

/// A top comment of a video.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub author: String,
    pub text: String,
    /// As YouTube shortens it: `"1.2K"`.
    pub likes: Option<String>,
    /// `"hace 2 años"`, in the language asked for.
    pub published: Option<String>,
}

/// The first page of a video's top comments, in `hl` (`"es"`, `"en"`).
pub fn comments(token: &str, hl: &'static str) -> Result<Vec<Comment>, YoutubeError> {
    let reply = InnerTube { hl, ..InnerTube::anonymous() }.call("next", json!({ "continuation": token }))?;
    Ok(parse_comments(&reply))
}

/// Comments in YouTube's newer entity format and in the older renderer.
pub fn parse_comments(reply: &Value) -> Vec<Comment> {
    let mut comments = Vec::new();
    walk(reply, &mut |map| {
        if let Some(c) = map.get("commentEntityPayload") {
            let body = c["properties"]["content"]["content"].as_str().unwrap_or_default();
            if !body.is_empty() {
                let some = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
                comments.push(Comment {
                    author: c["author"]["displayName"].as_str().unwrap_or_default().to_owned(),
                    text: body.to_owned(),
                    likes: some(&c["toolbar"]["likeCountNotliked"]).filter(|l| l.trim() != "0"),
                    published: some(&c["properties"]["publishedTime"]),
                });
            }
        } else if let Some(c) = map.get("commentRenderer")
            && let Some(body) = text(&c["contentText"]).filter(|t| !t.is_empty())
        {
            comments.push(Comment {
                author: text_or_empty(&c["authorText"]),
                text: body,
                likes: text(&c["voteCount"]),
                published: text(&c["publishedTimeText"]),
            });
        }
    });
    comments
}

fn text_or_empty(value: &Value) -> String {
    text(value).unwrap_or_default()
}

/// My vote on a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rating {
    #[default]
    None,
    Like,
    Dislike,
}

impl Rating {
    /// YouTube's `likeStatus`: `LIKE`, `DISLIKE`, `INDIFFERENT`.
    fn from_status(status: &str) -> Self {
        match status {
            "LIKE" => Self::Like,
            "DISLIKE" => Self::Dislike,
            _ => Self::None,
        }
    }

    fn endpoint(self) -> &'static str {
        match self {
            Self::None => "like/removelike",
            Self::Like => "like/like",
            Self::Dislike => "like/dislike",
        }
    }
}

/// The watch page of `id`: details and related videos (about a second).
/// With a login it also says how I rated the video.
pub fn watch(id: &str, cookies: Option<&Path>) -> Result<Watch, YoutubeError> {
    // English, like anonymous calls, so the dates and counts can be read.
    let signed = cookies.and_then(|c| InnerTube::new(c).ok()).map(|client| InnerTube { hl: "en", ..client });
    let logged_in = signed.is_some();
    let reply = signed.unwrap_or_else(InnerTube::anonymous).call("next", json!({ "videoId": id }))?;
    let mut watch = parse_watch(&reply, unix_now());
    if !logged_in {
        watch.rating = None;
    }
    Ok(watch)
}

/// Deletes one of my playlists, for good.
pub fn delete_playlist(cookies: &Path, id: &str) -> Result<(), YoutubeError> {
    InnerTube::new(cookies)?.call("playlist/delete", json!({ "playlistId": id }))?;
    Ok(())
}

/// Likes, dislikes or takes my vote back from a video.
pub fn rate(cookies: &Path, video_id: &str, rating: Rating) -> Result<(), YoutubeError> {
    InnerTube::new(cookies)?.call(rating.endpoint(), json!({ "target": { "videoId": video_id } }))?;
    Ok(())
}

pub fn parse_watch(reply: &Value, now: i64) -> Watch {
    let mut watch = Watch::default();
    let mut seen = HashSet::new();
    walk(reply, &mut |object| {
        if let Some(primary) = object.get("videoPrimaryInfoRenderer") {
            // `originalViewCount` is sometimes "0" while the text says "6,984 views".
            let views = &primary["viewCount"]["videoViewCountRenderer"];
            watch.views = text(&views["viewCount"])
                .and_then(|t| first_number(&t))
                .or_else(|| views["originalViewCount"].as_str().and_then(|v| v.parse().ok()).filter(|n| *n > 0));
            watch.published = text(&primary["relativeDateText"]).and_then(|t| parse_age(&t, now));
            let mut likes = None;
            walk(primary, &mut |o| {
                // "like this video along with 733 other people"
                if let Some(label) = o.get("accessibilityText").and_then(Value::as_str)
                    && label.contains("along with")
                {
                    likes = likes.or_else(|| first_number(label));
                }
            });
            watch.likes = likes;
        }
        if watch.rating.is_none()
            && let Some(status) = object.get("likeStatusEntity").and_then(|e| e["likeStatus"].as_str())
        {
            watch.rating = Some(Rating::from_status(status));
        }
        // The comments section starts empty, with the token that fills it.
        if watch.comments_token.is_none()
            && object.get("sectionIdentifier").and_then(Value::as_str) == Some("comment-item-section")
        {
            walk(&object["contents"], &mut |o| {
                if let Some(token) = o
                    .get("continuationCommand")
                    .and_then(|c| c["token"].as_str())
                    .filter(|_| watch.comments_token.is_none())
                {
                    watch.comments_token = Some(token.to_owned());
                }
            });
        }
        if let Some(owner) = object.get("videoOwnerRenderer") {
            watch.subscribers =
                text(&owner["subscriberCountText"]).and_then(|t| t.split_whitespace().next().map(str::to_owned));
        }
        if let Some(video) = object.get("lockupViewModel").and_then(|l| lockup_video(l, now))
            && seen.insert(video.id.clone())
        {
            watch.related.push(video);
        }
    });
    watch
}

/// A suggested video in YouTube's newer "lockup" format.
fn lockup_video(lockup: &Value, now: i64) -> Option<Video> {
    if lockup["contentType"] != "LOCKUP_CONTENT_TYPE_VIDEO" {
        return None; // playlists, mixes
    }
    let metadata = &lockup["metadata"]["lockupMetadataViewModel"];
    let rows: Vec<Vec<&str>> = metadata["metadata"]["contentMetadataViewModel"]["metadataRows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            row["metadataParts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p["text"]["content"].as_str())
                .collect()
        })
        .collect();
    let channel = &metadata["image"]["decoratedAvatarViewModel"]["rendererContext"]["commandContext"]["onTap"]["innertubeCommand"]
        ["browseEndpoint"];
    let mut badges = Vec::new();
    walk(&lockup["contentImage"], &mut |o| {
        if let Some(text) = o.get("thumbnailBadgeViewModel").and_then(|b| b["text"].as_str()) {
            badges.push(text);
        }
    });
    let live = if badges.iter().any(|b| b.eq_ignore_ascii_case("live")) { Live::Now } else { Live::No };
    Some(Video {
        id: lockup["contentId"].as_str()?.to_owned(),
        title: metadata["title"]["content"].as_str()?.to_owned(),
        channel: rows.first().and_then(|r| r.first()).map(|c| (*c).to_owned()).unwrap_or_default(),
        channel_id: channel["browseId"].as_str().map(str::to_owned),
        channel_url: channel["canonicalBaseUrl"].as_str().map(|path| format!("{ORIGIN}{path}")),
        duration: badges.iter().find_map(|b| parse_clock(b)),
        published: rows.get(1).and_then(|r| r.iter().find_map(|part| parse_age(part, now))),
        live,
        short: false, // Shorts come as `shortsLockupViewModel`, not read here
    })
}

/// `733` in "along with 733 other people"; `1,234` counts as 1234.
fn first_number(text: &str) -> Option<u64> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let digits: String =
        text[start..].chars().take_while(|c| c.is_ascii_digit() || *c == ',').filter(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn parse_clock(text: &str) -> Option<u32> {
    text.split(':').try_fold(0u32, |total, part| Some(total * 60 + part.trim().parse::<u32>().ok()?))
}

/// `"3 weeks ago"`, `"Streamed 1 year ago"`, and the short form of suggested
/// videos, `"3h ago"`, `"2mo ago"` → an approximate Unix time.
fn parse_age(text: &str, now: i64) -> Option<i64> {
    let text = text.trim_start_matches("Streamed ");
    let digits = text.find(|c: char| !c.is_ascii_digit()).filter(|end| *end > 0)?;
    let n: i64 = text[..digits].parse().ok()?;
    let rest = text[digits..].trim_start();
    if !rest.ends_with(" ago") {
        return None;
    }
    let unit = match rest.split_whitespace().next()?.trim_end_matches('s') {
        "second" => 1,
        "minute" | "m" | "min" => 60,
        "hour" | "h" => 3_600,
        "day" | "d" => 86_400,
        "week" | "w" | "wk" => 7 * 86_400,
        "month" | "mo" => 30 * 86_400,
        "year" | "y" | "yr" => 365 * 86_400,
        _ => return None,
    };
    Some(now - n * unit)
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Client for `youtubei/v1`: signed with the session cookies, or anonymous.
struct InnerTube {
    agent: ureq::Agent,
    cookie_header: String,
    jar: std::collections::HashMap<String, String>,
    /// Interface language of the answers; anonymous calls use English so
    /// relative dates ("3 weeks ago") can be read.
    hl: &'static str,
}

impl InnerTube {
    fn new(cookies: &Path) -> Result<Self, YoutubeError> {
        let text = std::fs::read_to_string(cookies).map_err(|_| YoutubeError::NoSession)?;
        let jar = session::parse_cookies(&text);
        session::authorization(&jar, 0).ok_or(YoutubeError::NoSession)?;
        let cookie_header = jar.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ");
        Ok(Self { agent: agent(), cookie_header, jar, hl: "es" })
    }

    /// Without login, for public data like searches.
    fn anonymous() -> Self {
        Self { agent: agent(), cookie_header: String::new(), jar: Default::default(), hl: "en" }
    }

    fn call(&self, endpoint: &str, mut body: Value) -> Result<Value, YoutubeError> {
        body["context"] = json!({ "client": { "clientName": "WEB", "clientVersion": CLIENT_VERSION, "hl": self.hl } });
        let mut request = self
            .agent
            .post(&format!("{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false"))
            .header("Content-Type", "application/json")
            .header("Origin", ORIGIN)
            .header("X-Origin", ORIGIN)
            .header("User-Agent", USER_AGENT);
        if !self.jar.is_empty() {
            let auth = session::authorization(&self.jar, unix_now() as u64).ok_or(YoutubeError::NoSession)?;
            request = request
                .header("Authorization", &auth)
                .header("X-Goog-AuthUser", "0")
                .header("Cookie", &self.cookie_header);
        }
        let text = request
            .send(body.to_string())
            .and_then(|mut reply| reply.body_mut().read_to_string())
            .map_err(|e| YoutubeError::Http(e.to_string()))?;
        Ok(serde_json::from_str(&text)?)
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into()
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
        // And the @handle in the other one.
        let handle =
            [&r["subscriberCountText"], &r["videoCountText"]].into_iter().filter_map(text).find(|t| t.starts_with('@'));
        channels.push(Channel {
            id: id.to_owned(),
            name: text(&r["title"]).unwrap_or_else(|| id.to_owned()),
            url: format!("{ORIGIN}/channel/{id}"),
            followers,
            handle,
            description: text(&r["descriptionSnippet"]).and_then(description),
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
                handle: None,
                description: None,
            });
        }
    });
    channels
}

/// Reads `updateSubscribeButtonAction.subscribed` from the API reply.
pub fn parse_subscription_reply(reply: &Value) -> Option<bool> {
    reply["actions"].as_array()?.iter().find_map(|a| a["updateSubscribeButtonAction"]["subscribed"].as_bool())
}

/// What `url` weighs in `format`, and how long it is. Blocking, a few seconds.
pub fn size(url: &str, format: &str, cookies: Option<&Path>) -> Result<crate::domain::Size, YoutubeError> {
    let mut args: Vec<String> =
        ["--no-warnings", "--skip-download", "--no-playlist", "-f", format].map(String::from).into();
    args.extend(["--print", "%(filesize,filesize_approx)s|%(duration)s|%(height)s"].map(String::from));
    if let Some(cookies) = cookies {
        args.extend(["--cookies".into(), cookies.display().to_string()]);
    }
    args.push(url.to_owned());
    Ok(crate::domain::parse_size(&run_yt_dlp(&args)?))
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
struct FlatList<T> {
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
    /// `https://www.youtube.com/shorts/…` for Shorts.
    url: Option<String>,
}

#[derive(Deserialize)]
struct ChannelEntry {
    id: String,
    title: Option<String>,
    url: Option<String>,
    channel_follower_count: Option<u64>,
    /// The @handle.
    uploader_id: Option<String>,
    description: Option<String>,
}

/// Parses `yt-dlp --flat-playlist -J` video lists. Entries without a title
/// (private or deleted videos) are skipped.
pub fn parse_videos(json: &str) -> Result<Vec<Video>, serde_json::Error> {
    let playlist: FlatList<VideoEntry> = serde_json::from_str(json)?;
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
                short: e.url.as_deref().is_some_and(|url| url.contains("/shorts/")),
                id: e.id,
            })
        })
        .collect())
}

/// A channel description without its blank lines and padding; none if empty.
fn description(text: String) -> Option<String> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Parses yt-dlp channel lists (channel search).
pub fn parse_channels(json: &str) -> Result<Vec<Channel>, serde_json::Error> {
    let playlist: FlatList<ChannelEntry> = serde_json::from_str(json)?;
    Ok(playlist
        .entries
        .into_iter()
        .flatten()
        .filter(|e| e.id.starts_with("UC"))
        .map(|e| Channel {
            url: e.url.unwrap_or_else(|| format!("{ORIGIN}/channel/{}", e.id)),
            name: e.title.unwrap_or_else(|| e.id.clone()),
            followers: e.channel_follower_count.map(|n| format!("{} suscriptores", compact(n))),
            handle: e.uploader_id.filter(|h| h.starts_with('@')),
            description: e.description.and_then(description),
            id: e.id,
        })
        .collect())
}

/// `950`, `12 k`, `1,2 M`.
pub fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{} k", n / 1_000),
        _ => format!("{},{} M", n / 1_000_000, n % 1_000_000 / 100_000),
    }
}

/// Starts the progress lines yt-dlp prints, `tubetui 45.3%|1.12MiB/s|00:12`
/// (read by `domain::parse_progress`).
pub const PROGRESS_PREFIX: &str = "tubetui ";

pub fn download_args(url: &str, format: &str, sub_lang: &str, dir: &Path, cookies: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-f".into(), format.into(), "--no-playlist".into()];
    // Progress lines, then the final file, on stdout.
    args.extend(["--newline", "--progress", "--progress-template"].map(String::from));
    args.push(format!(
        "download:{PROGRESS_PREFIX}%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s"
    ));
    args.extend(["--print", "after_move:filepath"].map(String::from));
    args.extend(["--write-subs", "--write-auto-subs", "--sub-langs"].map(String::from));
    args.push(sub_lang.into());
    args.extend(["-o".into(), dir.join("%(title)s.%(ext)s").display().to_string()]);
    if let Some(cookies) = cookies {
        args.extend(["--cookies".into(), cookies.display().to_string()]);
    }
    args.push(url.into());
    args
}

/// Downloads with subtitles, calling `progress` on every progress line;
/// blocking. The yt-dlp process waits in `slot` meanwhile, so another thread
/// can kill it (cancel). Returns the downloaded file.
pub fn download(
    args: &[String],
    slot: &std::sync::Mutex<Option<std::process::Child>>,
    mut progress: impl FnMut(u16, String, String),
) -> Result<std::path::PathBuf, YoutubeError> {
    use std::io::{BufRead, BufReader, Read};
    use std::process::Stdio;
    let mut child =
        Command::new("yt-dlp").args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let (Some(stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(YoutubeError::Failed("yt-dlp sin salida".into()));
    };
    // Read apart, or a chatty yt-dlp would block on a full pipe.
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    if let Ok(mut slot) = slot.lock() {
        *slot = Some(child);
    }
    let mut file = None;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        match line.strip_prefix(PROGRESS_PREFIX) {
            Some(rest) => {
                if let Some((permille, speed, eta)) = crate::domain::parse_progress(rest) {
                    progress(permille, speed, eta);
                }
            }
            None if !line.trim().is_empty() => file = Some(std::path::PathBuf::from(line.trim())),
            None => {}
        }
    }
    let status = slot.lock().ok().and_then(|mut s| s.take()).map(|mut child| child.wait()).transpose()?;
    let stderr = errors.join().unwrap_or_default();
    match file {
        Some(file) if status.is_some_and(|s| s.success()) => Ok(file),
        _ => Err(YoutubeError::Failed(failure_reason(&stderr))),
    }
}

/// The last `ERROR:` yt-dlp printed (warnings come after it sometimes), or its last line.
fn failure_reason(stderr: &str) -> String {
    let lines = || stderr.lines().rev().map(str::trim).filter(|l| !l.is_empty());
    let line = lines().find(|l| l.starts_with("ERROR:")).or_else(|| lines().next()).unwrap_or("sin detalle");
    line.trim_start_matches("ERROR:").trim().to_owned()
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
            {"id":"e","title":"E","duration":59.6,"url":"https://www.youtube.com/shorts/e"}]}"#;
        let videos = parse_videos(json).unwrap();
        assert_eq!(videos.len(), 4);
        assert!(videos[3].short && !videos[0].short);
        assert_eq!(videos[0].published, Some(100));
        assert_eq!(videos[0].channel_url.as_deref(), Some("https://www.youtube.com/@a"));
        assert_eq!(videos[1].live, Live::Now);
        assert_eq!(videos[2].live, Live::Upcoming);
        assert_eq!(videos[3].duration, Some(60));
    }

    #[test]
    fn parses_channel_search_and_skips_non_channels() {
        let json = r#"{"entries":[
            {"id":"UCabc","title":"Canal","url":"https://www.youtube.com/channel/UCabc","channel_follower_count":1234567,"description":"Un canal","uploader_id":"@canal"},
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
                            "subscriberCountText": { "simpleText": "@maria" },
                            "descriptionSnippet": { "runs": [{ "text": "  Música y charlas.\n\n Todos los viernes " }] } } },
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
        assert_eq!(channels[0].description.as_deref(), Some("Música y charlas.\nTodos los viernes"));
        assert_eq!(channels[1].description, None);
        assert_eq!(channels[0].handle.as_deref(), Some("@maria"));
        assert_eq!(channels[1].handle, None);
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

    fn search_reply() -> Value {
        let video = |id: &str, age: &str, extra: Value| {
            let mut renderer = json!({ "videoRenderer": {
                "videoId": id,
                "title": { "runs": [{ "text": format!("title {id}") }] },
                "ownerText": { "runs": [{ "text": "Codynn",
                    "navigationEndpoint": { "browseEndpoint": { "browseId": "UCcod", "canonicalBaseUrl": "/@codynn" } } }] },
                "lengthText": { "simpleText": "1:52:00" },
                "publishedTimeText": { "simpleText": age } } });
            if let Value::Object(extra) = extra {
                renderer["videoRenderer"].as_object_mut().unwrap().extend(extra);
            }
            renderer
        };
        json!({ "contents": { "sectionListRenderer": { "contents": [
            { "itemSectionRenderer": { "contents": [
                video("a", "1 day ago", json!({})),
                video("b", "Streamed 2 years ago", json!({})),
                video("live", "", json!({ "badges": [{ "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_LIVE_NOW", "label": "LIVE" } }] })),
                video("soon", "", json!({ "upcomingEventData": { "startTime": "1791900000" } })),
                video("a", "1 day ago", json!({})),
                { "shelfRenderer": { "title": { "simpleText": "People also watched" } } }
            ] } },
            { "continuationItemRenderer": { "continuationEndpoint": { "continuationCommand": { "token": "NEXT" } } } }
        ] } } })
    }

    #[test]
    fn parses_search_pages() {
        let now = 1_791_590_400;
        let page = parse_search(&search_reply(), now);
        let ids: Vec<&str> = page.videos.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "live", "soon"], "deduplicated, in order");
        let a = &page.videos[0];
        assert_eq!((a.title.as_str(), a.channel.as_str()), ("title a", "Codynn"));
        assert_eq!(a.channel_id.as_deref(), Some("UCcod"));
        assert_eq!(a.channel_url.as_deref(), Some("https://www.youtube.com/channel/UCcod"));
        assert_eq!(a.duration, Some(6720));
        assert_eq!(a.published, Some(now - 86_400));
        assert_eq!(page.videos[1].published, Some(now - 2 * 365 * 86_400));
        assert_eq!((page.videos[2].live, page.videos[3].live), (Live::Now, Live::Upcoming));
        assert_eq!(page.next.as_deref(), Some("NEXT"));
        assert_eq!(parse_search(&json!({}), now), SearchPage::default());
    }

    #[test]
    fn clock_and_age_texts() {
        assert_eq!(parse_clock("0:25"), Some(25));
        assert_eq!(parse_clock("LIVE"), None);
        assert_eq!(parse_age("3 weeks ago", 1_000_000_000), Some(1_000_000_000 - 21 * 86_400));
        assert_eq!(parse_age("1 hour ago", 100_000), Some(96_400));
        assert_eq!(parse_age("Premieres soon", 0), None);
        assert_eq!(parse_age("3h ago", 100_000), Some(100_000 - 3 * 3_600));
        assert_eq!(parse_age("2mo ago", 100_000_000), Some(100_000_000 - 60 * 86_400));
        assert_eq!(parse_age("8.2K", 100), None, "view counts are not ages");
    }

    #[test]
    fn targets() {
        assert_eq!(Source::Recommended.target(60), ":ytrec");
        assert!(!Source::Recommended.newest_first(), "the home feed keeps YouTube's ranking");
        assert!(Source::Subscriptions.newest_first());
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
        assert!(joined.contains("--print after_move:filepath"), "the final file, to play it");
        assert!(joined.contains(&format!("--progress-template download:{PROGRESS_PREFIX}%(progress._percent_str)s|")));
    }

    /// Same shape as a real `next` reply (trimmed).
    fn watch_reply() -> Value {
        let lockup = |id: &str, kind: &str, rows: Value, badge: &str| {
            json!({ "lockupViewModel": {
                "contentId": id, "contentType": kind,
                "contentImage": { "thumbnailViewModel": { "overlays": [{ "thumbnailBottomOverlayViewModel": {
                    "badges": [{ "thumbnailBadgeViewModel": { "text": badge } }] } }] } },
                "metadata": { "lockupMetadataViewModel": {
                    "title": { "content": format!("title {id}") },
                    "image": { "decoratedAvatarViewModel": { "rendererContext": { "commandContext": { "onTap": {
                        "innertubeCommand": { "browseEndpoint": { "browseId": "UCx", "canonicalBaseUrl": "/@x" } } } } } } },
                    "metadata": { "contentMetadataViewModel": { "metadataRows": rows } } } } } })
        };
        let parts = |texts: &[&str]| json!({ "metadataParts": texts.iter().map(|t| json!({ "text": { "content": t } })).collect::<Vec<_>>() });
        json!({ "contents": { "twoColumnWatchNextResults": {
            "results": { "results": { "contents": [
                { "videoPrimaryInfoRenderer": {
                    "viewCount": { "videoViewCountRenderer": {
                        "viewCount": { "simpleText": "5,771 views" }, "originalViewCount": "0" } },
                    "relativeDateText": { "simpleText": "3 hours ago" },
                    "videoActions": { "menuRenderer": { "topLevelButtons": [{ "segmentedLikeDislikeButtonViewModel": {
                        "likeButtonViewModel": {
                            "likeStatusEntity": { "likeStatus": "LIKE" },
                            "toggleButtonViewModel": { "defaultButtonViewModel": { "buttonViewModel": {
                            "accessibilityText": "like this video along with 1,733 other people" } } } } } }] } } } },
                { "videoSecondaryInfoRenderer": { "owner": { "videoOwnerRenderer": {
                    "subscriberCountText": { "simpleText": "559K subscribers" } } } } },
                { "itemSectionRenderer": { "sectionIdentifier": "comment-item-section", "contents": [
                    { "continuationItemRenderer": { "continuationEndpoint": {
                        "continuationCommand": { "token": "COMMENTS" } } } }] } }
            ] } },
            "secondaryResults": { "secondaryResults": { "results": [
                lockup("a", "LOCKUP_CONTENT_TYPE_VIDEO", json!([parts(&["Chan A"]), parts(&["8.2K", "2d ago"])]), "19:42"),
                lockup("mix", "LOCKUP_CONTENT_TYPE_PLAYLIST", json!([]), "50 videos"),
                lockup("b", "LOCKUP_CONTENT_TYPE_VIDEO", json!([parts(&["Chan B"]), parts(&["1K watching"])]), "LIVE"),
                lockup("a", "LOCKUP_CONTENT_TYPE_VIDEO", json!([]), "1:00"),
                { "continuationItemRenderer": {} }
            ] } } } } })
    }

    #[test]
    fn collects_history_ids_in_both_formats() {
        let page = json!({ "contents": [
            { "videoRenderer": { "videoId": "old" } },
            { "lockupViewModel": { "contentId": "new", "contentType": "LOCKUP_CONTENT_TYPE_VIDEO" } },
            { "lockupViewModel": { "contentId": "PLx", "contentType": "LOCKUP_CONTENT_TYPE_PLAYLIST" } }
        ] });
        assert_eq!(collect_video_ids(&page), HashSet::from(["old".to_string(), "new".to_string()]));
    }

    #[test]
    fn parses_watch_details_and_related_videos() {
        let now = 1_000_000;
        let watch = parse_watch(&watch_reply(), now);
        assert_eq!(watch.views, Some(5771));
        assert_eq!(watch.likes, Some(1733));
        assert_eq!(watch.published, Some(now - 3 * 3600));
        assert_eq!(watch.subscribers.as_deref(), Some("559K"));
        assert_eq!(watch.rating, Some(Rating::Like));
        assert_eq!(watch.comments_token.as_deref(), Some("COMMENTS"));
        let ids: Vec<&str> = watch.related.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"], "no playlists, no repeats");
        let a = &watch.related[0];
        assert_eq!((a.title.as_str(), a.channel.as_str()), ("title a", "Chan A"));
        assert_eq!(a.channel_id.as_deref(), Some("UCx"));
        assert_eq!(a.channel_url.as_deref(), Some("https://www.youtube.com/@x"));
        assert_eq!((a.duration, a.published, a.live), (Some(19 * 60 + 42), Some(now - 2 * 86_400), Live::No));
        assert_eq!((watch.related[1].duration, watch.related[1].live), (None, Live::Now));
        assert_eq!(first_number("no digits"), None);
    }

    #[test]
    fn discover_takes_turns_between_topics() {
        let renderer = |id: &str, views: &str| {
            json!({ "videoRenderer": {
                "videoId": id, "title": { "runs": [{ "text": format!("title {id}") }] },
                "ownerText": { "runs": [{ "text": "Chan", "navigationEndpoint": {
                    "browseEndpoint": { "browseId": "UCx" } } }] },
                "viewCountText": { "simpleText": views } } })
        };
        let rust = json!({ "contents": [renderer("a", "1,200 views"), renderer("b", "52,577 views")] });
        let nba = json!({ "contents": [renderer("c", "9,000 views"), renderer("d", "90,000 views"), renderer("b", "1 view")] });
        let per_topic = vec![parse_discover(&rust, 0), parse_discover(&nba, 0)];
        assert_eq!(per_topic[0][1].1, 52_577);
        let ids: Vec<String> = take_turns(per_topic).into_iter().map(|v| v.id).collect();
        assert_eq!(ids, ["b", "d", "a", "c"], "rust, nba, rust, nba; b only once");
        assert!(!Source::Discover(vec!["rust".into()]).newest_first());
    }

    #[test]
    fn a_stale_login_shows_as_an_empty_or_refused_feed() {
        assert!(looks_logged_out(&Ok(Items::Videos(Vec::new()))));
        assert!(!looks_logged_out(&Ok(Items::Profiles(vec!["brave".into()]))));
        let refused = YoutubeError::Http("ERROR: [youtube:subscriptions] Login details are needed".into());
        assert!(looks_logged_out(&Err(refused)));
        assert!(looks_logged_out(&Err(YoutubeError::Http("http status: 401".into()))), "the API refuses it");
        assert!(!looks_logged_out(&Err(YoutubeError::Http("http status: 500".into()))));
        assert!(Source::Recommended.needs_login() && !Source::Search("x".into()).needs_login());
        assert!(Source::MyPlaylists.needs_login());
    }

    #[test]
    fn ratings_map_to_youtube() {
        assert_eq!(Rating::from_status("DISLIKE"), Rating::Dislike);
        assert_eq!(Rating::from_status("INDIFFERENT"), Rating::None);
        assert_eq!(Rating::Like.endpoint(), "like/like");
        assert_eq!(Rating::None.endpoint(), "like/removelike");
    }

    #[test]
    fn parses_comments_in_both_formats() {
        let reply = json!({ "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
            { "payload": { "commentEntityPayload": {
                "properties": { "content": { "content": "Great\nsong" }, "publishedTime": "hace 6 años" },
                "author": { "displayName": "@ana" },
                "toolbar": { "likeCountNotliked": "568 K" } } } },
            { "payload": { "commentEntityPayload": {
                "properties": { "content": { "content": "meh" } }, "author": { "displayName": "@bo" },
                "toolbar": { "likeCountNotliked": " 0 " } } } },
            { "payload": { "commentEntityPayload": { "properties": { "content": { "content": "" } } } } },
            { "commentRenderer": { "authorText": { "simpleText": "@old" },
                "contentText": { "runs": [{ "text": "first " }, { "text": "comment" }] },
                "publishedTimeText": { "runs": [{ "text": "1 year ago" }] }, "voteCount": { "simpleText": "12" } } }
        ] } } });
        assert_eq!(
            parse_comments(&reply),
            [
                Comment {
                    author: "@ana".into(),
                    text: "Great\nsong".into(),
                    likes: Some("568 K".into()),
                    published: Some("hace 6 años".into())
                },
                Comment { author: "@bo".into(), text: "meh".into(), likes: None, published: None },
                Comment {
                    author: "@old".into(),
                    text: "first comment".into(),
                    likes: Some("12".into()),
                    published: Some("1 year ago".into())
                },
            ]
        );
    }

    #[test]
    fn download_failures_name_the_error_not_a_warning() {
        assert_eq!(failure_reason("ERROR: HTTP Error 403\nWARNING: no impersonate target\n"), "HTTP Error 403");
        assert_eq!(failure_reason("something odd\n\n"), "something odd");
        assert_eq!(failure_reason(""), "sin detalle");
    }

    #[test]
    fn collects_playlists_in_both_formats() {
        let page = json!({ "contents": [
            { "lockupViewModel": {
                "contentId": "PLnew", "contentType": "LOCKUP_CONTENT_TYPE_PLAYLIST",
                "contentImage": { "collectionThumbnailViewModel": { "primaryThumbnail": { "thumbnailViewModel": {
                    "overlays": [{ "thumbnailOverlayBadgeViewModel": { "thumbnailBadges": [
                        { "thumbnailBadgeViewModel": { "text": "12 videos" } }] } }] } } } },
                "metadata": { "lockupMetadataViewModel": { "title": { "content": "Jazz" } } } } },
            { "lockupViewModel": { "contentId": "vid", "contentType": "LOCKUP_CONTENT_TYPE_VIDEO" } },
            { "gridPlaylistRenderer": { "playlistId": "PLold", "title": { "runs": [{ "text": "Rust" }] },
                "videoCountShortText": { "simpleText": "3" } } },
            { "gridPlaylistRenderer": { "playlistId": "PLold", "title": { "simpleText": "Rust" } } }
        ]});
        assert_eq!(
            collect_playlists(&page),
            [
                Playlist { id: "PLnew".into(), title: "Jazz".into(), count: Some("12 videos".into()) },
                Playlist { id: "PLold".into(), title: "Rust".into(), count: Some("3".into()) },
            ]
        );
        assert_eq!(Source::Playlist("PLx".into()).target(10), "https://www.youtube.com/playlist?list=PLx");
    }

    #[test]
    fn compact_numbers() {
        assert_eq!(compact(950), "950");
        assert_eq!(compact(12_345), "12 k");
        assert_eq!(compact(1_234_567), "1,2 M");
    }
}
