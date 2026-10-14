//! Mouse events → actions, through what [`ui::spot`] finds under the pointer.

use ratatui::crossterm::event::{MouseButton, MouseEventKind};

use crate::app::{Action, State};
use crate::keymap::VOLUME_STEP;
use crate::ui::Spot;

/// Rows the wheel moves a list.
const WHEEL_ROWS: isize = 3;

/// `double`: a second left click on the same cell, quickly. `(x, y)`: the cell.
pub fn actions(
    state: &State,
    spot: Option<Spot>,
    kind: MouseEventKind,
    double: bool,
    (x, y): (u16, u16),
) -> Vec<Action> {
    if state.help {
        // Like any key: a click closes the help.
        return match kind {
            MouseEventKind::Down(_) => vec![Action::ToggleHelp],
            _ => Vec::new(),
        };
    }
    if state.context.is_some() {
        // A click picks a line of the menu; anywhere else, it closes it.
        return match (kind, spot) {
            (MouseEventKind::Down(_), Some(Spot::MenuItem(i))) => vec![Action::MenuPick(i)],
            (MouseEventKind::Moved, Some(Spot::MenuItem(i))) => vec![Action::Select(i)],
            (MouseEventKind::Down(_), _) => vec![Action::Back],
            (MouseEventKind::ScrollUp, _) => vec![Action::Up],
            (MouseEventKind::ScrollDown, _) => vec![Action::Down],
            _ => Vec::new(),
        };
    }
    let Some(spot) = spot else { return Vec::new() };
    let menu = |row| Action::ContextMenu { x, y, row };
    match (kind, spot) {
        (MouseEventKind::Down(MouseButton::Right), Spot::Row(i)) => vec![Action::Select(i), menu(true)],
        (MouseEventKind::Down(MouseButton::Right), Spot::Tab(i)) if i > 0 => vec![Action::GoTab(i), menu(false)],
        (MouseEventKind::Down(MouseButton::Right), Spot::Video | Spot::Progress(_)) => vec![menu(false)],
        (MouseEventKind::Down(MouseButton::Left), Spot::Section(i)) => vec![Action::OpenSection(i)],
        (MouseEventKind::Down(MouseButton::Left), Spot::Tab(i)) => vec![Action::GoTab(i)],
        (MouseEventKind::Down(MouseButton::Left), Spot::Row(i)) if double => vec![Action::Select(i), Action::Enter],
        (MouseEventKind::Down(MouseButton::Left), Spot::Row(i)) => vec![Action::Select(i)],
        // Two clicks pause and resume: the second one also toggles fullscreen.
        (MouseEventKind::Down(MouseButton::Left), Spot::Video) if double => {
            vec![Action::TogglePause, Action::ToggleFullscreen]
        }
        (MouseEventKind::Down(MouseButton::Left), Spot::Video) => vec![Action::TogglePause],
        (MouseEventKind::Down(MouseButton::Left), Spot::Progress(second)) => vec![Action::SeekTo(second)],
        (MouseEventKind::ScrollUp, Spot::Video | Spot::Progress(_)) => vec![Action::Volume(VOLUME_STEP)],
        (MouseEventKind::ScrollDown, Spot::Video | Spot::Progress(_)) => vec![Action::Volume(-VOLUME_STEP)],
        (MouseEventKind::ScrollUp, Spot::Row(_)) => vec![Action::Scroll(-WHEEL_ROWS)],
        (MouseEventKind::ScrollDown, Spot::Row(_)) => vec![Action::Scroll(WHEEL_ROWS)],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn state() -> State {
        State::new(Config::default(), Vec::new())
    }

    const CLICK: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
    const AT: (u16, u16) = (10, 5);

    #[test]
    fn clicks_and_wheel_by_spot() {
        let s = state();
        assert_eq!(actions(&s, Some(Spot::Section(3)), CLICK, false, AT), [Action::OpenSection(3)]);
        assert_eq!(actions(&s, Some(Spot::Tab(2)), CLICK, false, AT), [Action::GoTab(2)]);
        assert_eq!(actions(&s, Some(Spot::Row(4)), CLICK, false, AT), [Action::Select(4)]);
        assert_eq!(actions(&s, Some(Spot::Row(4)), CLICK, true, AT), [Action::Select(4), Action::Enter]);
        assert_eq!(actions(&s, Some(Spot::Video), CLICK, false, AT), [Action::TogglePause]);
        assert_eq!(actions(&s, Some(Spot::Video), CLICK, true, AT), [Action::TogglePause, Action::ToggleFullscreen]);
        assert_eq!(actions(&s, Some(Spot::Progress(42)), CLICK, false, AT), [Action::SeekTo(42)]);
        assert_eq!(actions(&s, Some(Spot::Video), MouseEventKind::ScrollUp, false, AT), [Action::Volume(VOLUME_STEP)]);
        assert_eq!(
            actions(&s, Some(Spot::Row(0)), MouseEventKind::ScrollDown, false, AT),
            [Action::Scroll(WHEEL_ROWS)]
        );
        assert!(actions(&s, Some(Spot::Section(0)), MouseEventKind::ScrollDown, false, AT).is_empty());
        assert!(actions(&s, None, CLICK, false, AT).is_empty());
        assert!(actions(&s, Some(Spot::Row(0)), MouseEventKind::Moved, false, AT).is_empty());
    }

    #[test]
    fn a_right_click_opens_the_menu_and_a_click_picks_from_it() {
        let mut s = state();
        let right = MouseEventKind::Down(MouseButton::Right);
        let menu = |row| Action::ContextMenu { x: 10, y: 5, row };
        assert_eq!(actions(&s, Some(Spot::Row(2)), right, false, AT), [Action::Select(2), menu(true)]);
        assert_eq!(actions(&s, Some(Spot::Tab(1)), right, false, AT), [Action::GoTab(1), menu(false)]);
        assert_eq!(actions(&s, Some(Spot::Video), right, false, AT), [menu(false)]);
        assert!(actions(&s, Some(Spot::Tab(0)), right, false, AT).is_empty());
        s.update(Action::GoTab(0));
        s.context = Some(crate::app::ContextMenu { at: AT, items: Vec::new(), selected: 0 });
        assert_eq!(actions(&s, Some(Spot::MenuItem(1)), CLICK, false, AT), [Action::MenuPick(1)]);
        assert_eq!(actions(&s, Some(Spot::MenuItem(1)), MouseEventKind::Moved, false, AT), [Action::Select(1)]);
        assert_eq!(actions(&s, Some(Spot::Row(0)), CLICK, false, AT), [Action::Back], "outside: closes");
    }

    #[test]
    fn a_click_closes_the_help() {
        let mut s = state();
        s.update(Action::ToggleHelp);
        assert_eq!(actions(&s, None, CLICK, false, AT), [Action::ToggleHelp]);
        assert!(actions(&s, None, MouseEventKind::ScrollUp, false, AT).is_empty());
    }
}
