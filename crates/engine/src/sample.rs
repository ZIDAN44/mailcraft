//! The sample mailbox (`mailcraft --sample`): a local-only account full of our own example mail,
//! so the app can be explored, screenshotted and tested without a server. Every word here is
//! original; addresses use the reserved `example` domains (RFC 2606).

use mailcraft_mime::{self as mime, Draft, OutAttachment};
use mailcraft_model::{AccountConfig, EmailAddress, Flags, Folder, FolderRole, MessageKey, sort_newest_first};
use mailcraft_store::{FolderState, Store, StoreError};

pub const SAMPLE_ACCOUNT: &str = "sample";

struct Mail {
    folder: &'static str,
    from: (&'static str, &'static str),
    to: &'static [(&'static str, &'static str)],
    cc: &'static [(&'static str, &'static str)],
    subject: &'static str,
    body: &'static str,
    /// Minutes before "now".
    ago: i64,
    seen: bool,
    flagged: bool,
    attachment: Option<(&'static str, Attach)>,
    reply_to_index: Option<usize>,
}

#[derive(Clone, Copy)]
enum Attach {
    Png,
    Text(&'static str),
}

const ME: (&str, &str) = ("Riley Moreno", "riley@studio.example");

/// An 8×8 PNG drawn in code (a small checkerboard), so the sample carries a real image.
pub fn sample_png() -> Vec<u8> {
    // Uncompressed DEFLATE (stored blocks) keeps this dependency-free.
    let (w, h) = (8u32, 8u32);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0u8);
        for x in 0..w {
            let on = (x / 2 + y / 2) % 2 == 0;
            raw.extend_from_slice(if on { &[0x3B, 0x5B, 0xDB] } else { &[0xF2, 0xF4, 0xFA] });
        }
    }
    let mut z = vec![0x78, 0x01, 0x01];
    let len = u16::try_from(raw.len()).unwrap_or(0);
    z.extend_from_slice(&len.to_le_bytes());
    z.extend_from_slice(&(!len).to_le_bytes());
    z.extend_from_slice(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let chunk = |out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
        let mut c = Vec::with_capacity(4 + data.len());
        c.extend_from_slice(ty);
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn mails() -> Vec<Mail> {
    vec![
        Mail {
            folder: "INBOX",
            from: ("Zoe Park", "zoe@studio.example"),
            to: &[ME],
            cc: &[("Sam Okafor", "sam@studio.example")],
            subject: "Storyboard review on Thursday",
            body: "Hi Riley,\n\nThe animatic is in good shape. Could you bring the revised frames for scenes 4 to 7 on Thursday? I attached the colour key we agreed on so we stay consistent.\n\nThanks,\nZoe",
            ago: 25,
            seen: false,
            flagged: true,
            attachment: Some(("colour-key.png", Attach::Png)),
            reply_to_index: None,
        },
        Mail {
            folder: "INBOX",
            from: ("Sam Okafor", "sam@studio.example"),
            to: &[("Zoe Park", "zoe@studio.example"), ME],
            cc: &[],
            subject: "Re: Storyboard review on Thursday",
            body: "Works for me. I'll book the small screening room from 2 pm and set up the projector.\n\nSam",
            ago: 12,
            seen: false,
            flagged: false,
            attachment: None,
            reply_to_index: Some(0),
        },
        Mail {
            folder: "INBOX",
            from: ("Print Shop", "orders@printshop.example"),
            to: &[ME],
            cc: &[],
            subject: "Your proof for the festival poster is ready",
            body: "Hello,\n\nThe proof for order 4471 (festival poster, A2, matte) is ready for your approval. Reply to this message with any changes, or with 'approved' and we will start printing tomorrow morning.\n\nPrint Shop",
            ago: 60 * 5,
            seen: true,
            flagged: false,
            attachment: Some(("proof-notes.txt", Attach::Text("Bleed: 3 mm\nColour profile: coated\nPaper: 170 gsm matte\n"))),
            reply_to_index: None,
        },
        Mail {
            folder: "INBOX",
            from: ("Ana Lima", "ana@soundlab.example"),
            to: &[ME],
            cc: &[],
            subject: "Sound mix, version 3",
            body: "Riley,\n\nVersion 3 of the mix is up on the shared drive. I lowered the music under the dialogue in the café scene and added the rain ambience you asked for.\n\nLet me know what you think!\nAna",
            ago: 60 * 26,
            seen: false,
            flagged: false,
            attachment: None,
            reply_to_index: None,
        },
        Mail {
            folder: "INBOX",
            from: ("Community Gallery", "hello@gallery.example"),
            to: &[ME],
            cc: &[],
            subject: "Open call: winter group show",
            body: "Dear artists,\n\nOur winter group show is open for submissions until the end of the month. Paintings, prints, photography and short films are all welcome. Each artist may submit up to three works.\n\nSee you at the opening!",
            ago: 60 * 24 * 3,
            seen: true,
            flagged: false,
            attachment: None,
            reply_to_index: None,
        },
        Mail {
            folder: "INBOX",
            from: ("Jun Watanabe", "jun@studio.example"),
            to: &[ME],
            cc: &[],
            subject: "Character turnarounds",
            body: "Here are the first turnarounds for the fox. Front, side and back are done; the three-quarter view needs another pass.\n\nJun",
            ago: 60 * 24 * 6,
            seen: true,
            flagged: false,
            attachment: Some(("fox-turnaround.png", Attach::Png)),
            reply_to_index: None,
        },
        Mail {
            folder: "INBOX",
            from: ("Studio Ops", "ops@studio.example"),
            to: &[ME],
            cc: &[],
            subject: "Render farm maintenance this weekend",
            body: "The render farm will be offline from Saturday 08:00 to Sunday 20:00 for upgrades. Please queue long renders before Friday evening.",
            ago: 60 * 24 * 12,
            seen: true,
            flagged: false,
            attachment: None,
            reply_to_index: None,
        },
        Mail {
            folder: "Sent",
            from: ME,
            to: &[("Print Shop", "orders@printshop.example")],
            cc: &[],
            subject: "Festival poster order",
            body: "Hello,\n\nPlease find the print-ready file for the festival poster attached. We need 40 copies in A2.\n\nBest,\nRiley",
            ago: 60 * 24 * 2,
            seen: true,
            flagged: false,
            attachment: None,
            reply_to_index: None,
        },
        Mail {
            folder: "Archive",
            from: ("Zoe Park", "zoe@studio.example"),
            to: &[ME],
            cc: &[],
            subject: "Kick-off notes",
            body: "Notes from the kick-off: running time 8 minutes, hand-painted backgrounds, festival deadline in March.",
            ago: 60 * 24 * 40,
            seen: true,
            flagged: false,
            attachment: None,
            reply_to_index: None,
        },
        Mail {
            folder: "Projects/Short film",
            from: ("Sam Okafor", "sam@studio.example"),
            to: &[ME],
            cc: &[],
            subject: "Shot list v2",
            body: "Updated shot list attached in the body below:\n\n1. Wide: rooftop at dusk\n2. Close: fox ears twitch\n3. Insert: lantern flickers\n",
            ago: 60 * 24 * 9,
            seen: true,
            flagged: true,
            attachment: None,
            reply_to_index: None,
        },
        Mail {
            folder: "Junk",
            from: ("Prize Desk", "win@prizes.example"),
            to: &[ME],
            cc: &[],
            subject: "You may have won a camera",
            body: "Click to claim your prize. (This is a sample junk message: MailCraft never loads links or remote images on its own.)",
            ago: 60 * 24 * 4,
            seen: false,
            flagged: false,
            attachment: None,
            reply_to_index: None,
        },
    ]
}

fn list(v: &[(&str, &str)]) -> Vec<EmailAddress> {
    v.iter().map(|(n, e)| EmailAddress::new(*n, *e)).collect()
}

/// Fill `store` with the sample account if it holds no accounts yet. Returns whether it did.
pub fn seed_if_empty(store: &Store, now: i64) -> Result<bool, StoreError> {
    if !store.load_accounts()?.is_empty() {
        return Ok(false);
    }
    let account = AccountConfig {
        id: SAMPLE_ACCOUNT.into(),
        name: ME.0.into(),
        email: ME.1.into(),
        username: ME.1.into(),
        signature: "Riley Moreno · Studio Lantern".into(),
        ..Default::default()
    };
    let folders: Vec<Folder> = [
        ("INBOX", FolderRole::Inbox),
        ("Drafts", FolderRole::Drafts),
        ("Sent", FolderRole::Sent),
        ("Archive", FolderRole::Archive),
        ("Junk", FolderRole::Junk),
        ("Trash", FolderRole::Trash),
        ("Projects", FolderRole::Other),
        ("Projects/Short film", FolderRole::Other),
    ]
    .into_iter()
    .map(|(p, role)| Folder { account: SAMPLE_ACCOUNT.into(), path: p.into(), delimiter: Some('/'), role, selectable: true, unread: 0, total: 0 })
    .collect();

    let mut states: Vec<(String, FolderState)> =
        folders.iter().map(|f| (f.path.clone(), FolderState { validity: 1, uid_next: 1, ..Default::default() })).collect();
    let mut ids: Vec<String> = Vec::new();
    for (i, m) in mails().into_iter().enumerate() {
        let date = now - m.ago * 60;
        let id = format!("sample-{i}@studio.example");
        let (in_reply_to, references) = match m.reply_to_index.and_then(|j| ids.get(j)) {
            Some(parent) => (parent.clone(), vec![parent.clone()]),
            None => (String::new(), Vec::new()),
        };
        let attachments = match m.attachment {
            Some((name, Attach::Png)) => vec![OutAttachment { name: name.into(), mime: "image/png".into(), data: sample_png() }],
            Some((name, Attach::Text(t))) => vec![OutAttachment { name: name.into(), mime: "text/plain".into(), data: t.as_bytes().to_vec() }],
            None => Vec::new(),
        };
        let d = Draft {
            to: list(m.to),
            cc: list(m.cc),
            bcc: Vec::new(),
            subject: m.subject.into(),
            body: m.body.into(),
            in_reply_to,
            references,
            attachments,
        };
        let mut raw = mime::build(&d, &EmailAddress::new(m.from.0, m.from.1), date, &id)
            .map_err(|e| StoreError::Io { path: "sample".into(), source: std::io::Error::other(e) })?;
        // The newsletter and the prize mail come from mailing lists (they land in Other).
        if let Some(list) = match m.from.1 {
            "hello@gallery.example" => Some("news.gallery.example"),
            "win@prizes.example" => Some("offers.prizes.example"),
            _ => None,
        } {
            let domain = list.split_once('.').map_or(list, |(_, d)| d);
            let head = format!("List-Id: <{list}>\r\nList-Unsubscribe: <mailto:leave@{domain}>\r\n");
            raw = [head.as_bytes(), &raw].concat();
        }
        ids.push(id);
        let Some((path, st)) = states.iter_mut().find(|(p, _)| p == m.folder) else { continue };
        let uid = st.uid_next;
        st.uid_next += 1;
        let key = MessageKey { account: SAMPLE_ACCOUNT.into(), folder: path.clone(), validity: 1, uid };
        let flags = Flags { seen: m.seen, flagged: m.flagged, ..Default::default() };
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        store.write_body(&key, &raw)?;
        st.messages.push(mime::summary_from_full(&raw, key, flags, size, date));
    }
    let mut folders = folders;
    for (path, st) in &mut states {
        sort_newest_first(&mut st.messages);
        store.save_folder(SAMPLE_ACCOUNT, path, st)?;
        if let Some(f) = folders.iter_mut().find(|f| &f.path == path) {
            f.unread = st.unread();
            f.total = u32::try_from(st.messages.len()).unwrap_or(0);
        }
    }
    store.save_folders(SAMPLE_ACCOUNT, &folders)?;
    store.save_accounts(&[account])?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_is_well_formed() {
        let png = sample_png();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }
}
