//! Just enough DNS (RFC 1035) for account setup: MX records (who handles a domain's mail) and
//! SRV records (RFC 6186: `_imaps._tcp`, `_submission._tcp`…), asked over UDP of the system's
//! own resolvers. Answers are hostile input: every length and pointer is checked, compression
//! loops are cut off.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

pub const MX: u16 = 15;
pub const SRV: u16 = 33;

/// An answer record we understand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Mx { preference: u16, host: String },
    Srv { priority: u16, weight: u16, port: u16, target: String },
}

/// A query for `name` of type `qtype`, with transaction id `id`.
pub fn query(id: u16, name: &str, qtype: u16) -> Option<Vec<u8>> {
    let mut q = Vec::with_capacity(32 + name.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00]); // recursion desired
    q.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // one question
    for label in name.trim_end_matches('.').split('.') {
        let len = u8::try_from(label.len()).ok().filter(|l| (1..=63).contains(l))?;
        if !label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return None;
        }
        q.push(len);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&[0, 1]); // class IN
    Some(q)
}

fn u16_at(m: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*m.get(i)?, *m.get(i + 1)?]))
}

/// A (possibly compressed) name at `pos`; returns it and the position after it.
fn name_at(m: &[u8], mut pos: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut end = None;
    for _ in 0..64 {
        let len = *m.get(pos)?;
        if len == 0 {
            return Some((labels.join("."), end.unwrap_or(pos + 1)));
        }
        if len & 0xC0 == 0xC0 {
            let ptr = usize::from(u16_at(m, pos)? & 0x3FFF);
            end.get_or_insert(pos + 2);
            if ptr >= pos {
                return None; // only backwards: no loops
            }
            pos = ptr;
            continue;
        }
        let l = usize::from(len & 0x3F);
        let label = m.get(pos + 1..pos + 1 + l)?;
        labels.push(String::from_utf8_lossy(label).to_ascii_lowercase());
        pos += 1 + l;
    }
    None
}

/// The records of a response to query `id`.
pub fn parse(m: &[u8], id: u16) -> Option<Vec<Record>> {
    if u16_at(m, 0)? != id || m.get(2)? & 0x80 == 0 {
        return None;
    }
    let rcode = m.get(3)? & 0x0F;
    if rcode != 0 {
        return Some(Vec::new());
    }
    let (qd, an) = (u16_at(m, 4)?, u16_at(m, 6)?);
    let mut pos = 12;
    for _ in 0..qd {
        pos = name_at(m, pos)?.1 + 4;
    }
    let mut out = Vec::new();
    for _ in 0..an.min(64) {
        let (_, after) = name_at(m, pos)?;
        let rtype = u16_at(m, after)?;
        let rdlen = usize::from(u16_at(m, after + 8)?);
        let rdata = after + 10;
        if rdata + rdlen > m.len() {
            return None;
        }
        match rtype {
            MX => out.push(Record::Mx { preference: u16_at(m, rdata)?, host: name_at(m, rdata + 2)?.0 }),
            SRV => out.push(Record::Srv {
                priority: u16_at(m, rdata)?,
                weight: u16_at(m, rdata + 2)?,
                port: u16_at(m, rdata + 4)?,
                target: name_at(m, rdata + 6)?.0,
            }),
            _ => {}
        }
        pos = rdata + rdlen;
    }
    Some(out)
}

/// The resolvers the system uses.
pub fn system_resolvers() -> Vec<IpAddr> {
    let mut v = Vec::new();
    #[cfg(windows)]
    if let Ok(adapters) = ipconfig::get_adapters() {
        for a in adapters.iter().filter(|a| a.oper_status() == ipconfig::OperStatus::IfOperStatusUp) {
            v.extend(a.dns_servers().iter().copied());
        }
    }
    #[cfg(not(windows))]
    if let Ok(text) = std::fs::read_to_string("/etc/resolv.conf") {
        for line in text.lines() {
            if let Some(ip) = line.trim().strip_prefix("nameserver").and_then(|r| r.trim().parse().ok()) {
                v.push(ip);
            }
        }
    }
    // Link-local IPv6 resolvers need a scope id we don't track.
    v.retain(|ip| !matches!(ip, IpAddr::V6(a) if (a.segments()[0] & 0xffc0) == 0xfe80));
    v.dedup();
    v.truncate(4);
    v
}

/// Ask the system's resolvers for `name`'s records of type `qtype`.
pub fn lookup(name: &str, qtype: u16) -> Vec<Record> {
    let id = (std::process::id() as u16) ^ (name.len() as u16).rotate_left(7) ^ qtype;
    let Some(q) = query(id, name, qtype) else { return Vec::new() };
    for server in system_resolvers() {
        let bind: SocketAddr = if server.is_ipv4() { ([0, 0, 0, 0], 0).into() } else { (std::net::Ipv6Addr::UNSPECIFIED, 0).into() };
        let Ok(sock) = UdpSocket::bind(bind) else { continue };
        let _ = sock.set_read_timeout(Some(Duration::from_secs(3)));
        if sock.send_to(&q, (server, 53)).is_err() {
            continue;
        }
        let mut buf = [0u8; 1500];
        for _ in 0..3 {
            let Ok((n, from)) = sock.recv_from(&mut buf) else { break };
            if from.ip() != server {
                continue;
            }
            if let Some(records) = buf.get(..n).and_then(|m| parse(m, id)) {
                return records;
            }
        }
    }
    Vec::new()
}

/// MX hosts of a domain, most preferred first.
pub fn mx(domain: &str) -> Vec<String> {
    let mut r: Vec<(u16, String)> = lookup(domain, MX)
        .into_iter()
        .filter_map(|r| match r {
            Record::Mx { preference, host } => Some((preference, host)),
            Record::Srv { .. } => None,
        })
        .collect();
    r.sort();
    r.into_iter().map(|(_, h)| h).collect()
}

/// The best SRV target of `name` (RFC 2782: lowest priority, then highest weight); `.` means
/// "not offered".
pub fn srv(name: &str) -> Option<(String, u16)> {
    let mut r: Vec<(u16, u16, u16, String)> = lookup(name, SRV)
        .into_iter()
        .filter_map(|r| match r {
            Record::Srv { priority, weight, port, target } => Some((priority, u16::MAX - weight, port, target)),
            Record::Mx { .. } => None,
        })
        .collect();
    r.sort();
    r.into_iter().find(|(_, _, port, t)| !t.is_empty() && *port != 0).map(|(_, _, port, t)| (t, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_queries() {
        let q = query(0x1234, "_imaps._tcp.example.org", SRV).unwrap();
        assert_eq!(&q[..2], &[0x12, 0x34]);
        assert!(q.windows(7).any(|w| w == b"\x06_imaps"));
        assert_eq!(&q[q.len() - 4..], &[0, 33, 0, 1]);
        assert!(query(1, "bad name.example", MX).is_none());
        assert!(query(1, &format!("{}.example", "a".repeat(64)), MX).is_none());
    }

    #[test]
    fn parses_answers_with_compression() {
        // A response to "example.org MX": two answers, the second name compressed.
        let mut m = vec![0xAB, 0xCD, 0x81, 0x80, 0, 1, 0, 2, 0, 0, 0, 0];
        m.extend_from_slice(b"\x07example\x03org\x00\x00\x0f\x00\x01");
        let mx = |pref: u16, host: &[u8]| {
            let mut a = vec![0xC0, 12, 0, 15, 0, 1, 0, 0, 1, 0];
            let mut rdata = pref.to_be_bytes().to_vec();
            rdata.extend_from_slice(host);
            a.extend_from_slice(&u16::try_from(rdata.len()).unwrap().to_be_bytes());
            a.extend_from_slice(&rdata);
            a
        };
        m.extend(mx(20, b"\x03alt\xC0\x0C"));
        m.extend(mx(10, b"\x04mail\x07example\x03org\x00"));
        let r = parse(&m, 0xABCD).unwrap();
        assert_eq!(
            r,
            vec![Record::Mx { preference: 20, host: "alt.example.org".into() }, Record::Mx { preference: 10, host: "mail.example.org".into() }]
        );
        assert!(parse(&m, 1).is_none(), "another query's answer");
        // Truncated, looping or forward pointers never panic.
        for cut in 0..m.len() {
            let _ = parse(&m[..cut], 0xABCD);
        }
        let mut looped = m.clone();
        let n = looped.len();
        looped[n - 2] = 0xC0;
        let _ = parse(&looped, 0xABCD);
        let srv = {
            let mut s = vec![0, 1, 0x81, 0x80, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 33, 0, 1, 0, 0, 0, 60];
            let target = b"\x04imap\x07example\x03org\x00";
            let mut rd = vec![0, 0, 0, 5, 3, 225];
            rd.extend_from_slice(target);
            s.extend_from_slice(&u16::try_from(rd.len()).unwrap().to_be_bytes());
            s.extend(rd);
            s
        };
        assert_eq!(parse(&srv, 1).unwrap(), vec![Record::Srv { priority: 0, weight: 5, port: 993, target: "imap.example.org".into() }]);
    }
}
