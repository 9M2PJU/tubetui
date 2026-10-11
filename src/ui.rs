//! Rendering. `areas` is the single source of truth for the layout, so the
//! runtime places the embedded video exactly where `draw` leaves room for it.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Tabs};

use crate::app::{Confirm, Focus, Group, Listing, Prompt, Section, Setting, State, Tab, language_label};
use crate::config::{IconSet, donations};
use crate::doctor::{Manager, Tool};
use crate::domain::{Channel, Live, Video, clock, date_labels};
use crate::i18n::Lang;
use crate::media::{LocalFile, MediaKind};
use crate::player::Info;
use crate::update::Install;
#[cfg(test)]
use crate::youtube::Watch;
use crate::youtube::{Items, compact};
use crate::{keymap, session};

pub const SIDEBAR_WIDTH: u16 = 26;
/// Width of the preview panel next to video lists.
const PREVIEW_WIDTH: u16 = 46;
/// Video lists narrower than this show no preview panel.
const PREVIEW_MIN_WIDTH: u16 = 96;
const CHANNEL_WIDTH: usize = 20;
/// Columns of the channel lists; the @handle goes last.
const CHANNEL_NAME_WIDTH: usize = 26;
const FOLLOWERS_WIDTH: usize = 24;
/// Width of the date column: `"10/10/26 hoy   "`.
const DATE_WIDTH: usize = 14;

// Tokyo Night.
const FG: Color = Color::Rgb(192, 202, 245);
const MUTED: Color = Color::Rgb(86, 95, 137);
const HIGHLIGHT: Color = Color::Rgb(41, 46, 66);
/// Separator lines inside dialogs: visible but quieter than text.
const LINE: Color = Color::Rgb(59, 66, 97);
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
    pub settings: [&'static str; 23],
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
    /// A video already watched.
    pub watched: &'static str,
}

const EMOJI: Icons = Icons {
    sections: ["🏠", "🧭", "📺", "🕒", "⭐", "🔎", "📡", "📼", "🎵", "🔧", "💜", "🚪"],
    settings: [
        "👤", "📄", "🔄", "🚪", "🌐", "💬", "📐", "🎬", "🎧", "🗣", "😀", "🖼", "🖱", "✨", "🌊", "💥", "📼", "🎵", "📁",
        "🩺", "👁", "⬆", "📝",
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
    watched: "👁",
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
        "\u{f0ac}", "\u{f118}", "\u{f03e}", "\u{f245}", "\u{f0d0}", "\u{f0e7}", "\u{f0a1}", "\u{f03d}", "\u{f001}",
        "\u{f07b}", "\u{f0f1}", "\u{f06e}", "\u{f01b}", "\u{f044}",
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
    watched: "\u{f06e}",
};

const UNICODE: Icons = Icons {
    sections: ["⌂", "◎", "▣", "◷", "★", "⌕", "⌁", "▷", "♫", "⚙", "♥", "⏻"],
    settings: [
        "☺", "▤", "↻", "⏻", "⇄", "≡", "▦", "▶", "♪", "⚑", "☻", "▨", "➚", "✦", "≈", "✸", "▷", "♫", "▭", "✚", "◉", "↑",
        "✎",
    ],
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
    watched: "◉",
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

/// What is under the mouse pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spot {
    /// A section of the menu, in `Section::ALL` order.
    Section(usize),
    /// A tab, in the menu or the tab bar; 0 is Inicio.
    Tab(usize),
    /// A row of the current list.
    Row(usize),
    /// The picture of the active video tab.
    Video,
    /// A second on the progress bar.
    Progress(u32),
}

/// What is at cell (`x`, `y`), laid out as [`draw`] does it.
pub fn spot(state: &State, playing: Option<&Info>, screen: Rect, x: u16, y: u16) -> Option<Spot> {
    let at = Position::new(x, y);
    if state.help || state.confirm.is_some() {
        return None;
    }
    if state.fullscreen {
        return Some(Spot::Video);
    }
    let areas = areas(screen, state.sidebar);
    if let Some(side) = areas.sidebar.filter(|a| a.contains(at)) {
        let inner = Block::bordered().inner(side);
        let line = usize::from(y.checked_sub(inner.y)?);
        return sidebar_items(state, icons(state.config.ui.icons)).get(line)?.1;
    }
    if areas.tabs.contains(at) {
        // Tabs draws ` title ` and a one-cell divider after each.
        let mut start = areas.tabs.x;
        for (i, title) in tab_titles(state, icons(state.config.ui.icons)).iter().enumerate() {
            let end = start + Span::raw(title.as_str()).width() as u16 + 2;
            if x < end {
                return (x >= start).then_some(Spot::Tab(i));
            }
            start = end + 1;
        }
        return None;
    }
    if !areas.content.contains(at) {
        return None;
    }
    if let Some(tab) = state.active_tab() {
        if let Some((bar, duration)) = playing.and_then(|info| progress_bar(info, areas.content))
            && bar.contains(at)
        {
            let fraction = (f64::from(x - bar.x) + 0.5) / f64::from(bar.width);
            return Some(Spot::Progress((fraction * duration) as u32));
        }
        if state.video_window {
            // The video is elsewhere: here are its details and the related list.
            state.related_shown()?;
            let list = Block::bordered().inner(watch_layout(Block::bordered().inner(areas.content)).related);
            return list_row(list, tab.related, tab.related_videos().len(), at);
        }
        let inner = Block::bordered().inner(areas.content);
        return (!tab.playback.audio_only && inner.contains(at)).then_some(Spot::Video);
    }
    if state.prompt == Some(Prompt::CookiesFile) {
        return None;
    }
    let listing = state.listing()?;
    let (_, rest) = split_prompt(state, areas.content);
    let list = match listing.items {
        Items::Videos(_) | Items::Channels(_) => split_preview(rest).0,
        Items::About | Items::System => return None,
        _ => rest,
    };
    list_row(Block::bordered().inner(list), listing.selected, listing.items.len(), at)
}

/// The row of a list (drawn in `inner`, `selected` of `len`) at `at`.
fn list_row(inner: Rect, selected: usize, len: usize, at: Position) -> Option<Spot> {
    if !inner.contains(at) || inner.height == 0 {
        return None;
    }
    // A list drawn from offset 0 scrolls just enough to show the selection last.
    let offset = selected.saturating_sub(usize::from(inner.height) - 1);
    let row = offset + usize::from(at.y - inner.y);
    (row < len).then_some(Spot::Row(row))
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
            } else if view.embedded || (state.video_window && playback.video.is_some()) {
                Paragraph::default() // the video, or the details drawn below
            } else {
                Paragraph::new(lang.pick("Se reproduce en una ventana de mpv", "Playing in an mpv window")).fg(MUTED)
            };
            let mut block = panel(title, BLUE);
            if let Some(info) = &view.playing {
                block = block.title_top(Line::from(playing_label(info, icons)).fg(CYAN).right_aligned());
                if let Some(progress) = progress_line(info, areas.content.width) {
                    block = block.title_bottom(progress);
                }
            }
            frame.render_widget(body.block(block), areas.content);
            if state.video_window && !view.embedded {
                draw_watch(frame, state, icons, view, tab, Block::bordered().inner(areas.content));
            }
        }
    }
    draw_status(frame, state, icons, view, areas.status);
    draw_hints(frame, &hints(state), areas.hints);
    if state.help {
        draw_help(frame, lang);
    }
    if let Some(confirm) = &state.confirm {
        draw_confirm(frame, confirm, lang);
    }
}

/// Cells of a video tab whose video plays in its own window.
struct WatchLayout {
    thumbnail: Rect,
    details: Rect,
    related: Rect,
}

/// Columns of the thumbnail on a video tab.
const WATCH_THUMBNAIL_WIDTH: u16 = 36;

fn watch_layout(inner: Rect) -> WatchLayout {
    let width = WATCH_THUMBNAIL_WIDTH.min(inner.width / 3);
    let rows = (u32::from(width) * 9 / 32) as u16;
    let [card, _, related] =
        Layout::vertical([Constraint::Length(rows.max(7)), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let [thumbnail, _, details] =
        Layout::horizontal([Constraint::Length(width), Constraint::Length(2), Constraint::Fill(1)]).areas(card);
    WatchLayout { thumbnail: Rect { height: rows.min(card.height), ..thumbnail }, details, related }
}

/// The video plays in its own window: the tab shows its details and what to watch next.
fn draw_watch(frame: &mut Frame, state: &State, icons: &Icons, view: &View, tab: &Tab, inner: Rect) {
    let lang = state.lang();
    let Some(video) = &tab.playback.video else { return };
    let layout = watch_layout(inner);
    let details = if view.thumbnails {
        let middle = Rect { y: layout.thumbnail.y + layout.thumbnail.height / 2, height: 1, ..layout.thumbnail };
        frame.render_widget(Paragraph::new("⏳").fg(MUTED).alignment(Alignment::Center), middle);
        layout.details
    } else {
        Rect { x: inner.x, width: inner.width, ..layout.details }
    };
    let watch = tab.watch.as_ref();
    let mut channel = vec![Span::styled(video.channel.as_str(), MAGENTA)];
    if let Some(subscribers) = watch.and_then(|w| w.subscribers.as_deref()) {
        channel.push(Span::styled(format!(" · {subscribers} {}", lang.pick("suscriptores", "subscribers")), MUTED));
    }
    if video.channel_id.as_ref().is_some_and(|c| state.subs.contains(c)) {
        channel.push(Span::styled(format!("  {} {}", icons.subscribed, lang.pick("Suscripto", "Subscribed")), GREEN));
    }
    let mut numbers = Vec::new();
    if let Some(views) = watch.and_then(|w| w.views) {
        numbers.push(format!("{} {}", compact(views), lang.pick("vistas", "views")));
    }
    if let Some(likes) = watch.and_then(|w| w.likes) {
        numbers.push(format!("👍 {}", compact(likes)));
    }
    let mut when = vec![video.duration_label()];
    if let Some(ts) = watch.and_then(|w| w.published).or(video.published) {
        let (date, age) = date_labels(ts, view.now, lang);
        when.push(format!("{age} ({date})"));
    }
    let mut lines = vec![Line::from(video.title.as_str()).fg(FG).bold(), Line::from(""), Line::from(channel)];
    for row in [numbers, when] {
        let row: Vec<String> = row.into_iter().filter(|p| !p.is_empty()).collect();
        if !row.is_empty() {
            lines.push(Line::from(row.join(" · ")).fg(MUTED));
        }
    }
    if state.later.iter().any(|l| l.id == video.id) {
        lines.push(Line::from(format!("{} {}", icons.later, lang.pick("En Ver después", "In Watch later"))).fg(CYAN));
    }
    if let Some(elapsed) = view.opening {
        lines.push(Line::from(loading_text(lang.pick("el video", "the video"), elapsed, lang)).fg(YELLOW));
    }
    frame.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true }), details);

    let related = tab.related_videos();
    let focused = tab.on_related && !related.is_empty();
    let title = match watch {
        None => format!(" {} ", lang.pick("Buscando videos relacionados…", "Looking for related videos…")),
        Some(_) if related.is_empty() => format!(" {} ", lang.pick("Sin videos relacionados", "No related videos")),
        Some(_) if focused => format!(" {} · {} ", lang.pick("A continuación", "Up next"), related.len()),
        Some(_) => format!(" {} · {} · Tab ", lang.pick("A continuación", "Up next"), related.len()),
    };
    let highlight = if focused { Style::new().bg(HIGHLIGHT).add_modifier(Modifier::BOLD) } else { Style::new() };
    let list = List::new(related.iter().map(|v| video_row(state, icons, view, v)))
        .block(panel(title, if focused { BLUE } else { MUTED }))
        .highlight_symbol(if focused { "▌" } else { " " })
        .highlight_style(highlight);
    let selected = (!related.is_empty()).then_some(tab.related);
    frame.render_stateful_widget(list, layout.related, &mut ListState::default().with_selected(selected));
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

/// Where the progress bar sits in the bottom border of the video tab:
/// the cells of the bar itself and the seconds it spans.
pub fn progress_bar(info: &Info, content: Rect) -> Option<(Rect, f64)> {
    info.position?;
    let duration = info.duration.filter(|d| *d > 0.0)?;
    let clock_width = clock(duration as u32).len() as u16;
    // ` ▶ 1:23 ` before the bar and ` 4:56 ` after it, inside the corners.
    let before = 2 + 2 + clock_width + 1;
    let after = 1 + clock_width + 1;
    let width = content.width.checked_sub(2 + before + after).filter(|w| *w >= 10)?;
    Some((Rect::new(content.x + 1 + before, content.bottom().saturating_sub(1), width, 1), duration))
}

/// ` ▶ 1:23 ━━━━━━━━━━━━──────── 4:56 `, the width of the tab's bottom border:
/// played in red, downloaded in grey, the rest dim.
fn progress_line(info: &Info, width: u16) -> Option<Line<'static>> {
    let (bar, duration) = progress_bar(info, Rect::new(0, 0, width, 1))?;
    let position = info.position.unwrap_or_default().clamp(0.0, duration);
    let cells = |seconds: f64| ((seconds.clamp(0.0, duration) / duration) * f64::from(bar.width)).round() as usize;
    let done = cells(position);
    let loaded = cells(info.buffered.unwrap_or_default()).saturating_sub(done);
    let rest = usize::from(bar.width) - done - loaded;
    let clock_width = clock(duration as u32).len();
    // One cell each: the emoji pause sign takes two in most terminals.
    let icon = if info.paused { "‖" } else { "▶" };
    Some(Line::from(vec![
        Span::styled(format!(" {icon} {:>clock_width$} ", clock(position as u32)), Style::new().fg(FG)),
        Span::styled("━".repeat(done), Style::new().fg(RED)),
        Span::styled("━".repeat(loaded), Style::new().fg(MUTED)),
        Span::styled("─".repeat(rest), Style::new().fg(MUTED)),
        Span::styled(format!(" {} ", clock(duration as u32)), Style::new().fg(FG)),
    ]))
}

fn panel<'a>(title: impl Into<Line<'a>>, color: Color) -> Block<'a> {
    Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(color)).title(title)
}

/// The menu's lines, each with what a click on it does.
fn sidebar_items(state: &State, icons: &Icons) -> Vec<(ListItem<'static>, Option<Spot>)> {
    let lang = state.lang();
    let on_inicio = state.active == 0;
    let mut items: Vec<(ListItem, Option<Spot>)> = Vec::new();
    let mut group = None;
    for (i, section) in Section::ALL.iter().enumerate() {
        if group != Some(section.group()) {
            group = Some(section.group());
            if i > 0 {
                items.push((ListItem::new(""), None));
            }
            if section.group() != Group::App {
                items.push((
                    ListItem::new(Line::from(format!(" {}", section.group().label(lang))).fg(MUTED).bold()),
                    None,
                ));
            }
        }
        let mut label = section.label(lang).to_owned();
        if *section == Section::VerDespues && !state.later.is_empty() {
            label.push_str(&format!(" ({})", state.later.len()));
        }
        if *section == Section::AcercaDe && state.update.is_some() {
            label.push_str(" ⬆");
        }
        let selected = i == state.menu;
        let focused = selected && on_inicio && state.focus == Focus::Menu;
        let color = SECTION_COLORS[i];
        let mut style = Style::new().fg(if selected { color } else { FG });
        if focused {
            style = style.bg(HIGHLIGHT).add_modifier(Modifier::BOLD);
        }
        let line = Line::from(vec![
            Span::styled(if focused { "▌" } else { " " }, color),
            Span::raw(format!("{} ", icons.sections[i])),
            Span::raw(label),
        ]);
        items.push((ListItem::new(line).style(style), Some(Spot::Section(i))));
    }
    if !state.tabs.is_empty() {
        items.push((ListItem::new(""), None));
        items.push((ListItem::new(Line::from(lang.pick(" PESTAÑAS", " TABS")).fg(MUTED).bold()), None));
        for (i, tab) in state.tabs.iter().enumerate() {
            let active = state.active == i + 1;
            let style = if active { Style::new().fg(MAGENTA).bg(HIGHLIGHT).bold() } else { Style::new().fg(FG) };
            let line =
                format!("{}{} {} {}", if active { "▌" } else { " " }, i + 1, tab_icon(icons, tab), tab.playback.title);
            items.push((ListItem::new(line).style(style), Some(Spot::Tab(i + 1))));
        }
    }
    items
}

fn draw_sidebar(frame: &mut Frame, state: &State, icons: &Icons, area: Rect) {
    let items = sidebar_items(state, icons).into_iter().map(|(item, _)| item);
    let title = Line::from(vec![
        Span::raw(format!(" {} ", icons.logo)),
        Span::styled("tube", Style::new().fg(RED).bold()),
        Span::styled("tui ", Style::new().fg(FG).bold()),
    ]);
    frame.render_widget(List::new(items).block(panel(title, MUTED)), area);
}

fn tab_titles(state: &State, icons: &Icons) -> Vec<String> {
    let home = Section::Inicio.label(state.lang());
    std::iter::once(format!("{} {home}", icons.sections[0]))
        .chain(
            state
                .tabs
                .iter()
                .enumerate()
                .map(|(i, t)| format!("{} {} {}", i + 1, tab_icon(icons, t), truncate(&t.playback.title, 20))),
        )
        .collect()
}

fn draw_tabs(frame: &mut Frame, state: &State, icons: &Icons, area: Rect) {
    let tabs = Tabs::new(tab_titles(state, icons))
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
    if let Some(tab) = state.active_tab() {
        let video = tab.playback.video.as_ref().filter(|_| state.video_window && !state.fullscreen)?;
        let inner = Block::bordered().inner(areas(screen, state.sidebar).content);
        return Some((video.id.clone(), watch_layout(inner).thumbnail)).filter(|(_, cells)| cells.height > 0);
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
                    icons.settings[18],
                    lang.pick("Carpeta de descargas", "Downloads folder")
                )),
                Prompt::MediaDirs(kind) => {
                    let (icon, what) = match kind {
                        MediaKind::Video => (icons.settings[16], lang.pick("Carpetas de videos", "Video folders")),
                        MediaKind::Music => (icons.settings[17], lang.pick("Carpetas de música", "Music folders")),
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
    if listing.items == Items::System {
        draw_system(frame, state, area);
        return;
    }
    let breadcrumb: Vec<&str> = state.stack.iter().map(|l| l.title.as_str()).collect();
    let title = match &state.loading {
        Some(loading) => format!(" {} ", loading_text(loading, view.loading.unwrap_or_default(), state.lang())),
        None if listing.items == Items::Settings => format!(" {} ", breadcrumb.join(" › ")),
        None => match &listing.items {
            Items::Videos(videos) if listing.all.is_some() => {
                let hidden = listing.all.as_ref().map_or(0, Vec::len) - videos.len();
                let note = match (state.show_watched, hidden) {
                    (true, _) => {
                        format!(" · {} {}", icons.watched, state.lang().pick("con los vistos", "watched included"))
                    }
                    (false, 0) => String::new(),
                    (false, n) => {
                        format!(" · {} {n} {}", icons.watched, state.lang().pick("vistos ocultos", "watched hidden"))
                    }
                };
                format!(" {} · {}{note} ", breadcrumb.join(" › "), videos.len())
            }
            Items::Files(files, _) if files.iter().any(|f| state.marked.contains(&f.path)) => {
                let marked = files.iter().filter(|f| state.marked.contains(&f.path)).count();
                let label = state.lang().pick("marcados", "marked");
                format!(" {} · {} · ✗ {marked} {label} ", breadcrumb.join(" › "), listing.items.len())
            }
            _ => format!(" {} · {} ", breadcrumb.join(" › "), listing.items.len()),
        },
    };
    let items: Vec<ListItem> = match &listing.items {
        Items::Videos(videos) => videos.iter().map(|v| video_row(state, icons, view, v)).collect(),
        Items::Channels(channels) => {
            // The description goes in the panel when there is one, other details in the row.
            let panel = split_preview(area).1.is_some();
            channels.iter().map(|c| channel_row(state, icons, c, panel)).collect()
        }
        Items::Profiles(profiles) => profiles.iter().map(|p| profile_row(state, icons, p)).collect(),
        Items::Settings => {
            Setting::ALL.iter().enumerate().map(|(i, s)| setting_row(state, icons.settings[i], *s)).collect()
        }
        Items::Files(files, kind) => {
            files.iter().map(|f| file_row(icons, f, *kind, state.marked.contains(&f.path))).collect()
        }
        Items::About | Items::System => Vec::new(),
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
    if let Some(handle) = &c.handle {
        lines.push(Line::from(handle.as_str()).fg(CYAN));
    }
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
    if state.watched.contains(&v.id) {
        // Already seen: marked and dimmed.
        spans.push(Span::styled(format!("{} ", icons.watched), MUTED));
        spans.push(Span::styled(v.title.as_str(), MUTED));
    } else {
        spans.push(Span::styled(v.title.as_str(), FG));
    }
    spans.push(Span::styled(format!("  {}", v.duration_label()), MUTED));
    ListItem::new(Line::from(spans))
}

/// Name, subscribers and @handle in columns; the description too unless
/// the side panel shows it.
fn channel_row<'a>(state: &State, icons: &Icons, c: &'a Channel, panel: bool) -> ListItem<'a> {
    let mark = if state.subs.contains(&c.id) { icons.subscribed } else { " " };
    let column = |text: &str, width: usize| pad_cells(&truncate(text, width - 2), width);
    let mut spans = vec![
        Span::styled(format!(" {mark} "), GREEN),
        Span::styled(column(&c.name, CHANNEL_NAME_WIDTH), MAGENTA),
        Span::styled(column(c.followers.as_deref().unwrap_or(""), FOLLOWERS_WIDTH), MUTED),
    ];
    if let Some(handle) = &c.handle {
        spans.push(Span::styled(handle.as_str(), CYAN));
    }
    if let Some(description) = c.description.as_ref().filter(|_| !panel) {
        spans.push(Span::styled(
            format!("  {}", description.replace('\n', " ")),
            Style::new().fg(FG).add_modifier(Modifier::DIM),
        ));
    }
    ListItem::new(Line::from(spans))
}

fn profile_row<'a>(state: &State, icons: &Icons, spec: &'a str) -> ListItem<'a> {
    let current = if spec == state.config.session.browser && state.config.session.cookies_file.is_empty() {
        "● "
    } else {
        "  "
    };
    let label = if spec.is_empty() {
        state.lang().pick("Sin sesión", "Not logged in").to_owned()
    } else {
        session::label(spec)
    };
    ListItem::new(Line::from(vec![
        Span::styled(format!(" {current}"), GREEN),
        Span::raw(format!("{} ", icons.user)),
        Span::styled(label, FG),
    ]))
}

fn file_row<'a>(icons: &Icons, file: &'a LocalFile, kind: MediaKind, marked: bool) -> ListItem<'a> {
    let icon = match kind {
        MediaKind::Video => icons.sections[7],
        MediaKind::Music => icons.sections[8],
    };
    ListItem::new(Line::from(vec![
        Span::styled(if marked { "✗" } else { " " }, Style::new().fg(RED).bold()),
        Span::raw(format!("{icon} ")),
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
    let mut hints = if let Some(confirm) = &state.confirm {
        let yes = match confirm {
            Confirm::Unsubscribe { .. } => key("s", "sí, desuscribirme", "yes, unsubscribe"),
            Confirm::Trash(_) => key("s", "sí, a la papelera", "yes, to the trash"),
        };
        vec![yes, key(lang.pick("otra tecla", "other key"), "no", "no")]
    } else if state.help {
        vec![key(lang.pick("cualquier tecla", "any key"), "cerrar la ayuda", "close the help")]
    } else if state.prefix {
        return keymap::prefix_rows(lang);
    } else if let Some(prompt) = &state.prompt {
        match prompt {
            Prompt::Videos { .. } => rows(keymap::IN_SEARCH),
            _ => vec![key("Enter", "aceptar", "accept"), key("Esc", "cancelar", "cancel")],
        }
    } else if state.key_context().related {
        vec![
            key("Enter", "ver", "play"),
            key("↑ ↓", "elegir", "choose"),
            key("Tab", "volver al video", "back to the video"),
            key("l", "ver después", "watch later"),
            key("c", "canal", "channel"),
            key("espacio", "pausa", "pause"),
        ]
    } else if state.active_tab().is_some() {
        let mut keys = rows(keymap::IN_TAB);
        if state.related_shown().is_some() {
            keys.insert(0, key("Tab", "A continuación", "Up next"));
        }
        keys.extend(rows(keymap::ON_VIDEO));
        keys
    } else if state.focus == Focus::Menu || state.listing().is_none() {
        rows(keymap::IN_MENU)
    } else {
        let back = key("Esc", "volver", "back");
        match state.listing().map(|l| &l.items) {
            Some(Items::Videos(_)) if state.listing().is_some_and(|l| l.all.is_some()) => vec![
                key("Enter", "ver", "play"),
                if state.show_watched {
                    key("v", "ocultar vistos", "hide watched")
                } else {
                    key("v", "mostrar vistos", "show watched")
                },
                key("l", "ver después", "watch later"),
                key("s", "suscribirse", "subscribe"),
                key("d", "descargar", "download"),
                key("c", "canal", "channel"),
                key("/", "buscar", "search"),
                back,
            ],
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
            Some(Items::Files(_, kind)) => vec![
                match kind {
                    MediaKind::Music => key("Enter", "escuchar desde esta canción", "play from this song"),
                    MediaKind::Video => key("Enter", "ver", "play"),
                },
                key(lang.pick("espacio", "space"), "marcar", "mark"),
                key("x", "a la papelera", "to the trash"),
                back,
            ],
            Some(Items::About) => vec![back],
            Some(Items::System) => vec![key("Enter", "revisar de nuevo", "check again"), back],
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
        Some(help_area(screen, state.lang()))
    } else {
        state.confirm.as_ref().map(|_| centered(screen, 56, 5))
    }
}

/// The help screen's two balanced columns, so everything fits on a laptop screen.
type HelpSection = (&'static str, Vec<(String, &'static str)>);

/// The help's sections in two columns of about the same height.
fn help_columns(lang: Lang) -> (Vec<HelpSection>, Vec<HelpSection>) {
    let mut sections = keymap::help(lang);
    let sizes: Vec<usize> = sections.iter().map(|(_, rows)| help_box_height(rows.len()) as usize).collect();
    let split =
        (1..sections.len()).min_by_key(|&k| sizes[..k].iter().sum::<usize>().max(sizes[k..].iter().sum())).unwrap_or(1);
    let right = sections.split_off(split);
    (sections, right)
}

/// A section's box: its rows between two border lines.
fn help_box_height(rows: usize) -> u16 {
    rows as u16 + 2
}

fn help_column_height(sections: &[HelpSection]) -> u16 {
    sections.iter().map(|(_, rows)| help_box_height(rows.len())).sum()
}

/// The help's size: the taller column inside the outer border.
fn help_area(screen: Rect, lang: Lang) -> Rect {
    let (left, right) = help_columns(lang);
    centered(screen, 118, help_column_height(&left).max(help_column_height(&right)) + 2)
}

fn draw_help(frame: &mut Frame, lang: Lang) {
    let (left, right) = help_columns(lang);
    let area = help_area(frame.area(), lang);
    frame.render_widget(Clear, area);
    let title =
        lang.pick(" ⌨  Atajos de teclado · cualquier tecla cierra ", " ⌨  Keyboard shortcuts · any key closes ");
    let block = panel(title, MAGENTA);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let columns: [Rect; 2] = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).spacing(1).areas(inner);
    for (sections, column) in [left, right].iter().zip(columns) {
        let heights = sections.iter().map(|(_, rows)| Constraint::Length(help_box_height(rows.len())));
        for ((title, rows), cell) in sections.iter().zip(Layout::vertical(heights).split(column).iter()) {
            draw_help_section(frame, title, rows, *cell);
        }
    }
}

/// One section of the help in its own box: keys aligned, then what they do.
fn draw_help_section(frame: &mut Frame, title: &str, rows: &[(String, &'static str)], area: Rect) {
    let width = rows.iter().map(|(keys, _)| Span::raw(keys.as_str()).width()).max().unwrap_or(0) + 2;
    let lines: Vec<Line> = rows
        .iter()
        .map(|(keys, description)| {
            Line::from(vec![
                Span::styled(format!(" {}", pad_cells(keys, width)), Style::new().fg(CYAN).bold()),
                Span::styled(*description, FG),
            ])
        })
        .collect();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(LINE))
        .title(Line::from(format!(" {title} ")).fg(BLUE).bold());
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_confirm(frame: &mut Frame, confirm: &Confirm, lang: Lang) {
    let area = centered(frame.area(), 56, 5);
    let question = match confirm {
        Confirm::Unsubscribe { name, .. } => Line::from(vec![
            Span::raw(lang.pick("¿Desuscribirte de ", "Unsubscribe from ")),
            Span::styled(name.as_str(), Style::new().fg(MAGENTA).bold()),
            Span::raw("?"),
        ]),
        Confirm::Trash(paths) => {
            let what = match paths.as_slice() {
                [one] => one.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                many => format!("{} {}", many.len(), lang.pick("archivos", "files")),
            };
            Line::from(vec![
                Span::raw(lang.pick("¿Mandar a la papelera ", "Move to the trash ")),
                Span::styled(truncate(&what, 30), Style::new().fg(MAGENTA).bold()),
                Span::raw("?"),
            ])
        }
    };
    let lines = vec![
        Line::from(""),
        question,
        Line::from(lang.pick("s = sí · cualquier otra tecla = no", "y = yes · any other key = no")).fg(MUTED),
    ];
    let title = lang.pick(" Confirmar ", " Confirm ");
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(panel(title, RED)), area);
}

/// What the system has and lacks, with the command that installs it.
fn draw_system(frame: &mut Frame, state: &State, area: Rect) {
    let lang = state.lang();
    let title = format!(" {} ", lang.pick("Revisión del sistema", "System check"));
    let block = panel(title, BLUE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some((manager, checks)) = &state.system else {
        let wait = lang.pick("Revisando…", "Checking…");
        frame.render_widget(Paragraph::new(wait).fg(YELLOW).alignment(Alignment::Center), inner);
        return;
    };
    let mut lines = vec![Line::from("")];
    let width = checks.iter().map(|c| c.tool.program().len()).max().unwrap_or(0) + 2;
    for check in checks {
        let (mark, color, note) = match (&check.found, check.outdated) {
            (Some(_), false) => ("✓", GREEN, String::new()),
            (Some((keyring, _)), true) if check.tool == Tool::Keyring => {
                let missing = if keyring.starts_with("KWALLET") { "kwallet-query" } else { "secretstorage" };
                (
                    "⚠",
                    YELLOW,
                    format!(
                        "{}: {missing}",
                        lang.pick("yt-dlp no lo puede abrir, falta", "yt-dlp can't open it, missing")
                    ),
                )
            }
            (Some(_), true) => {
                ("⚠", YELLOW, lang.pick("viejo: YouTube cambia seguido", "too old: YouTube changes often").into())
            }
            (None, _) if check.tool.required() => ("✗", RED, lang.pick("falta · necesario", "missing · needed").into()),
            (None, _) => ("·", MUTED, lang.pick("no está · opcional", "not installed · optional").into()),
        };
        let version = check.found.as_ref().and_then(|(_, v)| v.clone()).unwrap_or_default();
        let mut row = vec![
            Span::styled(format!("  {mark} "), Style::new().fg(color).bold()),
            Span::styled(pad(check.tool.program(), width), Style::new().fg(FG).bold()),
            Span::styled(pad(&version, 16), MUTED),
            Span::styled(check.tool.purpose().get(lang), FG),
        ];
        if !note.is_empty() {
            row.push(Span::styled(format!("  ({note})"), color));
        }
        lines.push(Line::from(row));
        if let Some(fix) = &check.fix {
            lines.push(Line::from(vec![Span::raw(" ".repeat(width + 5)), Span::styled(fix.as_str(), CYAN)]));
        } else if check.found.is_none() {
            let how =
                lang.pick("instalalo con el gestor de paquetes de tu sistema", "install it with your package manager");
            lines.push(Line::from(vec![Span::raw(" ".repeat(width + 5)), Span::styled(how, MUTED)]));
        }
    }
    lines.push(Line::from(""));
    let footer = match manager {
        Manager::Unknown => lang.pick("Gestor de paquetes no reconocido", "Package manager not recognized").to_owned(),
        manager => format!("{}: {}", lang.pick("Comandos para", "Commands for"), manager.name()),
    };
    lines.push(
        Line::from(format!(
            "  {footer} · {}",
            lang.pick("Shift + arrastrar copia · Enter revisa de nuevo", "Shift + drag copies · Enter checks again",)
        ))
        .fg(MUTED),
    );
    frame.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }), inner);
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
    if let Some(update) = &state.update {
        lines.push(Line::from(""));
        lines.push(
            Line::from(format!(
                "⬆ {} {}",
                lang.pick("Hay una versión nueva:", "A new version is out:"),
                update.version
            ))
            .fg(GREEN)
            .bold(),
        );
        let (how, value) = match &update.how {
            Install::Command(command) => (lang.pick("Actualizá con:", "Update with:"), command),
            Install::Link(url) => (lang.pick("Bajá el instalador:", "Get the installer:"), url),
        };
        lines.push(Line::from(how).fg(FG));
        lines.push(Line::from(value.as_str()).fg(CYAN));
        lines.push(
            Line::from(lang.pick("(Shift + arrastrar para seleccionar y copiar)", "(Shift + drag to select and copy)"))
                .fg(MUTED),
        );
    }
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
    // Wrapped: the update command can be longer than the panel.
    let about = Paragraph::new(lines).alignment(Alignment::Center).wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(about.block(panel(title, PURPLE)), area);
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
        render_sized(state, view, 130)
    }

    fn render_sized(state: &State, view: &View, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
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
            handle: Some("@x".into()),
            description: Some("Música\ny charlas".into()),
            ..channel("UCx")
        };
        let mut s = with_items(Items::Channels(vec![followed, channel("UCy")]));
        s.subs.insert("UCx".into());
        let name = pad_cells("name UCx", CHANNEL_NAME_WIDTH);
        let row = format!("✓ {name}{}@x", pad_cells("358 mil suscriptores", FOLLOWERS_WIDTH));
        let wide = render(&s);
        let line = wide.lines().find(|l| l.contains("✓ name UCx")).unwrap();
        assert!(line.contains(&row), "{line}");
        assert!(!line.contains("Música"), "the panel shows it: {line}");
        let narrow = render_sized(&s, &VIEW, 110);
        assert!(narrow.contains(&format!("{row}  Música y charlas")), "{narrow}");
        assert!(wide.contains("   name UCy"));
    }

    #[test]
    fn profile_rows_mark_current_session() {
        let mut s = with_items(Items::Profiles(vec!["brave:Default".into(), String::new()]));
        s.config.session.browser = "brave:Default".into();
        let screen = render(&s);
        assert!(screen.contains("● ☺ brave · Default"), "{screen}");
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
        assert_eq!(hint_keys(&list), ["Enter", "v", "l", "s", "d", "c", "/", "Esc", "C-y ?"], "Inicio: a feed");
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
        assert!(screen.contains("brave · Default"), "{screen}");
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
        let info =
            Info { volume: Some(85.0), muted: false, height: Some(480), codec: Some("h264".into()), ..Info::default() };
        let screen = render_with(&s, &View { playing: Some(info.clone()), ..VIEW });
        assert!(screen.contains("♪ 85 %  480p  H.264"), "{screen}");
        let muted = Info { muted: true, codec: Some("vp9".into()), ..info };
        assert_eq!(playing_label(&muted, icons(IconSet::Unicode)), " ✕ 85 %  480p  VP9 ");
        assert_eq!(playing_label(&Info::default(), icons(IconSet::Unicode)), "");
    }

    #[test]
    fn video_tab_shows_a_progress_bar() {
        let mut s = with_items(Items::Videos(vec![video("a")]));
        s.update(Action::Enter);
        let info = Info { position: Some(30.0), duration: Some(120.0), buffered: Some(60.0), ..Info::default() };
        let screen = render_with(&s, &View { playing: Some(info.clone()), ..VIEW });
        let bottom = screen.lines().find(|l| l.contains("2:00")).unwrap_or_default();
        assert!(bottom.contains("▶ 0:30 ━"), "{screen}");
        let content = areas(Rect::new(0, 0, 130, 40), s.sidebar).content;
        let (bar, duration) = progress_bar(&info, content).unwrap();
        assert_eq!(duration, 120.0);
        let track = bottom.split("0:30 ").nth(1).and_then(|t| t.split(" 2:00").next()).unwrap();
        assert_eq!(track.chars().count(), bar.width as usize, "{bottom}");
        let thick = track.chars().filter(|c| *c == '━').count();
        assert_eq!(thick, (f64::from(bar.width) / 2.0).round() as usize, "a quarter played, another downloaded");
        let line = progress_line(&info, content.width).unwrap();
        let played = &line.spans[1];
        assert_eq!(played.style.fg, Some(RED));
        assert_eq!(played.content.chars().count(), (f64::from(bar.width) / 4.0).round() as usize);
        assert_eq!(line.spans[2].style.fg, Some(MUTED));
        assert_eq!(bar.y, content.bottom() - 1);
        let paused = progress_line(&Info { paused: true, ..info.clone() }, 60).unwrap();
        assert!(paused.to_string().starts_with(" ‖ 0:30 "));
        assert!(progress_line(&Info { duration: None, ..info }, 60).is_none(), "live: no total");
    }

    /// The first cell whose row contains `text`, at the start of `text`.
    fn find(screen: &str, text: &str) -> (u16, u16) {
        let (y, line) = screen.lines().enumerate().find(|(_, l)| l.contains(text)).unwrap();
        let x = line[..line.find(text).unwrap()].chars().count();
        (x as u16, y as u16)
    }

    #[test]
    fn spots_under_the_pointer() {
        let screen_area = Rect::new(0, 0, 130, 40);
        let videos: Vec<Video> = (0..60).map(|i| video(&i.to_string())).collect();
        let mut s = with_items(Items::Videos(videos));
        let screen = render(&s);
        let spot_at = |s: &State, (x, y): (u16, u16)| spot(s, None, screen_area, x, y);
        let musica = Section::ALL.iter().position(|x| *x == Section::MiMusica).unwrap();
        assert_eq!(spot_at(&s, find(&screen, Section::MiMusica.label(Lang::Es))), Some(Spot::Section(musica)));
        assert_eq!(spot_at(&s, find(&screen, "title 2  1:00")), Some(Spot::Row(2)));
        assert_eq!(spot_at(&s, (0, 39)), None, "the key hints");
        // Scrolled: the selection is the last visible row.
        for _ in 0..50 {
            s.update(Action::Down);
        }
        let screen = render(&s);
        assert_eq!(spot_at(&s, find(&screen, "title 50  1:00")), Some(Spot::Row(50)));
        assert_eq!(spot_at(&s, find(&screen, "title 30  1:00")), Some(Spot::Row(30)));
        s.update(Action::Enter); // plays video 50 in tab 1
        let screen = render(&s);
        assert_eq!(spot_at(&s, find(&screen, "1 ▶ title 50")), Some(Spot::Tab(1)));
        assert_eq!(spot_at(&s, find(&screen, "Inicio ")), Some(Spot::Tab(0)));
        assert_eq!(spot_at(&s, (80, 20)), Some(Spot::Video));
        let info = Info { position: Some(0.0), duration: Some(100.0), ..Info::default() };
        let content = areas(screen_area, s.sidebar).content;
        let (bar, _) = progress_bar(&info, content).unwrap();
        assert_eq!(spot(&s, Some(&info), screen_area, bar.x, bar.y), Some(Spot::Progress(0)));
        let middle = spot(&s, Some(&info), screen_area, bar.x + bar.width / 2, bar.y);
        assert!(matches!(middle, Some(Spot::Progress(49..=51))), "{middle:?}");
        s.update(Action::ToggleHelp);
        assert_eq!(spot_at(&s, (80, 20)), None, "the help covers everything");
    }

    #[test]
    fn video_tab_in_window_mode_shows_details_and_related() {
        let mut s = with_items(Items::Videos(vec![video("a")]));
        s.video_window = true;
        s.update(Action::Enter);
        let tab = s.active_tab().unwrap().id;
        let screen = render_with(&s, &View { embedded: false, ..VIEW });
        assert!(screen.contains("Buscando videos relacionados"), "{screen}");
        let watch = Watch {
            views: Some(5771),
            likes: Some(739),
            published: None,
            subscribers: Some("559K".into()),
            related: vec![video("r1"), video("r2")],
        };
        s.update(Action::Watched(tab, Ok(watch)));
        let view = View { embedded: false, ..VIEW };
        let screen = render_with(&s, &view);
        for text in ["title a", "chan a · 559K suscriptores", "5 k vistas", "A continuación · 2 · Tab", "title r2"]
        {
            assert!(screen.contains(text), "{text}\n{screen}");
        }
        assert!(!screen.contains("ventana de mpv"));
        let screen_area = Rect::new(0, 0, 130, 40);
        let (id, cells) = thumbnail_area(&s, screen_area).unwrap();
        assert_eq!(id, "a");
        assert_eq!((cells.width, cells.height), (34, 34 * 9 / 32), "a third of the tab, at most 36 columns");
        let (x, y) = find(&screen, "title r2  1:00");
        assert_eq!(spot(&s, None, screen_area, x, y), Some(Spot::Row(1)));
        assert_eq!(hints(&s)[0].0, "Tab");
        s.update(Action::ToggleFocus);
        let screen = render_with(&s, &view);
        assert!(screen.contains("A continuación · 2 "), "{screen}");
        assert_eq!(hints(&s)[0].1, "ver");
        s.video_window = false;
        assert_eq!(thumbnail_area(&s, screen_area), None, "embedded: the video covers the tab");
    }

    #[test]
    fn a_new_version_is_announced_with_how_to_get_it() {
        let mut s = state();
        let update =
            crate::update::Update { version: "9.9.9".into(), how: Install::Command("cargo install tubetui".into()) };
        s.update(Action::UpdateFound(update));
        assert!(s.status.contains("⬆ tubetui 9.9.9 disponible"), "{}", s.status);
        let screen = render(&s);
        assert!(screen.contains("Acerca de ⬆"), "{screen}");
        let about = Section::ALL.iter().position(|x| *x == Section::AcercaDe).unwrap();
        s.update(Action::OpenSection(about));
        let screen = render(&s);
        assert!(screen.contains("Hay una versión nueva: 9.9.9"), "{screen}");
        assert!(screen.contains("cargo install tubetui"));
    }

    #[test]
    fn feeds_say_how_many_watched_videos_they_hide() {
        let mut s = state();
        s.watched.insert("b".into());
        let id = match s.update(Action::Enter).as_slice() {
            [Effect::Load { id, .. }, ..] => *id,
            other => panic!("{other:?}"),
        };
        s.update(Action::Loaded(id, Ok(Items::Videos(vec![video("a"), video("b")]))));
        let screen = render(&s);
        assert!(screen.contains("Inicio · 1 · ◉ 1 vistos ocultos"), "{screen}");
        assert!(hints(&s).iter().any(|(k, d)| k == "v" && *d == "mostrar vistos"));
        s.update(Action::ToggleWatched);
        let screen = render(&s);
        assert!(screen.contains("◉ con los vistos"), "{screen}");
        assert!(screen.contains("◉ title b"), "marked: {screen}");
    }

    #[test]
    fn system_check_page_says_what_to_install() {
        use crate::doctor::Check;
        let mut s = state();
        let missing = Check { tool: Tool::Mpv, found: None, outdated: false, fix: Some("sudo apt install mpv".into()) };
        let old = Check {
            tool: Tool::YtDlp,
            found: Some(("/usr/bin/yt-dlp".into(), Some("2024.04.09".into()))),
            outdated: true,
            fix: Some("sudo apt install pipx && pipx install yt-dlp".into()),
        };
        let fine = Check {
            tool: Tool::Kitty,
            found: Some(("/usr/bin/kitty".into(), Some("0.39.1".into()))),
            outdated: false,
            fix: None,
        };
        let keyring = Check {
            tool: Tool::Keyring,
            found: Some(("GNOMEKEYRING".into(), Some("GNOMEKEYRING".into()))),
            outdated: true,
            fix: Some("pipx inject yt-dlp secretstorage".into()),
        };
        s.update(Action::SystemChecked(Manager::Apt, vec![missing, old, fine, keyring]));
        let screen = render(&s);
        for text in [
            "✗ mpv",
            "falta · necesario",
            "sudo apt install mpv",
            "⚠ yt-dlp",
            "2024.04.09",
            "pipx install yt-dlp",
            "✓ kitty",
            "⚠ keyring",
            "falta: secretstorage",
            "pipx inject yt-dlp secretstorage",
            "Comandos para: apt",
        ] {
            assert!(screen.contains(text), "{text}\n{screen}");
        }
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
