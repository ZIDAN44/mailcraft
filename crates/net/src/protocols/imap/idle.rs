//! IDLE (RFC 2177): waiting on the selected folder for the server to say it changed.

use super::*;

impl<S: Read + Write + Timeouts> Client<S> {
    /// IDLE (RFC 2177) on the selected folder for up to `max`, checking `stop` about twice a
    /// second. Returns whether the server reported a change (new, expunged or changed mail).
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
        while !changed && start.elapsed() < max && !stop() {
            match self.conn.read() {
                Ok(Ok(r)) => {
                    if let Response::Data { status: Status::Bye, outcome } = &r {
                        err = Some(NetError::Closed(info(outcome)));
                        break;
                    }
                    changed |= is_change(&r);
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
