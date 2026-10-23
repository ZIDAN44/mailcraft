//! A JMAP server's answers: the input is split at form feeds into the bodies of the HTTP
//! responses to a session fetch, a folder list and a folder sync (the first two bytes of each
//! piece pick its status); plus the push stream and the URLs a session hands out.
#![no_main]

use std::cell::Cell;

use libfuzzer_sys::fuzz_target;
use mailcraft_jmap::{self as jmap, Client, Http, Request, Response, State, SyncOptions, Url, sse::SseReader};
use mailcraft_store::Store;

struct Server<'a> {
    answers: Vec<&'a [u8]>,
    next: Cell<usize>,
}

impl Http for Server<'_> {
    async fn send(&self, _req: Request) -> jmap::Result<Response> {
        let i = self.next.get();
        self.next.set(i + 1);
        let Some(a) = self.answers.get(i) else { return Err(jmap::Error::Closed("done".into())) };
        let (status, body) = match a {
            [b'!', s, rest @ ..] => (u16::from(*s) * 3, rest.to_vec()),
            _ => (200, a.to_vec()),
        };
        Ok(Response { status, body })
    }
}

fuzz_target!(|data: &[u8]| {
    let mut sse = SseReader::default();
    for piece in data.chunks(61) {
        let _ = sse.push(piece);
    }
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(u) = Url::parse(s) {
            let _ = u.join(s);
        }
        let _ = jmap::utc_date(s);
    }
    let server = Server { answers: data.split(|b| *b == 0x0c).collect(), next: Cell::new(0) };
    jmap::block_on(async {
        let Ok(c) = Client::connect(server, "https://jmap.example/", "me", "pw", false).await else { return };
        let _ = c.events_url();
        let store = Store::memory();
        let mut st = State::default();
        let _ = jmap::list_folders(&c, &mut st, "a").await;
        let opts = SyncOptions { window: 50, prefetch: 5, prefetch_max_bytes: 1 << 20 };
        let _ = jmap::sync_folder(&c, &mut st, &store, "a", "INBOX", &opts).await;
    });
});
