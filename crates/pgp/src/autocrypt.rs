//! The `Autocrypt:` header (Autocrypt Level 1, §2.1): a sender's address and public key, sent
//! with each message so whoever answers can encrypt to them.

/// What an `Autocrypt:` header says.
#[derive(Clone, PartialEq, Eq)]
pub struct Autocrypt {
    /// The address the key is for, lower-case (it must be the message's sender).
    pub addr: String,
    /// `prefer-encrypt=mutual`: they'd rather have mail encrypted.
    pub mutual: bool,
    /// The key (binary OpenPGP).
    pub keydata: Vec<u8>,
}

impl std::fmt::Debug for Autocrypt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Autocrypt").field("addr", &self.addr).field("mutual", &self.mutual).field("keydata", &self.keydata.len()).finish()
    }
}

/// The largest header value read (a key of a few kilobytes, in base64).
const VALUE_MAX: usize = 256 << 10;

/// The header's value (unfolded or not), when it's one MailCraft can use: `addr` and `keydata`
/// there, nothing it doesn't know but `_`-names (which may be ignored, §2.1).
pub fn parse(value: &str) -> Option<Autocrypt> {
    if value.len() > VALUE_MAX {
        return None;
    }
    let (mut addr, mut mutual, mut keydata) = (None, false, None);
    for attr in value.split(';') {
        let attr = attr.trim();
        if attr.is_empty() {
            continue;
        }
        let (name, v) = attr.split_once('=')?;
        let name = name.trim().to_ascii_lowercase();
        match name.as_str() {
            "addr" => addr = Some(v.trim().trim_matches('"').to_lowercase()),
            "prefer-encrypt" => mutual = v.trim().eq_ignore_ascii_case("mutual"),
            "keydata" => keydata = Some(unbase64(v)?),
            n if n.starts_with('_') => {}
            _ => return None,
        }
    }
    let addr = addr.filter(|a| a.contains('@') && !a.contains(char::is_whitespace))?;
    let keydata = keydata.filter(|k| !k.is_empty())?;
    Some(Autocrypt { addr, mutual, keydata })
}

/// The whole header field for `addr`'s key `keydata` (binary), folded in lines under 78, CRLF.
pub fn header(addr: &str, keydata: &[u8], mutual: bool) -> String {
    let mut out = format!("Autocrypt: addr={}; ", addr.trim());
    if mutual {
        out.push_str("prefer-encrypt=mutual; ");
    }
    out.push_str("keydata=");
    let text = base64(keydata);
    for line in text.as_bytes().chunks(72) {
        out.push_str("\r\n ");
        out.push_str(&String::from_utf8_lossy(line));
    }
    out.push_str("\r\n");
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            let c = if i <= chunk.len() { B64.get(((n >> (18 - 6 * i)) & 63) as usize).copied().unwrap_or(b'A') } else { b'=' };
            out.push(char::from(c));
        }
    }
    out
}

/// Base64 with white space (folding) in it; `None` for anything else that isn't base64.
fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut ended = false;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                ended = true;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        if ended {
            return None;
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_read_and_write() {
        let h = header("ana@posteo.example", &[1, 2, 3, 250, 251, 252, 9], true);
        assert!(h.starts_with("Autocrypt: addr=ana@posteo.example; prefer-encrypt=mutual; keydata=\r\n "));
        assert!(h.lines().all(|l| l.len() < 78));
        let value = h.trim_start_matches("Autocrypt:").replace("\r\n", "");
        let a = parse(&value).unwrap();
        assert_eq!((a.addr.as_str(), a.mutual, a.keydata.as_slice()), ("ana@posteo.example", true, &[1, 2, 3, 250, 251, 252, 9][..]));
        // Mixed case, an underscore name, no preference.
        let a = parse("ADDR=Ana@Posteo.example; _extra=1; keydata=AQID").unwrap();
        assert_eq!((a.addr.as_str(), a.mutual, a.keydata.as_slice()), ("ana@posteo.example", false, &[1, 2, 3][..]));
    }

    #[test]
    fn what_cant_be_used_is_ignored() {
        for bad in [
            "keydata=AQID",
            "addr=ana@posteo.example",
            "addr=ana@posteo.example; keydata=",
            "addr=ana@posteo.example; keydata=AQ!D",
            "addr=ana@posteo.example; keydata=AQ==ID",
            "addr=ana@posteo.example; type=2; keydata=AQID",
            "addr=not an address; keydata=AQID",
            "garbage",
            "",
        ] {
            assert!(parse(bad).is_none(), "{bad}");
        }
        assert!(parse(&format!("addr=a@b.example; keydata={}", "A".repeat(VALUE_MAX))).is_none());
    }
}
