//! Rendering. `areas` is the single source of truth for the layout, so the
//! runtime places the embedded video exactly where `draw` leaves room for it.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Tabs};

use crate::app::{Focus, Listing, Prompt, Section, Setting, State, language_label};
use crate::config::IconSet;
use crate::domain::{Channel, Live, Video, date_labels};
use crate::keymap;
use crate::youtube::Items;

pub const SIDEBAR_WIDTH: u16 = 26;
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
const SECTION_COLORS: [Color; 9] = [BLUE, CYAN, RED, YELLOW, ORANGE, GREEN, TEAL, PURPLE, MUTED];

pub struct Icons {
    /// In `Section::ALL` order.
    pub sections: [&'static str; 9],
    /// In `Setting::ALL` order.
    pub settings: [&'static str; 12],
    pub logo: &'static str,
    pub tab: &'static str,
    pub live: &'static str,
    pub upcoming: &'static str,
    pub later: &'static str,
    pub subscribed: &'static str,
    pub user: &'static str,
    pub language: &'static str,
}

const EMOJI: Icons = Icons {
    sections: ["🏠", "🧭", "📺", "🕒", "⭐", "🔎", "📡", "🔧", "🚪"],
    settings: ["👤", "📄", "🔄", "🚪", "🌐", "💬", "📐", "🎬", "🎧", "😀", "📁", "📝"],
    logo: "▶️",
    tab: "🎬",
    live: "🔴",
    upcoming: "⏳",
    later: "🕒",
    subscribed: "✓",
    user: "👤",
    language: "🌐",
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
        "\u{f013}",
        "\u{f08b}",
    ],
    settings: [
        "\u{f007}", "\u{f15b}", "\u{f021}", "\u{f08b}", "\u{f1ab}", "\u{f075}", "\u{f1de}", "\u{f008}", "\u{f025}",
        "\u{f118}", "\u{f07b}", "\u{f044}",
    ],
    logo: "\u{f16a}",
    tab: "\u{f144}",
    live: "\u{f111}",
    upcoming: "\u{f252}",
    later: "\u{f017}",
    subscribed: "\u{f00c}",
    user: "\u{f007}",
    language: "\u{f1ab}",
};

const UNICODE: Icons = Icons {
    sections: ["⌂", "◎", "▣", "◷", "★", "⌕", "⌁", "⚙", "⏻"],
    settings: ["☺", "▤", "↻", "⏻", "⇄", "≡", "▦", "▶", "♪", "☻", "▭", "✎"],
    logo: "▶",
    tab: "▶",
    live: "●",
    upcoming: "…",
    later: "◷",
    subscribed: "✓",
    user: "☺",
    language: "⇄",
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
    state.active_tab()?;
    Some(Block::bordered().inner(areas(area, state.sidebar).content))
}

pub fn draw(frame: &mut Frame, state: &State, view: &View) {
    let icons = icons(state.config.ui.icons);
    let areas = areas(frame.area(), state.sidebar);
    if let Some(side) = areas.sidebar {
        draw_sidebar(frame, state, icons, side);
    }
    draw_tabs(frame, state, icons, areas.tabs);
    match state.active_tab() {
        None => draw_inicio(frame, state, icons, view, areas.content),
        Some(tab) => {
            let title = Line::from(vec![
                Span::raw(format!(" {} ", icons.tab)),
                Span::styled(tab.video.title.clone(), Style::new().fg(FG).bold()),
                Span::styled(format!(" · {} ", tab.video.channel), MAGENTA),
            ]);
            let note = if view.embedded { "" } else { "Reproduciendo en una ventana de mpv" };
            frame.render_widget(Paragraph::new(note).fg(MUTED).block(panel(title, BLUE)), areas.content);
        }
    }
    draw_status(frame, state, icons, areas.status);
    draw_hints(frame, &hints(state), areas.hints);
    if state.help {
        draw_help(frame);
    }
    if let Some(u) = &state.confirm {
        draw_confirm(frame, &u.name);
    }
}

fn panel<'a>(title: impl Into<Line<'a>>, color: Color) -> Block<'a> {
    Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(color)).title(title)
}

fn draw_sidebar(frame: &mut Frame, state: &State, icons: &Icons, area: Rect) {
    let on_inicio = state.active == 0;
    let mut items: Vec<ListItem> = Section::ALL
        .iter()
        .enumerate()
        .map(|(i, section)| {
            let mut label = section.label().to_owned();
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
            ListItem::new(Line::from(vec![
                Span::styled(if focused { "▌" } else { " " }, color),
                Span::raw(format!("{} ", icons.sections[i])),
                Span::raw(label),
            ]))
            .style(style)
        })
        .collect();
    if !state.tabs.is_empty() {
        items.push(ListItem::new(""));
        items.push(ListItem::new(Line::from(" PESTAÑAS").fg(MUTED).bold()));
        for (i, tab) in state.tabs.iter().enumerate() {
            let active = state.active == i + 1;
            let style = if active { Style::new().fg(MAGENTA).bg(HIGHLIGHT).bold() } else { Style::new().fg(FG) };
            let line = format!("{}{} {} {}", if active { "▌" } else { " " }, i + 1, icons.tab, tab.video.title);
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
    let titles = std::iter::once(format!("{} Inicio", icons.sections[0])).chain(
        state.tabs.iter().enumerate().map(|(i, t)| format!("{} {} {}", i + 1, icons.tab, truncate(&t.video.title, 20))),
    );
    let tabs = Tabs::new(titles)
        .select(state.active)
        .style(Style::new().fg(MUTED))
        .highlight_style(Style::new().fg(BLUE).add_modifier(Modifier::BOLD));
    frame.render_widget(tabs, area);
}

fn draw_inicio(frame: &mut Frame, state: &State, icons: &Icons, view: &View, area: Rect) {
    let area = match &state.prompt {
        Some(prompt) => {
            let [input, rest] = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(area);
            let title = match prompt {
                Prompt::Videos { scope, .. } => Line::from(vec![
                    Span::raw(format!(" {} Buscar videos en ", icons.sections[5])),
                    Span::styled(scope.label(), Style::new().fg(YELLOW).bold()),
                    Span::raw(" · Tab cambia ").fg(MUTED),
                ]),
                Prompt::Channels => Line::from(format!(" {} Buscar canales ", icons.sections[6])),
                Prompt::CookiesFile => Line::from(format!(" {} Ruta del cookies.txt ", icons.settings[1])),
                Prompt::DownloadDir => Line::from(format!(" {} Carpeta de descargas ", icons.settings[10])),
            };
            frame.render_widget(Paragraph::new(state.query.as_str()).block(panel(title, CYAN)), input);
            let x = input.x + 1 + state.query.chars().count() as u16;
            frame.set_cursor_position(Position::new(x.min(input.right().saturating_sub(2)), input.y + 1));
            if *prompt == Prompt::CookiesFile {
                draw_cookies_help(frame, rest);
                return;
            }
            rest
        }
        None => area,
    };
    match state.listing() {
        Some(listing) => draw_listing(frame, state, icons, view, listing, area),
        None => draw_welcome(frame, state, icons, area),
    }
}

fn draw_cookies_help(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from("Para usar tu cuenta en una máquina sin navegador (servidor, SSH):").fg(FG),
        Line::from(""),
        Line::from("  1. En cualquier PC con navegador, entrá a youtube.com con tu cuenta.").fg(FG),
        Line::from("  2. Exportá las cookies en formato Netscape (cookies.txt), por ejemplo con").fg(FG),
        Line::from("     la extensión «Get cookies.txt LOCALLY», o con:").fg(FG),
        Line::from("     yt-dlp --cookies-from-browser firefox --cookies cookies.txt --skip-download :ytsubs").fg(CYAN),
        Line::from("  3. Copialo a esta máquina (scp cookies.txt servidor:~/) y escribí la ruta arriba.").fg(FG),
        Line::from(""),
        Line::from("tubetui comprueba que tenga una sesión de YouTube y guarda una copia privada (permisos 600).")
            .fg(MUTED),
        Line::from("Las cookies dan acceso a tu cuenta: no las compartas.").fg(YELLOW),
    ];
    frame.render_widget(Paragraph::new(lines).block(panel(" Importar sesión ", MUTED)), area);
}

fn draw_welcome(frame: &mut Frame, state: &State, icons: &Icons, area: Rect) {
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw(format!("{}  ", icons.logo)),
            Span::styled("tube", Style::new().fg(RED).bold()),
            Span::styled("tui", Style::new().fg(FG).bold()),
        ]),
        Line::from("YouTube en tu terminal").fg(MUTED),
        Line::from(""),
        Line::from("Elegí una sección del menú con ↑ ↓ y Enter").fg(FG),
        Line::from("Abajo siempre ves las teclas que sirven en cada momento").fg(MUTED),
        Line::from("Ctrl+y ?  muestra todos los atajos explicados").fg(MUTED),
    ];
    if let Some(title) = &state.loading {
        lines.push(Line::from(""));
        lines.push(Line::from(format!("Cargando {title}…")).fg(YELLOW));
    }
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(panel("", MUTED)), area);
}

fn draw_listing(frame: &mut Frame, state: &State, icons: &Icons, view: &View, listing: &Listing, area: Rect) {
    let breadcrumb: Vec<&str> = state.stack.iter().map(|l| l.title.as_str()).collect();
    let title = match &state.loading {
        Some(loading) => format!(" Cargando {loading}… "),
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
    };
    let focused = state.focus == Focus::List;
    let highlight = if focused { Style::new().bg(HIGHLIGHT).add_modifier(Modifier::BOLD) } else { Style::new() };
    let list = List::new(items)
        .block(panel(title, if focused { BLUE } else { MUTED }))
        .highlight_symbol(if focused { "▌" } else { " " })
        .highlight_style(highlight);
    let selected = (!listing.items.is_empty()).then_some(listing.selected);
    frame.render_stateful_widget(list, area, &mut ListState::default().with_selected(selected));
}

fn video_row<'a>(state: &State, icons: &Icons, view: &View, v: &'a Video) -> ListItem<'a> {
    let mut spans = match (v.live, v.published) {
        (Live::Now, _) => vec![Span::styled(pad_cells(&format!("{} en vivo", icons.live), DATE_WIDTH), RED)],
        (Live::Upcoming, _) => {
            vec![Span::styled(pad_cells(&format!("{} próximo", icons.upcoming), DATE_WIDTH), YELLOW)]
        }
        (Live::No, Some(ts)) => {
            let (date, age) = date_labels(ts, view.now);
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
    ListItem::new(Line::from(spans))
}

fn profile_row<'a>(state: &State, icons: &Icons, spec: &'a str) -> ListItem<'a> {
    let current = if spec == state.config.session.browser && state.config.session.cookies_file.is_empty() {
        "● "
    } else {
        "  "
    };
    let label = if spec.is_empty() { "Sin sesión" } else { spec };
    ListItem::new(Line::from(vec![
        Span::styled(format!(" {current}"), GREEN),
        Span::raw(format!("{} ", icons.user)),
        Span::styled(label, FG),
    ]))
}

fn setting_row(state: &State, icon: &str, setting: Setting) -> ListItem<'static> {
    let value = state.setting_value(setting);
    let mut spans = vec![Span::raw(format!(" {icon} ")), Span::styled(pad_cells(setting.label(), 44), FG)];
    if !value.is_empty() {
        spans.push(Span::styled(value, Style::new().fg(YELLOW).bold()));
    }
    ListItem::new(Line::from(spans))
}

fn draw_status(frame: &mut Frame, state: &State, icons: &Icons, area: Rect) {
    let right = format!(
        "{} {}  {} {} ",
        icons.user,
        state.session_label(),
        icons.language,
        language_label(state.config.player.language)
    );
    let right_width = Span::raw(right.as_str()).width() as u16 + 1;
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(right_width)]).areas(area);
    frame.render_widget(Paragraph::new(format!(" {}", state.status)).fg(FG), left_area);
    frame.render_widget(Paragraph::new(right).fg(MUTED).alignment(Alignment::Right), right_area);
}

/// Keys that work right now, each with what it does.
pub fn hints(state: &State) -> Vec<(String, &'static str)> {
    let rows = |table: &[(&str, &'static str)]| table.iter().map(|(k, d)| (k.to_string(), *d)).collect::<Vec<_>>();
    let all_keys = ("C-y ?".to_string(), "todos los atajos");
    let mut hints = if state.confirm.is_some() {
        vec![("s".into(), "sí, desuscribirme"), ("otra tecla".into(), "no")]
    } else if state.help {
        vec![("cualquier tecla".into(), "cerrar la ayuda")]
    } else if state.prefix {
        let mut commands: Vec<_> = keymap::PREFIX.iter().map(|(k, _, d)| (k.to_string(), *d)).collect();
        commands.insert(4, ("0-9".into(), "ir a la pestaña"));
        return commands;
    } else if let Some(prompt) = &state.prompt {
        match prompt {
            Prompt::Videos { .. } => rows(keymap::IN_SEARCH),
            _ => vec![("Enter".into(), "aceptar"), ("Esc".into(), "cancelar")],
        }
    } else if state.active_tab().is_some() {
        let mut keys = rows(keymap::IN_TAB);
        keys.extend(rows(keymap::ON_VIDEO));
        keys
    } else if state.focus == Focus::Menu || state.listing().is_none() {
        rows(keymap::IN_MENU)
    } else {
        let back = ("Esc".to_string(), "volver");
        match state.listing().map(|l| &l.items) {
            Some(Items::Videos(_)) => vec![
                ("Enter".into(), "ver"),
                ("l".into(), "ver después"),
                ("s".into(), "suscribirse"),
                ("d".into(), "descargar"),
                ("c".into(), "canal"),
                ("/".into(), "buscar"),
                back,
            ],
            Some(Items::Channels(_)) => {
                vec![("Enter".into(), "ver sus videos"), ("s".into(), "suscribirse / desuscribirse"), back]
            }
            Some(Items::Profiles(_)) => vec![("Enter".into(), "usar esta sesión"), back],
            _ => vec![("Enter".into(), "cambiar / abrir"), back],
        }
    };
    hints.push(all_keys);
    hints
}

fn draw_hints(frame: &mut Frame, hints: &[(String, &str)], area: Rect) {
    let mut spans = vec![Span::raw(" ")];
    for (key, description) in hints {
        spans.push(Span::styled(format!(" {key} "), Style::new().fg(Color::Black).bg(BLUE).bold()));
        spans.push(Span::styled(format!(" {description}  "), MUTED));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_help(frame: &mut Frame) {
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
    let mut sections = keymap::help();
    // Two balanced columns so everything fits on a laptop screen.
    let sizes: Vec<usize> = sections.iter().map(|(_, rows)| rows.len() + 2).collect();
    let split =
        (1..sections.len()).min_by_key(|&k| sizes[..k].iter().sum::<usize>().max(sizes[k..].iter().sum())).unwrap_or(1);
    let right = sections.split_off(split);
    let (left, right) = (lines_of(&sections), lines_of(&right));
    let height = left.len().max(right.len()) as u16 + 3;
    let area = centered(frame.area(), 118, height);
    frame.render_widget(Clear, area);
    let block = panel(" ⌨  Atajos de teclado · cualquier tecla cierra ", MAGENTA);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [l, r] = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).spacing(2).areas(inner);
    frame.render_widget(Paragraph::new(left), l);
    frame.render_widget(Paragraph::new(right), r);
}

fn draw_confirm(frame: &mut Frame, name: &str) {
    let area = centered(frame.area(), 56, 5);
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("¿Desuscribirte de "),
            Span::styled(name, Style::new().fg(MAGENTA).bold()),
            Span::raw("?"),
        ]),
        Line::from("s = sí · cualquier otra tecla = no").fg(MUTED),
    ];
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(panel(" Confirmar ", RED)), area);
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

    const VIEW: View = View { embedded: true, now: 1_791_590_400 };

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
            assert!(screen.contains(&format!("{icon} {}", section.label())), "{section:?}\n{screen}");
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
        let followed = Channel { followers: Some("358 mil suscriptores".into()), ..channel("UCx") };
        let mut s = with_items(Items::Channels(vec![followed, channel("UCy")]));
        s.subs.insert("UCx".into());
        let screen = render(&s);
        assert!(screen.contains("✓ name UCx  358 mil suscriptores"), "{screen}");
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
            assert!(screen.contains(setting.label()), "{setting:?}\n{screen}");
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
        for (title, rows) in keymap::help() {
            assert!(screen.contains(title), "{title}\n{screen}");
            for (_, description) in rows {
                assert!(screen.contains(description), "{description}\n{screen}");
            }
        }
    }

    #[test]
    fn hints_follow_the_context() {
        let mut s = state();
        assert_eq!(hint_keys(&s), ["↑ ↓", "Enter", "/", "C-y ?"]);
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
        assert!(screen.contains(" l  ver después"), "{screen}");
        assert!(screen.contains(" C-y ?  todos los atajos"));
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
    fn helpers() {
        assert_eq!(truncate("ñandú", 3), "ñan…");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(pad("abcdef", 4), "abcd");
        assert_eq!(pad_cells("🔴 en vivo", 14), "🔴 en vivo    ");
        assert_eq!(pad_cells("● en vivo", 14), "● en vivo     ");
    }
}
