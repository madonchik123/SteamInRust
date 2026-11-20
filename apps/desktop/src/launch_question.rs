//! A question a game's launch waits on (today: the Steam Cloud conflict,
//! "keep this PC's saves or the cloud's?") must be seen. A game started from
//! a desktop shortcut or a `steam://` link while SteamInRust sits in the
//! tray or is minimized would otherwise ask inside a window nobody can see,
//! and the game never starts.
//!
//! So while such a question waits, the main window comes back, goes on top
//! of other windows and flashes on the taskbar, like Steam's own dialog.
//! Once answered, the window goes back to where it was (the tray, or
//! minimized).

/// Where the main window was before a launch question brought it up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowWas {
    Tray,
    Minimized,
    Shown,
}

/// What to do with the main window this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// A question just came up: bring the window up (remembering where it was).
    Raise(WindowWas),
    /// The question was answered: put the window back.
    PutBack(WindowWas),
    Nothing,
}

/// `waiting`: a launch question is open. `raised`: where the window was
/// when this question brought it up (`None` when it hasn't).
pub fn step(waiting: bool, raised: Option<WindowWas>, hidden: bool, minimized: bool) -> Step {
    match (waiting, raised) {
        (true, None) => Step::Raise(if hidden {
            WindowWas::Tray
        } else if minimized {
            WindowWas::Minimized
        } else {
            WindowWas::Shown
        }),
        (false, Some(was)) => Step::PutBack(was),
        _ => Step::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_brings_the_window_up_from_wherever_it_is() {
        assert_eq!(step(true, None, true, false), Step::Raise(WindowWas::Tray));
        // Hidden wins: a window can be minimized and then hidden.
        assert_eq!(step(true, None, true, true), Step::Raise(WindowWas::Tray));
        assert_eq!(step(true, None, false, true), Step::Raise(WindowWas::Minimized));
        // Shown but maybe behind other windows: still brought to the front.
        assert_eq!(step(true, None, false, false), Step::Raise(WindowWas::Shown));
    }

    #[test]
    fn raised_once_per_question() {
        // While it waits, the user may minimize or hide the window again:
        // that's their choice, not fought every frame.
        assert_eq!(step(true, Some(WindowWas::Tray), true, false), Step::Nothing);
        assert_eq!(step(true, Some(WindowWas::Shown), false, true), Step::Nothing);
    }

    #[test]
    fn answered_puts_the_window_back() {
        assert_eq!(step(false, Some(WindowWas::Tray), false, false), Step::PutBack(WindowWas::Tray));
        assert_eq!(step(false, Some(WindowWas::Minimized), false, false), Step::PutBack(WindowWas::Minimized));
        assert_eq!(step(false, Some(WindowWas::Shown), false, false), Step::PutBack(WindowWas::Shown));
        assert_eq!(step(false, None, true, false), Step::Nothing);
    }
}
