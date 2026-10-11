//! tubetui binary: terminal setup and the effect runtime around the pure core.
//!
//! Slow work (yt-dlp, the YouTube API, reading browser cookies) runs on
//! short-lived threads that report back as `Action`s; each video is its own
//! mpv process.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use clap::Parser;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event, MouseButton,
    MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::Rect;

use tubetui::anim::Animations;
use tubetui::app::{Action, Effect, Playback, PlayerCommand, SettingValue, State};
use tubetui::config::{self, Config, PlayerMode};
use tubetui::domain::{TabId, Video};
use tubetui::i18n::Lang;
use tubetui::media::{self, MediaKind};
use tubetui::player::{self, Geometry, Launch, Mode, Player};
use tubetui::store::{self, Paths};
use tubetui::thumbs::Thumbnails;
use tubetui::youtube::{self, Items};
use tubetui::{keymap, mouse, session, ui};

const SUBS_MAX_AGE: Duration = Duration::from_secs(24 * 3600);
/// While the window is being resized the video stays hidden; it is placed
/// once the size has been stable this long (each placement restarts mpv's output).
const RESIZE_SETTLE: Duration = Duration::from_millis(250);
/// mpv may flush one more frame after being hidden; clear again after this.
const LATE_FRAME: Duration = Duration::from_millis(300);
/// Thumbnails downloaded ahead of the selection.
const THUMBS_AHEAD: usize = 10;
/// Time between animation frames.
const FRAME: Duration = Duration::from_millis(33);
/// How often an overlay follows the terminal window when it moves.
const OVERLAY_FOLLOW: Duration = Duration::from_millis(500);
/// Two clicks on the same cell within this time are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// YouTube in your terminal.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Config file [default: platform config dir, e.g. ~/.config/tubetui/config.toml]
    #[arg(long)]
    config: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.or_else(Config::default_path);
    let mut config = match &config_path {
        Some(path) => Config::load(path)?,
        None => Config::default(),
    };
    // Until a language is chosen, follow the system's.
    config.ui.lang.get_or_insert_with(|| {
        let locale =
            ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
        Lang::from_locale(&locale.unwrap_or_default())
    });
    let paths = Paths::system().context("no se encontró la carpeta personal")?;
    let later = load_later(&paths)?;
    let crash_log = paths.cache.join("crash.log");
    log_panics(crash_log.clone());

    let mut runtime = Runtime::new(config, config_path, paths, later);
    let mut terminal = ratatui::init();
    set_focus_reports(true);
    let result = runtime.run(&mut terminal);
    // Stop every mpv first so no frame arrives after the cleanup.
    drop(runtime);
    player::clear_images();
    set_mouse(false);
    set_focus_reports(false);
    ratatui::restore();
    if let Err(e) = &result {
        append(&crash_log, &format!("error: {e:#}"));
    }
    result
}

/// Launched from a window manager shortcut, the terminal closes with the
/// app and takes any error message with it; keep a copy on disk.
fn log_panics(path: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        append(&path, &format!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture()));
        // Otherwise the shell keeps getting mouse and focus codes.
        set_mouse(false);
        set_focus_reports(false);
        previous(info);
    }));
}

fn append(path: &Path, message: &str) {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{}] {message}", now());
    }
}

/// "Ver después", migrating the old ytv list the first time.
fn load_later(paths: &Paths) -> anyhow::Result<Vec<Video>> {
    if !paths.later.exists()
        && let Some(ytv) = directories::BaseDirs::new().map(|d| d.data_local_dir().join("ytv/despues.tsv"))
        && let Ok(text) = std::fs::read_to_string(ytv)
    {
        let videos = store::parse_ytv_later(&text);
        store::save_later(&paths.later, &videos)?;
        return Ok(videos);
    }
    store::load_later(&paths.later)
        .with_context(|| format!("no se pudo leer {} (no lo sobrescribo)", paths.later.display()))
}

struct Runtime {
    config_path: Option<PathBuf>,
    paths: Paths,
    state: State,
    players: HashMap<TabId, Player>,
    tx: mpsc::Sender<Action>,
    rx: mpsc::Receiver<Action>,
    /// Cookie file of the current login, once it is known to work.
    cookies: Option<PathBuf>,
    /// The video tab seen most recently; the one allowed to play in the background.
    last_watched: Option<TabId>,
    /// Last terminal resize, while it is still settling.
    resized_at: Option<Instant>,
    /// When to clear leftover video frames again.
    clear_at: Option<Instant>,
    /// The terminal window was put in fullscreen by us.
    window_fullscreen: bool,
    /// When the list being loaded was requested, for the loading indicator.
    loading_since: Option<Instant>,
    /// Players of closed tabs, quitting in the background.
    closing: Vec<std::thread::JoinHandle<()>>,
    animations: Animations,
    thumbs: Thumbnails,
    /// The terminal sends us mouse events (follows `ui.mouse`).
    mouse_on: bool,
    /// The last left click, to tell double clicks.
    last_click: Option<(Instant, u16, u16)>,
    /// `xwininfo`, `xdotool` and `xprop` are installed (overlay mode needs them).
    x11_tools: bool,
    /// Overlay: where the terminal's cell grid starts on the screen, and when it was checked.
    grid: Option<(i32, i32)>,
    grid_checked: Option<Instant>,
    /// The terminal lost the focus to another program: overlays hide.
    away: bool,
}

impl Drop for Runtime {
    /// Every mpv must be gone before the final cleanup, or a late frame stays.
    fn drop(&mut self) {
        for handle in self.closing.drain(..) {
            let _ = handle.join();
        }
    }
}

impl Runtime {
    fn new(config: Config, config_path: Option<PathBuf>, paths: Paths, later: Vec<Video>) -> Self {
        let mut state = State::new(config, later);
        state.update(Action::SubsLoaded(store::load_subs(&paths.subs)));
        state.update(Action::HistoryLoaded(store::load_watched(&paths.watched)));
        let (tx, rx) = mpsc::channel();
        let runtime = Self {
            config_path,
            paths,
            state,
            players: HashMap::new(),
            tx,
            rx,
            cookies: None,
            last_watched: None,
            resized_at: None,
            clear_at: None,
            window_fullscreen: false,
            loading_since: None,
            closing: Vec::new(),
            animations: Animations::default(),
            thumbs: Thumbnails::default(),
            mouse_on: false,
            last_click: None,
            x11_tools: ["xwininfo", "xdotool", "xprop"].iter().all(|tool| on_path(tool)),
            grid: None,
            grid_checked: None,
            away: false,
        };
        runtime.start_session(false);
        runtime.send(Action::AskBrowser);
        runtime.background(|| {
            let (manager, checks) = tubetui::doctor::run();
            Action::SystemChecked(manager, checks)
        });
        if runtime.state.config.ui.check_updates {
            // Quietly: no network or no news means nothing to say.
            let (tx, cache) = (runtime.tx.clone(), runtime.paths.cache.join("latest-release.txt"));
            std::thread::spawn(move || {
                if let Ok(Some(update)) = tubetui::update::check(&cache) {
                    let _ = tx.send(Action::UpdateFound(update));
                }
            });
        }
        runtime
    }

    fn tr(&self, es: &'static str, en: &'static str) -> &'static str {
        self.state.lang().pick(es, en)
    }

    fn mode(&self) -> Mode {
        match player::resolve_mode(self.state.config.player.mode, |key| std::env::var(key).ok()) {
            Mode::Overlay if !self.x11_tools => Mode::Window,
            mode => mode,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        let mut dirty = true;
        let mut was_opening = false;
        while !self.state.quit {
            // Animate the loading indicator.
            match (&self.state.loading, self.loading_since) {
                (Some(_), None) => self.loading_since = Some(Instant::now()),
                (None, Some(_)) => self.loading_since = None,
                _ => {}
            }
            // The video of the active tab still loading: animate its bar, then
            // redraw once more when it starts.
            let opening = self.state.active_tab().and_then(|t| self.players.get(&t.id)).and_then(Player::opening);
            // Each loop waits up to 100 ms for input: one spinner frame.
            if self.loading_since.is_some() || opening.is_some() || was_opening {
                dirty = true;
            }
            was_opening = opening.is_some();
            if self.resized_at.is_some_and(|t| t.elapsed() >= RESIZE_SETTLE) {
                self.resized_at = None;
                self.thumbs.forget();
                dirty = true;
            }
            if self.clear_at.is_some_and(|t| Instant::now() >= t) {
                self.clear_at = None;
                if ui::video_area(&self.state, terminal.size()?.into()).is_none() || self.resized_at.is_some() {
                    self.clear_images();
                    dirty = true;
                }
            }
            if self.thumbs.receive() {
                dirty = true;
            }
            // Every player, so the flags of hidden tabs do not pile up.
            if self.players.values().filter(|p| p.info_changed()).count() > 0 {
                dirty = true;
            }
            // Redraw only on change: every redraw competes with mpv for the terminal.
            if dirty || self.animations.running() {
                let loading = self.loading_since.map(|t| t.elapsed());
                let embedded = self.mode().in_pane();
                // Thumbnails need kitty itself, whatever the video mode.
                let kitty = player::resolve_mode(PlayerMode::Auto, |key| std::env::var(key).ok()) == Mode::Embedded;
                let thumbnails = kitty && self.state.config.ui.thumbnails;
                let playing = self.state.active_tab().and_then(|t| self.players.get(&t.id)).map(Player::info);
                let view = ui::View { embedded, now: now(), loading, opening, thumbnails, playing };
                let t = Instant::now();
                self.animations.observe(&self.state, terminal.size()?.into());
                terminal.draw(|f| {
                    ui::draw(f, &self.state, &view);
                    self.animations.render(f);
                })?;
                dbg(format!("draw {:?}", t.elapsed()));
                let t = Instant::now();
                self.place_videos(terminal.size()?.into());
                dbg(format!("place {:?}", t.elapsed()));
                if thumbnails {
                    let tab = self.state.active_tab();
                    let current = tab.and_then(|t| t.playback.video.as_ref()).map(|v| v.id.as_str());
                    self.thumbs.want(
                        current
                            .into_iter()
                            .chain(self.state.videos_from_selection(THUMBS_AHEAD).iter().map(|v| v.id.as_str())),
                    );
                    self.thumbs.show(ui::thumbnail_area(&self.state, terminal.size()?.into()));
                } else {
                    self.thumbs.show(None);
                }
                dirty = false;
            }
            self.state.video_window = self.mode() == Mode::Window;
            if self.mode() == Mode::Overlay
                && self.state.active_tab().is_some()
                && self.grid_checked.is_none_or(|t| t.elapsed() >= OVERLAY_FOLLOW)
            {
                self.grid_checked = Some(Instant::now());
                let grid = player::grid_origin(player::cell_size());
                if grid != self.grid {
                    self.grid = grid;
                    dirty = true;
                }
            }
            if self.mouse_on != self.state.config.ui.mouse {
                self.mouse_on = self.state.config.ui.mouse;
                set_mouse(self.mouse_on);
            }
            let mut pending = Vec::new();
            // Animations want about 30 frames a second; otherwise wait longer.
            let wait = if self.animations.running() { FRAME } else { Duration::from_millis(100) };
            if event::poll(wait).context("reading terminal events")? {
                match event::read()? {
                    Event::Key(key) => {
                        dbg(format!("key {:?} {:?}", key.code, key.kind));
                        pending.extend(keymap::map(key, self.state.key_context()))
                    }
                    // Another program in front: an overlay would float over it.
                    Event::FocusLost if self.mode() == Mode::Overlay && !player::overlay_has_focus() => {
                        self.away = true;
                        dirty = true;
                    }
                    Event::FocusGained if self.away => {
                        self.away = false;
                        dirty = true;
                    }
                    Event::Mouse(m) => {
                        let double = m.kind == MouseEventKind::Down(MouseButton::Left) && {
                            let again = self
                                .last_click
                                .is_some_and(|(at, x, y)| at.elapsed() < DOUBLE_CLICK && (x, y) == (m.column, m.row));
                            // A third click starts over.
                            self.last_click = (!again).then(|| (Instant::now(), m.column, m.row));
                            again
                        };
                        let playing = self.state.active_tab().and_then(|t| self.players.get(&t.id)).map(Player::info);
                        let spot = ui::spot(&self.state, playing.as_ref(), terminal.size()?.into(), m.column, m.row);
                        pending.extend(mouse::actions(&self.state, spot, m.kind, double));
                    }
                    // No `terminal.clear()`: it asks the terminal for the cursor
                    // position and, with kitty busy drawing video, can wait 2 s
                    // and fail. `draw` already resizes and repaints by itself.
                    Event::Resize(..) => {
                        self.resized_at = Some(Instant::now());
                        dirty = true;
                    }
                    _ => {}
                }
            }
            pending.extend(self.rx.try_iter());
            self.retry_with_cookies(terminal.size()?.into());
            pending.extend(
                (self.players.iter_mut())
                    .filter_map(|(id, p)| p.has_exited().then_some(Action::PlayerClosed(*id, None))),
            );
            for action in pending {
                dirty = true;
                if let Action::SessionReady { cookies, subs } = &action {
                    self.cookies = cookies.clone().filter(|_| subs.is_ok());
                }
                for effect in self.state.update(action) {
                    let t = Instant::now();
                    let name = format!("{effect:?}").chars().take(40).collect::<String>();
                    self.perform(effect, terminal)?;
                    dbg(format!("effect {name} {:?}", t.elapsed()));
                }
            }
            // Players whose tab is gone (the video ended or mpv failed).
            let gone: Vec<TabId> =
                self.players.keys().filter(|id| !self.state.tabs.iter().any(|t| t.id == **id)).copied().collect();
            for id in gone {
                self.stop(id);
            }
            self.closing.retain(|handle| !handle.is_finished());
            // Leaving fullscreen any way (Esc, another tab, closing) restores the window.
            if self.window_fullscreen && !self.state.fullscreen {
                player::set_window_fullscreen(false);
            }
            self.window_fullscreen = self.state.fullscreen && self.mode().in_pane();
        }
        if self.window_fullscreen {
            player::set_window_fullscreen(false);
        }
        Ok(())
    }

    fn perform(&mut self, effect: Effect, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        match effect {
            Effect::Load { id, source, limit } => {
                let cookies = self.cookies.clone();
                self.background(move || {
                    let result = youtube::fetch(&source, limit, cookies.as_deref()).map_err(|e| e.to_string());
                    Action::Loaded(id, result)
                });
            }
            Effect::Search { id, query, next } => {
                self.background(move || {
                    Action::Page(id, youtube::search(&query, next.as_deref()).map_err(|e| e.to_string()))
                });
            }
            Effect::CheckSystem => {
                self.background(|| {
                    let (manager, checks) = tubetui::doctor::run();
                    Action::SystemChecked(manager, checks)
                });
            }
            Effect::LoadHistory => {
                if let Some(cookies) = self.cookies.clone() {
                    // Without it only the local history hides videos.
                    self.background(move || {
                        Action::HistoryLoaded(youtube::watch_history(&cookies).unwrap_or_default())
                    });
                }
            }
            Effect::SaveWatched(id) => {
                if let Err(e) = store::add_watched(&self.paths.watched, &id) {
                    dbg(format!("watched.txt: {e}"));
                }
            }
            Effect::Trash(paths) => {
                self.background(move || {
                    Action::Trashed(trash::delete_all(&paths).map(|()| paths).map_err(|e| e.to_string()))
                });
            }
            Effect::Watch { tab, video } => {
                self.background(move || Action::Watched(tab, youtube::watch(&video).map_err(|e| e.to_string())));
            }
            Effect::ScanBrowsers { id } => {
                let home = directories::BaseDirs::new().map(|d| d.home_dir().to_owned()).unwrap_or_default();
                let mut profiles =
                    session::scan_browsers(&home, &session::resolve_keyring(&self.state.config.session.keyring));
                profiles.push(String::new()); // "Sin sesión"
                self.send(Action::Loaded(id, Ok(Items::Profiles(profiles))));
            }
            Effect::ScanMedia { id, kind } => {
                let roots = self.media_dirs(kind);
                self.background(move || {
                    let files = media::scan(&roots, kind);
                    Action::Loaded(id, Ok(Items::Files(files, kind)))
                });
            }
            Effect::StartSession { force } => self.start_session(force),
            Effect::SaveSetting { table, key, value } => {
                let Some(path) = &self.config_path else { return Ok(()) };
                let result = match value {
                    SettingValue::Text(text) => config::set_value(path, table, key, text),
                    SettingValue::Number(number) => config::set_value(path, table, key, number),
                    SettingValue::Bool(flag) => config::set_value(path, table, key, flag),
                    SettingValue::List(items) => {
                        config::set_value(path, table, key, items.into_iter().collect::<toml_edit::Array>())
                    }
                };
                if let Err(e) = result {
                    self.send(Action::Status(e.to_string()));
                }
            }
            Effect::ImportCookies(path) => {
                let dest = self.paths.cache.join("cookies-imported.txt");
                self.send(Action::CookiesImported(import_cookies(&expand_home(&path), &dest)));
            }
            Effect::EditConfig => self.edit_config(terminal)?,
            Effect::SaveLater(videos) => {
                if let Err(e) = store::save_later(&self.paths.later, &videos) {
                    let failed = self.tr("No se pudo guardar Ver después", "Could not save Watch later");
                    self.send(Action::Status(format!("{failed}: {e}")));
                }
            }
            Effect::SaveSubs(subs) => {
                // Cache only; a failure just means reading them again next time.
                let _ = store::save_subs(&self.paths.subs, &subs);
            }
            Effect::Subscribe { channel_id, name, subscribe } => {
                let cookies = self.cookies.clone();
                self.background(move || {
                    let result = match cookies {
                        Some(cookies) => {
                            youtube::set_subscription(&cookies, &channel_id, subscribe).map_err(|e| e.to_string())
                        }
                        None => Err(youtube::YoutubeError::NoSession.to_string()),
                    };
                    Action::SubscriptionDone { channel_id, name, result }
                });
            }
            Effect::Download(video) => {
                let dir = self.download_dir();
                let player = &self.state.config.player;
                let args = youtube::download_args(
                    &video.url(),
                    &player::ytdl_format(player, Mode::Window),
                    &player.sub_lang,
                    &dir,
                    self.cookies.as_deref(),
                );
                let lang = self.state.lang();
                self.background(move || match youtube::download(&args) {
                    Ok(()) => Action::Status(format!(
                        "✅ {} {}: {}",
                        lang.pick("Descarga lista en", "Downloaded to"),
                        dir.display(),
                        video.title
                    )),
                    Err(e) => Action::Status(format!(
                        "❌ {} {}: {e}",
                        lang.pick("Falló la descarga de", "Download failed:"),
                        video.title
                    )),
                });
            }
            Effect::Open(id, playback) => self.open(id, &playback, false, terminal.size()?.into()),
            Effect::Close(id) => self.stop(id),
            Effect::Command(id, command) => {
                if let PlayerCommand::Fullscreen(on) = command
                    && self.mode().in_pane()
                {
                    player::set_window_fullscreen(on);
                }
                if let Some(p) = self.players.get_mut(&id) {
                    p.send(command);
                }
            }
        }
        Ok(())
    }

    /// Starts mpv for a tab. Videos open without the login: yt-dlp takes
    /// several times longer with cookies (measured 7 s against 23 s), and
    /// only a few videos (age-restricted, members-only, private) need them.
    fn open(&mut self, id: TabId, playback: &Playback, with_cookies: bool, screen: Rect) {
        if self.mode() == Mode::Overlay {
            self.grid = player::grid_origin(player::cell_size());
        }
        let geometry = ui::video_area(&self.state, screen).map(|a| Geometry::new(a, player::cell_size())).map(|g| {
            match self.grid.filter(|_| self.mode() == Mode::Overlay) {
                Some(grid) => g.on_screen(grid),
                None => g,
            }
        });
        let launch = Launch {
            config: &self.state.config.player,
            mode: self.mode(),
            cookies: self.cookies.as_deref().filter(|_| with_cookies),
            queue: &playback.queue,
            audio_only: playback.audio_only,
        };
        match Player::spawn(&launch, geometry) {
            Ok(p) => {
                self.players.insert(id, p);
            }
            Err(e) => {
                let failed = self.tr("No se pudo iniciar mpv", "Could not start mpv");
                self.send(Action::PlayerClosed(id, Some(format!("{failed}: {e}"))))
            }
        }
    }

    /// A video that ended before playing may need the login: try once more with it.
    fn retry_with_cookies(&mut self, screen: Rect) {
        if self.cookies.is_none() {
            return;
        }
        let failed: Vec<TabId> = (self.players.iter_mut())
            .filter_map(|(id, p)| (!p.used_cookies() && p.opening().is_some() && p.has_exited()).then_some(*id))
            .collect();
        for id in failed {
            self.players.remove(&id);
            let Some(playback) = self.state.tabs.iter().find(|t| t.id == id).map(|t| t.playback.clone()) else {
                continue;
            };
            dbg(format!("retry {id:?} with cookies"));
            self.open(id, &playback, true, screen);
        }
    }

    /// Removes every image from the screen (videos and the thumbnail).
    fn clear_images(&mut self) {
        player::clear_images();
        self.thumbs.forget();
    }

    /// Stops the player of `id` without waiting for mpv to exit: a frozen UI
    /// makes people press the close key again and lose another tab.
    fn stop(&mut self, id: TabId) {
        let Some(mut player) = self.players.remove(&id) else { return };
        // No more frames; the last one would stay on top of everything.
        if player.hide(false) {
            self.clear_images();
            self.clear_at = Some(Instant::now() + LATE_FRAME);
        }
        self.closing.push(std::thread::spawn(move || drop(player)));
    }

    /// Reads the login in `config.session` and my channels in the
    /// background. `force` re-reads both even when the cached copies are fresh.
    fn start_session(&self, force: bool) {
        let session = self.state.config.session.clone();
        let cache = self.paths.cache.clone();
        let subs_cache = self.paths.subs.clone();
        self.background(move || {
            let cookies = if !session.cookies_file.is_empty() {
                PathBuf::from(&session.cookies_file)
            } else if !session.browser.is_empty() {
                let spec = session::with_keyring(&session.browser, &session::resolve_keyring(&session.keyring));
                let path = session::cookies_path(&cache, &spec);
                if let Err(e) = session::refresh_cookies(&spec, &path, force) {
                    return Action::SessionReady { cookies: None, subs: Err(e) };
                }
                path
            } else {
                return Action::SessionReady { cookies: None, subs: Ok(HashSet::new()) };
            };
            let subs = if !force && session::is_fresh(&subs_cache, SUBS_MAX_AGE) {
                Ok(store::load_subs(&subs_cache))
            } else {
                youtube::subscribed_channels(&cookies).map_err(|e| e.to_string())
            };
            Action::SessionReady { cookies: Some(cookies), subs }
        });
    }

    /// Opens `config.toml` in `$VISUAL`/`$EDITOR`, then reloads it.
    fn edit_config(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        let Some(path) = self.config_path.clone() else {
            self.send(Action::Status(self.tr("No hay archivo de configuración", "There is no config file").into()));
            return Ok(());
        };
        if !path.exists() {
            // Start from a file with every option and its default, ready to edit.
            config::set_value(&path, "ui", "icons", self.state.config.ui.icons.as_str())?;
        }
        for p in self.players.values_mut() {
            p.hide(false);
        }
        self.clear_images();
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| if cfg!(windows) { "notepad".into() } else { "nano".into() });
        let mut words = editor.split_whitespace();
        let program = words.next().unwrap_or("nano");
        set_mouse(false);
        set_focus_reports(false);
        self.mouse_on = false;
        ratatui::restore();
        let status = std::process::Command::new(program).args(words).arg(&path).status();
        *terminal = ratatui::init();
        set_focus_reports(true);
        let result = match status {
            Ok(_) => Config::load(&path).map_err(|e| e.to_string()),
            Err(e) => Err(format!("{} {program}: {e}", self.tr("no se pudo abrir", "could not open"))),
        };
        self.send(Action::ConfigReloaded(result));
        Ok(())
    }

    /// Configured folders for `kind`, or the system's videos / music folder.
    fn media_dirs(&self, kind: MediaKind) -> Vec<PathBuf> {
        let media = &self.state.config.media;
        let dirs = match kind {
            MediaKind::Video => &media.video_dirs,
            MediaKind::Music => &media.music_dirs,
        };
        if !dirs.is_empty() {
            return dirs.iter().map(|d| expand_home(d)).collect();
        }
        let system = directories::UserDirs::new().and_then(|d| match kind {
            MediaKind::Video => d.video_dir().map(Path::to_owned),
            MediaKind::Music => d.audio_dir().map(Path::to_owned),
        });
        system.into_iter().collect()
    }

    fn download_dir(&self) -> PathBuf {
        let dir = &self.state.config.download.dir;
        if !dir.is_empty() {
            return expand_home(dir);
        }
        directories::UserDirs::new()
            .and_then(|d| d.video_dir().map(Path::to_owned).or_else(|| Some(d.home_dir().join("Videos"))))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn background(&self, job: impl FnOnce() -> Action + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }

    fn send(&self, action: Action) {
        let _ = self.tx.send(action);
    }

    /// Shows the active tab's video where the layout left room and hides the
    /// rest. Only the last video watched may keep sounding in the background.
    fn place_videos(&mut self, screen: Rect) {
        let active = self.state.active_tab().map(|t| t.id);
        if active.is_some() {
            self.last_watched = active;
        }
        let audible = self.last_watched.filter(|_| self.state.config.player.background_audio || active.is_some());
        let overlay = self.mode() == Mode::Overlay;
        if overlay && self.grid.is_none() {
            self.grid = player::grid_origin(player::cell_size());
        }
        let geometry = ui::video_area(&self.state, screen)
            .filter(|_| self.resized_at.is_none() && !(overlay && self.away))
            .map(|a| Geometry::new(a, player::cell_size()))
            .map(|g| match self.grid.filter(|_| overlay) {
                Some(grid) => g.on_screen(grid),
                None => g,
            });
        let mut hidden_any = false;
        for (id, p) in &mut self.players {
            match geometry.filter(|_| Some(*id) == active) {
                Some(g) => p.show(g),
                None => hidden_any |= p.hide(Some(*id) == audible),
            }
        }
        if hidden_any {
            self.clear_images();
            self.clear_at = Some(Instant::now() + LATE_FRAME);
        }
    }
}

/// Validates a `cookies.txt` and keeps a private copy at `dest`.
fn import_cookies(source: &Path, dest: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(source).map_err(|e| format!("no se pudo leer {}: {e}", source.display()))?;
    if session::authorization(&session::parse_cookies(&text), 0).is_none() {
        return Err(format!("{} no tiene una sesión de YouTube (falta SAPISID)", source.display()));
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(dest, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(dest.display().to_string())
}

/// Timing trace for diagnosing lag: set `TUBETUI_DEBUG=/path/to/file`.
fn dbg(msg: impl AsRef<str>) {
    use std::io::Write;
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    if let Ok(path) = std::env::var("TUBETUI_DEBUG")
        && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(f, "{:>8.1} {}", start.elapsed().as_secs_f64() * 1000.0, msg.as_ref());
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), directories::BaseDirs::new()) {
        (Some(rest), Some(dirs)) => dirs.home_dir().join(rest),
        _ => PathBuf::from(path),
    }
}

/// Whether the terminal sends mouse events. While it does, selecting text
/// needs Shift.
fn set_mouse(on: bool) {
    let mut out = std::io::stdout();
    let _ = if on { execute!(out, EnableMouseCapture) } else { execute!(out, DisableMouseCapture) };
}

/// Whether the terminal tells us when it gains or loses the focus.
fn set_focus_reports(on: bool) {
    let mut out = std::io::stdout();
    let _ = if on { execute!(out, EnableFocusChange) } else { execute!(out, DisableFocusChange) };
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}
