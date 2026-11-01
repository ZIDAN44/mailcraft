//! A big generated mailbox, for the tests and benchmarks that need one the size of a real
//! file's (search.md §11): by default 50,000 messages in 40 folders, the Inbox 12,000 of them.
//! Every word is made up here (syllables chosen by a fixed xorshift sequence, so each run makes
//! the same mailbox); addresses use the reserved `example` domains (RFC 2606). Nothing in it
//! reads the clock: `now` is the caller's.
//!
//! Besides the made-up words, some messages carry what searches are measured on, counted as
//! they're made ([`Truth`]): a rare word, a common phrase, a common sender's domain, a word with
//! accents, CJK text, a word past the first 50,000 characters of a long text, files and pictures
//! shown in the text (which aren't files). Seven in ten are read.

use mailcraft_model::{AccountConfig, EmailAddress, Flags, Folder, FolderRole, MessageKey, Summary, sort_newest_first};
use mailcraft_store::{FolderState, Store, StoreError};

/// The account the mailbox is made in.
pub const ACCOUNT: &str = "big";
/// A word in about one message in a hundred.
pub const RARE: &str = "quokka";
/// A phrase in a third of the messages.
pub const PHRASE: &str = "thank you for the update";
/// The domain seven in ten senders write from.
pub const DOMAIN: &str = "studio.example";
/// A word written with accents in one message in fifty (found typed without them).
pub const ACCENTED: &str = "Zürich";
/// CJK text in one message in two hundred.
pub const CJK: &str = "東京";
/// A word that only comes after 120,000 characters of text, in one message in a thousand.
pub const FAR: &str = "farpoint";

/// How big a mailbox to make.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    /// Folders, the Inbox first.
    pub folders: usize,
    /// Messages in all.
    pub messages: u32,
    /// Of those, in the Inbox (the rest shared out over the other folders).
    pub inbox: u32,
}

impl Default for Shape {
    fn default() -> Shape {
        Shape { folders: 40, messages: 50_000, inbox: 12_000 }
    }
}

/// What the mailbox made holds: what a search for each should find.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Truth {
    pub messages: usize,
    pub read: usize,
    pub unread: usize,
    /// With [`RARE`] in the text.
    pub rare: usize,
    /// With [`PHRASE`] in the text.
    pub phrase: usize,
    /// From [`DOMAIN`].
    pub domain: usize,
    /// Carrying a file (a PDF).
    pub files: usize,
    /// Showing a picture in their text, and no file (their summaries say they carry files, as
    /// an old import's did).
    pub pictures: usize,
    /// With [`ACCENTED`].
    pub accented: usize,
    /// With [`CJK`].
    pub cjk: usize,
    /// With [`FAR`], past the first 120,000 characters.
    pub far: usize,
}

/// xorshift64*: the same words every run, not randomness.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number below `n` (0 for 0).
    fn below(&mut self, n: u64) -> u64 {
        self.next().checked_rem(n).unwrap_or(0)
    }
}

const CONSONANTS: &[u8] = b"bdfgklmnprstvz";
const VOWELS: &[u8] = b"aeiou";
/// Made-up words to choose from: each six letters, so none starts another.
const WORDS: u64 = 2048;

/// Made-up word `i`: three syllables, different for each `i` below 343,000.
fn word(i: u64) -> String {
    let (c, v) = (CONSONANTS.len() as u64, VOWELS.len() as u64);
    // A multiplier coprime with 14³·5³ spreads neighbours apart.
    let mut x = i.wrapping_mul(7919) % (c * v).pow(3);
    let mut w = String::with_capacity(6);
    for _ in 0..3 {
        w.push(char::from(CONSONANTS.get(usize::try_from(x % c).unwrap_or(0)).copied().unwrap_or(b'b')));
        x /= c;
        w.push(char::from(VOWELS.get(usize::try_from(x % v).unwrap_or(0)).copied().unwrap_or(b'a')));
        x /= v;
    }
    w
}

/// A word, the first ones far more often than the last (as in mail).
fn common_word(r: &mut Rng) -> String {
    let (a, b) = (r.below(WORDS), r.below(WORDS));
    word(a.saturating_mul(b) / WORDS)
}

fn words(r: &mut Rng, n: u64) -> String {
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(common_word(r));
    }
    out.join(" ")
}

/// A few hundred bytes standing for a PDF: what matters is its name and type.
const PDF_BASE64: &str = "JVBERi0xLjQKJcfsj6IKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyA+PgplbmRvYmoKdHJhaWxlcgo8PCAvUm9vdCAxIDAgUiA+PgolJUVPRgo=";
/// A tiny PNG's start: shown in the text, never decoded here.
const PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// Message `i` (0-based) of the mailbox: its source and summary (key and size filled by the
/// caller), and what it counts towards.
fn message(i: u32, now: i64, t: &mut Truth) -> (String, Summary) {
    let mut r = Rng(u64::from(i).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let n = 20 + r.below(40);
    let mut text = words(&mut r, n);
    if i.is_multiple_of(3) {
        text.push_str(" Thank you for the update.");
        t.phrase += 1;
    }
    if i.is_multiple_of(97) {
        text.push_str(&format!(" {RARE} {}", common_word(&mut r)));
        t.rare += 1;
    }
    if i.is_multiple_of(50) {
        text.push_str(&format!(" See you in {ACCENTED}."));
        t.accented += 1;
    }
    if i.is_multiple_of(200) {
        text.push_str(&format!(" {CJK} "));
        t.cjk += 1;
    }
    if i % 1000 == 999 {
        text.push_str(&" lorem".repeat(20_000));
        text.push_str(&format!(" {FAR}."));
        t.far += 1;
    }
    let (sender, domain) = if (i / 7) % 10 < 7 {
        t.domain += 1;
        (format!("Member {}", i % 37), DOMAIN.to_string())
    } else {
        (format!("Guest {}", i % 23), format!("partner{}.example", i % 9))
    };
    let from = EmailAddress::new(sender.clone(), format!("{}@{domain}", sender.to_lowercase().replace(' ', "")));
    let to = EmailAddress::new("Me", "me@home.example");
    let n = 3 + r.below(4);
    let subject = words(&mut r, n);
    let date = now.saturating_sub(i64::from(i).saturating_mul(1800)).saturating_sub(i64::try_from(r.below(600)).unwrap_or(0));
    let head = format!(
        "From: {} <{}>\r\nTo: Me <me@home.example>\r\nSubject: {subject}\r\nMessage-ID: <big-{i}@home.example>\r\nMIME-Version: 1.0\r\n",
        from.name, from.email
    );
    let (raw, files) = if i.is_multiple_of(25) {
        t.files += 1;
        let raw = format!(
            "{head}Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n--b\r\nContent-Type: application/pdf; name=\"report-{i}.pdf\"\r\nContent-Disposition: attachment; filename=\"report-{i}.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{PDF_BASE64}\r\n--b--\r\n"
        );
        (raw, true)
    } else if i % 40 == 7 {
        t.pictures += 1;
        let raw = format!(
            "{head}Content-Type: multipart/related; boundary=b\r\n\r\n--b\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>{text}</p><img src=\"cid:pic-{i}@home.example\">\r\n--b\r\nContent-Type: image/png\r\nContent-ID: <pic-{i}@home.example>\r\nContent-Disposition: inline; filename=\"picture-{i}.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{PNG_BASE64}\r\n--b--\r\n"
        );
        (raw, true)
    } else {
        (format!("{head}Content-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n"), false)
    };
    let seen = i % 10 < 7;
    if seen {
        t.read += 1;
    } else {
        t.unread += 1;
    }
    t.messages += 1;
    let summary = Summary {
        message_id: format!("big-{i}@home.example"),
        subject,
        from,
        to: vec![to],
        date,
        flags: Flags { seen, flagged: i.is_multiple_of(13), ..Default::default() },
        has_attachments: files,
        preview: text.chars().take(120).collect(),
        ..Default::default()
    };
    (raw, summary)
}

/// Make the mailbox in `store` (its account and folders added to what's there): what it holds.
pub fn generate(store: &Store, now: i64, shape: Shape) -> Result<Truth, StoreError> {
    let mut accounts = store.load_accounts()?;
    accounts.retain(|a| a.id != ACCOUNT);
    accounts.push(AccountConfig { id: ACCOUNT.into(), name: "Me".into(), email: "me@home.example".into(), ..Default::default() });
    store.save_accounts(&accounts)?;
    let folders: Vec<Folder> = (0..shape.folders.max(1))
        .map(|i| Folder {
            account: ACCOUNT.into(),
            path: if i == 0 { "INBOX".into() } else { format!("Folder {i:02}") },
            delimiter: Some('/'),
            role: if i == 0 { FolderRole::Inbox } else { FolderRole::Other },
            selectable: true,
            ..Default::default()
        })
        .collect();
    store.save_folders(ACCOUNT, &folders)?;
    let mut states: Vec<FolderState> = folders.iter().map(|_| FolderState { validity: 1, uid_next: 1, ..Default::default() }).collect();
    let others = folders.len().saturating_sub(1).max(1);
    let mut t = Truth::default();
    for i in 0..shape.messages {
        let fi = if i < shape.inbox || folders.len() < 2 { 0 } else { 1 + usize::try_from(i - shape.inbox).unwrap_or(0) % others };
        let (Some(folder), Some(st)) = (folders.get(fi), states.get_mut(fi)) else { continue };
        let (raw, mut summary) = message(i, now, &mut t);
        summary.key = MessageKey { account: ACCOUNT.into(), folder: folder.path.clone(), validity: 1, uid: st.uid_next };
        summary.size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        st.uid_next = st.uid_next.saturating_add(1);
        store.write_body(&summary.key, raw.as_bytes())?;
        st.messages.push(summary);
    }
    for (f, mut st) in folders.iter().zip(states) {
        sort_newest_first(&mut st.messages);
        store.save_folder(ACCOUNT, &f.path, &st)?;
    }
    Ok(t)
}

/// Bring `account`'s search index up to its mail in one pass on this thread, as the indexer
/// does: the generation written.
pub fn index(store: &Store, account: &str) -> Result<u64, StoreError> {
    let mut p = mailcraft_store::search::Pass::start(store, account, &[])?;
    let adds = p.adds().clone();
    for (i, m) in adds.iter().enumerate() {
        let doc = store.read_body(&m.key).map(|raw| mailcraft_mime::index_doc(&raw));
        p.add(store, i, doc.as_ref())?;
    }
    p.finish(store, true)?;
    Ok(p.generation())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn made_up_words_never_start_one_another() {
        let all: std::collections::HashSet<String> = (0..WORDS).map(word).collect();
        assert_eq!(all.len(), WORDS as usize, "each different");
        assert!(all.iter().all(|w| w.len() == 6 && w.is_ascii()));
        assert!(!all.contains(RARE) && !all.contains(FAR));
    }

    #[test]
    fn the_same_mailbox_each_time() {
        let made = || {
            let store = Store::memory();
            let t = generate(&store, 1_800_000_000, Shape { folders: 3, messages: 400, inbox: 200 }).unwrap();
            let inbox = store.load_folder(ACCOUNT, "INBOX").unwrap();
            (
                t,
                inbox.messages.iter().map(|m| (m.subject.clone(), m.date)).collect::<Vec<_>>(),
                store.load_folder(ACCOUNT, "Folder 02").unwrap().messages.len(),
            )
        };
        let (t, inbox, other) = made();
        assert_eq!((inbox.len(), other), (200, 100));
        assert_eq!(made().1, inbox);
        assert_eq!((t.messages, t.read, t.unread, t.files), (400, 280, 120, 16));
    }
}
