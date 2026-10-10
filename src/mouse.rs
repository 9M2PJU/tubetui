//! Mouse events → actions, through what [`ui::spot`] finds under the pointer.

use ratatui::crossterm::event::{MouseButton, MouseEventKind};

use crate::app::{Action, State};
use crate::keymap::VOLUME_STEP;
use crate::ui::Spot;

/// Rows the wheel moves a list.
const WHEEL_ROWS: isize = 3;

/// `double`: a second left click on the same cell, quickly.
pub fn actions(state: &State, spot: Option<Spot>, kind: MouseEventKind, double: bool) -> Vec<Action> {
    if state.help {
        // Like any key: a click closes the help.
        return match kind {
            MouseEventKind::Down(_) => vec![Action::ToggleHelp],
            _ => Vec::new(),
        };
    }
    let Some(spot) = spot else { return Vec::new() };
    match (kind, spot) {
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

    #[test]
    fn clicks_and_wheel_by_spot() {
        let s = state();
        assert_eq!(actions(&s, Some(Spot::Section(3)), CLICK, false), [Action::OpenSection(3)]);
        assert_eq!(actions(&s, Some(Spot::Tab(2)), CLICK, false), [Action::GoTab(2)]);
        assert_eq!(actions(&s, Some(Spot::Row(4)), CLICK, false), [Action::Select(4)]);
        assert_eq!(actions(&s, Some(Spot::Row(4)), CLICK, true), [Action::Select(4), Action::Enter]);
        assert_eq!(actions(&s, Some(Spot::Video), CLICK, false), [Action::TogglePause]);
        assert_eq!(actions(&s, Some(Spot::Video), CLICK, true), [Action::TogglePause, Action::ToggleFullscreen]);
        assert_eq!(actions(&s, Some(Spot::Progress(42)), CLICK, false), [Action::SeekTo(42)]);
        assert_eq!(actions(&s, Some(Spot::Video), MouseEventKind::ScrollUp, false), [Action::Volume(VOLUME_STEP)]);
        assert_eq!(actions(&s, Some(Spot::Row(0)), MouseEventKind::ScrollDown, false), [Action::Scroll(WHEEL_ROWS)]);
        assert!(actions(&s, Some(Spot::Section(0)), MouseEventKind::ScrollDown, false).is_empty());
        assert!(actions(&s, None, CLICK, false).is_empty());
        assert!(actions(&s, Some(Spot::Row(0)), MouseEventKind::Moved, false).is_empty());
    }

    #[test]
    fn a_click_closes_the_help() {
        let mut s = state();
        s.update(Action::ToggleHelp);
        assert_eq!(actions(&s, None, CLICK, false), [Action::ToggleHelp]);
        assert!(actions(&s, None, MouseEventKind::ScrollUp, false).is_empty());
    }
}
