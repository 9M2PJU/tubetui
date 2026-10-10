//! Keys → actions. Global commands live behind the `Ctrl+y` prefix
//! (tmux style: the prefix waits for exactly one more key); plain keys go to
//! whatever has focus, so the video tab can use space and arrows freely.
//! The help screen is generated from the same tables.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::Action;

/// The slice of state that decides how a key is read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyContext {
    /// `Ctrl+y` was pressed and the next key is a command.
    pub prefix: bool,
    pub help: bool,
    /// A yes/no question is open.
    pub confirm: bool,
    /// The search box has focus.
    pub typing: bool,
    /// A video tab is active.
    pub in_tab: bool,
}

pub const SEEK_SECONDS: i32 = 5;
pub const VOLUME_STEP: i32 = 5;

/// `Ctrl+y` commands. Digits (`0`–`9`, go to tab) are handled apart.
pub const PREFIX: &[(char, Action, &str)] = &[
    ('b', Action::ToggleSidebar, "mostrar / ocultar el menú"),
    ('n', Action::NextTab, "pestaña siguiente"),
    ('p', Action::PrevTab, "pestaña anterior"),
    ('x', Action::CloseTab, "cerrar la pestaña"),
    ('/', Action::FocusSearch, "buscar videos"),
    ('?', Action::ToggleHelp, "esta ayuda"),
    ('q', Action::Quit, "salir"),
];

/// Keys that act on the selected video, in lists and in video tabs.
pub const ON_VIDEO: &[(&str, &str)] = &[
    ("l", "agregar / quitar de Ver después"),
    ("s", "suscribirse / desuscribirse del canal"),
    ("d", "descargar (con subtítulos)"),
    ("c", "ver los videos del canal"),
];

pub const IN_MENU: &[(&str, &str)] = &[("↑ ↓", "elegir sección"), ("Enter", "abrir"), ("/", "buscar videos")];

pub const IN_LISTS: &[(&str, &str)] = &[
    ("↑ ↓  j k", "moverse"),
    ("Enter", "ver video · abrir canal · cambiar opción"),
    ("Esc  ←  h", "volver"),
    ("/", "buscar (dentro de un canal: en ese canal)"),
];

pub const IN_SEARCH: &[(&str, &str)] = &[
    ("Enter", "buscar / aceptar"),
    ("Tab", "dónde buscar: todo / mis canales / canal"),
    ("Esc", "cancelar"),
];

pub const IN_TAB: &[(&str, &str)] = &[
    ("espacio", "pausa"),
    ("← →", "retroceder / avanzar 5 s"),
    ("↑ ↓", "subir / bajar volumen"),
    ("m", "silenciar"),
    ("j", "subtítulos sí / no"),
];

pub fn map(key: KeyEvent, ctx: KeyContext) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    if ctx.help {
        return Some(Action::ToggleHelp);
    }
    if ctx.confirm {
        return Some(Action::Confirm(matches!(key.code, KeyCode::Char('s' | 'S' | 'y' | 'Y'))));
    }
    if ctrl {
        return (key.code == KeyCode::Char('y')).then_some(Action::Prefix);
    }
    if ctx.prefix {
        return Some(prefixed(key.code));
    }
    if ctx.typing {
        return typing(key.code);
    }
    match key.code {
        KeyCode::Char('l') => return Some(Action::ToggleLater),
        KeyCode::Char('s') => return Some(Action::ToggleSubscription),
        KeyCode::Char('d') => return Some(Action::Download),
        KeyCode::Char('c') => return Some(Action::OpenChannel),
        _ => {}
    }
    if ctx.in_tab { player(key.code) } else { lists(key.code) }
}

fn prefixed(code: KeyCode) -> Action {
    match code {
        KeyCode::Char(c @ '0'..='9') => Action::GoTab(c as usize - '0' as usize),
        KeyCode::Char(c) => {
            PREFIX.iter().find(|(key, ..)| *key == c).map_or(Action::CancelPrefix, |(_, action, _)| action.clone())
        }
        _ => Action::CancelPrefix,
    }
}

fn typing(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Char(c) => Some(Action::Input(c)),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Enter => Some(Action::Enter),
        KeyCode::Esc => Some(Action::Back),
        KeyCode::Tab => Some(Action::CycleScope),
        _ => None,
    }
}

fn player(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Char(' ') => Some(Action::TogglePause),
        KeyCode::Left => Some(Action::Seek(-SEEK_SECONDS)),
        KeyCode::Right => Some(Action::Seek(SEEK_SECONDS)),
        KeyCode::Up => Some(Action::Volume(VOLUME_STEP)),
        KeyCode::Down => Some(Action::Volume(-VOLUME_STEP)),
        KeyCode::Char('m') => Some(Action::ToggleMute),
        KeyCode::Char('j') => Some(Action::CycleSubtitles),
        _ => None,
    }
}

fn lists(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Char('j') | KeyCode::Down => Some(Action::Down),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::Up),
        KeyCode::Enter => Some(Action::Enter),
        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => Some(Action::Back),
        KeyCode::Char('/') => Some(Action::FocusSearch),
        _ => None,
    }
}

/// Sections of the help screen: title and `(keys, description)` rows.
pub fn help() -> Vec<(&'static str, Vec<(String, &'static str)>)> {
    let rows = |table: &[(&str, &'static str)]| table.iter().map(|(k, d)| (k.to_string(), *d)).collect();
    let mut commands: Vec<(String, &'static str)> = PREFIX.iter().map(|(k, _, d)| (k.to_string(), *d)).collect();
    commands.insert(4, ("0-9".into(), "ir a la pestaña (0 = Inicio)"));
    vec![
        ("Comandos: Ctrl+y y después una tecla", commands),
        ("Menú", rows(IN_MENU)),
        ("Listas", rows(IN_LISTS)),
        ("Sobre un video (lista o pestaña)", rows(ON_VIDEO)),
        ("Pestaña de video", rows(IN_TAB)),
        ("Cuadro de búsqueda", rows(IN_SEARCH)),
        ("Siempre", vec![("Ctrl+c".into(), "salir")]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEventState;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn char(c: char) -> KeyEvent {
        key(KeyCode::Char(c))
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    const LISTS: KeyContext = KeyContext { prefix: false, help: false, confirm: false, typing: false, in_tab: false };
    const TYPING: KeyContext = KeyContext { typing: true, ..LISTS };
    const TAB: KeyContext = KeyContext { in_tab: true, ..LISTS };
    const PREFIXED: KeyContext = KeyContext { prefix: true, ..LISTS };
    const HELP: KeyContext = KeyContext { help: true, ..LISTS };
    const CONFIRM: KeyContext = KeyContext { confirm: true, ..LISTS };

    #[test]
    fn ctrl_y_starts_prefix_everywhere_but_dialogs() {
        for ctx in [LISTS, TYPING, TAB] {
            assert_eq!(map(ctrl('y'), ctx), Some(Action::Prefix), "{ctx:?}");
        }
        assert_eq!(map(ctrl('y'), HELP), Some(Action::ToggleHelp));
    }

    #[test]
    fn prefix_commands_come_from_the_table() {
        for (c, action, _) in PREFIX {
            assert_eq!(map(char(*c), PREFIXED).as_ref(), Some(action), "{c}");
        }
        assert_eq!(map(char('0'), PREFIXED), Some(Action::GoTab(0)));
        assert_eq!(map(char('3'), PREFIXED), Some(Action::GoTab(3)));
        assert_eq!(map(char('?'), PREFIXED), Some(Action::ToggleHelp));
    }

    #[test]
    fn unknown_key_after_prefix_cancels_it() {
        assert_eq!(map(char('z'), PREFIXED), Some(Action::CancelPrefix));
        assert_eq!(map(key(KeyCode::Esc), PREFIXED), Some(Action::CancelPrefix));
    }

    #[test]
    fn prefix_wins_over_typing() {
        assert_eq!(map(char('b'), KeyContext { typing: true, ..PREFIXED }), Some(Action::ToggleSidebar));
    }

    #[test]
    fn any_key_closes_help() {
        assert_eq!(map(char('x'), HELP), Some(Action::ToggleHelp));
        assert_eq!(map(key(KeyCode::Esc), HELP), Some(Action::ToggleHelp));
    }

    #[test]
    fn confirm_only_accepts_yes() {
        assert_eq!(map(char('s'), CONFIRM), Some(Action::Confirm(true)));
        assert_eq!(map(char('y'), CONFIRM), Some(Action::Confirm(true)));
        assert_eq!(map(char('n'), CONFIRM), Some(Action::Confirm(false)));
        assert_eq!(map(key(KeyCode::Enter), CONFIRM), Some(Action::Confirm(false)));
    }

    #[test]
    fn typing_edits_the_query() {
        assert_eq!(map(char('l'), TYPING), Some(Action::Input('l')));
        assert_eq!(map(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT), TYPING), Some(Action::Input('A')));
        assert_eq!(map(key(KeyCode::Backspace), TYPING), Some(Action::Backspace));
        assert_eq!(map(key(KeyCode::Enter), TYPING), Some(Action::Enter));
        assert_eq!(map(key(KeyCode::Esc), TYPING), Some(Action::Back));
        assert_eq!(map(key(KeyCode::Tab), TYPING), Some(Action::CycleScope));
    }

    #[test]
    fn list_navigation() {
        assert_eq!(map(char('j'), LISTS), Some(Action::Down));
        assert_eq!(map(key(KeyCode::Down), LISTS), Some(Action::Down));
        assert_eq!(map(char('k'), LISTS), Some(Action::Up));
        assert_eq!(map(key(KeyCode::Enter), LISTS), Some(Action::Enter));
        assert_eq!(map(key(KeyCode::Esc), LISTS), Some(Action::Back));
        assert_eq!(map(char('h'), LISTS), Some(Action::Back));
        assert_eq!(map(char('/'), LISTS), Some(Action::FocusSearch));
    }

    #[test]
    fn video_keys_work_in_lists_and_tabs() {
        for ctx in [LISTS, TAB] {
            assert_eq!(map(char('l'), ctx), Some(Action::ToggleLater));
            assert_eq!(map(char('s'), ctx), Some(Action::ToggleSubscription));
            assert_eq!(map(char('d'), ctx), Some(Action::Download));
            assert_eq!(map(char('c'), ctx), Some(Action::OpenChannel));
        }
    }

    #[test]
    fn player_controls_in_tab() {
        assert_eq!(map(char(' '), TAB), Some(Action::TogglePause));
        assert_eq!(map(key(KeyCode::Left), TAB), Some(Action::Seek(-SEEK_SECONDS)));
        assert_eq!(map(key(KeyCode::Right), TAB), Some(Action::Seek(SEEK_SECONDS)));
        assert_eq!(map(key(KeyCode::Up), TAB), Some(Action::Volume(VOLUME_STEP)));
        assert_eq!(map(key(KeyCode::Down), TAB), Some(Action::Volume(-VOLUME_STEP)));
        assert_eq!(map(char('m'), TAB), Some(Action::ToggleMute));
        assert_eq!(map(char('j'), TAB), Some(Action::CycleSubtitles));
        assert_eq!(map(key(KeyCode::Enter), TAB), None);
    }

    #[test]
    fn plain_q_does_not_quit_but_ctrl_c_always_does() {
        assert_eq!(map(char('q'), LISTS), None);
        for ctx in [LISTS, TYPING, TAB, PREFIXED, HELP, CONFIRM] {
            assert_eq!(map(ctrl('c'), ctx), Some(Action::Quit), "{ctx:?}");
        }
    }

    #[test]
    fn key_releases_are_ignored() {
        // Windows reports both press and release.
        let release = KeyEvent { kind: KeyEventKind::Release, state: KeyEventState::NONE, ..ctrl('y') };
        assert_eq!(map(release, LISTS), None);
    }

    #[test]
    fn help_lists_every_prefix_command() {
        let help = help();
        let (_, commands) = &help[0];
        for (c, ..) in PREFIX {
            assert!(commands.iter().any(|(k, _)| *k == c.to_string()), "{c} missing from help");
        }
        assert!(commands.iter().any(|(k, _)| k == "0-9"));
    }
}
