//! A number for each showing of the window, and whether that showing still accepts a run.
//!
//! Run requests from the page can arrive late. If the window was closed and reopened by then, the request
//! is not for the list the user is looking at now. Requests carry the showing number so they can be matched.

#[derive(Debug, Default)]
pub struct Showing {
    /// The last number handed out.
    counter: u64,
    /// The current showing's number. 0 while the window is hidden.
    id: u64,
    /// Whether the current showing still accepts a run. One run per showing.
    accepting: bool,
    /// The showing that has a Siri input watcher (see `watch_step`). 0 for none.
    watched: u64,
}

impl Showing {
    /// The window was shown. Returns the new showing's number.
    pub fn begin(&mut self) -> u64 {
        self.counter += 1;
        self.id = self.counter;
        self.accepting = true;
        self.id
    }

    /// The current showing's number. 0 while the window is hidden.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Whether to accept a run request from showing `id`. Once accepted, further requests from it are refused.
    pub fn claim(&mut self, id: u64) -> bool {
        if !self.accepting || id != self.id {
            return false;
        }
        self.accepting = false;
        true
    }

    /// An accepted request did not lead to a run. If the showing is unchanged, allow another pick.
    pub fn release(&mut self, id: u64) {
        if id == self.id {
            self.accepting = true;
        }
    }

    /// The window was hidden.
    pub fn end(&mut self) {
        self.id = 0;
        self.accepting = false;
        self.watched = 0;
    }

    /// Whether to start a Siri input watcher for the current showing: only if it has none yet. A showing that began
    /// for a notice did not take the keyboard and has none; it gets one when it does.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn take_watch(&mut self) -> bool {
        if self.id == 0 || self.watched == self.id {
            return false;
        }
        self.watched = self.id;
        true
    }
}

/// What to do when a showing that took the keyboard checks, a beat later, whether key presses reach the page.
// Only the macOS panel can be shown without the keyboard, so only it checks.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCheck {
    /// The showing is over (hidden, or shown again), or the page already receives keys.
    Stop,
    /// Take the keyboard again, then check again if `retries_left` is not 0.
    Retake { retries_left: u32 },
}

/// The key check for showing `checking`, with `retries` attempts left (counting this one). `current` is the
/// current showing's number (0 while hidden).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn key_check(checking: u64, current: u64, receives_keys: bool, retries: u32) -> KeyCheck {
    if checking != current || receives_keys || retries == 0 {
        KeyCheck::Stop
    } else {
        KeyCheck::Retake {
            retries_left: retries - 1,
        }
    }
}

/// What a Siri input watcher does on each tick.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStep {
    /// Its showing is over.
    Stop,
    /// A Siri input window is up: hide it and take the keyboard back, then keep watching.
    Retake,
    /// Nothing to do; keep watching.
    Wait,
}

/// The tick of a watcher started for showing `watching`. `current` is the current showing's number (0 while hidden).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn watch_step(watching: u64, current: u64, siri_input_window_up: bool) -> WatchStep {
    if watching != current {
        WatchStep::Stop
    } else if siri_input_window_up {
        WatchStep::Retake
    } else {
        WatchStep::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_showing_gets_one_watcher_even_if_it_began_for_a_notice() {
        // Arrange
        let mut showing = Showing::default();

        // Act / Assert
        assert!(!showing.take_watch(), "hidden");
        showing.begin(); // shown for a notice: no watcher is asked for
        assert!(showing.take_watch(), "the notice showing takes the keyboard");
        assert!(!showing.take_watch(), "shown again while visible");
        showing.end();
        showing.begin();
        assert!(showing.take_watch(), "a new showing");
    }

    #[test]
    fn a_watcher_retakes_only_while_its_showing_lasts() {
        // Act / Assert
        assert_eq!(watch_step(2, 2, true), WatchStep::Retake);
        assert_eq!(watch_step(2, 2, false), WatchStep::Wait);
        assert_eq!(watch_step(2, 0, true), WatchStep::Stop, "hidden");
        assert_eq!(watch_step(2, 3, true), WatchStep::Stop, "shown again");
    }

    #[test]
    fn the_key_check_retakes_the_keyboard_only_while_its_showing_lasts_and_keys_do_not_arrive() {
        // Act / Assert
        assert_eq!(key_check(3, 3, false, 5), KeyCheck::Retake { retries_left: 4 });
        assert_eq!(key_check(3, 3, false, 1), KeyCheck::Retake { retries_left: 0 });
        assert_eq!(key_check(3, 3, false, 0), KeyCheck::Stop, "retries used up");
        assert_eq!(key_check(3, 3, true, 5), KeyCheck::Stop, "keys arrive");
        assert_eq!(key_check(3, 0, false, 5), KeyCheck::Stop, "hidden");
        assert_eq!(key_check(3, 4, false, 5), KeyCheck::Stop, "shown again");
    }

    #[test]
    fn a_showing_accepts_exactly_one_request() {
        // Arrange
        let mut showing = Showing::default();
        let id = showing.begin();

        // Act
        let first = showing.claim(id);
        let second = showing.claim(id);

        // Assert
        assert!(first);
        assert!(!second);
    }

    #[test]
    fn nothing_is_accepted_before_the_window_is_shown_or_after_it_is_hidden() {
        // Arrange
        let mut showing = Showing::default();

        // Act
        let before = showing.claim(0);
        let id = showing.begin();
        showing.end();
        let after_with_old_id = showing.claim(id);
        let after_with_zero = showing.claim(0);

        // Assert
        assert!(!before);
        assert!(!after_with_old_id);
        assert!(!after_with_zero);
        assert_eq!(showing.id(), 0);
    }

    #[test]
    fn a_request_from_an_earlier_showing_is_refused_after_reopening() {
        // Arrange
        let mut showing = Showing::default();
        let earlier = showing.begin();
        showing.end();
        let current = showing.begin();

        // Act
        let late = showing.claim(earlier);
        let fresh = showing.claim(current);

        // Assert
        assert_ne!(earlier, current);
        assert!(!late);
        assert!(fresh);
    }

    #[test]
    fn a_released_request_lets_the_same_showing_try_again() {
        // Arrange
        let mut showing = Showing::default();
        let id = showing.begin();
        assert!(showing.claim(id));

        // Act
        showing.release(id);

        // Assert
        assert!(showing.claim(id));
    }

    #[test]
    fn releasing_after_the_window_was_reopened_does_not_touch_the_new_showing() {
        // Arrange: after a request was accepted, the window was reopened and the new showing already ran one.
        let mut showing = Showing::default();
        let earlier = showing.begin();
        assert!(showing.claim(earlier));
        showing.end();
        let current = showing.begin();
        assert!(showing.claim(current));

        // Act
        showing.release(earlier);

        // Assert
        assert!(!showing.claim(current));
    }

    #[test]
    fn releasing_after_the_window_was_hidden_accepts_nothing() {
        // Arrange
        let mut showing = Showing::default();
        let id = showing.begin();
        assert!(showing.claim(id));
        showing.end();

        // Act
        showing.release(id);

        // Assert
        assert!(!showing.claim(id));
        assert!(!showing.claim(0));
    }
}
