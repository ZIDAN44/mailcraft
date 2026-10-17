//! MailCraft data model.
//!
//! Pure data and algorithms shared by every layer: accounts and servers, folders (with their
//! RFC 6154 special-use roles), message keys and summaries, flags, the jobs the engine asks the
//! network layer to run and the events it gets back, conversation threading (JWZ), search queries
//! and Outlook-style date grouping. Nothing here does I/O, so it builds for wasm.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod addr;
pub mod contacts;
pub mod dates;
pub mod ical;
pub mod job;
pub mod organize;
pub mod pim;
pub mod search;
pub mod thread;
pub mod utf7;
pub mod vcard;

use serde::{Deserialize, Serialize};

pub use addr::EmailAddress;
pub use job::{Event, Job, Op, SyncStatus};

/// How a connection to a server is secured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// TLS from the first byte (IMAPS 993, SMTPS 465). The default: RFC 8314 prefers it.
    #[default]
    Tls,
    /// Plain text upgraded with STARTTLS (IMAP 143, submission 587).
    StartTls,
    /// No encryption. Only for local test servers; the UI warns.
    None,
}

impl Security {
    pub fn code(self) -> &'static str {
        match self {
            Security::Tls => "tls",
            Security::StartTls => "starttls",
            Security::None => "none",
        }
    }

    pub fn from_code(s: &str) -> Option<Security> {
        match s.trim().to_ascii_lowercase().as_str() {
            "tls" | "ssl" | "ssl/tls" => Some(Security::Tls),
            "starttls" => Some(Security::StartTls),
            "none" | "plain" => Some(Security::None),
            _ => None,
        }
    }
}

/// One server endpoint.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub security: Security,
}

/// A mail account. Never holds the password: that lives in the system keychain.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountConfig {
    /// Stable local id (`a1`, `a2`…), used in message keys and cache paths.
    pub id: String,
    /// The name shown on mail you send.
    pub name: String,
    pub email: String,
    /// Login name; usually the email address.
    pub username: String,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    #[serde(default)]
    pub signature: String,
    /// The signature's look, as HTML (empty when it's plain text): bold, colours, sizes and
    /// links, as pasted from Outlook or set in the signature editor.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_html: String,
    /// The colour index the UI uses for this account's marker.
    #[serde(default)]
    pub color: u8,
    /// Signs in with OAuth through this provider (`google`, `microsoft`) instead of a password;
    /// the keychain then holds the refresh token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<String>,
    /// Syncs and sends over JMAP (RFC 8620/8621) through this session URL instead of IMAP and
    /// SMTP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jmap: Option<String>,
    /// Syncs and sends through Microsoft Graph (Exchange Online) at this API address instead of
    /// IMAP and SMTP; signs in with the account's OAuth token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
    /// Syncs and sends through Exchange Web Services at this address
    /// (`https://mail.example.com/EWS/Exchange.asmx`, on-premises Exchange) instead of IMAP and
    /// SMTP; signs in with the user name (`DOMAIN\user` or an address) and password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ews: Option<String>,
    /// Downloads mail from this POP3 server (RFC 1939) into folders kept on this computer, instead
    /// of syncing over IMAP; sends over `smtp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pop: Option<ServerConfig>,
    /// POP: remove mail from the server once it's downloaded (otherwise a copy stays there).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pop_delete: bool,
    /// POP, keeping a copy on the server: remove it this many days after it's downloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pop_keep_days: Option<u32>,
    /// POP, keeping a copy on the server: remove it once it's deleted here for good (from
    /// Deleted Items).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pop_delete_with_trash: bool,
    /// The address the OAuth provider says the browser signed in as (from its ID token), when it
    /// differs from `email`: an alias, or another account picked on the sign-in page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_user: Option<String>,
}

/// The EWS address of an account whose Exchange server Autodiscover finds when it connects.
pub const EWS_AUTODISCOVER: &str = "autodiscover";

/// Microsoft Graph's address, for Exchange Online accounts.
pub const GRAPH_URL: &str = "https://graph.microsoft.com/v1.0";

/// The OAuth provider that runs an IMAP server, when it's one MailCraft knows.
pub fn oauth_provider_for(imap_host: &str) -> Option<&'static str> {
    let h = imap_host.trim().to_ascii_lowercase();
    if h == "imap.gmail.com" || h == "imap.googlemail.com" {
        Some("google")
    } else if h == "outlook.office365.com" || h.ends_with(".outlook.com") || h == "imap-mail.outlook.com" {
        Some("microsoft")
    } else {
        None
    }
}

/// Microsoft's own addresses (Outlook.com, Hotmail, Live, MSN). Since September 2024 they
/// take no passwords from other mail apps over IMAP, POP or SMTP: only Microsoft sign-in.
pub fn microsoft_consumer(email: &str) -> bool {
    let domain = email.trim().rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()).unwrap_or_default();
    let name = domain.split('.').next().unwrap_or_default();
    matches!(name, "outlook" | "hotmail" | "live" | "msn" | "windowslive" | "passport") && domain.contains('.')
}

/// Why a Microsoft account signing in with a password can't connect (its status starts with this).
pub const MICROSOFT_NO_PASSWORDS: &str = "Microsoft doesn't accept passwords from mail apps for this account: it needs Sign in with Microsoft";

/// Why an account signed in with Microsoft can't open its mailbox over IMAP: Microsoft took the
/// sign-in, then its server said "User is authenticated but not connected". The status starts
/// with this, so the UI can offer what helps (signing in again, or Microsoft Graph).
pub const MICROSOFT_IMAP_CLOSED: &str = "Microsoft signed you in, but its mail server won't open this mailbox over IMAP";

/// An account that signs in to Microsoft's servers with a password rather than Microsoft
/// sign-in, which those servers refuse.
pub fn microsoft_password_account(a: &AccountConfig) -> bool {
    a.oauth.is_none()
        && a.graph.is_none()
        && a.ews.is_none()
        && (oauth_provider_for(&a.imap.host) == Some("microsoft") || microsoft_consumer(&a.email))
}

impl AccountConfig {
    /// Server settings guessed from an address, the way most hosts are laid out (`mail.<domain>`,
    /// IMAPS 993, SMTPS 465). The account dialog shows them for the user to correct.
    pub fn guess(email: &str) -> AccountConfig {
        let email = email.trim();
        let domain = email.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()).unwrap_or_default();
        let (imap_host, smtp_host) = match domain.as_str() {
            "gmail.com" | "googlemail.com" => ("imap.gmail.com".to_string(), "smtp.gmail.com".to_string()),
            _ if microsoft_consumer(email) => ("outlook.office365.com".to_string(), "smtp.office365.com".to_string()),
            "yahoo.com" => ("imap.mail.yahoo.com".to_string(), "smtp.mail.yahoo.com".to_string()),
            "icloud.com" | "me.com" => ("imap.mail.me.com".to_string(), "smtp.mail.me.com".to_string()),
            "fastmail.com" => ("imap.fastmail.com".to_string(), "smtp.fastmail.com".to_string()),
            "" => (String::new(), String::new()),
            d => (format!("mail.{d}"), format!("mail.{d}")),
        };
        let smtp = if smtp_host.ends_with("office365.com") || smtp_host.ends_with("me.com") {
            ServerConfig { host: smtp_host, port: 587, security: Security::StartTls }
        } else {
            ServerConfig { host: smtp_host, port: 465, security: Security::Tls }
        };
        AccountConfig {
            id: String::new(),
            name: email.split('@').next().unwrap_or_default().to_string(),
            email: email.to_string(),
            username: email.to_string(),
            imap: ServerConfig { host: imap_host, port: 993, security: Security::Tls },
            smtp,
            signature: String::new(),
            signature_html: String::new(),
            color: 0,
            oauth: None,
            jmap: None,
            graph: None,
            ews: None,
            pop: None,
            pop_delete: false,
            pop_keep_days: None,
            pop_delete_with_trash: false,
            oauth_user: None,
        }
    }

    /// Kept on this computer only, with no server (the sample mailbox).
    pub fn is_local(&self) -> bool {
        self.imap.host.trim().is_empty() && self.jmap.is_none() && self.graph.is_none() && self.ews.is_none() && self.pop.is_none()
    }

    /// Whether the account works through a web API (JMAP, Graph or EWS) rather than IMAP and
    /// SMTP.
    pub fn is_api(&self) -> bool {
        self.jmap.is_some() || self.graph.is_some() || self.ews.is_some()
    }
}

/// What a folder is for (RFC 6154 special-use attributes, or the usual names when a server has
/// none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderRole {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    #[default]
    Other,
}

impl FolderRole {
    /// The role an RFC 6154 attribute (`\Sent`) names.
    pub fn from_attribute(attr: &str) -> Option<FolderRole> {
        match attr.trim_start_matches('\\').to_ascii_lowercase().as_str() {
            "sent" => Some(FolderRole::Sent),
            "drafts" => Some(FolderRole::Drafts),
            "trash" => Some(FolderRole::Trash),
            "junk" | "spam" => Some(FolderRole::Junk),
            "archive" | "all" => Some(FolderRole::Archive),
            _ => None,
        }
    }

    /// The role a folder name suggests, for servers without special-use attributes.
    pub fn from_name(path: &str, delimiter: Option<char>) -> FolderRole {
        if path.eq_ignore_ascii_case("INBOX") {
            return FolderRole::Inbox;
        }
        let leaf = delimiter.and_then(|d| path.rsplit(d).next()).unwrap_or(path).to_ascii_lowercase();
        match leaf.as_str() {
            "sent" | "sent items" | "sent mail" | "sent messages" => FolderRole::Sent,
            "drafts" | "draft" => FolderRole::Drafts,
            "trash" | "deleted" | "deleted items" | "deleted messages" | "bin" => FolderRole::Trash,
            "junk" | "spam" | "junk e-mail" | "junk email" | "bulk mail" => FolderRole::Junk,
            "archive" | "archives" | "all mail" => FolderRole::Archive,
            _ => FolderRole::Other,
        }
    }

    /// Display order in the folder pane: Inbox first, then the usual folders, then the rest.
    pub fn rank(self) -> u8 {
        match self {
            FolderRole::Inbox => 0,
            FolderRole::Drafts => 1,
            FolderRole::Sent => 2,
            FolderRole::Archive => 3,
            FolderRole::Junk => 4,
            FolderRole::Trash => 5,
            FolderRole::Other => 6,
        }
    }

    /// The display name for a special folder (servers name them in many ways).
    pub fn label(self) -> Option<&'static str> {
        match self {
            FolderRole::Inbox => Some("Inbox"),
            FolderRole::Drafts => Some("Drafts"),
            FolderRole::Sent => Some("Sent Items"),
            FolderRole::Archive => Some("Archive"),
            FolderRole::Junk => Some("Junk Email"),
            FolderRole::Trash => Some("Deleted Items"),
            FolderRole::Other => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            FolderRole::Inbox => "inbox",
            FolderRole::Drafts => "drafts",
            FolderRole::Sent => "sent",
            FolderRole::Archive => "archive",
            FolderRole::Junk => "junk",
            FolderRole::Trash => "trash",
            FolderRole::Other => "other",
        }
    }

    pub fn from_code(s: &str) -> Option<FolderRole> {
        [FolderRole::Inbox, FolderRole::Drafts, FolderRole::Sent, FolderRole::Archive, FolderRole::Junk, FolderRole::Trash, FolderRole::Other]
            .into_iter()
            .find(|r| r.code().eq_ignore_ascii_case(s.trim()))
    }
}

/// A mailbox on the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub account: String,
    /// The server's name for it (modified UTF-7, as sent on the wire).
    pub path: String,
    /// Hierarchy delimiter, if the server has a hierarchy.
    pub delimiter: Option<char>,
    #[serde(default)]
    pub role: FolderRole,
    /// `false` for `\Noselect` containers.
    #[serde(default = "yes")]
    pub selectable: bool,
    #[serde(default)]
    pub unread: u32,
    #[serde(default)]
    pub total: u32,
}

fn yes() -> bool {
    true
}

impl Folder {
    /// The decoded leaf name (`Projects/Café` → `Café`), or the role's label for special folders.
    pub fn display_name(&self) -> String {
        if let Some(l) = self.role.label() {
            return l.to_string();
        }
        let decoded = utf7::decode(&self.path);
        match self.delimiter {
            Some(d) => decoded.rsplit(d).next().unwrap_or(&decoded).to_string(),
            None => decoded,
        }
    }

    /// Nesting depth (0 for top-level folders).
    pub fn depth(&self) -> usize {
        match self.delimiter {
            Some(d) if !self.path.eq_ignore_ascii_case("INBOX") => self.path.matches(d).count(),
            _ => 0,
        }
    }
}

/// Whether `path` is `parent` itself or a folder inside it.
pub fn is_within(path: &str, parent: &str, delimiter: Option<char>) -> bool {
    path == parent || delimiter.is_some_and(|d| path.strip_prefix(parent).is_some_and(|rest| rest.starts_with(d)))
}

/// `path` with its `from` prefix replaced by `to` (for folders inside a renamed folder).
pub fn rebase_path(path: &str, from: &str, to: &str, delimiter: Option<char>) -> Option<String> {
    if path == from {
        return Some(to.to_string());
    }
    let d = delimiter?;
    let rest = path.strip_prefix(from)?.strip_prefix(d)?;
    Some(format!("{to}{d}{rest}"))
}

/// The server path for a new folder called `name` (a display name, encoded to modified UTF-7)
/// inside `parent` (`None`: top level). Errors are user-facing.
pub fn child_path(parent: Option<&str>, delimiter: Option<char>, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("the folder name is empty".into());
    }
    if name.chars().any(|c| c.is_control()) || name.chars().count() > 200 {
        return Err("the folder name has characters that can't be used".into());
    }
    if let Some(d) = delimiter
        && name.contains(d)
    {
        return Err(format!("folder names can't contain `{d}` on this server"));
    }
    if matches!(name, "." | "..") || name.contains('*') || name.contains('%') {
        return Err(format!("`{name}` can't be used as a folder name"));
    }
    let leaf = utf7::encode(name);
    match parent.filter(|p| !p.is_empty()) {
        None => Ok(leaf),
        Some(p) => match delimiter {
            Some(d) => Ok(format!("{p}{d}{leaf}")),
            None => Err("this server has no subfolders".into()),
        },
    }
}

/// Sort folders for the folder pane: special folders in their usual order, then the others by
/// path so children follow their parents. A folder of the user's goes with the folder at the top
/// of its path, so one made in the Inbox shows under the Inbox, as Outlook shows it.
pub fn sort_folders(folders: &mut [Folder]) {
    let roles: std::collections::HashMap<String, FolderRole> = folders.iter().map(|f| (f.path.clone(), f.role)).collect();
    folders.sort_by_cached_key(|f| {
        let parts: Vec<String> = match f.delimiter {
            Some(d) => f.path.split(d).map(|p| utf7::decode(p).to_lowercase()).collect(),
            None => vec![utf7::decode(&f.path).to_lowercase()],
        };
        // A special folder keeps its own place; another goes where its top folder is.
        let role = if f.role == FolderRole::Other {
            let top = f.delimiter.and_then(|d| f.path.split(d).next()).unwrap_or(&f.path);
            roles.get(top).copied().unwrap_or(FolderRole::Other)
        } else {
            f.role
        };
        (if role == FolderRole::Other { 6 } else { role.rank() }, parts)
    });
}

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

/// A keyword for a category name (`To do` → `to_do`), for categories made in MailCraft.
pub fn keyword_for(name: &str) -> String {
    if !name.is_ascii() {
        // Keywords are ASCII atoms: other scripts get a stable hash of the name.
        let mut h: u32 = 2_166_136_261;
        for b in name.trim().to_lowercase().bytes() {
            h = (h ^ u32::from(b)).wrapping_mul(16_777_619);
        }
        return format!("cat_{h:08x}");
    }
    let k: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .filter(|c| c.is_ascii_graphic() && !"(){%*\"\\]".contains(*c))
        .take(64)
        .collect();
    if k.is_empty() || k.starts_with('$') { format!("cat_{k}") } else { k }
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
}

fn is_zero(n: &i8) -> bool {
    *n == 0
}

impl Summary {
    /// The subject with reply and forward prefixes removed, for conversation grouping.
    pub fn base_subject(&self) -> String {
        thread::base_subject(&self.subject)
    }
}

/// Sort newest first, with the key as a tie-breaker so the order is stable.
pub fn sort_newest_first(list: &mut [Summary]) {
    list.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.key.cmp(&a.key)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microsofts_own_addresses_need_its_sign_in() {
        for e in ["a@outlook.com", "a@hotmail.com", "a@hotmail.co.uk", "a@live.nl", "a@msn.com", "A@Outlook.FR"] {
            assert!(microsoft_consumer(e), "{e}");
        }
        for e in ["a@outlook-studio.example", "a@studio.example", "a@hotmail", "hotmail.com", ""] {
            assert!(!microsoft_consumer(e), "{e}");
        }
        let mut a = AccountConfig::guess("someone@hotmail.com");
        assert!(microsoft_password_account(&a));
        assert_eq!(AccountConfig::guess("a@live.nl").imap.host, "outlook.office365.com");
        a.oauth = Some("microsoft".into());
        assert!(!microsoft_password_account(&a));
        assert!(!microsoft_password_account(&AccountConfig::guess("zoe@studio.example")));
        // A work address on Microsoft 365's servers, signing in with a password.
        let mut work = AccountConfig::guess("zoe@contoso.example");
        work.imap.host = "outlook.office365.com".into();
        assert!(microsoft_password_account(&work));
    }

    #[test]
    fn a_folder_made_in_another_follows_it() {
        let f = |path: &str, role: FolderRole| Folder {
            account: "a".into(),
            path: path.into(),
            delimiter: Some('/'),
            role,
            selectable: true,
            unread: 0,
            total: 0,
        };
        let mut list = vec![
            f("Projects", FolderRole::Other),
            f("Deleted Items", FolderRole::Trash),
            f("INBOX/gzamer", FolderRole::Other),
            f("Sent Items", FolderRole::Sent),
            f("INBOX", FolderRole::Inbox),
            f("Projects/Short film", FolderRole::Other),
            f("[Gmail]/Sent Mail", FolderRole::Sent),
            f("[Gmail]", FolderRole::Other),
        ];
        sort_folders(&mut list);
        let order: Vec<&str> = list.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            order,
            ["INBOX", "INBOX/gzamer", "[Gmail]/Sent Mail", "Sent Items", "Deleted Items", "[Gmail]", "Projects", "Projects/Short film"],
        );
    }

    #[test]
    fn guesses_servers_from_the_domain() {
        let a = AccountConfig::guess("zoe@example.org");
        assert_eq!(a.imap.host, "mail.example.org");
        assert_eq!(a.imap.port, 993);
        assert_eq!(a.smtp.port, 465);
        assert_eq!(a.username, "zoe@example.org");
        let g = AccountConfig::guess("x@gmail.com");
        assert_eq!(g.imap.host, "imap.gmail.com");
        assert_eq!(AccountConfig::guess("nonsense").imap.host, "");
    }

    #[test]
    fn folder_roles_from_names_and_attributes() {
        assert_eq!(FolderRole::from_name("INBOX", Some('/')), FolderRole::Inbox);
        assert_eq!(FolderRole::from_name("INBOX.Sent", Some('.')), FolderRole::Sent);
        assert_eq!(FolderRole::from_name("Deleted Items", None), FolderRole::Trash);
        assert_eq!(FolderRole::from_name("Projects", Some('/')), FolderRole::Other);
        assert_eq!(FolderRole::from_attribute("\\Junk"), Some(FolderRole::Junk));
        assert_eq!(FolderRole::from_attribute("\\HasChildren"), None);
    }

    #[test]
    fn folder_names_and_depth() {
        let f = Folder { path: "Projects/Caf&AOk-".into(), delimiter: Some('/'), ..Default::default() };
        assert_eq!(f.display_name(), "Café");
        assert_eq!(f.depth(), 1);
        let inbox = Folder { path: "INBOX".into(), delimiter: Some('.'), role: FolderRole::Inbox, ..Default::default() };
        assert_eq!(inbox.display_name(), "Inbox");
        assert_eq!(inbox.depth(), 0);
    }

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
        assert_eq!(keyword_for("To do"), "to_do");
        assert!(keyword_for("Café").starts_with("cat_") && is_keyword(&keyword_for("Café")));
        assert_eq!(keyword_for("Café"), keyword_for(" café "));
        assert_eq!(keyword_for("$x"), "cat_$x");
        assert!(is_keyword(&keyword_for("Projects: Film!")));
    }

    #[test]
    fn folder_paths() {
        assert!(is_within("A/B", "A", Some('/')));
        assert!(is_within("A", "A", Some('/')));
        assert!(!is_within("AB", "A", Some('/')));
        assert!(!is_within("A/B", "A", None));
        assert_eq!(rebase_path("A/B/C", "A/B", "X", Some('/')).as_deref(), Some("X/C"));
        assert_eq!(rebase_path("A/BC", "A/B", "X", Some('/')), None);
        assert_eq!(child_path(Some("Projects"), Some('/'), "Café").as_deref(), Ok("Projects/Caf&AOk-"));
        assert_eq!(child_path(None, Some('.'), " Plans ").as_deref(), Ok("Plans"));
        assert!(child_path(None, Some('/'), "a/b").is_err());
        assert!(child_path(Some("X"), None, "a").is_err());
        assert!(child_path(None, None, "  ").is_err());
        assert!(child_path(None, None, "*").is_err());
    }

    #[test]
    fn folders_sort_special_first() {
        let mk = |p: &str, r| Folder { path: p.into(), role: r, delimiter: Some('/'), ..Default::default() };
        let mut v =
            vec![mk("Zeta", FolderRole::Other), mk("Trash", FolderRole::Trash), mk("INBOX", FolderRole::Inbox), mk("Alpha", FolderRole::Other)];
        sort_folders(&mut v);
        let names: Vec<_> = v.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["INBOX", "Trash", "Alpha", "Zeta"]);
    }
}
