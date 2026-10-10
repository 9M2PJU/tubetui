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
use ratatui::crossterm::event::{self, Event};
use ratatui::layout::Rect;

use tubetui::app::{Action, Effect, SettingValue, State};
use tubetui::config::{self, Config};
use tubetui::domain::{TabId, Video};
use tubetui::player::{self, Geometry, Launch, Mode, Player};
use tubetui::store::{self, Paths};
use tubetui::youtube::{self, Items};
use tubetui::{keymap, session, ui};

const SUBS_MAX_AGE: Duration = Duration::from_secs(24 * 3600);
/// While the window is being resized the video stays hidden; it is placed
/// once the size has been stable this long (each placement restarts mpv's output).
const RESIZE_SETTLE: Duration = Duration::from_millis(250);
/// mpv may flush one more frame after being hidden; clear again after this.
const LATE_FRAME: Duration = Duration::from_millis(300);

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
    let config = match &config_path {
        Some(path) => Config::load(path)?,
        None => Config::default(),
    };
    let paths = Paths::system().context("no se encontró la carpeta personal")?;
    let later = load_later(&paths)?;

    let mut runtime = Runtime::new(config, config_path, paths, later);
    let mut terminal = ratatui::init();
    let result = runtime.run(&mut terminal);
    ratatui::restore();
    result
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
}

impl Runtime {
    fn new(config: Config, config_path: Option<PathBuf>, paths: Paths, later: Vec<Video>) -> Self {
        let mut state = State::new(config, later);
        state.update(Action::SubsLoaded(store::load_subs(&paths.subs)));
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
        };
        runtime.start_session(false);
        runtime
    }

    fn mode(&self) -> Mode {
        player::resolve_mode(self.state.config.player.mode, |key| std::env::var(key).ok())
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        let mut dirty = true;
        while !self.state.quit {
            if self.resized_at.is_some_and(|t| t.elapsed() >= RESIZE_SETTLE) {
                self.resized_at = None;
                dirty = true;
            }
            if self.clear_at.is_some_and(|t| Instant::now() >= t) {
                self.clear_at = None;
                if ui::video_area(&self.state, terminal.size()?.into()).is_none() || self.resized_at.is_some() {
                    player::clear_images();
                }
            }
            // Redraw only on change: every redraw competes with mpv for the terminal.
            if dirty {
                let view = ui::View { embedded: self.mode() == Mode::Embedded, now: now() };
                terminal.draw(|f| ui::draw(f, &self.state, &view))?;
                self.place_videos(terminal.size()?.into());
                dirty = false;
            }
            let mut pending = Vec::new();
            if event::poll(Duration::from_millis(100)).context("reading terminal events")? {
                match event::read()? {
                    Event::Key(key) => pending.extend(keymap::map(key, self.state.key_context())),
                    Event::Resize(..) => {
                        terminal.clear()?;
                        self.resized_at = Some(Instant::now());
                        dirty = true;
                    }
                    _ => {}
                }
            }
            pending.extend(self.rx.try_iter());
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
                    self.perform(effect, terminal)?;
                }
            }
            self.players.retain(|id, _| self.state.tabs.iter().any(|t| t.id == *id));
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
            Effect::ScanBrowsers { id } => {
                let home = directories::BaseDirs::new().map(|d| d.home_dir().to_owned()).unwrap_or_default();
                let mut profiles = session::scan_browsers(&home, &self.state.config.session.keyring);
                profiles.push(String::new()); // "Sin sesión"
                self.send(Action::Loaded(id, Ok(Items::Profiles(profiles))));
            }
            Effect::StartSession { force } => self.start_session(force),
            Effect::SaveSetting { table, key, value } => {
                let Some(path) = &self.config_path else { return Ok(()) };
                let result = match value {
                    SettingValue::Text(text) => config::set_value(path, table, key, text),
                    SettingValue::Number(number) => config::set_value(path, table, key, number),
                    SettingValue::Bool(flag) => config::set_value(path, table, key, flag),
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
                    self.send(Action::Status(format!("No se pudo guardar Ver después: {e}")));
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
                    &player::ytdl_format(player),
                    &player.sub_lang,
                    &dir,
                    self.cookies.as_deref(),
                );
                self.background(move || match youtube::download(&args) {
                    Ok(()) => Action::Status(format!("✅ Descarga lista en {}: {}", dir.display(), video.title)),
                    Err(e) => Action::Status(format!("❌ Falló la descarga de {}: {e}", video.title)),
                });
            }
            Effect::Open(id, video) => {
                let mode = self.mode();
                let screen: Rect = terminal.size()?.into();
                let geometry = ui::video_area(&self.state, screen).map(|a| Geometry::new(a, player::cell_size()));
                let url = video.url();
                let launch =
                    Launch { config: &self.state.config.player, mode, cookies: self.cookies.as_deref(), url: &url };
                match Player::spawn(&launch, geometry) {
                    Ok(p) => {
                        self.players.insert(id, p);
                    }
                    Err(e) => self.send(Action::PlayerClosed(id, Some(format!("No se pudo iniciar mpv: {e}")))),
                }
            }
            Effect::Close(id) => {
                self.players.remove(&id);
            }
            Effect::Command(id, command) => {
                if let Some(p) = self.players.get_mut(&id) {
                    p.send(command);
                }
            }
        }
        Ok(())
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
                let path = session::cookies_path(&cache, &session.browser);
                if let Err(e) = session::refresh_cookies(&session.browser, &path, force) {
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
            self.send(Action::Status("No hay archivo de configuración".into()));
            return Ok(());
        };
        if !path.exists() {
            // Start from a file with every option and its default, ready to edit.
            config::set_value(&path, "ui", "icons", self.state.config.ui.icons.as_str())?;
        }
        for p in self.players.values_mut() {
            p.hide(false);
        }
        player::clear_images();
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| if cfg!(windows) { "notepad".into() } else { "nano".into() });
        let mut words = editor.split_whitespace();
        let program = words.next().unwrap_or("nano");
        ratatui::restore();
        let status = std::process::Command::new(program).args(words).arg(&path).status();
        *terminal = ratatui::init();
        terminal.clear()?;
        let result = match status {
            Ok(_) => Config::load(&path).map_err(|e| e.to_string()),
            Err(e) => Err(format!("no se pudo abrir {program}: {e}")),
        };
        self.send(Action::ConfigReloaded(result));
        Ok(())
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
        let geometry = ui::video_area(&self.state, screen)
            .filter(|_| self.resized_at.is_none())
            .map(|a| Geometry::new(a, player::cell_size()));
        let mut hidden_any = false;
        for (id, p) in &mut self.players {
            match geometry.filter(|_| Some(*id) == active) {
                Some(g) => p.show(g),
                None => hidden_any |= p.hide(Some(*id) == audible),
            }
        }
        if hidden_any {
            player::clear_images();
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

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), directories::BaseDirs::new()) {
        (Some(rest), Some(dirs)) => dirs.home_dir().join(rest),
        _ => PathBuf::from(path),
    }
}
