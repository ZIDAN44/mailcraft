//! Connections: TCP with timeouts, optionally wrapped in TLS.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::{NetError, Result};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
pub const IO_TIMEOUT: Duration = Duration::from_secs(60);

/// A stream whose read timeout can change (IDLE waits in short slices).
pub trait Timeouts {
    fn set_read_timeout(&mut self, t: Option<Duration>) -> io::Result<()>;
}

pub enum Stream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

impl Timeouts for Stream {
    fn set_read_timeout(&mut self, t: Option<Duration>) -> io::Result<()> {
        match self {
            Stream::Plain(s) => s.set_read_timeout(t),
            Stream::Tls(s) => s.sock.set_read_timeout(t),
        }
    }
}

fn tls_config() -> Result<Arc<ClientConfig>> {
    static CONFIG: OnceLock<std::result::Result<Arc<ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map(|b| Arc::new(b.with_root_certificates(roots).with_no_client_auth()))
                .map_err(|e| e.to_string())
        })
        .clone()
        .map_err(NetError::Tls)
}

/// Open a TCP connection, trying each resolved address in turn.
pub fn tcp(host: &str, port: u16) -> Result<TcpStream> {
    let addrs = (host, port).to_socket_addrs().map_err(|e| NetError::Io(io::Error::new(e.kind(), format!("{host}: {e}"))))?;
    let mut last = io::Error::new(io::ErrorKind::NotFound, format!("{host}: no address"));
    for a in addrs {
        match TcpStream::connect_timeout(&a, CONNECT_TIMEOUT) {
            Ok(s) => {
                s.set_read_timeout(Some(IO_TIMEOUT))?;
                s.set_write_timeout(Some(IO_TIMEOUT))?;
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = e,
        }
    }
    Err(NetError::Io(io::Error::new(last.kind(), format!("{host}:{port}: {last}"))))
}

/// Wrap a TCP connection in TLS, verifying the certificate against `host`.
pub fn tls(sock: TcpStream, host: &str) -> Result<Stream> {
    let name = ServerName::try_from(host.to_string()).map_err(|e| NetError::Tls(format!("{host}: {e}")))?;
    let conn = ClientConnection::new(tls_config()?, name).map_err(|e| NetError::Tls(e.to_string()))?;
    let mut s = StreamOwned::new(conn, sock);
    // Finish the handshake now so certificate problems surface as TLS errors, not later I/O.
    while s.conn.is_handshaking() {
        s.conn.complete_io(&mut s.sock).map_err(|e| NetError::Tls(e.to_string()))?;
    }
    Ok(Stream::Tls(Box::new(s)))
}
