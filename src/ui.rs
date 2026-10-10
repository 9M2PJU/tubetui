//! Rendering. `areas` is the single source of truth for the layout, so the
//! runtime places the embedded video exactly where `draw` leaves room for it.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Tabs};

use crate::app::{Focus, Group, Listing, Prompt, Section, Setting, State, Tab, language_label};
use crate::config::{IconSet, donations};
use crate::domain::{Channel, Live, Video, date_labels};
use crate::i18n::Lang;
use crate::keymap;
use crate::media::{LocalFile, MediaKind};
use crate::player::Info;
use crate::youtube::Items;

pub const SIDEBAR_WIDTH: u16 = 26;
/// Width of the preview panel next to video lists.
const PREVIEW_WIDTH: u16 = 46;
/// Video lists narrower than this show no preview panel.
const PREVIEW_MIN_WIDTH: u16 = 96;
const CHANNEL_WIDTH: usize = 20;
/// Width of the date column: `"10/10/26 hoy   "`.
const DATE_WIDTH: usize = 14;

// Tokyo Night.
const FG: Color = Color::Rgb(192, 202, 245);
const MUTED: Color = Color::Rgb(86, 95, 137);
const HIGHLIGHT: Color = Color::Rgb(41, 46, 66);
const BLUE: Color = Color::Rgb(122, 162, 247);
const CYAN: Color = Color::Rgb(125, 207, 255);
const MAGENTA: Color = Color::Rgb(187, 154, 247);
const GREEN: Color = Color::Rgb(158, 206, 106);
const ORANGE: Color = Color::Rgb(255, 158, 100);
const YELLOW: Color = Color::Rgb(224, 175, 104);
const RED: Color = Color::Rgb(247, 118, 142);
const TEAL: Color = Color::Rgb(115, 218, 202);
const PURPLE: Color = Color::Rgb(157, 124, 216);

/// One colour per menu entry, in `Section::ALL` order.
const SECTION_COLORS: [Color; 12] =
    [BLUE, CYAN, RED, YELLOW, ORANGE, GREEN, TEAL, PURPLE, MAGENTA, BLUE, PURPLE, MUTED];

pub struct Icons {
    /// In `Section::ALL` order.
    pub sections: [&'static str; 12],
    /// In `Setting::ALL` order.
    pub settings: [&'static str; 19],
    pub logo: &'static str,
    pub tab: &'static str,
    pub live: &'static str,
    pub upcoming: &'static str,
    pub later: &'static str,
    pub subscribed: &'static str,
    pub user: &'static str,
    pub language: &'static str,
    pub volume: &'static str,
    pub muted: &'static str,
}

const EMOJI: Icons = Icons {
    sections: ["🏠", "🧭", "📺", "🕒", "⭐", "🔎", "📡", "📼", "🎵", "🔧", "💜", "🚪"],
    settings: [
        "👤", "📄", "🔄", "🚪", "🌐", "💬", "📐", "🎬", "🎧", "🗣", "😀", "🖼", "✨", "🌊", "💥", "📼", "🎵", "📁", "📝",
    ],
    logo: "▶️",
    tab: "🎬",
    live: "🔴",
    upcoming: "⏳",
    later: "🕒",
    subscribed: "✓",
    user: "👤",
    language: "🌐",
    volume: "🔊",
    muted: "🔇",
};

const NERD: Icons = Icons {
    sections: [
        "\u{f015}",
        "\u{f14e}",
        "\u{f05c6}",
        "\u{f017}",
        "\u{f005}",
        "\u{f002}",
        "\u{f2ce}",
        "\u{f03d}",
        "\u{f001}",
        "\u{f013}",
        "\u{f004}",
        "\u{f08b}",
    ],
    settings: [
        "\u{f007}", "\u{f15b}", "\u{f021}", "\u{f08b}", "\u{f1ab}", "\u{f075}", "\u{f1de}", "\u{f008}", "\u{f025}",
        "\u{f0ac}", "\u{f118}", "\u{f03e}", "\u{f0d0}", "\u{f0e7}", "\u{f0a1}", "\u{f03d}", "\u{f001}", "\u{f07b}",
        "\u{f044}",
    ],
    logo: "\u{f16a}",
    tab: "\u{f144}",
    live: "\u{f111}",
    upcoming: "\u{f252}",
    later: "\u{f017}",
    subscribed: "\u{f00c}",
    user: "\u{f007}",
    language: "\u{f1ab}",
    volume: "\u{f028}",
    muted: "\u{f026}",
};

const UNICODE: Icons = Icons {
    sections: ["⌂", "◎", "▣", "◷", "★", "⌕", "⌁", "▷", "♫", "⚙", "♥", "⏻"],
    settings: ["☺", "▤", "↻", "⏻", "⇄", "≡", "▦", "▶", "♪", "⚑", "☻", "▨", "✦", "≈", "✸", "▷", "♫", "▭", "✎"],
    logo: "▶",
    tab: "▶",
    live: "●",
    upcoming: "…",
    later: "◷",
    subscribed: "✓",
    user: "☺",
    language: "⇄",
    volume: "♪",
    muted: "✕",
};

pub fn icons(set: IconSet) -> &'static Icons {
    match set {
        IconSet::Emoji => &EMOJI,
        IconSet::Nerd => &NERD,
        IconSet::Unicode => &UNICODE,
    }
}

/// Rendering options that do not belong to the app state.
pub struct View {
    pub embedded: bool,
    /// Unix time, for "hace 3 días".
    pub now: i64,
    /// How long the current list has been loading.
    pub loading: Option<Duration>,
    /// How long the video of the active tab has been loading, until it plays.
    pub opening: Option<Duration>,
    /// The preview panel leaves room for a thumbnail (the runtime draws it).
    pub thumbnails: bool,
    /// What mpv reports about the video of the active tab.
    pub playing: Option<Info>,
}

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// `"⠹ Cargando Suscripciones… 3 s"`.
fn loading_text(title: &str, elapsed: Duration, lang: Lang) -> String {
    let frame = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];
    format!("{frame} {} {title}… {} s", lang.pick("Cargando", "Loading"), elapsed.as_secs())
}

/// A block bouncing inside `width` cells: progress without a known total.
fn bouncing_bar(width: usize, elapsed: Duration) -> String {
    let block = 6.min(width);
    let span = width - block;
    let step = (elapsed.as_millis() / 60) as usize % (2 * span.max(1));
    let left = if step <= span { step } else { 2 * span - step };
    format!("{}{}{}", "░".repeat(left), "█".repeat(block), "░".repeat(span - left))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Areas {
    pub sidebar: Option<Rect>,
    pub tabs: Rect,
    pub content: Rect,
    pub status: Rect,
    /// Keys that work right now, with what they do.
    pub hints: Rect,
}

pub fn areas(area: Rect, sidebar: bool) -> Areas {
    let [body, status, hints] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1), Constraint::Length(1)]).areas(area);
    let width = if sidebar { SIDEBAR_WIDTH } else { 0 };
    let [side, main] = Layout::horizontal([Constraint::Length(width), Constraint::Fill(1)]).areas(body);
    let [tabs, content] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(main);
    Areas { sidebar: sidebar.then_some(side), tabs, content, status, hints }
}

/// The cells the embedded video may cover: the active video tab, unless a
/// dialog is open (kitty draws images above text, it would hide the dialog).
pub fn video_area(state: &State, area: Rect) -> Option<Rect> {
    if state.help || state.confirm.is_some() {
        return None;
    }
    state.active_tab().filter(|t| !t.playback.audio_only)?;
    if state.fullscreen {
        return Some(area);
    }
    Some(Block::bordered().inner(areas(area, state.sidebar).content))
}

pub fn draw(frame: &mut Frame, state: &State, view: &View) {
    let icons = icons(state.config.ui.icons);
    let lang = state.lang();
    if state.fullscreen {
        // Nothing but the video (embedded) or a hint (window mode).
        if !view.embedded {
            let note = lang.pick(
                "Pantalla completa en la ventana de mpv · f o Esc para salir",
                "Fullscreen in the mpv window · f or Esc to leave",
            );
            frame.render_widget(Paragraph::new(note).fg(MUTED).alignment(Alignment::Center), frame.area());
        }
        if state.help {
            draw_help(frame, lang);
        }
        return;
    }
    let areas = areas(frame.area(), state.sidebar);
    if let Some(side) = areas.sidebar {
        draw_sidebar(frame, state, icons, side);
    }
    draw_tabs(frame, state, icons, areas.tabs);
    match state.active_tab() {
        None => draw_inicio(frame, state, icons, view, areas.content),
        Some(tab) => {
            let playback = &tab.playback;
            let title = Line::from(vec![
                Span::raw(format!(" {} ", tab_icon(icons, tab))),
                Span::styled(playback.title.clone(), Style::new().fg(FG).bold()),
                Span::styled(format!(" · {} ", playback.subtitle), MAGENTA),
            ]);
            let body = if playback.audio_only {
                let songs = playback.queue.len();
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from(format!("{}  {}", icons.sections[8], playback.title)).fg(FG).bold(),
                    Line::from(playback.subtitle.clone()).fg(MAGENTA),
                    Line::from(""),
                    Line::from(format!(
                        "{songs} {}",
                        lang.pick(
                            "canciones en la cola · . siguiente · , anterior",
                            "songs queued · . next · , previous"
                        )
                    ))
                    .fg(MUTED),
                ])
                .alignment(Alignment::Center)
            } else if let Some(elapsed) = view.opening {
                // In embedded mode the first video frame covers it.
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from(loading_text(lang.pick("el video", "the video"), elapsed, lang)).fg(YELLOW),
                    Line::from(bouncing_bar(30, elapsed)).fg(BLUE),
                ])
                .alignment(Alignment::Center)
            } else if view.embedded {
                Paragraph::default()
            } else {
                Paragraph::new(lang.pick("Se reproduce en una ventana de mpv", "Playing in an mpv window")).fg(MUTED)
            };
            let mut block = panel(title, BLUE);
            if let Some(info) = &view.playing {
                block = block.title_top(Line::from(playing_label(info, icons)).fg(CYAN).right_aligned());
            }
            frame.render_widget(body.block(block), areas.content);
        }
    }
    draw_status(frame, state, icons, view, areas.status);
    draw_hints(frame, &hints(state), areas.hints);
    if state.help {
        draw_help(frame, lang);
    }
    if let Some(u) = &state.confirm {
        draw_confirm(frame, &u.name, lang);
    }
}

fn tab_icon<'a>(icons: &'a Icons, tab: &Tab) -> &'a str {
    if tab.playback.audio_only { icons.sections[8] } else { icons.tab }
}

/// `" 🔊 85 %  720p  H.264 "`: what the video keys change, as it is now.
fn playing_label(info: &Info, icons: &Icons) -> String {
    let mut parts = Vec::new();
    if let Some(volume) = info.volume {
        let icon = if info.muted { icons.muted } else { icons.volume };
        parts.push(format!("{icon} {volume:.0} %"));
    }
    if let Some(height) = info.height {
        parts.push(format!("{height}p"));
    }
    if let Some(codec) = &info.codec {
        parts.push(match codec.as_str() {
            "h264" => "H.264".into(),
            "hevc" => "H.265".into(),
            other => other.to_uppercase(),
        });
    }
    if parts.is_empty() { String::new() } else { format!(" {} ", parts.join("  ")) }
}

fn panel<'a>(title: impl Into<Line<'a>>, color: Color) -> Block<'a> {
    Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(color)).title(title)
}

fn draw_sidebar(frame: &mut Frame, state: &State, icons: &Icons, area: Rect) {
    let lang = state.lang();
    let on_inicio = state.active == 0;
    let mut items: Vec<ListItem> = Vec::new();
    let mut group = None;
    for (i, section) in Section::ALL.iter().enumerate() {
        if group != Some(section.group()) {
            group = Some(section.group());
            if i > 0 {
                items.push(ListItem::new(""));
            }
            if section.group() != Group::App {
                items.push(ListItem::new(Line::from(format!(" {}", section.group().label(lang))).fg(MUTED).bold()));
            }
        }
        let mut label = section.label(lang).to_owned();
        if *section == Section::VerDespues && !state.later.is_empty() {
            label.push_str(&format!(" ({})", state.later.len()));
        }
        let selected = i == state.menu;
        let focused = selected && on_inicio && state.focus == Focus::Menu;
        let color = SECTION_COLORS[i];
        let mut style = Style::new().fg(if selected { color } else { FG });
        if focused {
            style = style.bg(HIGHLIGHT).add_modifier(Modifier::BOLD);
        }
        items.push(
            ListItem::new(Line::from(vec![
                Span::styled(if focused { "▌" } else { " " }, color),
                Span::raw(format!("{} ", icons.sections[i])),
                Span::raw(label),
            ]))
            .style(style),
        );
    }
    if !state.tabs.is_empty() {
        items.push(ListItem::new(""));
        items.push(ListItem::new(Line::from(lang.pick(" PESTAÑAS", " TABS")).fg(MUTED).bold()));
        for (i, tab) in state.tabs.iter().enumerate() {
            let active = state.active == i + 1;
            let style = if active { Style::new().fg(MAGENTA).bg(HIGHLIGHT).bold() } else { Style::new().fg(FG) };
            let line =
                format!("{}{} {} {}", if active { "▌" } else { " " }, i + 1, tab_icon(icons, tab), tab.playback.title);
            items.push(ListItem::new(line).style(style));
        }
    }
    let title = Line::from(vec![
        Span::raw(format!(" {} ", icons.logo)),
        Span::styled("tube", Style::new().fg(RED).bold()),
        Span::styled("tui ", Style::new().fg(FG).bold()),
    ]);
    frame.render_widget(List::new(items).block(panel(title, MUTED)), area);
}

fn draw_tabs(frame: &mut Frame, state: &State, icons: &Icons, area: Rect) {
    let home = Section::Inicio.label(state.lang());
    let titles = std::iter::once(format!("{} {home}", icons.sections[0])).chain(
        state
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{} {} {}", i + 1, tab_icon(icons, t), truncate(&t.playback.title, 20))),
    );
    let tabs = Tabs::new(titles)
        .select(state.active)
        .style(Style::new().fg(MUTED))
        .highlight_style(Style::new().fg(BLUE).add_modifier(Modifier::BOLD));
    frame.render_widget(tabs, area);
}

/// The search box and the rest of Inicio below it.
fn split_prompt(state: &State, area: Rect) -> (Option<Rect>, Rect) {
    match state.prompt {
        Some(_) => {
            let [input, rest] = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(area);
            (Some(input), rest)
        }
        None => (None, area),
    }
}

/// A video list and, when there is room, the preview panel beside it.
fn split_preview(area: Rect) -> (Rect, Option<Rect>) {
    if area.width < PREVIEW_MIN_WIDTH {
        return (area, None);
    }
    let [list, preview] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(PREVIEW_WIDTH)]).areas(area);
    (list, Some(preview))
}

/// The cells of the thumbnail inside a preview panel: 16:9, taking terminal
/// cells as about twice as tall as wide.
fn thumbnail_cells(preview: Rect) -> Rect {
    let inner = Block::bordered().inner(preview);
    let rows = (inner.width as u32 * 9 / 32) as u16;
    Rect { height: rows.min(inner.height), ..inner }
}

/// Where the thumbnail of the selected video goes, with that video's id.
pub fn thumbnail_area(state: &State, screen: Rect) -> Option<(String, Rect)> {
    if state.help || state.confirm.is_some() || state.prompt == Some(Prompt::CookiesFile) {
        return None;
    }
    let video = state.videos_from_selection(0).first()?;
    let (_, list) = split_prompt(state, areas(screen, state.sidebar).content);
    let (_, preview) = split_preview(list);
    Some((video.id.clone(), thumbnail_cells(preview?)))
}

fn draw_inicio(frame: &mut Frame, state: &State, icons: &Icons, view: &View, area: Rect) {
    let lang = state.lang();
    let (input, rest) = split_prompt(state, area);
    let area = match (&state.prompt, input) {
        (Some(prompt), Some(input)) => {
            let title = match prompt {
                Prompt::Videos { scope, .. } => Line::from(vec![
                    Span::raw(format!(" {} {} ", icons.sections[5], lang.pick("Buscar videos en", "Search videos in"))),
                    Span::styled(scope.label(lang), Style::new().fg(YELLOW).bold()),
                    Span::raw(lang.pick(" · Tab cambia ", " · Tab changes ")).fg(MUTED),
                ]),
                Prompt::Channels => {
                    Line::from(format!(" {} {} ", icons.sections[6], lang.pick("Buscar canales", "Search channels")))
                }
                Prompt::CookiesFile => Line::from(format!(
                    " {} {} ",
                    icons.settings[1],
                    lang.pick("Ruta del cookies.txt", "cookies.txt path")
                )),
                Prompt::DownloadDir => Line::from(format!(
                    " {} {} ",
                    icons.settings[17],
                    lang.pick("Carpeta de descargas", "Downloads folder")
                )),
                Prompt::MediaDirs(kind) => {
                    let (icon, what) = match kind {
                        MediaKind::Video => (icons.settings[15], lang.pick("Carpetas de videos", "Video folders")),
                        MediaKind::Music => (icons.settings[16], lang.pick("Carpetas de música", "Music folders")),
                    };
                    let how = lang.pick(
                        "separadas por coma (vacío = la del sistema)",
                        "comma separated (empty = the system one)",
                    );
                    Line::from(format!(" {icon} {what}, {how} "))
                }
            };
            frame.render_widget(Paragraph::new(state.query.as_str()).block(panel(title, CYAN)), input);
            let x = input.x + 1 + state.query.chars().count() as u16;
            frame.set_cursor_position(Position::new(x.min(input.right().saturating_sub(2)), input.y + 1));
            if *prompt == Prompt::CookiesFile {
                draw_cookies_help(frame, rest, lang);
                return;
            }
            rest
        }
        _ => area,
    };
    match state.listing() {
        Some(listing) => draw_listing(frame, state, icons, view, listing, area),
        None => draw_welcome(frame, state, icons, view, area),
    }
}

fn draw_cookies_help(frame: &mut Frame, area: Rect, lang: Lang) {
    let line = |es, en, color| Line::from(lang.pick(es, en)).fg(color);
    let lines = vec![
        Line::from(""),
        line(
            "Para usar tu cuenta en una máquina sin navegador (servidor, SSH):",
            "To use your account on a machine without a browser (server, SSH):",
            FG,
        ),
        Line::from(""),
        line(
            "  1. En cualquier PC con navegador, entrá a youtube.com con tu cuenta.",
            "  1. On any computer with a browser, log in to youtube.com.",
            FG,
        ),
        line(
            "  2. Exportá las cookies en formato Netscape (cookies.txt), por ejemplo con",
            "  2. Export the cookies in Netscape format (cookies.txt), for example with",
            FG,
        ),
        line(
            "     la extensión «Get cookies.txt LOCALLY», o con:",
            "     the “Get cookies.txt LOCALLY” extension, or with:",
            FG,
        ),
        Line::from("     yt-dlp --cookies-from-browser firefox --cookies cookies.txt --skip-download :ytsubs").fg(CYAN),
        line(
            "  3. Copialo a esta máquina (scp cookies.txt servidor:~/) y escribí la ruta arriba.",
            "  3. Copy it to this machine (scp cookies.txt server:~/) and type its path above.",
            FG,
        ),
        Line::from(""),
        line(
            "tubetui comprueba que tenga una sesión de YouTube y guarda una copia privada (permisos 600).",
            "tubetui checks it holds a YouTube login and keeps a private copy (mode 600).",
            MUTED,
        ),
        line(
            "Las cookies dan acceso a tu cuenta: no las compartas.",
            "Cookies give access to your account: never share them.",
            YELLOW,
        ),
    ];
    let title = lang.pick(" Importar sesión ", " Import login ");
    frame.render_widget(Paragraph::new(lines).block(panel(title, MUTED)), area);
}

fn draw_welcome(frame: &mut Frame, state: &State, icons: &Icons, view: &View, area: Rect) {
    let lang = state.lang();
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw(format!("{}  ", icons.logo)),
            Span::styled("tube", Style::new().fg(RED).bold()),
            Span::styled("tui", Style::new().fg(FG).bold()),
        ]),
        Line::from(lang.pick("YouTube en tu terminal", "YouTube in your terminal")).fg(MUTED),
        Line::from(""),
        Line::from(
            lang.pick("Elegí una sección del menú con ↑ ↓ y Enter", "Pick a section from the menu with ↑ ↓ and Enter"),
        )
        .fg(FG),
        Line::from(lang.pick(
            "Abajo siempre ves las teclas que sirven en cada momento",
            "The bottom line always shows the keys that work right now",
        ))
        .fg(MUTED),
        Line::from(lang.pick(
            "Ctrl+y ?  muestra todos los atajos explicados · Ctrl+y l  English",
            "Ctrl+y ?  shows every shortcut, explained · Ctrl+y l  español",
        ))
        .fg(MUTED),
    ];
    if let Some(title) = &state.loading {
        lines.push(Line::from(""));
        lines.push(Line::from(loading_text(title, view.loading.unwrap_or_default(), lang)).fg(YELLOW));
        lines.push(Line::from(bouncing_bar(30, view.loading.unwrap_or_default())).fg(BLUE));
    }
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(panel("", MUTED)), area);
}

fn draw_listing(frame: &mut Frame, state: &State, icons: &Icons, view: &View, listing: &Listing, area: Rect) {
    if listing.items == Items::About {
        draw_about(frame, state, icons, area);
        return;
    }
    let breadcrumb: Vec<&str> = state.stack.iter().map(|l| l.title.as_str()).collect();
    let title = match &state.loading {
        Some(loading) => format!(" {} ", loading_text(loading, view.loading.unwrap_or_default(), state.lang())),
        None if listing.items == Items::Settings => format!(" {} ", breadcrumb.join(" › ")),
        None => format!(" {} · {} ", breadcrumb.join(" › "), listing.items.len()),
    };
    let items: Vec<ListItem> = match &listing.items {
        Items::Videos(videos) => videos.iter().map(|v| video_row(state, icons, view, v)).collect(),
        Items::Channels(channels) => channels.iter().map(|c| channel_row(state, icons, c)).collect(),
        Items::Profiles(profiles) => profiles.iter().map(|p| profile_row(state, icons, p)).collect(),
        Items::Settings => {
            Setting::ALL.iter().enumerate().map(|(i, s)| setting_row(state, icons.settings[i], *s)).collect()
        }
        Items::Files(files, kind) => files.iter().map(|f| file_row(icons, f, *kind)).collect(),
        Items::About => Vec::new(),
    };
    let focused = state.focus == Focus::List;
    let highlight = if focused { Style::new().bg(HIGHLIGHT).add_modifier(Modifier::BOLD) } else { Style::new() };
    let list = List::new(items)
        .block(panel(title, if focused { BLUE } else { MUTED }))
        .highlight_symbol(if focused { "▌" } else { " " })
        .highlight_style(highlight);
    let selected = (!listing.items.is_empty()).then_some(listing.selected);
    let area = match (&listing.items, split_preview(area)) {
        (Items::Videos(videos), (left, Some(preview))) => {
            draw_preview(frame, state, icons, view, videos.get(listing.selected), preview);
            left
        }
        (Items::Channels(channels), (left, Some(preview))) => {
            draw_channel_preview(frame, state, icons, channels.get(listing.selected), preview);
            left
        }
        _ => area,
    };
    frame.render_stateful_widget(list, area, &mut ListState::default().with_selected(selected));
}

/// The selected video: thumbnail (drawn by the runtime over the empty cells
/// left here) and its details.
fn draw_preview(frame: &mut Frame, state: &State, icons: &Icons, view: &View, video: Option<&Video>, area: Rect) {
    let lang = state.lang();
    let block = panel(lang.pick(" Vista previa ", " Preview "), MUTED);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(v) = video else { return };
    let text_area = if view.thumbnails {
        let image = thumbnail_cells(area);
        frame.render_widget(
            Paragraph::new("⏳").fg(MUTED).alignment(Alignment::Center),
            Rect { y: image.y + image.height / 2, height: 1, ..image },
        );
        Rect { y: image.bottom() + 1, height: inner.bottom().saturating_sub(image.bottom() + 1), ..inner }
    } else {
        inner
    };
    let mut details = vec![v.duration_label()];
    if let Some(ts) = v.published {
        let (date, age) = date_labels(ts, view.now, lang);
        details.push(format!("{age} ({date})"));
    }
    let mut lines = vec![
        Line::from(v.title.as_str()).fg(FG).bold(),
        Line::from(""),
        Line::from(v.channel.as_str()).fg(MAGENTA),
        Line::from(details.into_iter().filter(|d| !d.is_empty()).collect::<Vec<_>>().join(" · ")).fg(MUTED),
    ];
    match v.live {
        Live::Now => lines.push(Line::from(format!("{} {}", icons.live, lang.pick("en vivo", "live"))).fg(RED)),
        Live::Upcoming => {
            lines.push(Line::from(format!("{} {}", icons.upcoming, lang.pick("próximo", "upcoming"))).fg(YELLOW))
        }
        Live::No => {}
    }
    if state.later.iter().any(|l| l.id == v.id) {
        lines.push(Line::from(format!("{} {}", icons.later, lang.pick("En Ver después", "In Watch later"))).fg(CYAN));
    }
    if v.channel_id.as_ref().is_some_and(|c| state.subs.contains(c)) {
        lines.push(Line::from(format!("{} {}", icons.subscribed, lang.pick("Suscripto", "Subscribed"))).fg(GREEN));
    }
    frame.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true }), text_area);
}

/// The selected channel: name, subscribers and its whole description.
fn draw_channel_preview(frame: &mut Frame, state: &State, icons: &Icons, channel: Option<&Channel>, area: Rect) {
    let lang = state.lang();
    let block = panel(lang.pick(" Canal ", " Channel "), MUTED);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(c) = channel else { return };
    let mut lines = vec![Line::from(c.name.as_str()).fg(MAGENTA).bold()];
    if let Some(followers) = &c.followers {
        lines.push(Line::from(followers.as_str()).fg(MUTED));
    }
    if state.subs.contains(&c.id) {
        lines.push(Line::from(format!("{} {}", icons.subscribed, lang.pick("Suscripto", "Subscribed"))).fg(GREEN));
    }
    lines.push(Line::from(""));
    match &c.description {
        Some(description) => lines.extend(description.lines().map(|l| Line::from(l).fg(FG))),
        None => lines.push(Line::from(lang.pick("Sin descripción", "No description")).fg(MUTED)),
    }
    frame.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true }), inner);
}

fn video_row<'a>(state: &State, icons: &Icons, view: &View, v: &'a Video) -> ListItem<'a> {
    let lang = state.lang();
    let mut spans = match (v.live, v.published) {
        (Live::Now, _) => {
            let live = format!("{} {}", icons.live, lang.pick("en vivo", "live"));
            vec![Span::styled(pad_cells(&live, DATE_WIDTH), RED)]
        }
        (Live::Upcoming, _) => {
            let upcoming = format!("{} {}", icons.upcoming, lang.pick("próximo", "upcoming"));
            vec![Span::styled(pad_cells(&upcoming, DATE_WIDTH), YELLOW)]
        }
        (Live::No, Some(ts)) => {
            let (date, age) = date_labels(ts, view.now, lang);
            vec![Span::styled(date, GREEN), Span::styled(format!(" {age:<5}"), MUTED)]
        }
        (Live::No, None) => vec![Span::raw(" ".repeat(DATE_WIDTH))],
    };
    spans.push(Span::styled(format!(" {} ", pad(&v.channel, CHANNEL_WIDTH)), MAGENTA));
    if state.later.iter().any(|l| l.id == v.id) {
        spans.push(Span::raw(format!("{} ", icons.later)));
    }
    spans.push(Span::styled(v.title.as_str(), FG));
    spans.push(Span::styled(format!("  {}", v.duration_label()), MUTED));
    ListItem::new(Line::from(spans))
}

fn channel_row<'a>(state: &State, icons: &Icons, c: &'a Channel) -> ListItem<'a> {
    let mark = if state.subs.contains(&c.id) { icons.subscribed } else { " " };
    let mut spans = vec![Span::styled(format!(" {mark} "), GREEN), Span::styled(c.name.as_str(), MAGENTA)];
    if let Some(followers) = &c.followers {
        spans.push(Span::styled(format!("  {followers}"), MUTED));
    }
    if let Some(description) = &c.description {
        spans.push(Span::styled(format!("  {}", description.replace('\n', " ")), Style::new().fg(FG).add_modifier(Modifier::DIM)));
    }
    ListItem::new(Line::from(spans))
}

fn profile_row<'a>(state: &State, icons: &Icons, spec: &'a str) -> ListItem<'a> {
    let current = if spec == state.config.session.browser && state.config.session.cookies_file.is_empty() {
        "● "
    } else {
        "  "
    };
    let label = if spec.is_empty() { state.lang().pick("Sin sesión", "Not logged in") } else { spec };
    ListItem::new(Line::from(vec![
        Span::styled(format!(" {current}"), GREEN),
        Span::raw(format!("{} ", icons.user)),
        Span::styled(label, FG),
    ]))
}

fn file_row<'a>(icons: &Icons, file: &'a LocalFile, kind: MediaKind) -> ListItem<'a> {
    let icon = match kind {
        MediaKind::Video => icons.sections[7],
        MediaKind::Music => icons.sections[8],
    };
    ListItem::new(Line::from(vec![
        Span::raw(format!(" {icon} ")),
        Span::styled(format!("{} ", pad(&file.folder, 28)), MAGENTA),
        Span::styled(file.name.as_str(), FG),
    ]))
}

fn setting_row(state: &State, icon: &str, setting: Setting) -> ListItem<'static> {
    let value = state.setting_value(setting);
    let mut spans = vec![Span::raw(format!(" {icon} ")), Span::styled(pad_cells(setting.label(state.lang()), 44), FG)];
    if !value.is_empty() {
        spans.push(Span::styled(value, Style::new().fg(YELLOW).bold()));
    }
    ListItem::new(Line::from(spans))
}

fn draw_status(frame: &mut Frame, state: &State, icons: &Icons, view: &View, area: Rect) {
    let right = format!(
        "{} {}  {} {} ",
        icons.user,
        state.session_label(),
        icons.language,
        language_label(state.config.player.language, state.lang())
    );
    let right_width = Span::raw(right.as_str()).width() as u16 + 1;
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(right_width)]).areas(area);
    let left = match &state.loading {
        Some(title) => {
            Paragraph::new(format!(" {}", loading_text(title, view.loading.unwrap_or_default(), state.lang())))
                .fg(YELLOW)
        }
        None => Paragraph::new(format!(" {}", state.status)).fg(FG),
    };
    // A shaded bar, so it reads apart from the keys below it.
    frame.render_widget(Block::new().bg(HIGHLIGHT), area);
    frame.render_widget(left, left_area);
    frame.render_widget(Paragraph::new(right).fg(BLUE).alignment(Alignment::Right), right_area);
}

/// Keys that work right now, each with what it does.
pub fn hints(state: &State) -> Vec<(String, &'static str)> {
    let lang = state.lang();
    let rows = |table| keymap::rows(table, lang);
    let key = |k: &str, es, en| (k.to_string(), lang.pick(es, en));
    let all_keys = key("C-y ?", "todos los atajos", "every shortcut");
    let mut hints = if state.confirm.is_some() {
        vec![key("s", "sí, desuscribirme", "yes, unsubscribe"), key(lang.pick("otra tecla", "other key"), "no", "no")]
    } else if state.help {
        vec![key(lang.pick("cualquier tecla", "any key"), "cerrar la ayuda", "close the help")]
    } else if state.prefix {
        return keymap::prefix_rows(lang);
    } else if let Some(prompt) = &state.prompt {
        match prompt {
            Prompt::Videos { .. } => rows(keymap::IN_SEARCH),
            _ => vec![key("Enter", "aceptar", "accept"), key("Esc", "cancelar", "cancel")],
        }
    } else if state.active_tab().is_some() {
        let mut keys = rows(keymap::IN_TAB);
        keys.extend(rows(keymap::ON_VIDEO));
        keys
    } else if state.focus == Focus::Menu || state.listing().is_none() {
        rows(keymap::IN_MENU)
    } else {
        let back = key("Esc", "volver", "back");
        match state.listing().map(|l| &l.items) {
            Some(Items::Videos(_)) => vec![
                key("Enter", "ver", "play"),
                key("l", "ver después", "watch later"),
                key("s", "suscribirse", "subscribe"),
                key("d", "descargar", "download"),
                key("c", "canal", "channel"),
                key("/", "buscar", "search"),
                back,
            ],
            Some(Items::Channels(_)) => vec![
                key("Enter", "ver sus videos", "its videos"),
                key("s", "suscribirse / desuscribirse", "subscribe / unsubscribe"),
                back,
            ],
            Some(Items::Profiles(_)) => vec![key("Enter", "usar esta sesión", "use this login"), back],
            Some(Items::Files(_, MediaKind::Music)) => {
                vec![key("Enter", "escuchar desde esta canción", "play from this song"), back]
            }
            Some(Items::Files(_, MediaKind::Video)) => vec![key("Enter", "ver", "play"), back],
            Some(Items::About) => vec![back],
            _ => vec![key("Enter", "cambiar / abrir", "change / open"), back],
        }
    };
    hints.push(all_keys);
    hints
}

/// As many hints as fit in the line, whole; the last one (`C-y ?`, every
/// shortcut) is always kept.
fn draw_hints(frame: &mut Frame, hints: &[(String, &str)], area: Rect) {
    let hint = |(key, description): &(String, &str)| {
        [Span::styled(key.clone(), Style::new().fg(CYAN).bold()), Span::styled(format!(" {description}   "), MUTED)]
    };
    let width = |spans: &[Span]| spans.iter().map(Span::width).sum::<usize>();
    let Some((last, rest)) = hints.split_last() else { return };
    let last = hint(last);
    let mut spans = vec![Span::raw(" ")];
    let room = (area.width as usize).saturating_sub(width(&last) + 1);
    for h in rest {
        let next = hint(h);
        if width(&spans) + width(&next) > room {
            break;
        }
        spans.extend(next);
    }
    spans.extend(last);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Where the help or the confirmation dialog is drawn, if one is open.
pub fn dialog_area(state: &State, screen: Rect) -> Option<Rect> {
    if state.help {
        let (left, right) = help_columns(state.lang());
        Some(centered(screen, 118, left.len().max(right.len()) as u16 + 3))
    } else {
        state.confirm.as_ref().map(|_| centered(screen, 56, 5))
    }
}

/// The help screen's two balanced columns, so everything fits on a laptop screen.
fn help_columns(lang: Lang) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let lines_of = |sections: &[(&'static str, Vec<(String, &'static str)>)]| {
        let mut lines = Vec::new();
        for (title, rows) in sections {
            lines.push(Line::from(*title).fg(BLUE).bold());
            for (keys, description) in rows {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {keys:<11}"), Style::new().fg(CYAN).bold()),
                    Span::styled(*description, FG),
                ]));
            }
            lines.push(Line::from(""));
        }
        lines
    };
    let mut sections = keymap::help(lang);
    let sizes: Vec<usize> = sections.iter().map(|(_, rows)| rows.len() + 2).collect();
    let split =
        (1..sections.len()).min_by_key(|&k| sizes[..k].iter().sum::<usize>().max(sizes[k..].iter().sum())).unwrap_or(1);
    let right = sections.split_off(split);
    (lines_of(&sections), lines_of(&right))
}

fn draw_help(frame: &mut Frame, lang: Lang) {
    let (left, right) = help_columns(lang);
    let area = centered(frame.area(), 118, left.len().max(right.len()) as u16 + 3);
    frame.render_widget(Clear, area);
    let title =
        lang.pick(" ⌨  Atajos de teclado · cualquier tecla cierra ", " ⌨  Keyboard shortcuts · any key closes ");
    let block = panel(title, MAGENTA);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [l, r] = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).spacing(2).areas(inner);
    frame.render_widget(Paragraph::new(left), l);
    frame.render_widget(Paragraph::new(right), r);
}

fn draw_confirm(frame: &mut Frame, name: &str, lang: Lang) {
    let area = centered(frame.area(), 56, 5);
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw(lang.pick("¿Desuscribirte de ", "Unsubscribe from ")),
            Span::styled(name, Style::new().fg(MAGENTA).bold()),
            Span::raw("?"),
        ]),
        Line::from(lang.pick("s = sí · cualquier otra tecla = no", "y = yes · any other key = no")).fg(MUTED),
    ];
    let title = lang.pick(" Confirmar ", " Confirm ");
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(panel(title, RED)), area);
}

/// Version, links and the donation addresses from `donate.toml`.
fn draw_about(frame: &mut Frame, state: &State, icons: &Icons, area: Rect) {
    let lang = state.lang();
    let label = |text: &'static str| Span::styled(format!("{text:>12}  "), MUTED);
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw(format!("{}  ", icons.logo)),
            Span::styled("tube", Style::new().fg(RED).bold()),
            Span::styled("tui", Style::new().fg(FG).bold()),
            Span::styled(format!("  v{}", env!("CARGO_PKG_VERSION")), MUTED),
        ]),
        Line::from(lang.pick(
            "YouTube y tus videos y música, en la terminal",
            "YouTube and your own videos and music, in the terminal",
        ))
        .fg(FG),
        Line::from(""),
        Line::from(vec![label("GitHub"), Span::styled(env!("CARGO_PKG_REPOSITORY"), CYAN)]),
        Line::from(vec![label("crates.io"), Span::styled("https://crates.io/crates/tubetui", CYAN)]),
        Line::from(vec![label(lang.pick("Licencia", "License")), Span::styled("MIT / Apache-2.0", FG)]),
        Line::from(vec![
            label(lang.pick("Errores", "Bugs")),
            Span::styled(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues"), CYAN),
        ]),
    ];
    let wallets = donations();
    if !wallets.is_empty() {
        lines.push(Line::from(""));
        lines.push(
            Line::from(lang.pick(
                "☕ ¿Te sirve? Podés apoyar el proyecto con una donación:",
                "☕ Find it useful? You can support the project with a donation:",
            ))
            .fg(YELLOW)
            .bold(),
        );
        for wallet in wallets {
            lines.push(Line::from(vec![
                Span::styled(format!("{:>12}  ", wallet.network), MAGENTA),
                Span::styled(wallet.address, Style::new().fg(GREEN).bold()),
            ]));
        }
    }
    let title = format!(" {} {} ", icons.sections[10], Section::AcercaDe.label(lang));
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(panel(title, PURPLE)), area);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [row] = Layout::vertical([Constraint::Length(height.min(area.height))]).flex(Flex::Center).areas(area);
    let [cell] = Layout::horizontal([Constraint::Length(width.min(area.width))]).flex(Flex::Center).areas(row);
    cell
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

/// Truncates or pads to exactly `width` characters, for aligned columns.
fn pad(text: &str, width: usize) -> String {
    let text: String = text.chars().take(width).collect();
    format!("{text:<width$}")
}

/// Pads to `width` terminal cells; emoji take two cells, so `{:<w}` is not enough.
fn pad_cells(text: &str, width: usize) -> String {
    let used = Span::raw(text).width();
    format!("{text}{}", " ".repeat(width.saturating_sub(used)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Effect};
    use crate::config::Config;
    use crate::domain::{channel, video};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const VIEW: View =
        View { embedded: true, now: 1_791_590_400, loading: None, opening: None, thumbnails: true, playing: None };

    fn render_with(state: &State, view: &View) -> String {
        let mut terminal = Terminal::new(TestBackend::new(130, 40)).unwrap();
        terminal.draw(|f| draw(f, state, view)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content
            .chunks(buffer.area.width as usize)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render(state: &State) -> String {
        render_with(state, &VIEW)
    }

    /// Plain-Unicode icons keep every symbol one cell wide in the test buffer.
    fn state() -> State {
        let mut config = Config::default();
        config.ui.icons = IconSet::Unicode;
        State::new(config, Vec::new())
    }

    /// Opens "Inicio" from the menu and fills it with `items`.
    fn with_items(items: Items) -> State {
        let mut s = state();
        let id = match s.update(Action::Enter).as_slice() {
            [Effect::Load { id, .. }] => *id,
            other => panic!("{other:?}"),
        };
        s.update(Action::Loaded(id, Ok(items)));
        s
    }

    fn with_tab() -> State {
        let mut s = with_items(Items::Videos(vec![video("a"), video("b")]));
        s.update(Action::Enter);
        s
    }

    fn open_settings() -> State {
        let mut s = state();
        s.menu = Section::ALL.iter().position(|x| *x == Section::Configuracion).unwrap();
        s.update(Action::Enter);
        s
    }

    fn hint_keys(state: &State) -> Vec<String> {
        hints(state).into_iter().map(|(k, _)| k).collect()
    }

    #[test]
    fn layout_reserves_status_and_hint_lines() {
        let area = Rect::new(0, 0, 100, 30);
        let with = areas(area, true);
        assert_eq!(with.sidebar.unwrap().width, SIDEBAR_WIDTH);
        assert_eq!(with.content.x, SIDEBAR_WIDTH);
        let without = areas(area, false);
        assert_eq!(without.sidebar, None);
        assert_eq!(without.content, Rect::new(0, 1, 100, 27));
        assert_eq!(without.status, Rect::new(0, 28, 100, 1));
        assert_eq!(without.hints, Rect::new(0, 29, 100, 1));
    }

    #[test]
    fn video_area_only_for_video_tabs_without_dialogs() {
        let area = Rect::new(0, 0, 100, 30);
        let mut s = with_tab();
        assert_eq!(video_area(&s, area), Some(Block::bordered().inner(areas(area, true).content)));
        s.update(Action::ToggleHelp);
        assert_eq!(video_area(&s, area), None);
        s.update(Action::ToggleHelp);
        s.update(Action::GoTab(0));
        assert_eq!(video_area(&s, area), None);
    }

    #[test]
    fn menu_shows_every_section_with_icons_and_later_count() {
        let mut s = state();
        s.later = vec![video("x"), video("y")];
        let screen = render(&s);
        for (section, icon) in Section::ALL.iter().zip(UNICODE.sections) {
            assert!(screen.contains(&format!("{icon} {}", section.label(Lang::Es))), "{section:?}\n{screen}");
        }
        assert!(screen.contains("Ver después (2)"));
        assert!(screen.contains("Elegí una sección"));
    }

    #[test]
    fn icon_set_follows_the_config() {
        let mut s = state();
        s.config.ui.icons = IconSet::Nerd;
        assert!(render(&s).contains(&format!("{} Inicio", NERD.sections[0])));
    }

    #[test]
    fn video_rows_show_date_channel_title_duration_and_later_mark() {
        let dated = Video { published: Some(VIEW.now - 3 * 86_400), ..video("a") };
        let live = Video { live: Live::Now, ..video("b") };
        let mut s = with_items(Items::Videos(vec![dated, live]));
        s.later = vec![video("a")];
        let screen = render(&s);
        assert!(screen.contains("07/10/26 3d"), "{screen}");
        assert!(screen.contains("● en vivo"));
        assert!(screen.contains("chan a"));
        assert!(screen.contains("◷ title a"));
        assert!(screen.contains("1:00"));
        assert!(screen.contains("Inicio · 2"));
    }

    #[test]
    fn channel_rows_mark_my_channels() {
        let followed = Channel {
            followers: Some("358 mil suscriptores".into()),
            description: Some("Música\ny charlas".into()),
            ..channel("UCx")
        };
        let mut s = with_items(Items::Channels(vec![followed, channel("UCy")]));
        s.subs.insert("UCx".into());
        let screen = render(&s);
        assert!(screen.contains("✓ name UCx  358 mil suscriptores  Música y charlas"), "{screen}");
        assert!(screen.contains("   name UCy"));
    }

    #[test]
    fn profile_rows_mark_current_session() {
        let mut s = with_items(Items::Profiles(vec!["brave:Default".into(), String::new()]));
        s.config.session.browser = "brave:Default".into();
        let screen = render(&s);
        assert!(screen.contains("● ☺ brave:Default"), "{screen}");
        assert!(screen.contains("☺ Sin sesión"));
    }

    #[test]
    fn settings_screen_shows_labels_and_values() {
        let screen = render(&open_settings());
        for setting in Setting::ALL {
            assert!(screen.contains(setting.label(Lang::Es)), "{setting:?}\n{screen}");
        }
        assert!(screen.contains("720p"));
        assert!(screen.contains("~/Videos"));
        assert!(screen.contains("original + subtítulos"));
    }

    #[test]
    fn search_box_shows_scope_and_query() {
        let mut s = state();
        s.update(Action::FocusSearch);
        for c in "lofi".chars() {
            s.update(Action::Input(c));
        }
        let screen = render(&s);
        assert!(screen.contains("Buscar videos en todo YouTube"), "{screen}");
        assert!(screen.contains("lofi"));
        s.update(Action::CycleScope);
        assert!(render(&s).contains("Buscar videos en mis canales"));
    }

    #[test]
    fn cookies_prompt_explains_how_to_export() {
        let mut s = open_settings();
        s.stack[0].selected = Setting::ALL.iter().position(|x| *x == Setting::ImportCookies).unwrap();
        s.update(Action::Enter);
        let screen = render(&s);
        assert!(screen.contains("Ruta del cookies.txt"));
        assert!(screen.contains("scp cookies.txt"));
    }

    #[test]
    fn tabs_appear_in_bar_and_sidebar() {
        let screen = render(&with_tab());
        assert!(screen.contains("PESTAÑAS"));
        assert_eq!(screen.matches("1 ▶ title a").count(), 2, "{screen}");
    }

    #[test]
    fn window_mode_tab_explains_where_video_is() {
        assert!(render_with(&with_tab(), &View { embedded: false, ..VIEW }).contains("ventana de mpv"));
        assert!(!render(&with_tab()).contains("ventana de mpv"));
    }

    #[test]
    fn help_shows_every_section_with_explanations() {
        let mut s = state();
        s.update(Action::ToggleHelp);
        let screen = render(&s);
        for (title, rows) in keymap::help(Lang::Es) {
            assert!(screen.contains(title), "{title}\n{screen}");
            for (_, description) in rows {
                assert!(screen.contains(description), "{description}\n{screen}");
            }
        }
    }

    #[test]
    fn hints_follow_the_context() {
        let mut s = state();
        assert_eq!(hint_keys(&s), ["↑ ↓", "Enter", "Tab", "/", "C-y ?"]);
        s.update(Action::Prefix);
        assert!(hint_keys(&s).contains(&"b".to_string()) && hint_keys(&s).contains(&"0-9".to_string()));
        let tab = with_tab();
        assert!(hint_keys(&tab).contains(&"espacio".to_string()) && hint_keys(&tab).contains(&"m".to_string()));
        let list = with_items(Items::Videos(vec![video("a")]));
        assert_eq!(hint_keys(&list), ["Enter", "l", "s", "d", "c", "/", "Esc", "C-y ?"]);
        let mut search = state();
        search.update(Action::FocusSearch);
        assert!(hint_keys(&search).contains(&"Tab".to_string()));
    }

    #[test]
    fn hint_bar_shows_keys_with_descriptions() {
        let screen = render(&with_items(Items::Videos(vec![video("a")])));
        assert!(screen.contains("l ver después"), "{screen}");
        assert!(screen.contains("C-y ? todos los atajos"));
    }

    #[test]
    fn confirm_dialog_names_the_channel() {
        let mut s = with_items(Items::Videos(vec![video("a")]));
        s.subs.insert("UCa".into());
        s.update(Action::ToggleSubscription);
        let screen = render(&s);
        assert!(screen.contains("¿Desuscribirte de chan a?"));
        assert!(screen.contains("sí, desuscribirme"));
    }

    #[test]
    fn status_bar_shows_session_and_language() {
        let mut s = state();
        s.config.session.browser = "brave:Default".into();
        let screen = render(&s);
        assert!(screen.contains("brave:Default"));
        assert!(screen.contains("original + subtítulos"));
    }

    #[test]
    fn menu_shows_group_headers() {
        let screen = render(&state());
        assert!(screen.contains("YOUTUBE"));
        assert!(screen.contains("EN TU EQUIPO"));
        assert!(screen.contains("▷ Mis videos") && screen.contains("♫ Mi música"));
    }

    #[test]
    fn music_tab_shows_the_song_and_takes_no_video_area() {
        let file = |n: &str| LocalFile { path: format!("/m/{n}.mp3").into(), name: n.into(), folder: "Disco".into() };
        let mut s = with_items(Items::Files(vec![file("Tema uno"), file("Tema dos")], MediaKind::Music));
        let screen = render(&s);
        assert!(screen.contains("♫ Disco"), "{screen}");
        assert!(screen.contains("escuchar desde esta canción"));
        s.update(Action::Enter);
        let screen = render(&s);
        assert!(screen.contains("2 canciones en la cola"), "{screen}");
        assert_eq!(video_area(&s, Rect::new(0, 0, 100, 30)), None);
    }

    #[test]
    fn fullscreen_gives_the_whole_terminal_to_the_video() {
        let mut s = with_tab();
        s.update(Action::ToggleFullscreen);
        let area = Rect::new(0, 0, 100, 30);
        assert_eq!(video_area(&s, area), Some(area));
        assert!(!render(&s).contains("tubetui"), "no sidebar or bars");
    }

    #[test]
    fn loading_shows_spinner_seconds_and_a_moving_bar() {
        let mut s = state();
        s.update(Action::Enter); // Inicio starts loading
        let view = View { loading: Some(Duration::from_millis(3_250)), ..VIEW };
        let screen = render_with(&s, &view);
        assert!(screen.contains("Cargando Inicio… 3 s"), "{screen}");
        assert!(screen.contains("█"));
        let later = View { loading: Some(Duration::from_millis(3_550)), ..VIEW };
        let (now, then) = (view.loading.unwrap(), later.loading.unwrap());
        assert_ne!(bouncing_bar(30, now), bouncing_bar(30, then), "the bar moves");
        assert_eq!(bouncing_bar(30, now).chars().count(), 30);
    }

    #[test]
    fn video_tab_shows_a_loading_bar_until_the_video_plays() {
        let opening = View { opening: Some(Duration::from_millis(2_400)), ..VIEW };
        let screen = render_with(&with_tab(), &opening);
        assert!(screen.contains("Cargando el video… 2 s"), "{screen}");
        assert!(screen.contains("█"));
        assert!(!render(&with_tab()).contains("Cargando"), "playing: nothing under the video");
        let window = View { embedded: false, ..VIEW };
        assert!(render_with(&with_tab(), &window).contains("ventana de mpv"));
    }

    #[test]
    fn english_interface() {
        let mut s = with_tab();
        s.update(Action::ToggleUiLang);
        s.update(Action::GoTab(0));
        let screen = render(&s);
        for text in ["Home", "Watch later", "ON THIS COMPUTER", "Settings", "About", "TABS", "every shortcut"] {
            assert!(screen.contains(text), "{text} missing\n{screen}");
        }
        assert!(!screen.contains("Suscripciones"), "{screen}");
        s.update(Action::ToggleHelp);
        assert!(render(&s).contains("Keyboard shortcuts"));
    }

    #[test]
    fn about_shows_version_and_links() {
        let mut s = state();
        s.menu = Section::ALL.iter().position(|x| *x == Section::AcercaDe).unwrap();
        s.update(Action::Enter);
        let screen = render(&s);
        assert!(screen.contains(env!("CARGO_PKG_VERSION")), "{screen}");
        assert!(screen.contains("github.com/polblancoo/tubetui"), "{screen}");
    }

    #[test]
    fn hints_fit_whole_and_keep_the_help_key() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        let hints = hints(&with_tab());
        terminal.draw(|f| draw_hints(f, &hints, f.area())).unwrap();
        let line: String = terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect();
        assert!(line.contains("C-y ? todos los atajos"), "{line}");
        assert!(line.contains("espacio pausa"), "{line}");
        assert!(!line.contains("silenciar"), "does not fit, left out whole: {line}");
    }

    #[test]
    fn wide_video_lists_show_a_preview_of_the_selection() {
        let mut s = with_items(Items::Videos(vec![video("a"), video("b")]));
        s.update(Action::Down);
        let screen = render(&s); // 130 columns: room for the panel
        assert!(screen.contains("Vista previa"), "{screen}");
        let screen_area = Rect::new(0, 0, 130, 40);
        let (id, cells) = thumbnail_area(&s, screen_area).unwrap();
        assert_eq!(id, "b", "the selected video");
        assert_eq!(cells.width, PREVIEW_WIDTH - 2);
        assert_eq!(cells.height, (PREVIEW_WIDTH - 2) * 9 / 32);
        s.update(Action::ToggleHelp);
        assert_eq!(thumbnail_area(&s, screen_area), None, "kitty would draw it over the help");
        s.update(Action::ToggleHelp);
        assert_eq!(thumbnail_area(&s, Rect::new(0, 0, 100, 40)), None, "too narrow");
        assert!(!render_with(&s, &View { thumbnails: false, ..VIEW }).contains("⏳"));
    }

    #[test]
    fn wide_channel_lists_show_the_whole_description() {
        let described = Channel { description: Some("Primera línea\nSegunda línea".into()), ..channel("UCx") };
        let s = with_items(Items::Channels(vec![described, channel("UCy")]));
        let screen = render(&s);
        assert!(screen.contains(" Canal "), "{screen}");
        assert!(screen.contains("│Primera línea"), "{screen}");
        assert!(screen.contains("│Segunda línea"), "kept on its own line: {screen}");
        assert_eq!(thumbnail_area(&s, Rect::new(0, 0, 130, 40)), None, "channels have no thumbnail");
    }

    #[test]
    fn video_tab_shows_volume_quality_and_codec() {
        let mut s = with_items(Items::Videos(vec![video("a")]));
        s.update(Action::Enter);
        let info = Info { volume: Some(85.0), muted: false, height: Some(480), codec: Some("h264".into()) };
        let screen = render_with(&s, &View { playing: Some(info.clone()), ..VIEW });
        assert!(screen.contains("♪ 85 %  480p  H.264"), "{screen}");
        let muted = Info { muted: true, codec: Some("vp9".into()), ..info };
        assert_eq!(playing_label(&muted, icons(IconSet::Unicode)), " ✕ 85 %  480p  VP9 ");
        assert_eq!(playing_label(&Info::default(), icons(IconSet::Unicode)), "");
    }

    #[test]
    fn helpers() {
        assert_eq!(truncate("ñandú", 3), "ñan…");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(pad("abcdef", 4), "abcd");
        assert_eq!(pad_cells("🔴 en vivo", 14), "🔴 en vivo    ");
        assert_eq!(pad_cells("● en vivo", 14), "● en vivo     ");
    }
}
