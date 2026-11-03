//! Message files put into a folder (issue #38), as Outlook takes `.msg` files dropped on one:
//! `.eml` files as they are, Outlook's `.msg` made standard messages, each filed as mail received
//! and read; and a `.msg` read to be shown, as a mail file opened with MailCraft. (With the
//! imports: they bring mail in from files too.)

use mailcraft_model::Flags;

use super::*;

/// The most files taken at once.
const MAX_FILES: usize = 500;

/// A message file read as a standard message: an Outlook `.msg` made one, else the file as it is
/// when it reads as a message.
pub(crate) fn read_message_file(path: &str) -> std::result::Result<Vec<u8>, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > u64::try_from(crate::msg::MAX_FILE).unwrap_or(u64::MAX) {
        return Err("too big to be a message".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if crate::msg::is_compound_file(&bytes) {
        return crate::msg::to_mime(&bytes);
    }
    // A message has a header: a field before its first blank line.
    let head = bytes.split(|b| *b == b'\n').next().unwrap_or_default();
    let field = head.iter().position(|b| *b == b':').is_some_and(|i| i > 0 && head[..i].iter().all(|b| b.is_ascii_graphic()));
    if !field || mailcraft_mime::parse(&bytes).is_none() {
        return Err("not a message".into());
    }
    Ok(bytes)
}

/// A file's name, for saying which one couldn't go in.
pub(in crate::worker) fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// Read each file, and put the messages in with `put`: how many went in, and those that didn't.
fn file_each(paths: &[String], mut put: impl FnMut(&[u8]) -> Result<()>) -> (usize, Vec<(String, String)>) {
    let (mut added, mut failed) = (0usize, Vec::new());
    for path in paths.iter().take(MAX_FILES) {
        match read_message_file(path) {
            Ok(raw) => match put(&raw) {
                Ok(()) => added += 1,
                Err(e) => failed.push((file_name(path), e.to_string())),
            },
            Err(e) => failed.push((file_name(path), e)),
        }
    }
    (added, failed)
}

/// Into a folder of an account on this computer (no server): on the pool's thread for changes,
/// with the others made there.
pub(in crate::worker) fn file_here(store: &Store, sink: &Sink, account: &str, folder: &str, paths: &[String]) {
    let flags = Flags { seen: true, ..Default::default() };
    let (added, failed) = file_each(paths, |raw| crate::pop::file_locally(store, account, folder, raw, flags.clone(), now()).map(|_| ()));
    if added > 0 {
        sink.emit(Event::FolderChanged { account: account.to_string(), folder: folder.to_string() });
    }
    sink.emit(Event::MessagesFiled { account: account.to_string(), folder: folder.to_string(), added, failed });
}

impl Pool {
    /// A message file read on a thread of its own, answered by [`Event::MessageFileRead`].
    pub(in crate::worker) fn read_message_file(&self, path: String) {
        let sink = self.sink.clone();
        let spawned = std::thread::Builder::new().name("mail-message-file".into()).spawn({
            let path = path.clone();
            move || {
                let raw = read_message_file(&path);
                sink.emit(Event::MessageFileRead { path, raw });
            }
        });
        if let Err(e) = spawned {
            self.sink.emit(Event::MessageFileRead { path, raw: Err(e.to_string()) });
        }
    }
}

impl Worker {
    /// Into a folder of the account's server, each as received and read; the folder read again.
    pub(in crate::worker) fn file_messages(&mut self, folder: String, paths: Vec<String>) {
        if self.conn.is_none()
            && let Err(e) = self.connect()
        {
            let failed = paths.iter().map(|p| (file_name(p), e.to_string())).collect();
            self.sink.emit(Event::MessagesFiled { account: self.id(), folder, added: 0, failed });
            return;
        }
        let (store, account) = (self.store.clone(), self.id());
        let (added, failed) = {
            let jstate = &self.jstate;
            let conn = &mut self.conn;
            file_each(&paths, |raw| match conn.as_mut() {
                Some(Conn::Imap(c)) => c.append(&folder, "\\Seen", raw).map(|_| ()),
                Some(Conn::Jmap(c)) => jmap::append(c, jstate, &folder, raw, &["$seen"]).map(|_| ()),
                // Exchange: EWS takes it as received (not a draft); Graph only makes drafts of
                // what it's given whole.
                Some(Conn::Ews(c)) => c.create_received(raw, jstate.mailbox(&folder)?).map(|_| ()),
                Some(Conn::Graph(c)) => crate::graph::append(c, jstate, &folder, raw).map(|_| ()),
                Some(Conn::Pop) => {
                    let flags = Flags { seen: true, ..Default::default() };
                    crate::pop::file_locally(&store, &account, &folder, raw, flags, now()).map(|_| ())
                }
                None => Err(NetError::Closed("not connected".into())),
            })
        };
        if added > 0 {
            if matches!(self.conn, Some(Conn::Pop)) {
                self.sink.emit(Event::FolderChanged { account: self.id(), folder: folder.clone() });
            } else {
                self.sync_one(&folder);
            }
        }
        self.sink.emit(Event::MessagesFiled { account: self.id(), folder, added, failed });
    }
}

#[cfg(test)]
mod tests;
