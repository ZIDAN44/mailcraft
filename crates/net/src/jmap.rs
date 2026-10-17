//! JMAP on the desktop: the portable transport in `mailcraft-jmap`, run over this crate's HTTP
//! client (each request waits for its answer, so the transport's futures are ready at once),
//! plus push over the session's EventSource (RFC 8620 §7.3).

use mailcraft_model::{Folder, MessageKey, Op};
use mailcraft_store::Store;

pub use mailcraft_jmap::{Mailbox, State, VALIDITY, block_on, folders, session_url, utc_date};

use crate::sync::{SyncOptions, SyncReport};
use crate::{NetError, Result, http};

impl From<mailcraft_jmap::Error> for NetError {
    fn from(e: mailcraft_jmap::Error) -> NetError {
        use mailcraft_jmap::Error as E;
        match e {
            E::Io(m) => NetError::Io(std::io::Error::other(m)),
            E::Tls(m) => NetError::Tls(m),
            E::Protocol(m) => NetError::Protocol(m),
            E::No(m) => NetError::No(m),
            E::Bad(m) => NetError::Bad(m),
            E::Auth(m) => NetError::Auth(m),
            E::Closed(m) => NetError::Closed(m),
            E::Send(m) => NetError::Smtp(m),
            E::Store(e) => NetError::Store(e),
        }
    }
}

impl From<NetError> for mailcraft_jmap::Error {
    fn from(e: NetError) -> mailcraft_jmap::Error {
        use mailcraft_jmap::Error as E;
        match e {
            NetError::Io(e) => E::Io(e.to_string()),
            NetError::Tls(m) => E::Tls(m),
            NetError::Protocol(m) => E::Protocol(m),
            NetError::No(m) | NetError::Refused(m) => E::No(m),
            NetError::Bad(m) => E::Bad(m),
            NetError::Auth(m) => E::Auth(m),
            NetError::Closed(m) => E::Closed(m),
            NetError::Smtp(m) | NetError::Later(m) => E::Send(m),
            NetError::Store(e) => E::Store(e),
        }
    }
}

/// This crate's HTTP client, for the portable transport.
pub struct Blocking;

impl mailcraft_jmap::Http for Blocking {
    async fn send(&self, req: mailcraft_jmap::Request) -> mailcraft_jmap::Result<mailcraft_jmap::Response> {
        let r = http::Request { method: req.method, url: req.url, headers: req.headers, body: req.body, max: req.max };
        let resp = http::send(&r)?;
        Ok(mailcraft_jmap::Response { status: resp.status, body: resp.body })
    }
}

/// A signed-in JMAP session.
pub struct Client(mailcraft_jmap::Client<Blocking>);

impl Client {
    /// Fetch the session and check it offers mail. `bearer` signs in with an OAuth access token.
    pub fn connect(url: &str, user: &str, secret: &str, bearer: bool) -> Result<Client> {
        Ok(Client(block_on(mailcraft_jmap::Client::connect(Blocking, url, user, secret, bearer))?))
    }

    /// Listen for changes (`state` events, with `ping`s to keep the line open); `None` when the
    /// server doesn't push.
    pub fn events(&self) -> Result<Option<http::EventStream<crate::stream::Stream>>> {
        let Some(url) = self.0.events_url() else { return Ok(None) };
        http::open_events(&url, &[("Authorization", self.0.authorization())]).map(Some)
    }

    /// Space used and the limit in bytes for the whole account (RFC 9425); `None` without quotas.
    pub fn quota(&self) -> Result<Option<(u64, u64)>> {
        Ok(block_on(self.0.quota())?)
    }

    /// The automatic replies (RFC 8621 §8); an error when the server offers none.
    pub fn vacation(&self) -> Result<mailcraft_model::job::Vacation> {
        Ok(block_on(self.0.vacation())?)
    }

    pub fn set_vacation(&self, v: &mailcraft_model::job::Vacation) -> Result<()> {
        Ok(block_on(self.0.set_vacation(v))?)
    }
}

/// Refresh the folder list (and the mailbox ids behind it).
pub fn list_folders(c: &Client, st: &mut State, account: &str) -> Result<Vec<Folder>> {
    Ok(block_on(mailcraft_jmap::list_folders(&c.0, st, account))?)
}

/// Bring one folder's cache up to date.
pub fn sync_folder(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions) -> Result<SyncReport> {
    Ok(block_on(mailcraft_jmap::sync_folder(&c.0, st, store, account, folder, opts))?)
}

/// Download one message's source (and cache it).
pub fn fetch_body(c: &Client, st: &State, store: &Store, key: &MessageKey) -> Result<Vec<u8>> {
    Ok(block_on(mailcraft_jmap::fetch_body(&c.0, st, store, key))?)
}

/// Apply one change on the server.
pub fn apply_remote(c: &Client, st: &State, op: &Op) -> Result<()> {
    Ok(block_on(mailcraft_jmap::apply_remote(&c.0, st, op))?)
}

/// Store a message in a folder (a draft, or the copy of a sent message).
pub fn append(c: &Client, st: &State, folder: &str, raw: &[u8], keywords: &[&str]) -> Result<String> {
    Ok(block_on(mailcraft_jmap::append(&c.0, st, folder, raw, keywords))?)
}

/// Send a message: stored in the Sent folder, then submitted.
pub fn send(c: &Client, st: &State, sent: Option<&str>, raw: &[u8], from: &str, recipients: &[String]) -> Result<()> {
    Ok(block_on(mailcraft_jmap::send(&c.0, st, sent, raw, from, recipients))?)
}
