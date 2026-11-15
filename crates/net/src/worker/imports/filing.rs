//! Message files put into a folder (issue #38), as Outlook takes `.msg` files dropped on one:
//! `.eml` files as they are, Outlook's `.msg` made standard messages, each filed as mail received
//! and read; and a `.msg` read to be shown, as a mail file opened with MailCraft. (With the
//! imports: they bring mail in from files too.)

use mailcraft_model::job::MESSAGE_FILES_MAX;
use mailcraft_model::{Flags, MessageKey};

use super::*;

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
    mailcraft_model::file_name(path).to_string()
}

/// Read each file, and put the messages in with `put`: how many went in, and those that didn't.
fn file_each(paths: &[String], mut put: impl FnMut(&[u8]) -> Result<()>) -> (usize, Vec<(String, String)>) {
    let (mut added, mut failed) = (0usize, Vec::new());
    for path in paths.iter().take(MESSAGE_FILES_MAX) {
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

/// When message `raw` came, by its own header (none when it doesn't say).
fn received(raw: &[u8], key: &MessageKey) -> Option<i64> {
    // No later than now: its header is its sender's word.
    Some(mailcraft_mime::summary_from_header(raw, key.clone(), Flags::default(), 0, 0).date.min(now())).filter(|d| *d > 0)
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
    /// Message `raw` put into `folder` of the account's server ([`Conn::put`]).
    fn put(&mut self, folder: &str, raw: &[u8], flags: &Flags, date: Option<i64>) -> Result<()> {
        match self.conn.as_mut() {
            Some(c) => c.put(&self.jstate, &self.store, &self.acc.id, folder, raw, flags, date),
            None => Err(NetError::Closed("not connected".into())),
        }
    }

    /// A folder mail was put in, read again; a POP account's is on this computer only: said to
    /// have changed.
    fn read_filed(&mut self, folder: &str) {
        if matches!(self.conn, Some(Conn::Pop)) {
            self.sink.emit(Event::FolderChanged { account: self.id(), folder: folder.to_string() });
        } else {
            self.sync_one(folder);
        }
    }

    /// Into a folder of the account's server, each as received and read; the folder read again.
    pub(in crate::worker) fn file_messages(&mut self, folder: String, paths: Vec<String>) {
        if let Err(e) = self.connected_for_job() {
            let failed = paths.iter().map(|p| (file_name(p), e.clone())).collect();
            self.sink.emit(Event::MessagesFiled { account: self.id(), folder, added: 0, failed });
            return;
        }
        let read = Flags { seen: true, ..Default::default() };
        let (added, failed) = file_each(&paths, |raw| self.put(&folder, raw, &read, None));
        if added > 0 {
            self.read_filed(&folder);
        }
        self.sink.emit(Event::MessagesFiled { account: self.id(), folder, added, failed });
    }

    /// Copies of messages kept here into a folder of the account's server (issue #82), each
    /// marked as it was; the folder read again. One whose source isn't here, or that the server
    /// won't take, is named with why: nothing is deleted for it.
    pub(in crate::worker) fn file_copies(&mut self, folder: String, mail: Vec<(MessageKey, Flags)>, moved: bool) {
        if let Err(e) = self.connected_for_job() {
            let failed = mail.into_iter().map(|(k, _)| (k, e.clone())).collect();
            self.sink.emit(Event::CopiesFiled { account: self.id(), folder, done: Vec::new(), failed, moved });
            return;
        }
        let (mut done, mut failed) = (Vec::new(), Vec::new());
        // The connection lost on the way: the rest stay where they are, none tried on it.
        let mut lost: Option<NetError> = None;
        for (key, flags) in mail {
            if let Some(e) = &lost {
                failed.push((key, e.to_string()));
                continue;
            }
            let Some(raw) = self.store.read_body(&key).filter(|raw| !mailcraft_mime::is_text_first(raw)) else {
                failed.push((key, "it isn't downloaded".to_string()));
                continue;
            };
            match self.put(&folder, &raw, &flags, received(&raw, &key)) {
                Ok(()) => done.push(key),
                Err(e) => {
                    failed.push((key, e.to_string()));
                    if e.is_fatal() {
                        lost = Some(e);
                    }
                }
            }
        }
        if let Some(e) = lost {
            // Signed in again before anything else is asked on it.
            self.sink.emit(Event::CopiesFiled { account: self.id(), folder, done, failed, moved });
            self.failed(e);
            return;
        }
        if !done.is_empty() {
            self.read_filed(&folder);
        }
        self.sink.emit(Event::CopiesFiled { account: self.id(), folder, done, failed, moved });
    }
}

#[cfg(test)]
mod tests;
