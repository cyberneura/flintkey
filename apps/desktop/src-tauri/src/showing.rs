//! 窓の表示 1 回ごとの番号と、その表示がまだ実行を受け付けるかどうか。
//!
//! ページからの実行要求は遅れて届くことがある。届いた時点で窓が閉じて開き直されていたら、
//! その要求は利用者がいま見ている一覧に対するものではない。要求に表示の番号を付けさせて照合する。

#[derive(Debug, Default)]
pub struct Showing {
    /// 最後に振った番号。
    counter: u64,
    /// 今の表示の番号。窓が隠れている間は 0。
    id: u64,
    /// 今の表示が、まだ実行を受け付けるか。1 回の表示で実行するのは 1 件だけ。
    accepting: bool,
}

impl Showing {
    /// 窓を出した。新しい表示の番号を返す。
    pub fn begin(&mut self) -> u64 {
        self.counter += 1;
        self.id = self.counter;
        self.accepting = true;
        self.id
    }

    /// 今の表示の番号。窓が隠れている間は 0。
    pub fn id(&self) -> u64 {
        self.id
    }

    /// 表示 `id` からの実行要求を受け付けるか。受け付けたら、同じ表示からの次の要求は断る。
    pub fn claim(&mut self, id: u64) -> bool {
        if !self.accepting || id != self.id {
            return false;
        }
        self.accepting = false;
        true
    }

    /// 受け付けた要求が実行に至らなかった。同じ表示のままなら、選び直せるように戻す。
    pub fn release(&mut self, id: u64) {
        if id == self.id {
            self.accepting = true;
        }
    }

    /// 窓を隠した。
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
        // Arrange: 要求を受け付けたあとに窓が閉じて開き直され、新しい表示が 1 件を実行済み。
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
