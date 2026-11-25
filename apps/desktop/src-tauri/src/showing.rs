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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
