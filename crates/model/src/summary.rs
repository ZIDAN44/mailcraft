//! Messages as the list shows them: the key that names one, its flags and keywords, and its
//! summary (what the header says, its size and a preview of the text).

use serde::{Deserialize, Serialize};

use crate::{EmailAddress, organize, thread};

/// Identifies one message: the account, the folder, and the folder's UIDVALIDITY and the message's
/// UID (RFC 3501 §2.3.1.1: the pair is unique and never reused).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MessageKey {
    pub account: String,
    pub folder: String,
    pub validity: u32,
    pub uid: u32,
}

impl MessageKey {
    /// `account/validity/uid/folder`: a compact string form for agents and the CLI.
    pub fn to_id(&self) -> String {
        format!("{}:{}:{}:{}", self.account, self.validity, self.uid, self.folder)
    }

    pub fn from_id(s: &str) -> Option<MessageKey> {
        let mut it = s.splitn(4, ':');
        let account = it.next()?.to_string();
        let validity = it.next()?.parse().ok()?;
        let uid = it.next()?.parse().ok()?;
        let folder = it.next()?.to_string();
        (!account.is_empty() && !folder.is_empty()).then_some(MessageKey { account, folder, validity, uid })
    }

    /// The heap its account and folder take: each key holds its own copy of both.
    pub fn heap_bytes(&self) -> usize {
        string_room(&self.account).saturating_add(string_room(&self.folder))
    }
}

/// The heap an allocation of `n` bytes takes: rounded up to the 16 bytes an allocator hands
/// out (its own bookkeeping beside each isn't counted); none for none.
pub fn heap_room(n: usize) -> usize {
    n.checked_next_multiple_of(16).unwrap_or(n)
}

/// The heap a string takes: its capacity, not its length (a `&str` has none).
#[allow(clippy::ptr_arg)]
pub fn string_room(s: &String) -> usize {
    heap_room(s.capacity())
}

/// The heap a list of `T` takes: its room (its capacity, a slice has none), then what each
/// item holds.
#[allow(clippy::ptr_arg)]
pub fn list_room<T>(v: &Vec<T>, each: impl Fn(&T) -> usize) -> usize {
    let room = heap_room(v.capacity().saturating_mul(std::mem::size_of::<T>()));
    v.iter().fold(room, |n, x| n.saturating_add(each(x)))
}

/// System flags (RFC 3501 §2.3.2). Keywords other than these are kept as strings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    #[serde(default)]
    pub seen: bool,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub answered: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub deleted: bool,
    /// `$Forwarded` (RFC 5550).
    #[serde(default)]
    pub forwarded: bool,
    /// Other keywords (RFC 3501 §2.3.2): categories (`$label1`, `work`…), `$Pinned`, `$Junk`…
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
}

/// The keyword that pins a message to the top of its folder.
pub const PINNED: &str = "$Pinned";

/// Whether `k` can be sent as an IMAP keyword: an atom (no spaces, brackets, wildcards, quotes or
/// controls), not a system flag, at most 64 characters.
pub fn is_keyword(k: &str) -> bool {
    !k.is_empty() && k.len() <= 64 && !k.starts_with('\\') && k.chars().all(|c| c.is_ascii_graphic() && !"(){%*\"\\]".contains(c))
}

/// The keyword for a category name, as Thunderbird makes a tag's (`To do` → `to=20do`,
/// `Rückfrage` → `r=c3=bcckfrage`): [`organize::keywords::tag_key`].
pub fn keyword_for(name: &str) -> String {
    organize::keywords::tag_key(name.trim())
}

impl Flags {
    pub fn from_imap<'a>(flags: impl IntoIterator<Item = &'a str>) -> Flags {
        let mut f = Flags::default();
        for fl in flags {
            match fl.to_ascii_lowercase().as_str() {
                "\\seen" => f.seen = true,
                "\\flagged" => f.flagged = true,
                "\\answered" => f.answered = true,
                "\\draft" => f.draft = true,
                "\\deleted" => f.deleted = true,
                "$forwarded" => f.forwarded = true,
                other
                    if !other.starts_with('\\')
                        && is_keyword(fl)
                        && f.keywords.len() < 32
                        && !f.keywords.iter().any(|k| k.eq_ignore_ascii_case(fl)) =>
                {
                    f.keywords.push(fl.to_string());
                }
                _ => {}
            }
        }
        f.keywords.sort_by_key(|k| k.to_ascii_lowercase());
        f
    }

    pub fn has(&self, keyword: &str) -> bool {
        self.keywords.iter().any(|k| k.eq_ignore_ascii_case(keyword))
    }

    /// Add or remove a keyword.
    pub fn set_keyword(&mut self, keyword: &str, on: bool) {
        self.keywords.retain(|k| !k.eq_ignore_ascii_case(keyword));
        if on {
            self.keywords.push(keyword.to_string());
            self.keywords.sort_by_key(|k| k.to_ascii_lowercase());
        }
    }

    pub fn pinned(&self) -> bool {
        self.has(PINNED)
    }

    /// The heap its keywords take.
    pub fn heap_bytes(&self) -> usize {
        list_room(&self.keywords, string_room)
    }

    /// The flags as an IMAP flag list (without parentheses).
    pub fn to_imap(&self) -> String {
        let mut v = Vec::new();
        for (on, name) in [
            (self.seen, "\\Seen"),
            (self.flagged, "\\Flagged"),
            (self.answered, "\\Answered"),
            (self.draft, "\\Draft"),
            (self.deleted, "\\Deleted"),
            (self.forwarded, "$Forwarded"),
        ] {
            if on {
                v.push(name);
            }
        }
        v.extend(self.keywords.iter().map(String::as_str));
        v.join(" ")
    }
}

/// A flag a command can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Flag {
    Seen,
    Flagged,
    Answered,
    Forwarded,
    Deleted,
}

impl Flag {
    pub fn imap(self) -> &'static str {
        match self {
            Flag::Seen => "\\Seen",
            Flag::Flagged => "\\Flagged",
            Flag::Answered => "\\Answered",
            Flag::Forwarded => "$Forwarded",
            Flag::Deleted => "\\Deleted",
        }
    }

    pub fn set(self, flags: &mut Flags, on: bool) {
        match self {
            Flag::Seen => flags.seen = on,
            Flag::Flagged => flags.flagged = on,
            Flag::Answered => flags.answered = on,
            Flag::Forwarded => flags.forwarded = on,
            Flag::Deleted => flags.deleted = on,
        }
    }

    pub fn get(self, flags: &Flags) -> bool {
        match self {
            Flag::Seen => flags.seen,
            Flag::Flagged => flags.flagged,
            Flag::Answered => flags.answered,
            Flag::Forwarded => flags.forwarded,
            Flag::Deleted => flags.deleted,
        }
    }
}

/// What the message list shows for a message: everything from its header, its flags and size,
/// and a short preview of the text once the body has been fetched.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub key: MessageKey,
    /// `Message-ID` without angle brackets.
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub in_reply_to: String,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub from: EmailAddress,
    #[serde(default)]
    pub to: Vec<EmailAddress>,
    #[serde(default)]
    pub cc: Vec<EmailAddress>,
    /// Unix seconds (UTC) from the `Date` header, or the server's INTERNALDATE.
    #[serde(default)]
    pub date: i64,
    /// Unix seconds (UTC) it was delivered, when that's known and isn't `date` (a data file's
    /// delivery time); 0 otherwise. The list places a message by [`Summary::when`].
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub received: i64,
    #[serde(default)]
    pub size: u32,
    #[serde(default)]
    pub flags: Flags,
    #[serde(default)]
    pub has_attachments: bool,
    #[serde(default)]
    pub preview: String,
    /// CONDSTORE modification sequence of the last flag change seen (RFC 7162).
    #[serde(default)]
    pub modseq: u64,
    /// Sent in bulk: a mailing list, newsletter or automatic message (`List-Id`,
    /// `List-Unsubscribe`, `Precedence: bulk`, `Auto-Submitted`). Such mail goes to Other.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bulk: bool,
    /// How important the sender marked it: 1 high, -1 low, 0 normal (`Importance`, `X-Priority`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub importance: i8,
    /// Signed or encrypted with S/MIME, as its header says (a claim until the message is opened
    /// and checked; an encrypted one may be signed inside too).
    #[serde(default, skip_serializing_if = "Smime::is_none")]
    pub smime: Smime,
    /// Outlook's conversation index (`Thread-Index`, MS-OXOMSG 2.2.1.3) as hashes, so threading
    /// never decodes it: the whole index, the index of the message it answers, and the
    /// conversation's; 0 for none ([`thread::index_keys`]).
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub thread_own: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub thread_parent: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub thread_conversation: u64,
}

/// What a message's header says protects it (S/MIME, RFC 8551 §3.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Smime {
    #[default]
    None,
    Signed,
    Encrypted,
}

impl Smime {
    pub fn is_none(&self) -> bool {
        *self == Smime::None
    }

    /// From a message's top-level `Content-Type`: its type and subtype, and its `protocol`,
    /// `smime-type` and `name` parameters.
    pub fn from_content_type(kind: &str, subtype: &str, protocol: Option<&str>, smime_type: Option<&str>, name: Option<&str>) -> Smime {
        let (kind, subtype) = (kind.to_ascii_lowercase(), subtype.to_ascii_lowercase());
        let lower = |v: Option<&str>| v.map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
        if kind == "multipart" && subtype == "signed" {
            return match lower(protocol).as_str() {
                "application/pkcs7-signature" | "application/x-pkcs7-signature" => Smime::Signed,
                _ => Smime::None,
            };
        }
        let p7m = lower(name).ends_with(".p7m");
        if kind == "application" && (subtype == "pkcs7-mime" || subtype == "x-pkcs7-mime" || (subtype == "octet-stream" && p7m)) {
            return if lower(smime_type) == "signed-data" { Smime::Signed } else { Smime::Encrypted };
        }
        Smime::None
    }
}

fn is_zero(n: &i8) -> bool {
    *n == 0
}

fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

impl Summary {
    /// When the list places the message: when it was received, else its date.
    pub fn when(&self) -> i64 {
        if self.received > 0 { self.received } else { self.date }
    }

    /// What a follow-up date is kept under: the Message-ID, or (without one) the message's place.
    pub fn follow_up_id(&self) -> String {
        if self.message_id.trim().is_empty() { self.key.to_id() } else { self.message_id.trim().to_string() }
    }

    /// Keep a conversation index as its [`thread::index_keys`].
    pub fn set_thread_index(&mut self, index: &[u8]) {
        let k = thread::index_keys(index);
        (self.thread_own, self.thread_parent, self.thread_conversation) = (k.own, k.parent, k.conversation);
    }

    /// The subject with reply and forward prefixes removed, for conversation grouping.
    pub fn base_subject(&self) -> String {
        thread::base_subject(&self.subject)
    }

    /// The heap it holds, besides its own `size_of::<Summary>()`: every string and list in it,
    /// each rounded as [`heap_room`]. What the lists kept in memory cost is counted with it.
    pub fn heap_bytes(&self) -> usize {
        [
            self.key.heap_bytes(),
            string_room(&self.message_id),
            string_room(&self.in_reply_to),
            list_room(&self.references, string_room),
            string_room(&self.subject),
            self.from.heap_bytes(),
            list_room(&self.to, EmailAddress::heap_bytes),
            list_room(&self.cc, EmailAddress::heap_bytes),
            self.flags.heap_bytes(),
            string_room(&self.preview),
        ]
        .into_iter()
        .fold(0usize, usize::saturating_add)
    }
}

/// A summary as the lists in memory hold it: shared, not copied, between a folder's list in the
/// cache, the list shown and a search's finds (each copy of a folder's or a search's tens of
/// thousands was most of what showing them cost, and twice their memory). Changed in place with
/// `Arc::make_mut`, which copies one only while another list shares it: what the cache and the
/// list hold of a message part as they did when each held its own copy. Kept on disk as the
/// summary itself (serde's `rc`).
pub type SharedSummary = std::sync::Arc<Summary>;

/// Sort newest first ([`Summary::when`]), with the key as a tie-breaker so the order is stable.
pub fn sort_newest_first<S: std::borrow::Borrow<Summary>>(list: &mut [S]) {
    list.sort_by(|a, b| {
        let (a, b): (&Summary, &Summary) = (a.borrow(), b.borrow());
        b.when().cmp(&a.when()).then_with(|| b.key.cmp(&a.key))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_key_round_trips_through_its_id() {
        let k = MessageKey { account: "a1".into(), folder: "INBOX/Sub:folder".into(), validity: 7, uid: 42 };
        assert_eq!(MessageKey::from_id(&k.to_id()), Some(k));
        assert_eq!(MessageKey::from_id("junk"), None);
        assert_eq!(MessageKey::from_id("a:x:1:F"), None);
    }

    #[test]
    fn flags_round_trip() {
        let f = Flags::from_imap(["\\Seen", "\\FLAGGED", "$Forwarded", "$Junk", "\\Recent", "work", "WORK"]);
        assert!(f.seen && f.flagged && f.forwarded && !f.answered);
        assert_eq!(f.keywords, vec!["$Junk".to_string(), "work".to_string()]);
        assert_eq!(f.to_imap(), "\\Seen \\Flagged $Forwarded $Junk work");
        let mut f = Flags::default();
        f.set_keyword(PINNED, true);
        assert!(f.pinned() && f.has("$pinned"));
        f.set_keyword("$PINNED", false);
        assert!(!f.pinned());
    }

    #[test]
    fn keywords() {
        assert!(is_keyword("$label1") && is_keyword("to_do"));
        assert!(!is_keyword("two words") && !is_keyword("\\Seen") && !is_keyword("a(b") && !is_keyword(""));
        assert_eq!(keyword_for("To do"), "to=20do");
        assert_eq!(keyword_for("Café"), "caf=c3=a9");
        assert_eq!(keyword_for(" Café "), keyword_for("café"));
        assert!(is_keyword(&keyword_for("Projects: Film!")));
    }

    #[test]
    fn received_places_a_message() {
        let at = |uid: u32, date: i64, received: i64| Summary { key: MessageKey { uid, ..Default::default() }, date, received, ..Default::default() };
        // Sent before the others, received after them.
        let mut list = vec![at(1, 300, 0), at(2, 100, 500), at(3, 200, 0)];
        sort_newest_first(&mut list);
        assert_eq!(list.iter().map(|m| m.key.uid).collect::<Vec<_>>(), vec![2, 1, 3]);
        assert_eq!((list[0].when(), list[1].when()), (500, 300));
    }

    #[test]
    fn heap_bytes_counts_every_string_and_list() {
        assert_eq!(Summary::default().heap_bytes(), 0);
        assert_eq!((heap_room(0), heap_room(1), heap_room(16), heap_room(17)), (0, 16, 16, 32));
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 2 };
        assert_eq!(key.heap_bytes(), 32);
        let mut m = Summary { key, subject: "x".repeat(20), preview: String::with_capacity(100), ..Default::default() };
        assert_eq!(m.heap_bytes(), 32 + 32 + 112, "capacity counted, not length");
        // A list's own room, then each item's.
        m.to = vec![EmailAddress::new("Zoe", "zoe@example.org")];
        m.references = vec!["a@b".into(), "c@d".into()];
        m.flags.keywords = vec!["$label1".into()];
        let to = heap_room(std::mem::size_of::<EmailAddress>()) + 16 + 16;
        let refs = heap_room(2 * std::mem::size_of::<String>()) + 32;
        let keywords = heap_room(std::mem::size_of::<String>()) + 16;
        assert_eq!(m.heap_bytes(), 32 + 32 + 112 + to + refs + keywords);
    }

    #[test]
    fn lists_from_before_the_new_fields_read() {
        let old = r#"{"key":{"account":"a","folder":"INBOX","validity":1,"uid":2},"subject":"s","date":7}"#;
        let m: Summary = serde_json::from_str(old).unwrap();
        assert_eq!((m.received, m.thread_own, m.thread_parent, m.thread_conversation, m.when()), (0, 0, 0, 0, 7));
        // Unset, they aren't written; set, they come back.
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("received") && !json.contains("thread_"), "{json}");
        let m = Summary { received: 9, thread_own: u64::MAX, thread_parent: 1, thread_conversation: 2, ..m };
        let back: Summary = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}
