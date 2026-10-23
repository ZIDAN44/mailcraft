//! Connections: TCP with timeouts, optionally wrapped in TLS.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::{NetError, Result};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
pub const IO_TIMEOUT: Duration = Duration::from_secs(60);
/// Per-address budget when more candidates remain. With a black-holed IPv6 route each AAAA
/// record used to cost the full [`CONNECT_TIMEOUT`] before any A record was tried.
const SHORT_TRY: Duration = Duration::from_secs(3);

/// Set the first time an IPv4 address connects after an IPv6 one of the same host didn't.
static V4_LEADS: AtomicBool = AtomicBool::new(false);

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

impl Timeouts for TcpStream {
    fn set_read_timeout(&mut self, t: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, t)
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

/// Interleave address families (RFC 8305 §4): sort by each address's rank among those of its
/// own family, ties going to the leading family, which is the resolver's first unless
/// `prefer_v4`. Order within a family is kept.
fn attempt_order(addrs: Vec<SocketAddr>, prefer_v4: bool) -> Vec<SocketAddr> {
    let lead_is_v6 = match addrs.first() {
        Some(a) => a.is_ipv6() && !prefer_v4,
        None => return addrs,
    };
    let mut count = [0usize; 2];
    let mut ranked: Vec<(usize, bool, SocketAddr)> = addrs
        .into_iter()
        .map(|a| {
            let fam = usize::from(a.is_ipv6());
            let rank = count[fam];
            count[fam] += 1;
            (rank, a.is_ipv6() != lead_is_v6, a)
        })
        .collect();
    ranked.sort_by_key(|(rank, trailing, _)| (*rank, *trailing));
    ranked.into_iter().map(|(_, _, a)| a).collect()
}

/// Open a TCP connection, trying each resolved address in turn.
pub fn tcp(host: &str, port: u16) -> Result<TcpStream> {
    connect(host, port, false)
}

/// [`tcp`] to the internet only: never this computer, its network, or a link-local address,
/// whatever the name resolves to. For what a message makes MailCraft fetch (its pictures): a
/// picture address must not reach a router's or printer's web page, or a service on this
/// computer. Checked on the addresses connected to, so a name that resolves to one is refused.
pub fn tcp_public(host: &str, port: u16) -> Result<TcpStream> {
    connect(host, port, true)
}

/// Whether an address is on the public internet: not loopback, private (RFC 1918, IPv6 unique
/// local), link-local, shared (100.64/10), unspecified, multicast or broadcast; an IPv4 address
/// written as IPv6 is judged as itself.
pub fn is_public(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let [first, second, third, _] = v4.octets();
            // What the standard library already knows how to recognise…
            let named = [
                v4.is_unspecified(),
                v4.is_loopback(),
                v4.is_private(),
                v4.is_link_local(),
                v4.is_broadcast(),
                v4.is_documentation(),
                v4.is_multicast(),
            ];
            // …and the blocks it doesn't: 0/8, shared 100.64/10, IETF 192.0.0/24, reserved 240/4.
            let shared = first == 100 && (second & 0xc0) == 0x40;
            let ietf = (first, second, third) == (192, 0, 0);
            !(named.contains(&true) || first == 0 || shared || ietf || first >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first & 0xffc0) == 0xfec0
                || first == 0x2001 && v6.segments()[1] == 0x0db8)
        }
    }
}

fn connect(host: &str, port: u16, public_only: bool) -> Result<TcpStream> {
    // A name that doesn't resolve is almost always a mistyped or guessed server name.
    let addrs = (host, port).to_socket_addrs().map_err(|e| {
        NetError::Io(io::Error::new(
            e.kind(),
            format!("no server called {host} could be found: check the server name in Server settings, and that this computer is online ({e})"),
        ))
    })?;
    let mut last = io::Error::new(io::ErrorKind::NotFound, format!("{host}: no address"));
    let found: Vec<SocketAddr> = addrs.take(64).collect();
    let reachable: Vec<SocketAddr> = if public_only { found.iter().copied().filter(|a| is_public(a.ip())).collect() } else { found.clone() };
    if reachable.is_empty() && !found.is_empty() {
        return Err(NetError::Protocol(format!("{host} is on this computer or its network: only addresses on the internet are fetched")));
    }
    let candidates = attempt_order(reachable, V4_LEADS.load(Ordering::Relaxed));
    let mut v6_missed = false;
    let mut left = candidates.len();
    for a in &candidates {
        left -= 1;
        let budget = if left == 0 { CONNECT_TIMEOUT } else { SHORT_TRY };
        match TcpStream::connect_timeout(a, budget) {
            Ok(s) => {
                if v6_missed && a.is_ipv4() {
                    V4_LEADS.store(true, Ordering::Relaxed);
                }
                s.set_read_timeout(Some(IO_TIMEOUT))?;
                s.set_write_timeout(Some(IO_TIMEOUT))?;
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => {
                v6_missed |= a.is_ipv6();
                last = e;
            }
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

#[cfg(test)]
mod tests {
    /// What a message's pictures may reach: the internet, never this computer or its network.
    #[test]
    fn only_internet_addresses_are_public() {
        for ip in ["93.184.216.34", "8.8.8.8", "2606:4700::1111", "1.1.1.1"] {
            assert!(super::is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "::ffff:192.168.1.1",
            "::ffff:127.0.0.1",
            "192.0.0.8",
            "240.0.0.1",
        ] {
            assert!(!super::is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    /// A picture address on this computer is refused, even with a server listening there.
    #[test]
    fn a_picture_on_this_computer_is_not_fetched() {
        let local = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = local.local_addr().unwrap();
        let port = address.port();
        let why = |host: &str| match super::tcp_public(host, port) {
            Ok(_) => String::from("connected"),
            Err(e) => e.to_string(),
        };
        let by_address = why("127.0.0.1");
        assert!(by_address.contains("only addresses on the internet"), "{by_address}");
        let by_name = why("localhost");
        assert!(by_name.contains("only addresses on the internet"), "a name resolving to it too: {by_name}");
        assert!(super::tcp("127.0.0.1", port).is_ok(), "the account's own servers may be anywhere");
    }

    #[test]
    fn a_server_name_that_does_not_exist_says_what_to_check() {
        // `.invalid` never resolves (RFC 2606).
        let e = super::tcp("mail.no-such-host.invalid", 995).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("no server called mail.no-such-host.invalid") && e.contains("Server settings"), "{e}");
    }
}

#[cfg(test)]
mod attempt_order_tests {
    use super::*;

    #[test]
    fn connection_attempts_alternate_ip_versions() {
        let p = |s: &str| -> SocketAddr { s.parse().unwrap() };
        let (x1, x2, x3) = (p("198.51.100.7:993"), p("198.51.100.8:993"), p("198.51.100.9:993"));
        let (y1, y2) = (p("[2001:db8:0:1::7]:993"), p("[2001:db8:0:1::8]:993"));
        // Resolver led with AAAA: v6, v4, v6, v4, then the spare v4.
        assert_eq!(attempt_order(vec![y1, y2, x1, x2, x3], false), vec![y1, x1, y2, x2, x3]);
        // After a v6 miss, A records lead.
        assert_eq!(attempt_order(vec![y1, y2, x1, x2, x3], true), vec![x1, y1, x2, y2, x3]);
        // Resolver led with A: unchanged lead.
        assert_eq!(attempt_order(vec![x1, y1, y2], false), vec![x1, y1, y2]);
        assert_eq!(attempt_order(vec![], true), Vec::<SocketAddr>::new());
    }
}
