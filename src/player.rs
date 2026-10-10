//! mpv, driven through its JSON IPC.
//!
//! In embedded mode mpv draws into a pane using the kitty graphics protocol;
//! hidden tabs drop their video track (`vid=no`) so they stop decoding.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use ratatui::layout::Rect;
use serde_json::{Value, json};

use crate::app::PlayerCommand;
use crate::config::{Language, PlayerConfig, PlayerMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Embedded,
    Window,
}

/// `Auto` embeds only in kitty and not inside a multiplexer, which would
/// swallow the graphics protocol.
pub fn resolve_mode(mode: PlayerMode, env: impl Fn(&str) -> Option<String>) -> Mode {
    match mode {
        PlayerMode::Embedded => Mode::Embedded,
        PlayerMode::Window => Mode::Window,
        PlayerMode::Auto => {
            let kitty = env("KITTY_WINDOW_ID").is_some() || env("TERM").as_deref() == Some("xterm-kitty");
            if kitty && env("TMUX").is_none() && env("ZELLIJ").is_none() { Mode::Embedded } else { Mode::Window }
        }
    }
}

/// Where the video goes, in the units mpv's kitty output expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// 1-based cell column.
    pub left: u16,
    /// 1-based cell row.
    pub top: u16,
    pub cols: u16,
    pub rows: u16,
    pub width_px: u32,
    pub height_px: u32,
}

impl Geometry {
    /// `cell` is the size of one terminal cell in pixels.
    pub fn new(area: Rect, cell: (u16, u16)) -> Self {
        Self {
            left: area.x + 1,
            top: area.y + 1,
            cols: area.width,
            rows: area.height,
            width_px: u32::from(area.width) * u32::from(cell.0),
            height_px: u32::from(area.height) * u32::from(cell.1),
        }
    }

    fn options(&self) -> [(&'static str, u32); 6] {
        [
            ("left", self.left.into()),
            ("top", self.top.into()),
            ("cols", self.cols.into()),
            ("rows", self.rows.into()),
            ("width", self.width_px),
            ("height", self.height_px),
        ]
    }
}

/// Pixel size of a terminal cell, with a sane fallback when unknown.
pub fn cell_size() -> (u16, u16) {
    match ratatui::crossterm::terminal::window_size() {
        Ok(ws) if ws.columns > 0 && ws.rows > 0 && ws.width > 0 && ws.height > 0 => {
            (ws.width / ws.columns, ws.height / ws.rows)
        }
        _ => (10, 20),
    }
}

/// yt-dlp format selector: preferred codec first, then anything that fits.
/// Dubbed audio prefers the `sub_lang` track and falls back to the original.
pub fn ytdl_format(config: &PlayerConfig) -> String {
    let fits = format!("[height<={}]", config.max_height);
    let audio = match config.language {
        Language::Dubbed => format!("ba[language^={}]/ba", config.sub_lang),
        Language::Subtitles | Language::Original => "ba".into(),
    };
    let any = format!("bv*{fits}+{audio}/b{fits}/b");
    match config.prefer_codec.as_str() {
        "" => any,
        codec => format!("bv*{fits}[vcodec^={codec}]+{audio}/{any}"),
    }
}

/// Everything about one playback except where it is drawn.
pub struct Launch<'a> {
    pub config: &'a PlayerConfig,
    pub mode: Mode,
    pub cookies: Option<&'a std::path::Path>,
    pub url: &'a str,
}

pub fn mpv_args(launch: &Launch, geometry: Option<Geometry>, ipc: &str) -> Vec<String> {
    let Launch { config, mode, cookies, url } = *launch;
    let lang = &config.sub_lang;
    let raw = raw_options(config, cookies);
    let mut args = vec![
        format!("--input-ipc-server={ipc}"),
        format!("--ytdl-format={}", ytdl_format(config)),
        format!("--ytdl-raw-options={raw}"),
        // Subtitles are always fetched so `j` can turn them on later.
        format!("--slang={lang}"),
        "--sub-auto=fuzzy".into(),
        format!("--sid={}", if config.language == Language::Subtitles { "auto" } else { "no" }),
        "--really-quiet".into(),
        "--input-terminal=no".into(),
    ];
    // Embedded frames are copied back to the CPU, so they need the `-copy` decoders.
    args.push(match (config.hwdec, mode) {
        (false, _) => "--hwdec=no".into(),
        (true, Mode::Window) => "--hwdec=auto".into(),
        (true, Mode::Embedded) => "--hwdec=auto-copy".into(),
    });
    match mode {
        Mode::Window => args.push("--force-window=immediate".into()),
        Mode::Embedded => {
            args.extend(["--vo=kitty", "--vo-kitty-alt-screen=no", "--vo-kitty-config-clear=no"].map(String::from));
            if cfg!(unix) {
                args.push("--vo-kitty-use-shm=yes".into());
            }
            if let Some(g) = geometry {
                args.extend(g.options().iter().map(|(name, value)| format!("--vo-kitty-{name}={value}")));
            }
        }
    }
    args.push(url.into());
    args
}

/// Deletes every image drawn with the kitty graphics protocol. mpv stops
/// drawing when its video is hidden, but its last frame stays on screen.
pub fn clear_images() {
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b_Ga=d,d=a\x1b\\");
    let _ = out.flush();
}

/// Reopens `path` at `seconds`, so new format/subtitle options take effect.
pub fn reload_line(path: &str, seconds: f64) -> String {
    json!({ "command": ["loadfile", path, "replace", -1, format!("start={seconds:.1}")] }).to_string()
}

/// yt-dlp options passed through mpv: subtitles to fetch and the login.
fn raw_options(config: &PlayerConfig, cookies: Option<&std::path::Path>) -> String {
    let mut raw = format!("sub-langs=[{}],write-subs=,write-auto-subs=", config.sub_lang);
    if let Some(cookies) = cookies {
        raw.push_str(&format!(",cookies={}", cookies.display()));
    }
    raw
}

/// IPC lines for a command. Most are JSON; reloading uses mpv's text
/// command form, the only one that expands `${time-pos}`.
/// `config` must already reflect `Language` / `SubLang` changes (see [`Player::send`]).
pub fn command_lines(config: &PlayerConfig, cookies: Option<&std::path::Path>, command: PlayerCommand) -> Vec<String> {
    let line = |value: Value| vec![value.to_string()];
    match command {
        PlayerCommand::TogglePause => line(json!({ "command": ["cycle", "pause"] })),
        PlayerCommand::Seek(seconds) => line(json!({ "command": ["seek", seconds, "relative"] })),
        PlayerCommand::Volume(delta) => line(json!({ "command": ["add", "volume", delta] })),
        PlayerCommand::ToggleMute => line(json!({ "command": ["cycle", "mute"] })),
        PlayerCommand::CycleSubtitles => line(json!({ "command": ["cycle", "sub"] })),
        PlayerCommand::Language(_) | PlayerCommand::SubLang(_) => {
            let sid = if config.language == Language::Subtitles { "auto" } else { "no" };
            vec![
                set_property("ytdl-format", ytdl_format(config).into()).to_string(),
                set_property("ytdl-raw-options", raw_options(config, cookies).into()).to_string(),
                set_property("slang", config.sub_lang.clone().into()).to_string(),
                set_property("sid", sid.into()).to_string(),
            ]
        }
    }
}

fn set_property(name: &str, value: Value) -> Value {
    json!({ "command": ["set_property", name, value] })
}

/// One running mpv instance, killed on drop.
pub struct Player {
    child: Child,
    ipc_path: String,
    ipc: Option<Box<dyn Write + Send>>,
    mode: Mode,
    config: PlayerConfig,
    cookies: Option<std::path::PathBuf>,
    /// Where the video is drawn; `None` while hidden or in window mode.
    shown_at: Option<Geometry>,
    /// We paused it because another video took over; a user pause is left alone.
    paused_by_us: bool,
}

impl Player {
    pub fn spawn(launch: &Launch, geometry: Option<Geometry>) -> std::io::Result<Self> {
        let ipc_path = ipc_path();
        let mode = launch.mode;
        let stdout = match mode {
            Mode::Embedded => Stdio::inherit(), // frames go to our terminal
            Mode::Window => Stdio::null(),
        };
        let child = Command::new("mpv")
            .args(mpv_args(launch, geometry, &ipc_path))
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(Stdio::null())
            .spawn()?;
        let shown_at = if mode == Mode::Embedded { geometry } else { None };
        let (config, cookies) = (launch.config.clone(), launch.cookies.map(Into::into));
        Ok(Self { child, ipc_path, ipc: None, mode, config, cookies, shown_at, paused_by_us: false })
    }

    pub fn send(&mut self, command: PlayerCommand) {
        match command {
            PlayerCommand::Language(language) => self.config.language = language,
            PlayerCommand::SubLang(code) => self.config.sub_lang = code.into(),
            _ => {}
        }
        for line in command_lines(&self.config, self.cookies.as_deref(), command) {
            self.write(&line);
        }
        if matches!(command, PlayerCommand::Language(_) | PlayerCommand::SubLang(_)) {
            // Same video, new audio/subtitles, same second.
            let path = self.query("path").and_then(|v| v.as_str().map(str::to_owned));
            let seconds = self.query("time-pos").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if let Some(path) = path {
                self.write(&reload_line(&path, seconds));
            }
        }
    }

    /// Reads a property on a separate connection; `None` if mpv does not answer quickly.
    fn query(&self, property: &str) -> Option<Value> {
        use std::io::{BufRead, BufReader};
        let mut stream = connect_stream(&self.ipc_path).ok()?;
        #[cfg(unix)]
        stream.set_read_timeout(Some(std::time::Duration::from_millis(500))).ok()?;
        writeln!(stream, "{}", json!({ "command": ["get_property", property], "request_id": 1 })).ok()?;
        BufReader::new(stream)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
            .find(|reply| reply["request_id"] == 1)
            .map(|reply| reply["data"].clone())
    }

    /// Shows the video at `geometry`, resuming it if we had paused it.
    /// No-op in window mode.
    pub fn show(&mut self, geometry: Geometry) {
        if self.mode != Mode::Embedded {
            return;
        }
        if self.shown_at != Some(geometry) {
            // The old placement would stay on screen at the old size/position.
            clear_images();
            for (name, value) in geometry.options() {
                self.write(&set_property(&format!("options/vo-kitty-{name}"), value.into()).to_string());
            }
            // Recreating the video output applies the new geometry and keeps the position.
            self.write(&set_property("vid", "no".into()).to_string());
            self.write(&set_property("vid", "auto".into()).to_string());
            self.shown_at = Some(geometry);
        }
        self.resume();
    }

    /// Stops drawing and decoding video. With `keep_audio` it keeps playing
    /// (background listening); otherwise it is paused until shown again.
    /// Returns whether it was on screen (its last frame must be cleared).
    pub fn hide(&mut self, keep_audio: bool) -> bool {
        if self.mode != Mode::Embedded {
            return false;
        }
        let was_shown = self.shown_at.take().is_some();
        if was_shown {
            self.write(&set_property("vid", "no".into()).to_string());
        }
        if keep_audio {
            self.resume();
        } else if !self.paused_by_us {
            self.write(&set_property("pause", true.into()).to_string());
            self.paused_by_us = true;
        }
        was_shown
    }

    fn resume(&mut self) {
        if std::mem::take(&mut self.paused_by_us) {
            self.write(&set_property("pause", false.into()).to_string());
        }
    }

    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Best effort: if mpv is not listening yet the message is dropped.
    fn write(&mut self, line: &str) {
        if self.ipc.is_none() {
            self.ipc = connect(&self.ipc_path).ok();
            // We never read replies or events; keep mpv from queueing them.
            if let Some(ipc) = self.ipc.as_mut() {
                let _ = writeln!(ipc, "{}", json!({ "command": ["disable_event", "all"] }));
            }
        }
        if let Some(ipc) = self.ipc.as_mut()
            && writeln!(ipc, "{line}").is_err()
        {
            self.ipc = None;
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.ipc_path);
    }
}

fn ipc_path() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!("tubetui-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    if cfg!(windows) {
        format!(r"\\.\pipe\{name}")
    } else {
        std::env::temp_dir().join(format!("{name}.sock")).display().to_string()
    }
}

/// Write side of the IPC connection. mpv answers every command; a thread
/// discards the answers, otherwise the socket fills up and writes block.
fn connect(path: &str) -> std::io::Result<Box<dyn Write + Send>> {
    let stream = connect_stream(path)?;
    let mut replies = stream.try_clone()?;
    std::thread::spawn(move || std::io::copy(&mut replies, &mut std::io::sink()));
    Ok(Box::new(stream))
}

#[cfg(unix)]
fn connect_stream(path: &str) -> std::io::Result<std::os::unix::net::UnixStream> {
    std::os::unix::net::UnixStream::connect(path)
}

#[cfg(windows)]
fn connect_stream(path: &str) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string())
    }

    #[test]
    fn explicit_modes_win() {
        assert_eq!(resolve_mode(PlayerMode::Window, env(&[("KITTY_WINDOW_ID", "1")])), Mode::Window);
        assert_eq!(resolve_mode(PlayerMode::Embedded, env(&[])), Mode::Embedded);
    }

    #[test]
    fn auto_embeds_only_in_kitty_without_multiplexer() {
        assert_eq!(resolve_mode(PlayerMode::Auto, env(&[("KITTY_WINDOW_ID", "1")])), Mode::Embedded);
        assert_eq!(resolve_mode(PlayerMode::Auto, env(&[("TERM", "xterm-kitty")])), Mode::Embedded);
        assert_eq!(resolve_mode(PlayerMode::Auto, env(&[("TERM", "xterm-256color")])), Mode::Window);
        assert_eq!(resolve_mode(PlayerMode::Auto, env(&[("KITTY_WINDOW_ID", "1"), ("TMUX", "/tmp/t")])), Mode::Window);
    }

    #[test]
    fn geometry_is_one_based_with_pixel_size() {
        let g = Geometry::new(Rect::new(25, 2, 80, 20), (10, 22));
        assert_eq!(g, Geometry { left: 26, top: 3, cols: 80, rows: 20, width_px: 800, height_px: 440 });
    }

    fn launch(config: &PlayerConfig, mode: Mode) -> Launch<'_> {
        Launch { config, mode, cookies: None, url: "URL" }
    }

    #[test]
    fn format_prefers_codec_then_falls_back() {
        assert_eq!(
            ytdl_format(&PlayerConfig::default()),
            "bv*[height<=720][vcodec^=avc1]+ba/bv*[height<=720]+ba/b[height<=720]/b"
        );
        let any = PlayerConfig { max_height: 480, prefer_codec: String::new(), ..Default::default() };
        assert_eq!(ytdl_format(&any), "bv*[height<=480]+ba/b[height<=480]/b");
    }

    #[test]
    fn dubbed_prefers_audio_in_sub_lang() {
        let config = PlayerConfig { language: Language::Dubbed, prefer_codec: String::new(), ..Default::default() };
        assert_eq!(ytdl_format(&config), "bv*[height<=720]+ba[language^=es]/ba/b[height<=720]/b");
    }

    #[test]
    fn subtitles_are_fetched_and_shown_only_in_subtitles_mode() {
        let config = PlayerConfig::default();
        let args = mpv_args(&launch(&config, Mode::Window), None, "s");
        assert!(args.contains(&"--sid=auto".into()));
        assert!(args.contains(&"--slang=es".into()));
        assert!(args.contains(&"--ytdl-raw-options=sub-langs=[es],write-subs=,write-auto-subs=".into()));
        let original = PlayerConfig { language: Language::Original, ..Default::default() };
        assert!(mpv_args(&launch(&original, Mode::Window), None, "s").contains(&"--sid=no".into()));
    }

    #[test]
    fn cookies_go_to_yt_dlp() {
        let config = PlayerConfig::default();
        let with = Launch { cookies: Some(std::path::Path::new("/c.txt")), ..launch(&config, Mode::Window) };
        let args = mpv_args(&with, None, "s");
        assert!(
            args.iter().any(|a| a.starts_with("--ytdl-raw-options=") && a.ends_with(",cookies=/c.txt")),
            "{args:?}"
        );
    }

    #[test]
    fn window_args() {
        let config = PlayerConfig::default();
        let args = mpv_args(&launch(&config, Mode::Window), None, "/tmp/s");
        assert!(args.contains(&"--hwdec=auto".into()));
        assert!(args.contains(&"--force-window=immediate".into()));
        assert!(!args.iter().any(|a| a.starts_with("--vo")));
        assert_eq!(args.last().unwrap(), "URL");
    }

    #[test]
    fn embedded_args_place_video_and_leave_screen_to_us() {
        let g = Geometry::new(Rect::new(0, 1, 40, 10), (10, 20));
        let config = PlayerConfig::default();
        let args = mpv_args(&launch(&config, Mode::Embedded), Some(g), "/tmp/s");
        for expected in [
            "--vo=kitty",
            "--vo-kitty-alt-screen=no",
            "--vo-kitty-config-clear=no",
            "--hwdec=auto-copy",
            "--vo-kitty-left=1",
            "--vo-kitty-top=2",
            "--vo-kitty-width=400",
            "--vo-kitty-height=200",
            "--input-ipc-server=/tmp/s",
        ] {
            assert!(args.contains(&expected.to_string()), "missing {expected}: {args:?}");
        }
    }

    #[test]
    fn hwdec_can_be_disabled() {
        let config = PlayerConfig { hwdec: false, ..Default::default() };
        assert!(mpv_args(&launch(&config, Mode::Embedded), None, "s").contains(&"--hwdec=no".into()));
    }

    #[test]
    fn commands_serialize_to_mpv_ipc() {
        let config = PlayerConfig::default();
        let one = |command| command_lines(&config, None, command).join("\n");
        assert_eq!(one(PlayerCommand::TogglePause), r#"{"command":["cycle","pause"]}"#);
        assert_eq!(one(PlayerCommand::Seek(-5)), r#"{"command":["seek",-5,"relative"]}"#);
        assert_eq!(one(PlayerCommand::Volume(5)), r#"{"command":["add","volume",5]}"#);
        assert_eq!(one(PlayerCommand::ToggleMute), r#"{"command":["cycle","mute"]}"#);
        assert_eq!(one(PlayerCommand::CycleSubtitles), r#"{"command":["cycle","sub"]}"#);
    }

    #[test]
    fn language_change_reloads_at_the_same_second() {
        let config = PlayerConfig {
            prefer_codec: String::new(),
            language: Language::Dubbed,
            sub_lang: "en".into(),
            ..Default::default()
        };
        let cookies = std::path::Path::new("/c.txt");
        let lines = command_lines(&config, Some(cookies), PlayerCommand::SubLang("en"));
        assert_eq!(
            lines,
            [
                r#"{"command":["set_property","ytdl-format","bv*[height<=720]+ba[language^=en]/ba/b[height<=720]/b"]}"#,
                r#"{"command":["set_property","ytdl-raw-options","sub-langs=[en],write-subs=,write-auto-subs=,cookies=/c.txt"]}"#,
                r#"{"command":["set_property","slang","en"]}"#,
                r#"{"command":["set_property","sid","no"]}"#,
            ]
        );
        assert_eq!(
            reload_line("https://youtu.be/x", 83.456),
            r#"{"command":["loadfile","https://youtu.be/x","replace",-1,"start=83.5"]}"#
        );
        let subs = PlayerConfig { language: Language::Subtitles, ..config };
        let lines = command_lines(&subs, None, PlayerCommand::Language(Language::Subtitles));
        assert_eq!(lines[3], r#"{"command":["set_property","sid","auto"]}"#);
    }

    #[test]
    fn ipc_paths_are_unique() {
        assert_ne!(ipc_path(), ipc_path());
    }
}
