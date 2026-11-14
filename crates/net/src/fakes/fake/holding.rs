//! A command held unanswered until the test lets it go ([`State::hold_on`]): a set point in what
//! the client does, waited for and ended by the test, with no clock to race.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use super::*;

/// How a held command goes on once let go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LetGo {
    /// Answered as any other.
    Answer,
    /// Answered, then the connection is dropped as the next command comes on it, unanswered.
    AnswerThenHangUp,
}

/// The server's side of a hold: the first command containing `text` waits for `go`, having said
/// on `came` that it's there.
pub struct Hold {
    text: String,
    came: Sender<()>,
    go: Receiver<LetGo>,
}

/// The test's side of a hold.
pub struct Holding {
    came: Receiver<()>,
    go: Sender<LetGo>,
}

impl Hold {
    /// Hold the first command containing `text` (put in [`State::hold_on`]).
    pub fn on(text: &str) -> (Hold, Holding) {
        let (came_tx, came_rx) = channel();
        let (go_tx, go_rx) = channel();
        (Hold { text: text.to_string(), came: came_tx, go: go_rx }, Holding { came: came_rx, go: go_tx })
    }
}

impl Holding {
    /// Wait until the held command has come, unanswered. A minute at most: one that never
    /// comes fails the test rather than hanging it.
    pub fn came(&self) {
        assert!(self.came.recv_timeout(Duration::from_secs(60)).is_ok(), "the held command never came");
    }

    /// Let the held command go on. Dropped without this, it's answered.
    pub fn go(self, how: LetGo) {
        let _ = self.go.send(how);
    }
}

impl Fake {
    /// Wait here while `line` is the command [`State::hold_on`] holds, until the test lets it go.
    pub(super) fn held(&mut self, line: &str) {
        let hold = match self.st.lock() {
            Ok(mut s) if s.hold_on.as_ref().is_some_and(|h| line.contains(h.text.as_str())) => s.hold_on.take(),
            _ => None,
        };
        let Some(hold) = hold else { return };
        let _ = hold.came.send(());
        if hold.go.recv() == Ok(LetGo::AnswerThenHangUp) {
            self.hang_up_next = true;
        }
    }
}
