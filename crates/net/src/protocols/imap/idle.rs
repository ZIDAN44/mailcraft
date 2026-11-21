//! IDLE (RFC 2177): waiting on the selected folder for the server to say it changed; and, where
//! the server offers NOTIFY (RFC 5465), told of every folder of the user's too (issue #232: mail a
//! server's own rules filed into a folder showed only at the next look).

use super::*;

/// What the server is asked to tell (RFC 5465 §5): new and expunged mail and flags in the
/// selected folder, as IDLE does; new and expunged mail in every other folder of the user's, as
/// an untagged `STATUS`.
const NOTIFY_SET: &str = "NOTIFY SET (selected (MessageNew (UID) MessageExpunge FlagChange)) (personal (MessageNew MessageExpunge))";

impl<S: Read + Write> Client<S> {
    /// Ask the server to tell of changes in every folder of the user's (NOTIFY), once on this
    /// connection, when it offers it. Whether it does: a server that refuses it (or hasn't it)
    /// tells of the selected folder alone, as before.
    pub fn notify(&mut self) -> Result<bool> {
        if self.notifying || !self.has("NOTIFY") {
            return Ok(self.notifying);
        }
        match self.cmd(&[Part::Text(NOTIFY_SET)]) {
            Ok(_) => {
                self.notifying = true;
                Ok(true)
            }
            // Refused (NO, BAD): left off, and not asked again on this connection.
            Err(NetError::No(e) | NetError::Bad(e) | NetError::Protocol(e)) => {
                log::info!("NOTIFY refused: {e}");
                self.caps.remove("NOTIFY");
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// The folders the server said changed since this was last asked (NOTIFY), the selected one's
    /// aside.
    pub fn take_notified(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notified)
    }
}

impl<S: Read + Write + Timeouts> Client<S> {
    /// IDLE (RFC 2177) on the selected folder for up to `max`, checking `stop` about twice a
    /// second, and ending early when another folder changed (NOTIFY: [`Client::take_notified`]).
    /// Returns whether the server reported a change in the selected folder (new, expunged or
    /// changed mail).
    pub fn idle(&mut self, max: Duration, mut stop: impl FnMut() -> bool) -> Result<bool> {
        let tag = self.conn.next_tag();
        self.conn.begin();
        self.conn.send(format!("{tag} IDLE\r\n").as_bytes())?;
        let mut ignored = Vec::new();
        self.conn.wait_continue(&tag, &mut ignored)?;
        let mut changed = ignored.iter().any(is_change);
        drop(ignored);
        self.conn.waiting();
        let start = Instant::now();
        self.conn.get_mut().set_read_timeout(Some(Duration::from_millis(500)))?;
        let mut err = None;
        while !changed && self.notified.is_empty() && start.elapsed() < max && !stop() {
            match self.conn.read() {
                Ok(Ok(r)) => {
                    if let Response::Data { status: Status::Bye, outcome } = &r {
                        err = Some(NetError::Closed(info(outcome)));
                        break;
                    }
                    changed |= is_change(&r);
                    // Another folder changed (NOTIFY): the wait ends, to read it.
                    self.note_notified(&r);
                }
                Ok(Err(_)) => {}
                Err(NetError::Io(e)) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
        self.conn.get_mut().set_read_timeout(Some(IO_TIMEOUT))?;
        if let Some(e) = err {
            return Err(e);
        }
        self.conn.begin();
        self.conn.send(b"DONE\r\n")?;
        let rs = self.conn.finish(&tag, Vec::new())?;
        Ok(changed || rs.iter().any(is_change))
    }
}

pub(super) fn is_change(r: &Response<'_>) -> bool {
    matches!(
        r,
        Response::MailboxData(MailboxDatum::Exists(_) | MailboxDatum::Recent(_))
            | Response::Expunge(_)
            | Response::Vanished { .. }
            | Response::Fetch(..)
    )
}
