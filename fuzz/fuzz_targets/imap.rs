//! An IMAP server's side of a session: the input is everything the server sends, read by the
//! client through a run of commands (greeting, CAPABILITY, LOGIN, LIST, SELECT, a folder sync,
//! STATUS, QUOTA, APPEND, MOVE). Each command reads on after the last one failed, as a
//! reconnect would see a fresh server.
#![no_main]

use std::io::{self, Read, Write};

use libfuzzer_sys::fuzz_target;
use mailcraft_net::imap::{self, Client};
use mailcraft_net::sync::{self, SyncOptions};
use mailcraft_store::Store;

/// The server: its bytes read in small pieces (as a socket hands them over), writes dropped.
struct Server<'a> {
    data: &'a [u8],
}

impl Read for Server<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.data.len()).min(97);
        let (now, rest) = self.data.split_at(n);
        buf[..n].copy_from_slice(now);
        self.data = rest;
        Ok(n)
    }
}

/// Whatever the client sends is accepted whole and forgotten.
impl Write for Server<'_> {
    fn write(&mut self, sent: &[u8]) -> io::Result<usize> {
        let taken = sent.len();
        Ok(taken)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::Result::Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = imap::parse_internal_date(s);
    }
    let Ok(mut c) = Client::from_stream(Server { data }) else { return };
    let _ = c.capability();
    let _ = c.login("me", "pw");
    let _ = c.list("a");
    let store = Store::memory();
    let opts = SyncOptions { window: 50, prefetch: 5, prefetch_max_bytes: 1 << 20 };
    let _ = sync::sync_folder(&mut c, &store, "a", "INBOX", &opts);
    let _ = c.select_qresync("Archive", 7, 9, "1:5");
    let _ = c.status("Sent");
    let _ = c.uid_search("ALL");
    let _ = c.uid_fetch("1:*", "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])", "");
    let _ = c.quota();
    let _ = c.append("Drafts", r"(\Draft)", b"Subject: x\r\n\r\nbody\r\n");
    let _ = c.uid_move_to("1:3", "Trash");
    let _ = c.noop();
    let _ = c.take_news();
});
