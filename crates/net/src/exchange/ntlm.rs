//! NTLMv2 sign-in (MS-NLMP), as on-premises Exchange asks for it over HTTPS, and IMAP, POP3 and
//! SMTP servers as Secure Password Authentication (issue #123): a negotiate message, the
//! server's challenge, and the answer proving the password without sending it.
//! Only NTLMv2 responses are made (never the older LM or NTLMv1 ones), with no signing or
//! sealing: the connection is TLS.
//!
//! The challenge is hostile input: every offset and length in it is checked.

use hmac::{Hmac, KeyInit, Mac};
use md4::{Digest, Md4};
use md5::Md5;

use crate::{NetError, Result};

const SIGNATURE: &[u8; 8] = b"NTLMSSP\0";
const NEGOTIATE_UNICODE: u32 = 0x0000_0001;
const REQUEST_TARGET: u32 = 0x0000_0004;
const NEGOTIATE_NTLM: u32 = 0x0000_0200;
const ALWAYS_SIGN: u32 = 0x0000_8000;
const EXTENDED_SESSION_SECURITY: u32 = 0x0008_0000;
const NEGOTIATE_TARGET_INFO: u32 = 0x0080_0000;
const NEGOTIATE_128: u32 = 0x2000_0000;
const NEGOTIATE_56: u32 = 0x8000_0000;
const FLAGS: u32 = NEGOTIATE_UNICODE
    | REQUEST_TARGET
    | NEGOTIATE_NTLM
    | ALWAYS_SIGN
    | EXTENDED_SESSION_SECURITY
    | NEGOTIATE_TARGET_INFO
    | NEGOTIATE_128
    | NEGOTIATE_56;
/// The target information kept at most.
const MAX_TARGET_INFO: usize = 4096;
/// `MsvAvChannelBindings` (MS-NLMP 2.2.2.1): the hash of the TLS channel's bindings.
const AV_CHANNEL_BINDINGS: u16 = 0x000a;

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn hmac_md5(key: &[u8], parts: &[&[u8]]) -> [u8; 16] {
    let Ok(mut mac) = <Hmac<Md5> as KeyInit>::new_from_slice(key) else { return [0; 16] };
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

/// The NT hash of a password: MD4 of its UTF-16.
fn nt_hash(password: &str) -> [u8; 16] {
    Md4::digest(utf16(password)).into()
}

/// NTOWFv2: the key the responses are made with.
fn response_key(user: &str, domain: &str, password: &str) -> [u8; 16] {
    let who = utf16(&format!("{}{domain}", user.to_uppercase()));
    hmac_md5(&nt_hash(password), &[&who])
}

/// The negotiate message (type 1).
pub fn negotiate() -> Vec<u8> {
    let mut m = Vec::with_capacity(32);
    m.extend_from_slice(SIGNATURE);
    m.extend_from_slice(&1u32.to_le_bytes());
    m.extend_from_slice(&FLAGS.to_le_bytes());
    // Empty domain and workstation fields.
    m.extend_from_slice(&[0; 16]);
    m
}

/// What a challenge message (type 2) says.
#[derive(Debug, PartialEq, Eq)]
pub struct Challenge {
    pub flags: u32,
    pub server_challenge: [u8; 8],
    pub target_info: Vec<u8>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// Read a challenge message.
pub fn challenge(m: &[u8]) -> Result<Challenge> {
    let bad = || NetError::Protocol("not an NTLM challenge".into());
    if m.get(..8) != Some(SIGNATURE.as_slice()) || u32_at(m, 8) != Some(2) {
        return Err(bad());
    }
    let flags = u32_at(m, 20).ok_or_else(bad)?;
    let server_challenge: [u8; 8] = m.get(24..32).and_then(|c| c.try_into().ok()).ok_or_else(bad)?;
    let mut target_info = Vec::new();
    if let (Some(len), Some(off)) = (u16_at(m, 40), u32_at(m, 44)) {
        let (len, off) = (usize::from(len), usize::try_from(off).unwrap_or(usize::MAX));
        if len > 0 {
            target_info = m.get(off..off.saturating_add(len)).filter(|t| t.len() <= MAX_TARGET_INFO).ok_or_else(bad)?.to_vec();
        }
    }
    Ok(Challenge { flags, server_challenge, target_info })
}

/// The server's time in the target information (`MsvAvTimestamp`), when it gives one.
fn timestamp(info: &[u8]) -> Option<[u8; 8]> {
    let mut at = 0;
    while let (Some(id), Some(len)) = (u16_at(info, at), u16_at(info, at + 2)) {
        let len = usize::from(len);
        match id {
            0 => return None,
            7 if len == 8 => return info.get(at + 4..at + 12).and_then(|t| t.try_into().ok()),
            _ => at += 4 + len,
        }
    }
    None
}

/// The target information answered with: the server's, its pairs up to its end, with the TLS
/// channel's `binding` (see [`crate::stream::binding`]) in place of any it named, then the end.
fn with_binding(info: &[u8], binding: &[u8; 16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(info.len() + 24);
    let mut at = 0;
    while let (Some(id), Some(len)) = (u16_at(info, at), u16_at(info, at + 2)) {
        let end = at + 4 + usize::from(len);
        let Some(pair) = info.get(at..end) else { break };
        if id == 0 {
            break;
        }
        if id != AV_CHANNEL_BINDINGS {
            out.extend_from_slice(pair);
        }
        at = end;
    }
    out.extend_from_slice(&AV_CHANNEL_BINDINGS.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(binding);
    // MsvAvEOL.
    out.extend_from_slice(&[0; 4]);
    out
}

/// The NTLMv2 response: (LM response, NT response).
fn responses(key: &[u8; 16], c: &Challenge, client_challenge: [u8; 8], time: [u8; 8]) -> (Vec<u8>, Vec<u8>) {
    let mut blob = vec![1, 1, 0, 0, 0, 0, 0, 0];
    blob.extend_from_slice(&time);
    blob.extend_from_slice(&client_challenge);
    blob.extend_from_slice(&[0; 4]);
    blob.extend_from_slice(&c.target_info);
    blob.extend_from_slice(&[0; 4]);
    let proof = hmac_md5(key, &[&c.server_challenge, &blob]);
    let mut nt = proof.to_vec();
    nt.extend_from_slice(&blob);
    let mut lm = hmac_md5(key, &[&c.server_challenge, &client_challenge]).to_vec();
    lm.extend_from_slice(&client_challenge);
    (lm, nt)
}

/// The authenticate message answering the challenge message `m` (as the server sent it,
/// decoded) as `user` with `password`: a client challenge of its own and the time now, as each
/// sign-in makes them (EWS over HTTPS; IMAP, POP and SMTP with Secure Password Authentication).
/// `binding`: the TLS channel's, which EWS over HTTPS sends (issue #249: Exchange's Extended
/// Protection refuses a sign-in without it).
pub fn answer(m: &[u8], user: &str, password: &str, binding: Option<&[u8; 16]>) -> Result<Vec<u8>> {
    let c = challenge(m)?;
    let mut client_challenge = [0u8; 8];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut client_challenge)
        .map_err(|_| NetError::Protocol("no randomness".into()))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let filetime = u64::try_from(crate::data_files::outlook::filetime(now)).unwrap_or(0);
    Ok(authenticate(&c, user, password, filetime, client_challenge, binding))
}

/// The authenticate message (type 3) answering a challenge as `user` (`DOMAIN\user`,
/// `user@domain` or a bare name) with `password`, and the TLS channel's `binding` when there
/// is one.
pub fn authenticate(c: &Challenge, user: &str, password: &str, now_filetime: u64, client_challenge: [u8; 8], binding: Option<&[u8; 16]>) -> Vec<u8> {
    let (domain, name) = match user.split_once('\\') {
        Some((d, u)) => (d.to_string(), u.to_string()),
        None => (String::new(), user.to_string()),
    };
    let key = response_key(&name, &domain, password);
    // With the server's time the LM response is left empty (MS-NLMP 3.1.5.1.2).
    let (time, lm_empty) = match timestamp(&c.target_info) {
        Some(t) => (t, true),
        None => (now_filetime.to_le_bytes(), false),
    };
    let bound;
    let c = match binding {
        Some(b) => {
            bound = Challenge { flags: c.flags, server_challenge: c.server_challenge, target_info: with_binding(&c.target_info, b) };
            &bound
        }
        None => c,
    };
    let (lm, nt) = responses(&key, c, client_challenge, time);
    let lm = if lm_empty { vec![0; 24] } else { lm };
    let fields: [Vec<u8>; 5] = [lm, nt, utf16(&domain), utf16(&name), Vec::new()];
    let flags = FLAGS & (c.flags | NEGOTIATE_UNICODE | NEGOTIATE_NTLM);
    let header = 64usize;
    let mut m = Vec::with_capacity(header + fields.iter().map(Vec::len).sum::<usize>());
    m.extend_from_slice(SIGNATURE);
    m.extend_from_slice(&3u32.to_le_bytes());
    let mut offset = header;
    let mut payload = Vec::new();
    // LM, NT, domain, user, workstation; then an empty session key.
    for f in fields.iter().chain(std::iter::once(&Vec::new())) {
        let len = u16::try_from(f.len()).unwrap_or(0);
        m.extend_from_slice(&len.to_le_bytes());
        m.extend_from_slice(&len.to_le_bytes());
        m.extend_from_slice(&u32::try_from(offset).unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(f);
        offset += f.len();
    }
    m.extend_from_slice(&flags.to_le_bytes());
    m.extend_from_slice(&payload);
    m
}

/// Check an authenticate message against the password (for the test server).
#[cfg(test)]
pub fn verify(c: &Challenge, m: &[u8], user: &str, domain: &str, password: &str) -> bool {
    let field = |at: usize| -> Option<&[u8]> {
        let len = usize::from(u16_at(m, at)?);
        let off = usize::try_from(u32_at(m, at + 4)?).ok()?;
        m.get(off..off + len)
    };
    let (Some(nt), Some(u)) = (field(20), field(36)) else { return false };
    if u != utf16(user).as_slice() || nt.len() < 16 {
        return false;
    }
    let key = response_key(user, domain, password);
    let (proof, blob) = nt.split_at(16);
    hmac_md5(&key, &[&c.server_challenge, blob]) == proof
}

/// A challenge message (for the test server).
#[cfg(test)]
pub fn make_challenge(server_challenge: [u8; 8], target_info: &[u8]) -> Vec<u8> {
    let mut m = Vec::new();
    m.extend_from_slice(SIGNATURE);
    m.extend_from_slice(&2u32.to_le_bytes());
    m.extend_from_slice(&[0, 0, 0, 0, 48, 0, 0, 0]);
    m.extend_from_slice(&FLAGS.to_le_bytes());
    m.extend_from_slice(&server_challenge);
    m.extend_from_slice(&[0; 8]);
    let len = u16::try_from(target_info.len()).unwrap_or(0).to_le_bytes();
    m.extend_from_slice(&len);
    m.extend_from_slice(&len);
    m.extend_from_slice(&48u32.to_le_bytes());
    m.extend_from_slice(target_info);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    // MS-NLMP 4.2.4: NTLMv2 authentication, user "User", domain "Domain", password "Password".
    const TARGET: &[u8] = &[
        0x02, 0x00, 0x0c, 0x00, 0x44, 0x00, 0x6f, 0x00, 0x6d, 0x00, 0x61, 0x00, 0x69, 0x00, 0x6e, 0x00, 0x01, 0x00, 0x0c, 0x00, 0x53, 0x00, 0x65,
        0x00, 0x72, 0x00, 0x76, 0x00, 0x65, 0x00, 0x72, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn the_specification_vectors() {
        assert_eq!(hex(&nt_hash("Password")), "a4f49c406510bdcab6824ee7c30fd852");
        let key = response_key("User", "Domain", "Password");
        assert_eq!(hex(&key), "0c868a403bfd7a93a3001ef22ef02e3f");
        let c = Challenge { flags: FLAGS, server_challenge: [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef], target_info: TARGET.to_vec() };
        let (lm, nt) = responses(&key, &c, [0xaa; 8], [0; 8]);
        assert_eq!(hex(&lm), "86c35097ac9cec102554764a57cccc19aaaaaaaaaaaaaaaa");
        assert_eq!(hex(nt.get(..16).unwrap()), "68cd0ab851e51c96aabc927bebef6a1c");
    }

    #[test]
    fn a_challenge_is_answered_and_checks_out() {
        let ch = make_challenge([7; 8], TARGET);
        let c = challenge(&ch).unwrap();
        assert_eq!(c.server_challenge, [7; 8]);
        assert_eq!(c.target_info, TARGET);
        let m = authenticate(&c, "CORP\\riley", "pw", 0, [3; 8], None);
        assert!(verify(&c, &m, "riley", "CORP", "pw"));
        assert!(!verify(&c, &m, "riley", "CORP", "wrong"));
        assert!(negotiate().starts_with(SIGNATURE));
        // Junk is refused, never a crash.
        assert!(challenge(b"NTLMSSP\0").is_err());
        assert!(challenge(&[0; 3]).is_err());
        let mut lying = ch.clone();
        lying[44] = 0xff;
        assert!(challenge(&lying).is_err());
        assert_eq!(timestamp(&[7, 0, 8, 0, 1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0]), Some([1, 2, 3, 4, 5, 6, 7, 8]));
        assert_eq!(timestamp(&[9, 0, 0xff, 0xff]), None);
    }

    /// The pairs of a target information list, as (id, value), up to its end.
    fn pairs(info: &[u8]) -> Vec<(u16, Vec<u8>)> {
        let mut out = Vec::new();
        let mut at = 0;
        while let (Some(id), Some(len)) = (u16_at(info, at), u16_at(info, at + 2)) {
            let end = at + 4 + usize::from(len);
            out.push((id, info.get(at + 4..end).unwrap_or_default().to_vec()));
            if id == 0 {
                break;
            }
            at = end;
        }
        out
    }

    /// Over HTTPS the answer carries the channel's bindings (issue #249: Exchange's Extended
    /// Protection refused a sign-in without them, which read as a wrong password): the server's
    /// pairs kept in their order, the bindings' hash after them (once, replacing any the server
    /// named), then the end; and the answer still proves the password.
    #[test]
    fn the_channel_binding_goes_in_the_answer() {
        let c = challenge(&make_challenge([7; 8], TARGET)).unwrap();
        let binding = [0x5a; 16];
        let m = authenticate(&c, "CORP\\riley", "pw", 0, [3; 8], Some(&binding));
        assert!(verify(&c, &m, "riley", "CORP", "pw"));
        let nt_len = usize::from(u16_at(&m, 20).unwrap());
        let nt_off = usize::try_from(u32_at(&m, 24).unwrap()).unwrap();
        // The NT response: its proof (16), the blob's header (28), the target information, 4 zeros.
        let info = m.get(nt_off + 16 + 28..nt_off + nt_len - 4).unwrap();
        let got = pairs(info);
        let ids: Vec<u16> = got.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [2, 1, AV_CHANNEL_BINDINGS, 0]);
        assert_eq!(got[2].1, binding);
        // A server that named bindings already (all zeros, as some do): replaced, not doubled.
        let named = with_binding(&with_binding(TARGET, &[0; 16]), &binding);
        assert_eq!(pairs(&named).iter().filter(|(id, _)| *id == AV_CHANNEL_BINDINGS).count(), 1);
        assert_eq!(pairs(&named)[2].1, binding);
        // Without one (IMAP, POP and SMTP): the server's list as it sent it.
        let plain = authenticate(&c, "CORP\\riley", "pw", 0, [3; 8], None);
        assert!(!plain.windows(4).any(|w| w == [0x0a, 0, 16, 0]));
        // Target information cut short or lying about its lengths: answered, never a panic.
        for cut in 0..TARGET.len() {
            let _ = with_binding(&TARGET[..cut], &binding);
        }
        let _ = with_binding(&[1, 0, 0xff, 0xff, 1], &binding);
    }
}
