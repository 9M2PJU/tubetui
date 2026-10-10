//! Application state and the pure `update` function.
//!
//! Tab 0 is always "Inicio": the section menu plus a stack of lists (opening
//! a channel pushes its videos, `Esc` pops back). Tabs 1.. are open videos,
//! each backed by its own player. The live configuration lives here too, so
//! the settings screen is just state changes plus `SaveSetting` effects.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::config::{Config, Language, PlayerMode, language_name, next_language, next_quality};
use crate::domain::{TabId, Video};
use crate::keymap::KeyContext;
use crate::media::{LocalFile, MediaKind, parse_dirs};
use crate::youtube::{Items, Source};

/// Lists that are filtered afterwards ask YouTube for more videos.
pub const FILTERED_LIMIT: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Inicio,
    Descubrir,
    Suscripciones,
    VerDespues,
    Canales,
    BuscarVideos,
    BuscarCanales,
    MisVideos,
    MiMusica,
    Configuracion,
    Salir,
}

/// Menu groups, drawn with a header each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    YouTube,
    Local,
    App,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Self::YouTube => "YOUTUBE",
            Self::Local => "EN TU EQUIPO",
            Self::App => "",
        }
    }
}

impl Section {
    pub const ALL: [Self; 11] = [
        Self::Inicio,
        Self::Descubrir,
        Self::Suscripciones,
        Self::VerDespues,
        Self::Canales,
        Self::BuscarVideos,
        Self::BuscarCanales,
        Self::MisVideos,
        Self::MiMusica,
        Self::Configuracion,
        Self::Salir,
    ];

    pub fn group(self) -> Group {
        match self {
            Self::MisVideos | Self::MiMusica => Group::Local,
            Self::Configuracion | Self::Salir => Group::App,
            _ => Group::YouTube,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Inicio => "Inicio",
            Self::Descubrir => "Descubrir",
            Self::Suscripciones => "Suscripciones",
            Self::VerDespues => "Ver después",
            Self::Canales => "Mis canales",
            Self::BuscarVideos => "Buscar videos",
            Self::BuscarCanales => "Buscar canales",
            Self::MisVideos => "Mis videos",
            Self::MiMusica => "Mi música",
            Self::Configuracion => "Configuración",
            Self::Salir => "Salir",
        }
    }
}

/// Rows of the settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Browser,
    ImportCookies,
    Refresh,
    Logout,
    Language,
    SubLang,
    Quality,
    VideoMode,
    BackgroundAudio,
    Icons,
    VideoDirs,
    MusicDirs,
    DownloadDir,
    EditFile,
}

impl Setting {
    pub const ALL: [Self; 14] = [
        Self::Browser,
        Self::ImportCookies,
        Self::Refresh,
        Self::Logout,
        Self::Language,
        Self::SubLang,
        Self::Quality,
        Self::VideoMode,
        Self::BackgroundAudio,
        Self::Icons,
        Self::VideoDirs,
        Self::MusicDirs,
        Self::DownloadDir,
        Self::EditFile,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Browser => "Sesión desde un navegador",
            Self::ImportCookies => "Importar cookies.txt (sin navegador / SSH)",
            Self::Refresh => "Releer la sesión",
            Self::Logout => "Cerrar sesión",
            Self::Language => "Idioma",
            Self::SubLang => "Idioma de subtítulos y doblaje",
            Self::Quality => "Calidad máxima",
            Self::VideoMode => "Video",
            Self::BackgroundAudio => "Al salir de la pestaña del video",
            Self::Icons => "Íconos",
            Self::VideoDirs => "Carpetas de videos",
            Self::MusicDirs => "Carpetas de música",
            Self::DownloadDir => "Carpeta de descargas",
            Self::EditFile => "Editar config.toml",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Menu,
    List,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRef {
    pub name: String,
    pub url: String,
}

/// Where a video search looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Mine,
    Channel(ChannelRef),
}

impl Scope {
    pub fn label(&self) -> String {
        match self {
            Self::All => "todo YouTube".into(),
            Self::Mine => "mis canales".into(),
            Self::Channel(c) => format!("canal {}", c.name),
        }
    }
}

/// What the text box is asking for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// `channel` is the channel offered as a scope, when there is one.
    Videos {
        scope: Scope,
        channel: Option<ChannelRef>,
    },
    Channels,
    CookiesFile,
    DownloadDir,
    /// Comma-separated folders for local videos or music.
    MediaDirs(MediaKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListKind {
    Plain,
    /// "Ver después", kept in sync with `State::later`.
    Later,
    /// A channel's videos; `/` searches inside it.
    Channel(ChannelRef),
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub title: String,
    pub items: Items,
    pub selected: usize,
    pub kind: ListKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsubscribe {
    pub channel_id: String,
    pub name: String,
}

/// A value written to `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    Text(String),
    Number(i64),
    Bool(bool),
    List(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Prefix,
    CancelPrefix,
    ToggleSidebar,
    ToggleHelp,
    Up,
    Down,
    Enter,
    Back,
    /// Search videos from anywhere (`/`, `C-y /`).
    FocusSearch,
    /// Next search scope (Tab in the search box).
    CycleScope,
    Input(char),
    Backspace,
    /// A list requested with `Effect::Load` / `Effect::ScanBrowsers` arrived.
    Loaded(u64, Result<Items, String>),
    /// Cached channel ids, read at startup.
    SubsLoaded(HashSet<String>),
    /// The login was (re)read. `cookies` is for the runtime; `subs` are my channels.
    SessionReady {
        cookies: Option<PathBuf>,
        subs: Result<HashSet<String>, String>,
    },
    /// A `cookies.txt` was validated and copied; carries its new path.
    CookiesImported(Result<String, String>),
    /// `config.toml` was edited outside the app.
    ConfigReloaded(Result<Config, String>),
    ToggleLater,
    ToggleSubscription,
    /// Answer to "¿Desuscribirte?".
    Confirm(bool),
    SubscriptionDone {
        channel_id: String,
        name: String,
        result: Result<bool, String>,
    },
    Download,
    OpenChannel,
    Status(String),
    NextTab,
    PrevTab,
    /// 0 is Inicio, n is the n-th video tab.
    GoTab(usize),
    CloseTab,
    /// The player went away on its own (window closed, video ended, failed to start).
    PlayerClosed(TabId, Option<String>),
    TogglePause,
    Seek(i32),
    Volume(i32),
    ToggleMute,
    CycleSubtitles,
    NextTrack,
    PrevTrack,
    /// Next language mode (subtitles → original → dubbed) for this and later videos.
    CycleLanguage,
    /// Next subtitle / dubbing language (es → en → pt …) for this and later videos.
    CycleSubLang,
    /// Jump to the section menu from anywhere.
    FocusMenu,
    /// Switch focus between the menu and the list.
    ToggleFocus,
    /// The video fills the whole terminal (or screen, in window mode).
    ToggleFullscreen,
    Quit,
}

/// Side effects requested by `update`, performed by the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Load {
        id: u64,
        source: Source,
        limit: u32,
    },
    ScanBrowsers {
        id: u64,
    },
    /// List local media from the configured folders.
    ScanMedia {
        id: u64,
        kind: MediaKind,
    },
    /// (Re)read the login described by `config.session`.
    StartSession {
        force: bool,
    },
    SaveSetting {
        table: &'static str,
        key: &'static str,
        value: SettingValue,
    },
    ImportCookies(String),
    EditConfig,
    SaveLater(Vec<Video>),
    SaveSubs(HashSet<String>),
    Subscribe {
        channel_id: String,
        name: String,
        subscribe: bool,
    },
    Download(Video),
    Open(TabId, Playback),
    Close(TabId),
    Command(TabId, PlayerCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerCommand {
    TogglePause,
    Seek(i32),
    Volume(i32),
    ToggleMute,
    CycleSubtitles,
    NextTrack,
    PrevTrack,
    /// Reload the video with other audio / subtitles, at the same second.
    Language(Language),
    /// Same, with another subtitle / dubbing language code.
    SubLang(&'static str),
    /// Only meaningful for the mpv window.
    Fullscreen(bool),
}

/// What a tab plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playback {
    pub title: String,
    /// Channel or folder.
    pub subtitle: String,
    /// The YouTube video, which enables `l`/`s`/`d`/`c`; `None` for local files.
    pub video: Option<Video>,
    /// URLs or paths, played in order.
    pub queue: Vec<String>,
    /// Music: no video output, `.`/`,` move through the queue.
    pub audio_only: bool,
}

impl Playback {
    pub fn youtube(video: Video) -> Self {
        Self {
            title: video.title.clone(),
            subtitle: video.channel.clone(),
            queue: vec![video.url()],
            video: Some(video),
            audio_only: false,
        }
    }

    /// `files[start]` and, for music, everything after it.
    pub fn local(files: &[LocalFile], start: usize, kind: MediaKind) -> Option<Self> {
        let first = files.get(start)?;
        let queue = match kind {
            MediaKind::Video => vec![first.path.display().to_string()],
            MediaKind::Music => files[start..].iter().map(|f| f.path.display().to_string()).collect(),
        };
        Some(Self {
            title: first.name.clone(),
            subtitle: first.folder.clone(),
            video: None,
            queue,
            audio_only: kind == MediaKind::Music,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: TabId,
    pub playback: Playback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    None,
    /// Keep only videos from my channels.
    Mine,
    /// Drop videos from my channels ("Descubrir").
    NotMine,
}

#[derive(Debug, Clone)]
struct Pending {
    id: u64,
    title: String,
    filter: Filter,
    push: bool,
    kind: ListKind,
}

#[derive(Debug, Clone)]
pub struct State {
    pub config: Config,
    pub sidebar: bool,
    pub prefix: bool,
    pub help: bool,
    /// The active video fills the terminal.
    pub fullscreen: bool,
    pub focus: Focus,
    /// Selected entry of `Section::ALL`.
    pub menu: usize,
    pub prompt: Option<Prompt>,
    pub query: String,
    /// Lists on Inicio; the last one is shown.
    pub stack: Vec<Listing>,
    /// Title of the list being loaded.
    pub loading: Option<String>,
    /// "Ver después", newest first.
    pub later: Vec<Video>,
    /// Ids of my channels.
    pub subs: HashSet<String>,
    pub confirm: Option<Unsubscribe>,
    pub tabs: Vec<Tab>,
    /// 0 = Inicio, n = `tabs[n - 1]`.
    pub active: usize,
    pub status: String,
    pub quit: bool,
    pending: Option<Pending>,
    next_tab: u64,
    next_request: u64,
}

impl State {
    pub fn new(config: Config, later: Vec<Video>) -> Self {
        Self {
            sidebar: config.ui.sidebar,
            focus: if config.ui.sidebar { Focus::Menu } else { Focus::List },
            config,
            prefix: false,
            help: false,
            fullscreen: false,
            menu: 0,
            prompt: None,
            query: String::new(),
            stack: Vec::new(),
            loading: None,
            later,
            subs: HashSet::new(),
            confirm: None,
            tabs: Vec::new(),
            active: 0,
            status: String::new(),
            quit: false,
            pending: None,
            next_tab: 0,
            next_request: 0,
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.checked_sub(1).and_then(|i| self.tabs.get(i))
    }

    pub fn listing(&self) -> Option<&Listing> {
        self.stack.last()
    }

    pub fn key_context(&self) -> KeyContext {
        KeyContext {
            prefix: self.prefix,
            help: self.help,
            confirm: self.confirm.is_some(),
            typing: self.prompt.is_some(),
            in_tab: self.active_tab().is_some(),
        }
    }

    /// "brave+GNOMEKEYRING:Default", "cookies.txt importado" or "sin sesión".
    pub fn session_label(&self) -> String {
        let session = &self.config.session;
        if !session.cookies_file.is_empty() {
            "cookies.txt importado".into()
        } else if !session.browser.is_empty() {
            session.browser.clone()
        } else {
            "sin sesión".into()
        }
    }

    /// Current value of a setting, for the settings screen.
    pub fn setting_value(&self, setting: Setting) -> String {
        let player = &self.config.player;
        match setting {
            Setting::Browser | Setting::Logout => self.session_label(),
            Setting::ImportCookies | Setting::Refresh | Setting::EditFile => String::new(),
            Setting::Language => language_label(player.language).into(),
            Setting::SubLang => language_name(&player.sub_lang),
            Setting::Quality => format!("{}p", player.max_height),
            Setting::VideoMode => match player.mode {
                PlayerMode::Auto => "automático (integrado en kitty)".into(),
                PlayerMode::Embedded => "integrado".into(),
                PlayerMode::Window => "ventana aparte".into(),
            },
            Setting::BackgroundAudio => {
                if player.background_audio {
                    "sigue sonando".into()
                } else {
                    "se pausa".into()
                }
            }
            Setting::Icons => self.config.ui.icons.as_str().into(),
            Setting::VideoDirs => dirs_label(&self.config.media.video_dirs, "~/Videos"),
            Setting::MusicDirs => dirs_label(&self.config.media.music_dirs, "~/Música"),
            Setting::DownloadDir => match self.config.download.dir.as_str() {
                "" => "~/Videos".into(),
                dir => dir.into(),
            },
        }
    }

    pub fn update(&mut self, action: Action) -> Vec<Effect> {
        self.prefix = matches!(action, Action::Prefix);
        let effects = self.apply(action);
        // Fullscreen belongs to a video tab; leaving it ends fullscreen.
        if self.active_tab().is_none() {
            self.fullscreen = false;
        }
        effects
    }

    fn apply(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::Prefix | Action::CancelPrefix => {}
            Action::ToggleSidebar => {
                self.sidebar = !self.sidebar;
                self.focus = if self.sidebar && self.stack.is_empty() { Focus::Menu } else { Focus::List };
            }
            Action::ToggleHelp => self.help = !self.help,
            Action::FocusMenu => {
                self.active = 0;
                self.prompt = None;
                self.sidebar = true;
                self.focus = Focus::Menu;
            }
            Action::ToggleFocus => {
                self.focus = match self.focus {
                    Focus::Menu if !self.stack.is_empty() => Focus::List,
                    Focus::List if self.sidebar => Focus::Menu,
                    focus => focus,
                };
            }
            Action::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen && self.active_tab().is_some();
                return self.command(PlayerCommand::Fullscreen(self.fullscreen));
            }
            Action::Up => self.move_selection(-1),
            Action::Down => self.move_selection(1),
            Action::Enter => return self.enter(),
            Action::Back => self.back(),
            Action::FocusSearch => self.search_videos(),
            Action::CycleScope => self.cycle_scope(),
            Action::Input(c) => self.query.push(c),
            Action::Backspace => {
                self.query.pop();
            }
            Action::Loaded(id, result) => self.loaded(id, result),
            Action::SubsLoaded(subs) => self.subs = subs,
            Action::SessionReady { subs, .. } => return self.session_ready(subs),
            Action::CookiesImported(Ok(path)) => {
                self.status = "cookies.txt importado, leyendo tus canales…".into();
                return self.set_session(String::new(), path);
            }
            Action::CookiesImported(Err(error)) | Action::ConfigReloaded(Err(error)) => self.status = error,
            Action::ConfigReloaded(Ok(config)) => {
                let session_changed = config.session != self.config.session;
                self.config = config;
                self.status = "Configuración recargada".into();
                if session_changed {
                    return vec![Effect::StartSession { force: true }];
                }
            }
            Action::ToggleLater => return self.toggle_later(),
            Action::ToggleSubscription => return self.toggle_subscription(),
            Action::Confirm(yes) => {
                if let Some(u) = self.confirm.take().filter(|_| yes) {
                    self.status = format!("Desuscribiendo de {}…", u.name);
                    return vec![Effect::Subscribe { channel_id: u.channel_id, name: u.name, subscribe: false }];
                }
            }
            Action::SubscriptionDone { channel_id, name, result } => {
                return self.subscription_done(channel_id, name, result);
            }
            Action::Download => {
                if let Some(video) = self.selected_video().cloned() {
                    self.status = format!("⬇ Descargando: {}", video.title);
                    return vec![Effect::Download(video)];
                }
            }
            Action::OpenChannel => return self.open_channel(),
            Action::Status(text) => self.status = text,
            Action::NextTab => self.active = (self.active + 1) % (self.tabs.len() + 1),
            Action::PrevTab => self.active = self.active.checked_sub(1).unwrap_or(self.tabs.len()),
            Action::GoTab(n) if n <= self.tabs.len() => self.active = n,
            Action::GoTab(_) => {}
            Action::CloseTab => {
                if let Some(id) = self.active_tab().map(|t| t.id) {
                    self.remove_tab(id);
                    return vec![Effect::Close(id)];
                }
            }
            Action::PlayerClosed(id, error) => {
                self.remove_tab(id);
                if let Some(error) = error {
                    self.status = error;
                }
            }
            Action::TogglePause => return self.command(PlayerCommand::TogglePause),
            Action::Seek(seconds) => return self.command(PlayerCommand::Seek(seconds)),
            Action::Volume(delta) => return self.command(PlayerCommand::Volume(delta)),
            Action::ToggleMute => return self.command(PlayerCommand::ToggleMute),
            Action::CycleSubtitles => return self.command(PlayerCommand::CycleSubtitles),
            Action::NextTrack => return self.command(PlayerCommand::NextTrack),
            Action::PrevTrack => return self.command(PlayerCommand::PrevTrack),
            Action::CycleLanguage => {
                let player = &mut self.config.player;
                player.language = player.language.next();
                let language = player.language;
                self.status = format!("Idioma: {} (recargando el video…)", language_label(language));
                let mut effects = vec![Effect::SaveSetting {
                    table: "player",
                    key: "language",
                    value: SettingValue::Text(language.as_str().into()),
                }];
                effects.extend(self.command(PlayerCommand::Language(language)));
                return effects;
            }
            Action::CycleSubLang => {
                let code = next_language(&self.config.player.sub_lang);
                self.config.player.sub_lang = code.into();
                self.status = format!("Subtítulos y doblaje en {} (recargando el video…)", language_name(code));
                let mut effects = vec![Effect::SaveSetting {
                    table: "player",
                    key: "sub_lang",
                    value: SettingValue::Text(code.into()),
                }];
                effects.extend(self.command(PlayerCommand::SubLang(code)));
                return effects;
            }
            Action::Quit => self.quit = true,
        }
        Vec::new()
    }

    fn move_selection(&mut self, delta: isize) {
        let step = |value: usize, len: usize| value.saturating_add_signed(delta).min(len.saturating_sub(1));
        match self.focus {
            Focus::Menu => self.menu = step(self.menu, Section::ALL.len()),
            Focus::List => {
                if let Some(l) = self.stack.last_mut() {
                    l.selected = step(l.selected, l.items.len());
                }
            }
        }
    }

    fn enter(&mut self) -> Vec<Effect> {
        if let Some(prompt) = self.prompt.clone() {
            return self.submit(prompt);
        }
        if self.active_tab().is_some() {
            return Vec::new();
        }
        if self.focus == Focus::Menu {
            return self.open_section(Section::ALL[self.menu]);
        }
        let Some(listing) = self.stack.last() else { return Vec::new() };
        let selected = listing.selected;
        match &listing.items {
            Items::Videos(videos) => match videos.get(selected).cloned() {
                Some(video) => self.play(Playback::youtube(video)),
                None => Vec::new(),
            },
            Items::Files(files, kind) => match Playback::local(files, selected, *kind) {
                Some(playback) => self.play(playback),
                None => Vec::new(),
            },
            Items::Channels(channels) => match channels.get(selected).cloned() {
                Some(c) => self.open_channel_list(c.name, c.url),
                None => Vec::new(),
            },
            Items::Profiles(profiles) => match profiles.get(selected).cloned() {
                Some(spec) => {
                    self.status = if spec.is_empty() {
                        "Cerrando sesión…".into()
                    } else {
                        format!("Leyendo la sesión de {spec}…")
                    };
                    self.set_session(spec, String::new())
                }
                None => Vec::new(),
            },
            Items::Settings => match Setting::ALL.get(selected) {
                Some(setting) => self.change_setting(*setting),
                None => Vec::new(),
            },
        }
    }

    fn open_section(&mut self, section: Section) -> Vec<Effect> {
        match section {
            Section::Inicio => self.load(Source::Recommended, "Inicio".into(), Filter::None, false, ListKind::Plain),
            Section::Descubrir => {
                self.load(Source::Recommended, "Descubrir".into(), Filter::NotMine, false, ListKind::Plain)
            }
            Section::Suscripciones => {
                self.load(Source::Subscriptions, "Suscripciones".into(), Filter::None, false, ListKind::Plain)
            }
            Section::Canales => {
                self.load(Source::MyChannels, "Mis canales".into(), Filter::None, false, ListKind::Plain)
            }
            Section::VerDespues => {
                self.show(Listing {
                    title: "Ver después".into(),
                    items: Items::Videos(self.later.clone()),
                    selected: 0,
                    kind: ListKind::Later,
                });
                Vec::new()
            }
            Section::BuscarVideos => {
                self.search_videos();
                Vec::new()
            }
            Section::BuscarCanales => {
                self.start_prompt(Prompt::Channels, String::new());
                Vec::new()
            }
            Section::MisVideos => self.scan_media("Mis videos", MediaKind::Video),
            Section::MiMusica => self.scan_media("Mi música", MediaKind::Music),
            Section::Configuracion => {
                self.show(Listing {
                    title: "Configuración".into(),
                    items: Items::Settings,
                    selected: 0,
                    kind: ListKind::Settings,
                });
                Vec::new()
            }
            Section::Salir => {
                self.quit = true;
                Vec::new()
            }
        }
    }

    fn scan_media(&mut self, title: &str, kind: MediaKind) -> Vec<Effect> {
        let id = self.request(title.into(), Filter::None, false, ListKind::Plain);
        vec![Effect::ScanMedia { id, kind }]
    }

    fn show(&mut self, listing: Listing) {
        self.stack = vec![listing];
        self.focus = Focus::List;
    }

    fn change_setting(&mut self, setting: Setting) -> Vec<Effect> {
        let save = |table, key, value| vec![Effect::SaveSetting { table, key, value }];
        let player = &mut self.config.player;
        match setting {
            Setting::Browser => {
                let id = self.request("Navegadores".into(), Filter::None, true, ListKind::Plain);
                vec![Effect::ScanBrowsers { id }]
            }
            Setting::ImportCookies => {
                self.start_prompt(Prompt::CookiesFile, String::new());
                Vec::new()
            }
            Setting::Refresh => {
                self.status = format!("Releyendo la sesión ({})…", self.session_label());
                vec![Effect::StartSession { force: true }]
            }
            Setting::Logout => {
                self.status = "Sesión cerrada".into();
                self.set_session(String::new(), String::new())
            }
            Setting::Language => {
                player.language = player.language.next();
                save("player", "language", SettingValue::Text(player.language.as_str().into()))
            }
            Setting::SubLang => {
                player.sub_lang = next_language(&player.sub_lang).into();
                self.status = "Se aplica a los próximos videos".into();
                save("player", "sub_lang", SettingValue::Text(self.config.player.sub_lang.clone()))
            }
            Setting::Quality => {
                player.max_height = next_quality(player.max_height);
                save("player", "max_height", SettingValue::Number(player.max_height.into()))
            }
            Setting::VideoMode => {
                player.mode = player.mode.next();
                self.status = "Se aplica a los próximos videos".into();
                save("player", "mode", SettingValue::Text(self.config.player.mode.as_str().into()))
            }
            Setting::BackgroundAudio => {
                player.background_audio = !player.background_audio;
                save("player", "background_audio", SettingValue::Bool(player.background_audio))
            }
            Setting::Icons => {
                self.config.ui.icons = self.config.ui.icons.next();
                save("ui", "icons", SettingValue::Text(self.config.ui.icons.as_str().into()))
            }
            Setting::VideoDirs => {
                let current = self.config.media.video_dirs.join(", ");
                self.start_prompt(Prompt::MediaDirs(MediaKind::Video), current);
                Vec::new()
            }
            Setting::MusicDirs => {
                let current = self.config.media.music_dirs.join(", ");
                self.start_prompt(Prompt::MediaDirs(MediaKind::Music), current);
                Vec::new()
            }
            Setting::DownloadDir => {
                let current = self.config.download.dir.clone();
                self.start_prompt(Prompt::DownloadDir, current);
                Vec::new()
            }
            Setting::EditFile => vec![Effect::EditConfig],
        }
    }

    /// Switches the login to a browser profile or an imported file (both empty = none).
    fn set_session(&mut self, browser: String, cookies_file: String) -> Vec<Effect> {
        self.config.session.browser = browser.clone();
        self.config.session.cookies_file = cookies_file.clone();
        vec![
            Effect::SaveSetting { table: "session", key: "browser", value: SettingValue::Text(browser) },
            Effect::SaveSetting { table: "session", key: "cookies_file", value: SettingValue::Text(cookies_file) },
            Effect::StartSession { force: true },
        ]
    }

    /// Opens the search box; inside a channel the search starts scoped to it.
    fn search_videos(&mut self) {
        let channel = match self.stack.last() {
            Some(Listing { kind: ListKind::Channel(c), .. }) if self.active == 0 => Some(c.clone()),
            _ => None,
        };
        let scope = channel.clone().map_or(Scope::All, Scope::Channel);
        self.start_prompt(Prompt::Videos { scope, channel }, String::new());
    }

    fn cycle_scope(&mut self) {
        if let Some(Prompt::Videos { scope, channel }) = &mut self.prompt {
            *scope = match (&*scope, channel) {
                (Scope::All, _) => Scope::Mine,
                (Scope::Mine, Some(c)) => Scope::Channel(c.clone()),
                (Scope::Mine, None) | (Scope::Channel(_), _) => Scope::All,
            };
        }
    }

    fn start_prompt(&mut self, prompt: Prompt, query: String) {
        self.active = 0;
        self.prompt = Some(prompt);
        self.query = query;
    }

    fn submit(&mut self, prompt: Prompt) -> Vec<Effect> {
        let query = self.query.trim().to_owned();
        if query.is_empty() && !matches!(prompt, Prompt::DownloadDir | Prompt::MediaDirs(_)) {
            return Vec::new();
        }
        self.prompt = None;
        match prompt {
            Prompt::Videos { scope: Scope::All, .. } => {
                self.load(Source::Search(query.clone()), query, Filter::None, false, ListKind::Plain)
            }
            Prompt::Videos { scope: Scope::Mine, .. } => {
                let title = format!("{query} · mis canales");
                self.load(Source::Search(query), title, Filter::Mine, false, ListKind::Plain)
            }
            Prompt::Videos { scope: Scope::Channel(c), .. } => {
                let title = format!("«{query}»");
                let source = Source::SearchInChannel { url: c.url, query };
                self.load(source, title, Filter::None, true, ListKind::Plain)
            }
            Prompt::Channels => {
                self.load(Source::ChannelSearch(query.clone()), query, Filter::None, false, ListKind::Plain)
            }
            Prompt::CookiesFile => {
                self.status = "Importando cookies.txt…".into();
                vec![Effect::ImportCookies(query)]
            }
            Prompt::DownloadDir => {
                self.config.download.dir = query.clone();
                vec![Effect::SaveSetting { table: "download", key: "dir", value: SettingValue::Text(query) }]
            }
            Prompt::MediaDirs(kind) => {
                let dirs = parse_dirs(&query);
                let key = match kind {
                    MediaKind::Video => {
                        self.config.media.video_dirs = dirs.clone();
                        "video_dirs"
                    }
                    MediaKind::Music => {
                        self.config.media.music_dirs = dirs.clone();
                        "music_dirs"
                    }
                };
                vec![Effect::SaveSetting { table: "media", key, value: SettingValue::List(dirs) }]
            }
        }
    }

    fn load(&mut self, source: Source, title: String, filter: Filter, push: bool, kind: ListKind) -> Vec<Effect> {
        let limit = if filter == Filter::None { self.config.search.results } else { FILTERED_LIMIT };
        let id = self.request(title, filter, push, kind);
        vec![Effect::Load { id, source, limit }]
    }

    /// Registers a pending list; only the latest request is shown when it lands.
    fn request(&mut self, title: String, filter: Filter, push: bool, kind: ListKind) -> u64 {
        let id = self.next_request;
        self.next_request += 1;
        self.status = format!("Cargando {title}…");
        self.loading = Some(title.clone());
        self.pending = Some(Pending { id, title, filter, push, kind });
        id
    }

    fn loaded(&mut self, id: u64, result: Result<Items, String>) {
        let Some(pending) = self.pending.take_if(|p| p.id == id) else {
            return; // superseded by a newer request
        };
        self.loading = None;
        let mut items = match result {
            Ok(items) => items,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if let Items::Videos(videos) = &mut items {
            let mine = |v: &Video| v.channel_id.as_ref().is_some_and(|c| self.subs.contains(c));
            match pending.filter {
                Filter::None => {}
                Filter::Mine => videos.retain(mine),
                Filter::NotMine => videos.retain(|v| v.channel_id.is_some() && !mine(v)),
            }
        }
        self.status =
            if items.is_empty() { "Sin resultados".into() } else { format!("{} · {}", pending.title, items.len()) };
        let listing = Listing { title: pending.title, items, selected: 0, kind: pending.kind };
        if pending.push {
            self.stack.push(listing);
            self.focus = Focus::List;
        } else {
            self.show(listing);
        }
    }

    fn back(&mut self) {
        if self.prompt.take().is_some() {
            return;
        }
        if self.fullscreen {
            self.fullscreen = false;
            return;
        }
        if self.focus == Focus::List {
            if self.stack.len() > 1 {
                self.stack.pop();
            } else if self.sidebar {
                self.focus = Focus::Menu;
            }
        }
    }

    fn session_ready(&mut self, subs: Result<HashSet<String>, String>) -> Vec<Effect> {
        match subs {
            Ok(subs) => {
                self.status = match self.session_label().as_str() {
                    "sin sesión" => "Sin sesión".into(),
                    label => format!("Sesión: {label} · {} canales", subs.len()),
                };
                self.subs = subs;
                vec![Effect::SaveSubs(self.subs.clone())]
            }
            Err(error) => {
                self.status = error;
                Vec::new()
            }
        }
    }

    /// The video the user is pointing at: the open tab, or the selected list row.
    fn selected_video(&self) -> Option<&Video> {
        if let Some(tab) = self.active_tab() {
            return tab.playback.video.as_ref();
        }
        match self.stack.last() {
            Some(Listing { items: Items::Videos(videos), selected, .. }) if self.focus == Focus::List => {
                videos.get(*selected)
            }
            _ => None,
        }
    }

    /// `(id, name, url)` of the channel the user is pointing at.
    fn selected_channel(&self) -> Option<(String, String, Option<String>)> {
        if let Some(v) = self.selected_video() {
            return v.channel_id.clone().map(|id| (id, v.channel.clone(), v.channel_url.clone()));
        }
        match self.stack.last() {
            Some(Listing { items: Items::Channels(channels), selected, .. }) if self.focus == Focus::List => {
                channels.get(*selected).map(|c| (c.id.clone(), c.name.clone(), Some(c.url.clone())))
            }
            _ => None,
        }
    }

    fn toggle_later(&mut self) -> Vec<Effect> {
        let Some(video) = self.selected_video().cloned() else { return Vec::new() };
        if let Some(i) = self.later.iter().position(|v| v.id == video.id) {
            self.later.remove(i);
            self.status = format!("🗑 Quitado de Ver después: {}", video.title);
        } else {
            self.status = format!("🕒 Guardado para después: {}", video.title);
            self.later.insert(0, video);
        }
        for listing in self.stack.iter_mut().filter(|l| l.kind == ListKind::Later) {
            listing.items = Items::Videos(self.later.clone());
            listing.selected = listing.selected.min(self.later.len().saturating_sub(1));
        }
        vec![Effect::SaveLater(self.later.clone())]
    }

    fn toggle_subscription(&mut self) -> Vec<Effect> {
        let Some((channel_id, name, _)) = self.selected_channel() else {
            if self.selected_video().is_some() {
                self.status = "Este video no tiene canal".into();
            }
            return Vec::new();
        };
        if self.subs.contains(&channel_id) {
            self.confirm = Some(Unsubscribe { channel_id, name });
            Vec::new()
        } else {
            self.status = format!("Suscribiendo a {name}…");
            vec![Effect::Subscribe { channel_id, name, subscribe: true }]
        }
    }

    fn subscription_done(&mut self, channel_id: String, name: String, result: Result<bool, String>) -> Vec<Effect> {
        match result {
            Ok(true) => {
                self.subs.insert(channel_id);
                self.status = format!("🔔 Suscripto: {name}");
            }
            Ok(false) => {
                self.subs.remove(&channel_id);
                self.status = format!("🔕 Desuscripto: {name}");
            }
            Err(error) => {
                self.status = format!("❌ No se pudo cambiar la suscripción a {name}: {error}");
                return Vec::new();
            }
        }
        vec![Effect::SaveSubs(self.subs.clone())]
    }

    fn open_channel(&mut self) -> Vec<Effect> {
        match self.selected_channel() {
            Some((_, name, Some(url))) => {
                self.active = 0;
                self.open_channel_list(name, url)
            }
            _ => Vec::new(),
        }
    }

    fn open_channel_list(&mut self, name: String, url: String) -> Vec<Effect> {
        let kind = ListKind::Channel(ChannelRef { name: name.clone(), url: url.clone() });
        self.load(Source::Channel(url), name, Filter::None, true, kind)
    }

    fn play(&mut self, playback: Playback) -> Vec<Effect> {
        if let Some(i) = self.tabs.iter().position(|t| t.playback.queue.first() == playback.queue.first()) {
            self.active = i + 1;
            return Vec::new();
        }
        if self.tabs.len() >= self.config.ui.max_tabs {
            self.status = format!("Máximo de {} pestañas: cerrá una con C-y x", self.config.ui.max_tabs);
            return Vec::new();
        }
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        self.tabs.push(Tab { id, playback: playback.clone() });
        self.active = self.tabs.len();
        vec![Effect::Open(id, playback)]
    }

    /// Removes a tab; the active one moves to its right neighbour, like browsers do.
    fn remove_tab(&mut self, id: TabId) {
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        self.tabs.remove(i);
        let index = i + 1;
        if self.active > index {
            self.active -= 1;
        } else if self.active == index {
            self.active = index.min(self.tabs.len());
        }
    }

    fn command(&self, command: PlayerCommand) -> Vec<Effect> {
        self.active_tab().map(|t| Effect::Command(t.id, command)).into_iter().collect()
    }
}

fn dirs_label(dirs: &[String], default: &str) -> String {
    if dirs.is_empty() { format!("{default} (del sistema)") } else { dirs.join(", ") }
}

pub fn language_label(language: Language) -> &'static str {
    match language {
        Language::Subtitles => "original + subtítulos",
        Language::Original => "original, sin subtítulos",
        Language::Dubbed => "audio doblado",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{IconSet, UiConfig};
    use crate::domain::{channel, video};

    fn state() -> State {
        State::new(Config::default(), Vec::new())
    }

    fn section_index(section: Section) -> usize {
        Section::ALL.iter().position(|s| *s == section).unwrap()
    }

    /// Opens `section` from the menu and returns its effects.
    fn open(s: &mut State, section: Section) -> Vec<Effect> {
        s.update(Action::GoTab(0));
        s.prompt = None;
        s.focus = Focus::Menu;
        s.menu = section_index(section);
        s.update(Action::Enter)
    }

    fn load_id(effects: &[Effect]) -> u64 {
        match effects {
            [Effect::Load { id, .. }] | [Effect::ScanBrowsers { id }] => *id,
            other => panic!("expected a load, got {other:?}"),
        }
    }

    fn with_videos(ids: &[&str]) -> State {
        let mut s = state();
        let id = load_id(&open(&mut s, Section::Inicio));
        s.update(Action::Loaded(id, Ok(Items::Videos(ids.iter().map(|i| video(i)).collect()))));
        s
    }

    /// Plays the video at `index` of the current list and returns the new tab id.
    fn play(s: &mut State, index: usize) -> TabId {
        s.update(Action::GoTab(0));
        s.stack.last_mut().unwrap().selected = index;
        match s.update(Action::Enter).as_slice() {
            [Effect::Open(id, _)] => *id,
            other => panic!("expected Open, got {other:?}"),
        }
    }

    fn type_text(s: &mut State, text: &str) {
        for c in text.chars() {
            s.update(Action::Input(c));
        }
    }

    /// Opens Configuración and activates `setting`.
    fn change(s: &mut State, setting: Setting) -> Vec<Effect> {
        open(s, Section::Configuracion);
        s.stack[0].selected = Setting::ALL.iter().position(|x| *x == setting).unwrap();
        s.update(Action::Enter)
    }

    fn saved(table: &'static str, key: &'static str, value: SettingValue) -> Effect {
        Effect::SaveSetting { table, key, value }
    }

    fn text(value: &str) -> SettingValue {
        SettingValue::Text(value.into())
    }

    /// Inside the channel "name UCx", with one video loaded.
    fn inside_channel() -> State {
        let mut s = state();
        let id = load_id(&open(&mut s, Section::Canales));
        s.update(Action::Loaded(id, Ok(Items::Channels(vec![channel("UCx")]))));
        let id = load_id(&s.update(Action::Enter));
        s.update(Action::Loaded(id, Ok(Items::Videos(vec![video("v")]))));
        s
    }

    #[test]
    fn starts_on_the_menu() {
        let s = state();
        assert_eq!((s.active, s.focus, s.menu), (0, Focus::Menu, 0));
        let config = Config { ui: UiConfig { sidebar: false, ..Default::default() }, ..Default::default() };
        assert_eq!(State::new(config, vec![]).focus, Focus::List);
    }

    #[test]
    fn prefix_lasts_one_action() {
        let mut s = state();
        s.update(Action::Prefix);
        assert!(s.prefix);
        s.update(Action::ToggleSidebar);
        assert!(!s.prefix);
        assert!(!s.sidebar);
    }

    #[test]
    fn hiding_the_sidebar_moves_focus_to_the_list() {
        let mut s = state();
        s.update(Action::ToggleSidebar);
        assert_eq!(s.focus, Focus::List);
        s.update(Action::ToggleSidebar);
        assert_eq!(s.focus, Focus::Menu);
    }

    #[test]
    fn help_toggles() {
        let mut s = state();
        s.update(Action::ToggleHelp);
        assert!(s.help && s.key_context().help);
        s.update(Action::ToggleHelp);
        assert!(!s.help);
    }

    #[test]
    fn menu_selection_is_clamped() {
        let mut s = state();
        s.update(Action::Up);
        assert_eq!(s.menu, 0);
        for _ in 0..50 {
            s.update(Action::Down);
        }
        assert_eq!(s.menu, Section::ALL.len() - 1);
    }

    #[test]
    fn feed_sections_load_their_source() {
        let cases = [
            (Section::Inicio, Source::Recommended, 60),
            (Section::Descubrir, Source::Recommended, FILTERED_LIMIT),
            (Section::Suscripciones, Source::Subscriptions, 60),
            (Section::Canales, Source::MyChannels, 60),
        ];
        for (section, expected, expected_limit) in cases {
            let mut s = state();
            match open(&mut s, section).as_slice() {
                [Effect::Load { source, limit, .. }] => {
                    assert_eq!((source, *limit), (&expected, expected_limit), "{section:?}");
                }
                other => panic!("{section:?}: {other:?}"),
            }
            assert!(s.loading.is_some(), "{section:?}");
        }
    }

    #[test]
    fn loaded_list_replaces_stack_and_focuses_it() {
        let s = with_videos(&["a", "b"]);
        assert_eq!(s.stack.len(), 1);
        assert_eq!(s.focus, Focus::List);
        assert_eq!(s.loading, None);
        assert_eq!(s.listing().unwrap().items, Items::Videos(vec![video("a"), video("b")]));
    }

    #[test]
    fn stale_results_are_ignored() {
        let mut s = state();
        let first = load_id(&open(&mut s, Section::Inicio));
        let second = load_id(&open(&mut s, Section::Suscripciones));
        s.update(Action::Loaded(first, Ok(Items::Videos(vec![video("old")]))));
        assert!(s.stack.is_empty());
        s.update(Action::Loaded(second, Ok(Items::Videos(vec![video("new")]))));
        assert_eq!(s.listing().unwrap().title, "Suscripciones");
    }

    #[test]
    fn load_error_is_reported_and_keeps_the_list() {
        let mut s = with_videos(&["a"]);
        let id = load_id(&open(&mut s, Section::Suscripciones));
        s.update(Action::Loaded(id, Err("sin sesión".into())));
        assert_eq!(s.status, "sin sesión");
        assert_eq!(s.listing().unwrap().title, "Inicio");
    }

    #[test]
    fn discover_hides_my_channels_and_channelless_entries() {
        let mut s = state();
        s.update(Action::SubsLoaded(HashSet::from(["UCa".to_string()])));
        let id = load_id(&open(&mut s, Section::Descubrir));
        let mix = Video { channel_id: None, ..video("RDmix") };
        s.update(Action::Loaded(id, Ok(Items::Videos(vec![video("a"), video("b"), mix]))));
        assert_eq!(s.listing().unwrap().items, Items::Videos(vec![video("b")]));
    }

    #[test]
    fn search_all_youtube_by_default() {
        let mut s = state();
        assert!(open(&mut s, Section::BuscarVideos).is_empty());
        assert_eq!(s.prompt, Some(Prompt::Videos { scope: Scope::All, channel: None }));
        type_text(&mut s, "rust x");
        s.update(Action::Backspace);
        s.update(Action::Backspace);
        match s.update(Action::Enter).as_slice() {
            [Effect::Load { source: Source::Search(q), limit: 60, .. }] => assert_eq!(q, "rust"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.prompt, None);
    }

    #[test]
    fn search_in_my_channels_filters_results() {
        let mut s = state();
        s.update(Action::SubsLoaded(HashSet::from(["UCa".to_string()])));
        s.update(Action::FocusSearch);
        s.update(Action::CycleScope);
        type_text(&mut s, "rust");
        let id = match s.update(Action::Enter).as_slice() {
            [Effect::Load { id, source: Source::Search(_), limit: FILTERED_LIMIT }] => *id,
            other => panic!("{other:?}"),
        };
        s.update(Action::Loaded(id, Ok(Items::Videos(vec![video("a"), video("b")]))));
        assert_eq!(s.listing().unwrap().items, Items::Videos(vec![video("a")]));
        assert_eq!(s.listing().unwrap().title, "rust · mis canales");
    }

    #[test]
    fn search_inside_a_channel_starts_scoped_and_cycles() {
        let mut s = inside_channel();
        s.update(Action::FocusSearch);
        let here = ChannelRef { name: "name UCx".into(), url: "https://www.youtube.com/channel/UCx".into() };
        assert_eq!(s.prompt, Some(Prompt::Videos { scope: Scope::Channel(here.clone()), channel: Some(here.clone()) }));
        type_text(&mut s, "tui");
        match s.update(Action::Enter).as_slice() {
            [Effect::Load { source: Source::SearchInChannel { url, query }, .. }] => {
                assert_eq!((url.as_str(), query.as_str()), (here.url.as_str(), "tui"));
            }
            other => panic!("{other:?}"),
        }
        // Tab cycles: canal → todo → mis canales → canal.
        s.update(Action::FocusSearch);
        let scopes: Vec<String> = (0..3)
            .map(|_| {
                s.update(Action::CycleScope);
                match &s.prompt {
                    Some(Prompt::Videos { scope, .. }) => scope.label(),
                    other => panic!("{other:?}"),
                }
            })
            .collect();
        assert_eq!(scopes, ["todo YouTube", "mis canales", "canal name UCx"]);
    }

    #[test]
    fn without_a_channel_scope_cycles_between_two() {
        let mut s = state();
        s.update(Action::FocusSearch);
        s.update(Action::CycleScope);
        s.update(Action::CycleScope);
        assert_eq!(s.prompt, Some(Prompt::Videos { scope: Scope::All, channel: None }));
    }

    #[test]
    fn channel_search_prompt() {
        let mut s = state();
        open(&mut s, Section::BuscarCanales);
        type_text(&mut s, "tango");
        assert!(matches!(s.update(Action::Enter).as_slice(), [Effect::Load { source: Source::ChannelSearch(_), .. }]));
    }

    #[test]
    fn blank_search_does_nothing_and_esc_cancels() {
        let mut s = state();
        s.update(Action::FocusSearch);
        s.update(Action::Input(' '));
        assert!(s.update(Action::Enter).is_empty());
        s.update(Action::Back);
        assert_eq!(s.prompt, None);
        assert_eq!(s.focus, Focus::Menu);
    }

    #[test]
    fn search_from_a_tab_goes_home() {
        let mut s = with_videos(&["a"]);
        play(&mut s, 0);
        s.update(Action::FocusSearch);
        assert_eq!(s.active, 0);
        assert!(s.prompt.is_some());
    }

    #[test]
    fn list_selection_is_clamped() {
        let mut s = with_videos(&["a", "b"]);
        s.update(Action::Up);
        assert_eq!(s.listing().unwrap().selected, 0);
        s.update(Action::Down);
        s.update(Action::Down);
        assert_eq!(s.listing().unwrap().selected, 1);
    }

    #[test]
    fn channel_list_pushes_and_esc_pops() {
        let mut s = inside_channel();
        assert_eq!(s.stack.len(), 2);
        assert_eq!(s.listing().unwrap().title, "name UCx");
        assert!(matches!(s.listing().unwrap().kind, ListKind::Channel(_)));
        s.update(Action::Back);
        assert_eq!((s.stack.len(), s.focus), (1, Focus::List));
        s.update(Action::Back);
        assert_eq!(s.focus, Focus::Menu);
    }

    #[test]
    fn open_channel_from_a_video() {
        let mut s = with_videos(&["a"]);
        match s.update(Action::OpenChannel).as_slice() {
            [Effect::Load { source: Source::Channel(url), .. }] => assert!(url.ends_with("UCa")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn play_opens_a_tab_and_switches_to_it() {
        let mut s = with_videos(&["a", "b"]);
        let id = play(&mut s, 1);
        assert_eq!(s.active, 1);
        assert_eq!(s.active_tab().unwrap(), &Tab { id, playback: Playback::youtube(video("b")) });
    }

    #[test]
    fn playing_an_open_video_reuses_its_tab() {
        let mut s = with_videos(&["a", "b"]);
        play(&mut s, 0);
        play(&mut s, 1);
        s.update(Action::GoTab(0));
        s.stack[0].selected = 0;
        assert!(s.update(Action::Enter).is_empty());
        assert_eq!((s.tabs.len(), s.active), (2, 1));
    }

    #[test]
    fn max_tabs_is_enforced() {
        let config = Config { ui: UiConfig { max_tabs: 1, ..Default::default() }, ..Default::default() };
        let mut s = State::new(config, vec![]);
        let id = load_id(&open(&mut s, Section::Inicio));
        s.update(Action::Loaded(id, Ok(Items::Videos(vec![video("a"), video("b")]))));
        play(&mut s, 0);
        s.update(Action::GoTab(0));
        s.stack[0].selected = 1;
        assert!(s.update(Action::Enter).is_empty());
        assert!(s.status.contains("Máximo"));
    }

    #[test]
    fn tabs_navigate_and_close_like_a_browser() {
        let mut s = with_videos(&["a", "b", "c"]);
        play(&mut s, 0);
        let b = play(&mut s, 1);
        play(&mut s, 2);
        s.update(Action::NextTab);
        assert_eq!(s.active, 0);
        s.update(Action::PrevTab);
        assert_eq!(s.active, 3);
        s.update(Action::GoTab(9));
        assert_eq!(s.active, 3);
        s.update(Action::GoTab(2));
        assert_eq!(s.update(Action::CloseTab), vec![Effect::Close(b)]);
        assert_eq!(s.active_tab().unwrap().playback.video, Some(video("c")));
        s.update(Action::CloseTab);
        s.update(Action::CloseTab);
        assert_eq!(s.active, 0);
        assert!(s.update(Action::CloseTab).is_empty());
    }

    #[test]
    fn player_closed_drops_tab_without_closing_again() {
        let mut s = with_videos(&["a", "b"]);
        let a = play(&mut s, 0);
        play(&mut s, 1);
        assert!(s.update(Action::PlayerClosed(a, Some("mpv no está".into()))).is_empty());
        assert_eq!(s.active_tab().unwrap().playback.video, Some(video("b")));
        assert_eq!(s.status, "mpv no está");
        s.update(Action::PlayerClosed(TabId(99), None));
        assert_eq!(s.tabs.len(), 1);
    }

    #[test]
    fn player_commands_target_the_active_tab() {
        let mut s = with_videos(&["a"]);
        let a = play(&mut s, 0);
        for (action, command) in [
            (Action::TogglePause, PlayerCommand::TogglePause),
            (Action::Seek(-5), PlayerCommand::Seek(-5)),
            (Action::Volume(5), PlayerCommand::Volume(5)),
            (Action::ToggleMute, PlayerCommand::ToggleMute),
            (Action::CycleSubtitles, PlayerCommand::CycleSubtitles),
        ] {
            assert_eq!(s.update(action), vec![Effect::Command(a, command)]);
        }
        s.update(Action::GoTab(0));
        assert!(s.update(Action::Volume(5)).is_empty());
    }

    #[test]
    fn later_toggles_from_list_and_tab_and_persists() {
        let mut s = with_videos(&["a", "b"]);
        assert_eq!(s.update(Action::ToggleLater), vec![Effect::SaveLater(vec![video("a")])]);
        play(&mut s, 1);
        assert_eq!(s.update(Action::ToggleLater), vec![Effect::SaveLater(vec![video("b"), video("a")])]);
        s.update(Action::ToggleLater);
        assert_eq!(s.later, vec![video("a")]);
        assert!(s.status.contains("Quitado"));
    }

    #[test]
    fn later_section_shows_and_tracks_the_list() {
        let mut s = State::new(Config::default(), vec![video("a"), video("b")]);
        assert!(open(&mut s, Section::VerDespues).is_empty());
        assert_eq!(s.listing().unwrap().kind, ListKind::Later);
        s.update(Action::Down);
        s.update(Action::ToggleLater);
        assert_eq!(s.listing().unwrap().items, Items::Videos(vec![video("a")]));
        assert_eq!(s.listing().unwrap().selected, 0);
    }

    #[test]
    fn later_needs_a_video() {
        let mut s = state();
        assert!(s.update(Action::ToggleLater).is_empty());
    }

    #[test]
    fn subscribe_directly_but_confirm_unsubscribe() {
        let mut s = with_videos(&["a"]);
        let subscribe = Effect::Subscribe { channel_id: "UCa".into(), name: "chan a".into(), subscribe: true };
        assert_eq!(s.update(Action::ToggleSubscription), vec![subscribe]);
        s.update(Action::SubscriptionDone { channel_id: "UCa".into(), name: "chan a".into(), result: Ok(true) });
        assert!(s.subs.contains("UCa"));

        assert!(s.update(Action::ToggleSubscription).is_empty());
        assert!(s.key_context().confirm);
        assert!(s.update(Action::Confirm(false)).is_empty());
        assert_eq!(s.confirm, None);

        s.update(Action::ToggleSubscription);
        let unsubscribe = Effect::Subscribe { channel_id: "UCa".into(), name: "chan a".into(), subscribe: false };
        assert_eq!(s.update(Action::Confirm(true)), vec![unsubscribe]);
        let effects =
            s.update(Action::SubscriptionDone { channel_id: "UCa".into(), name: "chan a".into(), result: Ok(false) });
        assert_eq!(effects, vec![Effect::SaveSubs(HashSet::new())]);
    }

    #[test]
    fn subscription_failure_keeps_state() {
        let mut s = with_videos(&["a"]);
        let done = Action::SubscriptionDone { channel_id: "UCa".into(), name: "n".into(), result: Err("403".into()) };
        assert!(s.update(done).is_empty());
        assert!(s.subs.is_empty());
        assert!(s.status.contains("403"));
    }

    #[test]
    fn subscribe_from_channel_list() {
        let mut s = state();
        let id = load_id(&open(&mut s, Section::Canales));
        s.update(Action::Loaded(id, Ok(Items::Channels(vec![channel("UCx")]))));
        assert!(matches!(s.update(Action::ToggleSubscription).as_slice(), [Effect::Subscribe { subscribe: true, .. }]));
    }

    #[test]
    fn video_without_channel_cannot_subscribe() {
        let mut s = state();
        let id = load_id(&open(&mut s, Section::Inicio));
        s.update(Action::Loaded(id, Ok(Items::Videos(vec![Video { channel_id: None, ..video("a") }]))));
        assert!(s.update(Action::ToggleSubscription).is_empty());
        assert_eq!(s.status, "Este video no tiene canal");
    }

    #[test]
    fn download_selected_video() {
        let mut s = with_videos(&["a"]);
        assert_eq!(s.update(Action::Download), vec![Effect::Download(video("a"))]);
        s.focus = Focus::Menu;
        assert!(s.update(Action::Download).is_empty());
    }

    #[test]
    fn settings_screen_lists_every_setting() {
        let mut s = state();
        assert!(open(&mut s, Section::Configuracion).is_empty());
        assert_eq!(s.listing().unwrap().kind, ListKind::Settings);
        assert_eq!(s.listing().unwrap().items.len(), Setting::ALL.len());
        assert_eq!(s.setting_value(Setting::Quality), "720p");
        assert_eq!(s.setting_value(Setting::DownloadDir), "~/Videos");
        assert_eq!(s.setting_value(Setting::Browser), "sin sesión");
    }

    #[test]
    fn cycling_settings_saves_them() {
        let mut s = state();
        assert_eq!(change(&mut s, Setting::Language), [saved("player", "language", text("original"))]);
        assert_eq!(s.config.player.language, Language::Original);
        assert_eq!(change(&mut s, Setting::Quality), [saved("player", "max_height", SettingValue::Number(1080))]);
        assert_eq!(change(&mut s, Setting::VideoMode), [saved("player", "mode", text("embedded"))]);
        assert_eq!(s.config.player.mode, PlayerMode::Embedded);
        assert_eq!(change(&mut s, Setting::Icons), [saved("ui", "icons", text("nerd"))]);
        assert_eq!(s.config.ui.icons, IconSet::Nerd);
        assert_eq!(change(&mut s, Setting::EditFile), [Effect::EditConfig]);
    }

    #[test]
    fn subtitle_language_cycles_and_saves() {
        let mut s = state();
        assert_eq!(s.setting_value(Setting::SubLang), "español (es)");
        assert_eq!(change(&mut s, Setting::SubLang), [saved("player", "sub_lang", text("en"))]);
        assert_eq!(s.setting_value(Setting::SubLang), "inglés (en)");
    }

    #[test]
    fn background_audio_toggles_and_saves() {
        let mut s = state();
        assert_eq!(s.setting_value(Setting::BackgroundAudio), "sigue sonando");
        assert_eq!(
            change(&mut s, Setting::BackgroundAudio),
            [saved("player", "background_audio", SettingValue::Bool(false))]
        );
        assert_eq!(s.setting_value(Setting::BackgroundAudio), "se pausa");
    }

    #[test]
    fn language_changes_the_current_video_and_is_saved() {
        let mut s = with_videos(&["a"]);
        let a = play(&mut s, 0);
        assert_eq!(
            s.update(Action::CycleLanguage),
            [
                saved("player", "language", text("original")),
                Effect::Command(a, PlayerCommand::Language(Language::Original)),
            ]
        );
        assert!(s.status.contains("recargando"));
        s.update(Action::GoTab(0));
        assert_eq!(s.update(Action::CycleLanguage), [saved("player", "language", text("dubbed"))]);
    }

    #[test]
    fn sub_language_changes_the_current_video_and_is_saved() {
        let mut s = with_videos(&["a"]);
        let a = play(&mut s, 0);
        assert_eq!(
            s.update(Action::CycleSubLang),
            [saved("player", "sub_lang", text("en")), Effect::Command(a, PlayerCommand::SubLang("en"))]
        );
        assert_eq!(s.config.player.sub_lang, "en");
    }

    fn files(names: &[&str]) -> Vec<LocalFile> {
        names
            .iter()
            .map(|n| LocalFile { path: format!("/m/{n}.mp3").into(), name: n.to_string(), folder: "Disco".into() })
            .collect()
    }

    #[test]
    fn menu_is_grouped() {
        let groups: Vec<Group> = Section::ALL.iter().map(|s| s.group()).collect();
        assert_eq!(groups[0], Group::YouTube);
        assert_eq!(Section::MisVideos.group(), Group::Local);
        assert_eq!(Section::MiMusica.group(), Group::Local);
        assert_eq!(groups.last(), Some(&Group::App));
        // Groups are contiguous, so each header appears once.
        assert!(groups.windows(2).filter(|w| w[0] != w[1]).count() == 2);
    }

    #[test]
    fn local_sections_scan_their_kind() {
        let mut s = state();
        assert!(matches!(
            open(&mut s, Section::MisVideos).as_slice(),
            [Effect::ScanMedia { kind: MediaKind::Video, .. }]
        ));
        assert!(matches!(
            open(&mut s, Section::MiMusica).as_slice(),
            [Effect::ScanMedia { kind: MediaKind::Music, .. }]
        ));
    }

    #[test]
    fn music_plays_from_the_chosen_song_to_the_end() {
        let mut s = state();
        let id = match open(&mut s, Section::MiMusica).as_slice() {
            [Effect::ScanMedia { id, .. }] => *id,
            other => panic!("{other:?}"),
        };
        s.update(Action::Loaded(id, Ok(Items::Files(files(&["a", "b", "c"]), MediaKind::Music))));
        s.update(Action::Down);
        let playback = match s.update(Action::Enter).as_slice() {
            [Effect::Open(_, playback)] => playback.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(playback.queue, ["/m/b.mp3", "/m/c.mp3"]);
        assert!(playback.audio_only);
        assert_eq!((playback.title.as_str(), playback.video), ("b", None));
        let tab = s.active_tab().unwrap().id;
        assert_eq!(s.update(Action::NextTrack), [Effect::Command(tab, PlayerCommand::NextTrack)]);
        // No YouTube video: watch later / subscribe do nothing.
        assert!(s.update(Action::ToggleLater).is_empty());
    }

    #[test]
    fn a_local_video_plays_alone() {
        let playback = Playback::local(&files(&["a", "b"]), 0, MediaKind::Video).unwrap();
        assert_eq!(playback.queue, ["/m/a.mp3"]);
        assert!(!playback.audio_only);
        assert!(Playback::local(&files(&["a"]), 5, MediaKind::Video).is_none());
    }

    #[test]
    fn media_folders_are_edited_as_a_list() {
        let mut s = state();
        assert_eq!(s.setting_value(Setting::MusicDirs), "~/Música (del sistema)");
        assert!(change(&mut s, Setting::MusicDirs).is_empty());
        assert_eq!(s.prompt, Some(Prompt::MediaDirs(MediaKind::Music)));
        type_text(&mut s, "~/Música, /mnt/discos");
        let list = SettingValue::List(vec!["~/Música".into(), "/mnt/discos".into()]);
        assert_eq!(s.update(Action::Enter), [saved("media", "music_dirs", list)]);
        assert_eq!(s.setting_value(Setting::MusicDirs), "~/Música, /mnt/discos");
    }

    #[test]
    fn menu_is_reachable_from_anywhere() {
        let mut s = with_videos(&["a"]);
        play(&mut s, 0);
        s.update(Action::ToggleSidebar);
        s.update(Action::FocusMenu);
        assert_eq!((s.active, s.focus, s.sidebar), (0, Focus::Menu, true));
    }

    #[test]
    fn tab_switches_between_menu_and_list() {
        let mut s = state();
        s.update(Action::ToggleFocus);
        assert_eq!(s.focus, Focus::Menu, "nothing to focus yet");
        let mut s = with_videos(&["a"]);
        s.update(Action::ToggleFocus);
        assert_eq!(s.focus, Focus::Menu);
        s.update(Action::ToggleFocus);
        assert_eq!(s.focus, Focus::List);
    }

    #[test]
    fn fullscreen_only_for_a_video_and_esc_leaves_it() {
        let mut s = with_videos(&["a"]);
        s.update(Action::ToggleFullscreen);
        assert!(!s.fullscreen, "no video on Inicio");
        let a = play(&mut s, 0);
        assert_eq!(s.update(Action::ToggleFullscreen), [Effect::Command(a, PlayerCommand::Fullscreen(true))]);
        assert!(s.fullscreen);
        s.update(Action::Back);
        assert!(!s.fullscreen);
        s.update(Action::ToggleFullscreen);
        s.update(Action::GoTab(0));
        assert!(!s.fullscreen, "leaving the tab ends fullscreen");
    }

    #[test]
    fn download_dir_is_edited_in_place() {
        let mut s = state();
        assert!(change(&mut s, Setting::DownloadDir).is_empty());
        assert_eq!(s.prompt, Some(Prompt::DownloadDir));
        type_text(&mut s, "~/Descargas");
        assert_eq!(s.update(Action::Enter), [saved("download", "dir", text("~/Descargas"))]);
        assert_eq!(s.config.download.dir, "~/Descargas");
    }

    #[test]
    fn browser_session_is_picked_from_the_scan() {
        let mut s = state();
        let id = load_id(&change(&mut s, Setting::Browser));
        s.update(Action::Loaded(id, Ok(Items::Profiles(vec!["brave:Default".into(), String::new()]))));
        assert_eq!(s.stack.len(), 2);
        assert_eq!(
            s.update(Action::Enter),
            [
                saved("session", "browser", text("brave:Default")),
                saved("session", "cookies_file", text("")),
                Effect::StartSession { force: true },
            ]
        );
        assert_eq!(s.session_label(), "brave:Default");
    }

    #[test]
    fn cookies_file_import_replaces_the_browser() {
        let mut s = state();
        s.config.session.browser = "brave".into();
        assert!(change(&mut s, Setting::ImportCookies).is_empty());
        assert_eq!(s.prompt, Some(Prompt::CookiesFile));
        type_text(&mut s, "~/cookies.txt");
        assert_eq!(s.update(Action::Enter), [Effect::ImportCookies("~/cookies.txt".into())]);
        let effects = s.update(Action::CookiesImported(Ok("/cache/cookies-imported.txt".into())));
        assert_eq!(effects[0], saved("session", "browser", text("")));
        assert_eq!(effects[1], saved("session", "cookies_file", text("/cache/cookies-imported.txt")));
        assert_eq!(s.session_label(), "cookies.txt importado");
        s.update(Action::CookiesImported(Err("no tiene sesión".into())));
        assert_eq!(s.status, "no tiene sesión");
    }

    #[test]
    fn logout_and_refresh() {
        let mut s = state();
        s.config.session.browser = "brave".into();
        let effects = change(&mut s, Setting::Logout);
        assert_eq!(effects.last(), Some(&Effect::StartSession { force: true }));
        assert_eq!(s.session_label(), "sin sesión");
        assert_eq!(change(&mut s, Setting::Refresh), [Effect::StartSession { force: true }]);
    }

    #[test]
    fn session_ready_updates_channels() {
        let mut s = state();
        s.config.session.browser = "brave:Default".into();
        let subs = HashSet::from(["UCa".to_string()]);
        let effects = s.update(Action::SessionReady { cookies: None, subs: Ok(subs.clone()) });
        assert_eq!(effects, vec![Effect::SaveSubs(subs)]);
        assert!(s.status.contains("1 canales"));
        assert!(s.update(Action::SessionReady { cookies: None, subs: Err("keyring".into()) }).is_empty());
        assert_eq!(s.status, "keyring");
    }

    #[test]
    fn reloaded_config_restarts_session_only_when_it_changed() {
        let mut s = state();
        let mut edited = Config::default();
        edited.player.max_height = 480;
        assert!(s.update(Action::ConfigReloaded(Ok(edited.clone()))).is_empty());
        assert_eq!(s.config.player.max_height, 480);
        edited.session.browser = "firefox".into();
        assert_eq!(s.update(Action::ConfigReloaded(Ok(edited))), [Effect::StartSession { force: true }]);
        s.update(Action::ConfigReloaded(Err("línea 3".into())));
        assert_eq!(s.status, "línea 3");
    }

    #[test]
    fn salir_quits() {
        let mut s = state();
        open(&mut s, Section::Salir);
        assert!(s.quit);
    }
}
