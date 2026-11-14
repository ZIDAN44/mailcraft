//! A proxy between MailCraft and a server on this computer, for tests of a link that misbehaves:
//! one that drops the connection, hangs, or holds a request until the test lets it go.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// What a [`Proxy`] does once what it looks for has gone through it. The bytes that hold it are
/// not passed on, but by [`Cut::Hold`].
#[derive(Clone)]
pub(crate) enum Cut {
    /// The connection is dropped, both ways, each time.
    Drop,
    /// The connection is dropped, both ways, the first time only (on any connection).
    DropOnce,
    /// The first time only (on any connection): the connection goes silent, both ways, until
    /// the test closes it ([`Hang::closed`]; [`HANG_MOST`] at most), then is dropped.
    Hang(Arc<Hang>),
    /// The first time only (on any connection): what holds it waits, with all after it, until
    /// the test lets it go ([`Hang::closed`]; [`HANG_MOST`] at most); then it goes on, the link
    /// as before. A slow server, as slow as the test likes.
    Hold(Arc<Hang>),
}

/// A link that hangs ([`Cut::Hang`]) or holds ([`Cut::Hold`]): whether it has, and, set by the
/// test, that it's closed (let go).
#[derive(Default)]
pub(crate) struct Hang {
    pub hung: AtomicBool,
    pub closed: AtomicBool,
}

/// The longest a hung link waits to be closed: longer than tests wait for anything, so a
/// connection that waited for it is caught.
pub(crate) const HANG_MOST: Duration = Duration::from_secs(60);

/// How a proxy behaves; by default it passes everything.
#[derive(Default)]
pub(crate) struct Proxy {
    /// Looked for in what the client sends, and what's done once it's there.
    pub client: Option<(&'static [u8], Cut)>,
    /// Looked for in what the server answers, and what's done once it's there.
    pub server: Option<(&'static [u8], Cut)>,
}

/// A proxy started: its port, and how many connections it has taken.
pub(crate) struct Running {
    pub port: u16,
    pub taken: Arc<AtomicUsize>,
}

impl Proxy {
    /// The proxy started, in front of the server at `port` on this computer.
    pub(crate) fn to(self, port: u16) -> Running {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let at = listener.local_addr().unwrap().port();
        let taken = Arc::new(AtomicUsize::new(0));
        let count = taken.clone();
        // Whether a cut made the first time only has been made.
        let once = Arc::new(AtomicBool::new(false));
        std::thread::spawn(move || {
            for client in listener.incoming().flatten() {
                count.fetch_add(1, Ordering::Relaxed);
                let Ok(server) = TcpStream::connect(("127.0.0.1", port)) else { continue };
                let (Ok(from_client), Ok(from_server)) = (client.try_clone(), server.try_clone()) else { continue };
                let (rule, once_here) = (self.client.clone(), once.clone());
                std::thread::spawn(move || pass(from_client, server, rule, &once_here));
                let (rule, once) = (self.server.clone(), once.clone());
                std::thread::spawn(move || pass(from_server, client, rule, &once));
            }
        });
        Running { port: at, taken }
    }
}

/// What comes from `from` passed on to `to` until either ends or `rule` cuts the link; then both
/// are dropped, both ways.
fn pass(mut from: TcpStream, mut to: TcpStream, rule: Option<(&'static [u8], Cut)>, once: &AtomicBool) {
    // The last bytes read, for what's looked for split across reads.
    let (mut tail, mut buf) = (Vec::new(), [0u8; 8192]);
    while let Ok(n @ 1..) = from.read(&mut buf) {
        let read = buf.get(..n).unwrap_or_default();
        if let Some((marker, cut)) = &rule {
            tail.extend_from_slice(read);
            if tail.windows(marker.len()).any(|w| w == *marker) && cuts(cut, once) {
                break;
            }
            tail.drain(..tail.len().saturating_sub(marker.len().saturating_sub(1)));
        }
        if to.write_all(read).is_err() {
            break;
        }
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}

/// Whether `cut` is made now; a hang or a hold is waited out here.
fn cuts(cut: &Cut, once: &AtomicBool) -> bool {
    match cut {
        Cut::Drop => true,
        Cut::DropOnce => !once.swap(true, Ordering::Relaxed),
        Cut::Hang(hang) | Cut::Hold(hang) => {
            if once.swap(true, Ordering::Relaxed) {
                return false;
            }
            hang.hung.store(true, Ordering::Relaxed);
            let end = Instant::now() + HANG_MOST;
            while !hang.closed.load(Ordering::Relaxed) && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(20));
            }
            matches!(cut, Cut::Hang(_))
        }
    }
}
