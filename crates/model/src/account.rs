//! Accounts and their servers: how a connection is secured, the settings an account keeps,
//! servers guessed from an address, what's known of providers (Gmail, Microsoft), and the
//! certificates and User-Agent an account may name.

use serde::{Deserialize, Serialize};

use super::*;

pub mod signature;

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

/// How MailCraft signs in to a mail server with a password (issue #123), as Outlook's "Require
/// logon using Secure Password Authentication" and "My outgoing server requires authentication"
/// choose it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignInMethod {
    /// As the server offers: the password (`PLAIN` or `LOGIN`), else NTLM; an outgoing server
    /// that offers no sign-in is sent to without one.
    #[default]
    Automatic,
    /// The password, as the server asks for it (`PLAIN` or `LOGIN`).
    Password,
    /// NTLM, Outlook's Secure Password Authentication: the password never crosses the network.
    Ntlm,
    /// No sign-in: an outgoing server that takes mail from where it's sent (a company's own).
    None,
}

impl SignInMethod {
    pub fn code(self) -> &'static str {
        match self {
            SignInMethod::Automatic => "automatic",
            SignInMethod::Password => "password",
            SignInMethod::Ntlm => "ntlm",
            SignInMethod::None => "none",
        }
    }

    pub fn from_code(s: &str) -> Option<SignInMethod> {
        match s.trim().to_ascii_lowercase().as_str() {
            "automatic" | "auto" | "" => Some(SignInMethod::Automatic),
            "password" | "plain" | "login" => Some(SignInMethod::Password),
            "ntlm" | "spa" => Some(SignInMethod::Ntlm),
            "none" => Some(SignInMethod::None),
            _ => None,
        }
    }

    pub fn is_automatic(&self) -> bool {
        *self == SignInMethod::Automatic
    }
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
    /// The outgoing server's own user name, when it doesn't share the incoming server's sign-in
    /// (its password is kept apart in the keychain); otherwise SMTP signs in as IMAP or POP does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_username: Option<String>,
    /// How the incoming server (IMAP or POP) is signed in to with the password (issue #123).
    #[serde(default, skip_serializing_if = "SignInMethod::is_automatic")]
    pub sign_in: SignInMethod,
    /// How the outgoing server (SMTP) is signed in to, or that it isn't (issue #123).
    #[serde(default, skip_serializing_if = "SignInMethod::is_automatic")]
    pub smtp_sign_in: SignInMethod,
    /// Its signatures by name (issue #11: "Full", "Short"…), as Outlook's Signatures keeps them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<Signature>,
    /// The signature (by name) put into new messages, and the one put into replies and forwards
    /// (Outlook's "New messages" and "Replies/forwards"); `None` puts none in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_new: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_reply: Option<String>,
    /// The one signature an account had before it could have several (its text, its look as
    /// HTML, its HTML code): moved into `signatures` ([`AccountConfig::migrate_signature`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_html: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_code: String,
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
    /// Its other addresses (issue #81): the same mailbox's, each one that mail is written from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identities: Vec<Identity>,
    /// Other mailboxes this account opens, added by hand (Exchange: Outlook's "Open these
    /// additional mailboxes"); those the server maps to it open by themselves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_mailboxes: Vec<String>,
    /// Online only (issue #4): none of the account's mail is kept on this computer. Its folders,
    /// messages, changes waiting for the server and Outbox are held in memory while MailCraft
    /// runs, and a message is downloaded when it's opened. Not for POP, whose mail has no other
    /// home, nor an account with no server ([`AccountConfig::keeps_online`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub online_only: bool,
    /// How often it looks for new mail by itself, in minutes (issue #58). `None`: as soon as the
    /// server says there's some, where a server can, and otherwise every two minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_every: Option<u32>,
    /// It looks for new mail only when asked (issue #104): not as MailCraft starts, nor by
    /// itself after, nor with Send / Receive for all accounts; Send / Receive on the account
    /// does, and so does sending, or opening a message not downloaded yet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub check_by_hand: bool,
    /// Calendars and contacts given by hand (issue #7), for a server MailCraft can't find from the
    /// address on its own, or one that wants a password of its own (an app password).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dav: Option<DavSettings>,
    /// The User-Agent its Exchange (EWS) requests name instead of MailCraft's (issue #9), for a
    /// server that lets in only the clients it knows. Checked by [`user_agent`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Certificates the user chose to trust for this account's mail servers when asked, by their
    /// SHA-256 fingerprints in hex: one a server on this computer made itself (issue #77: a bridge
    /// to a mail service), or one no authority vouches for elsewhere (issue #105: a company's or
    /// home server's), then only while it's in date and for the server's name. Never for web
    /// addresses, nor a major mail service's servers (`net/src/stream/trust.rs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_certificates: Vec<String>,
    /// Certificates the user imported from files for this account's mail servers (issue #105): an
    /// authority their organisation runs, or a server's own certificate, in base64 (DER). As
    /// limited as [`Self::trusted_certificates`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_authorities: Vec<String>,
}

impl AccountConfig {
    /// The name the account signs in with: its user name, or its address when it has none.
    pub fn login_name(&self) -> &str {
        if self.username.is_empty() { &self.email } else { &self.username }
    }

    /// Whether `other` connects and signs in as this does: the same servers, sign-in names
    /// (the address is one, when there's no user name), mailboxes and how often it looks. Every
    /// field is named in [`Self::connection`], so one added to the account is decided on there
    /// (a list of comparisons kept apart missed the address and who OAuth signed in as).
    pub fn same_connection(&self, other: &AccountConfig) -> bool {
        self.connection() == other.connection()
    }

    /// What the connection is made from.
    fn connection(&self) -> impl PartialEq + '_ {
        let AccountConfig {
            // What shows the account, or goes into what it writes: not how it connects.
            id: _,
            name: _,
            color: _,
            signatures: _,
            signature_new: _,
            signature_reply: _,
            signature: _,
            signature_html: _,
            signature_code: _,
            identities: _,
            email,
            username,
            imap,
            smtp,
            smtp_username,
            sign_in,
            smtp_sign_in,
            oauth,
            jmap,
            graph,
            ews,
            pop,
            pop_delete,
            pop_keep_days,
            pop_delete_with_trash,
            oauth_user,
            shared_mailboxes,
            online_only,
            check_every,
            check_by_hand,
            dav,
            user_agent,
            trusted_certificates,
            trusted_authorities,
        } = self;
        (
            (email, username, imap, smtp, pop),
            (jmap, graph, ews, smtp_username, oauth, oauth_user, (sign_in, smtp_sign_in)),
            (pop_delete, pop_keep_days, pop_delete_with_trash, shared_mailboxes, online_only, (check_every, check_by_hand)),
            (dav, user_agent, trusted_certificates, trusted_authorities),
        )
    }
}

/// The most certificates an account trusts by hand, and imports from files (and how big one is).
pub const MAX_TRUSTED_CERTIFICATES: usize = 8;
pub const MAX_TRUSTED_AUTHORITIES: usize = 4;
pub const MAX_AUTHORITY_BYTES: usize = 16 << 10;

/// Whether `s` is a SHA-256 fingerprint in hex.
pub fn is_certificate_fingerprint(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Longest User-Agent an account can name.
pub const MAX_USER_AGENT: usize = 256;

/// A User-Agent as an account may name it (RFC 9110 10.1.5: visible ASCII and spaces), trimmed;
/// `None` when it's empty, too long or has anything else (a line break would end the header).
pub fn user_agent(s: &str) -> Option<String> {
    let s = s.trim();
    let ok = !s.is_empty() && s.len() <= MAX_USER_AGENT && s.bytes().all(|b| b == b' ' || b.is_ascii_graphic());
    ok.then(|| s.to_string())
}

/// Where an account's calendars and contacts are, given by hand: CalDAV and CardDAV addresses
/// (any of the server's, its own address, a home or one calendar: each is followed to the user's
/// home), and their own user name, whose password is kept apart in the keychain.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DavSettings {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub caldav: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub carddav: String,
    /// Signs in as this user with its own password (`None`: as the mail server does).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// The EWS address of an account whose Exchange server Autodiscover finds when it connects.
pub const EWS_AUTODISCOVER: &str = "autodiscover";

/// Microsoft Graph's address, for Exchange Online accounts.
pub const GRAPH_URL: &str = "https://graph.microsoft.com/v1.0";

/// Whether the outgoing server at `smtp_host` files a copy of what it sends in Sent itself, as
/// Gmail's do: Google's own domains and their servers only, never a host that merely ends in
/// the same letters (`smtp.notgmail.com`).
pub fn files_sent_mail_itself(smtp_host: &str) -> bool {
    let h = smtp_host.trim().trim_end_matches('.').to_ascii_lowercase();
    ["gmail.com", "googlemail.com"].iter().any(|d| lookalike::is_under(&h, d))
}

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
    let domain = lookalike::domain_of(email);
    let name = domain.split('.').next().unwrap_or_default();
    matches!(name, "outlook" | "hotmail" | "live" | "msn" | "windowslive" | "passport") && domain.contains('.')
}

/// Why a Microsoft account signing in with a password can't connect: its status starts with this
/// and is of the kind [`FailureKind::MicrosoftNoPasswords`].
pub const MICROSOFT_NO_PASSWORDS: &str = "Microsoft doesn't accept passwords from mail apps for this account: it needs Sign in with Microsoft";

/// Why an account signed in with Microsoft can't open its mailbox over IMAP: Microsoft took the
/// sign-in, then its server said "User is authenticated but not connected". The status starts
/// with this and is of the kind [`FailureKind::MicrosoftImapClosed`], so the UI can offer what
/// helps (signing in again, or Microsoft Graph).
pub const MICROSOFT_IMAP_CLOSED: &str = "Microsoft signed you in, but its mail server won't open this mailbox over IMAP";

/// Where Google's mail servers' app passwords are made.
pub const GOOGLE_APP_PASSWORDS: &str = "https://myaccount.google.com/apppasswords";

/// Why a Google account's password was refused when Google asks for an app password (issue
/// #164): the account's own password isn't taken from mail apps while 2-Step Verification is on.
/// Said in place of Google's English text, so it can be translated.
pub const GOOGLE_APP_PASSWORD_NEEDED: &str = "Google doesn't let mail apps sign in with your Google password while 2-Step Verification is on. Make an app password at https://myaccount.google.com/apppasswords and enter it here instead.";

/// Google's refusal of an account's own password where an app password is needed: IMAP's
/// `[ALERT] Application-specific password required: https://support.google.com/…`, SMTP's
/// `534-5.7.9 Application-specific password required.`
pub fn google_wants_app_password(server_said: &str) -> bool {
    server_said.to_ascii_lowercase().contains("application-specific password required")
}

/// An address at Gmail, or one whose mail servers are Google's (Google Workspace): its sign-in
/// takes an app password when 2-Step Verification is on.
pub fn google_mail(email: &str, imap_host: &str) -> bool {
    matches!(lookalike::domain_of(email).as_str(), "gmail.com" | "googlemail.com") || oauth_provider_for(imap_host) == Some("google")
}

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
        let domain = lookalike::domain_of(email);
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
            ..AccountConfig::default()
        }
    }

    /// Whether the account's mail is kept online only: asked for, and on a server that keeps it.
    pub fn keeps_online(&self) -> bool {
        self.online_only && self.can_be_online_only()
    }

    /// Whether the account can be kept online only: its mail lives on a server (IMAP, JMAP,
    /// Graph, EWS), not only here (POP downloads it; the sample mailbox has no server).
    pub fn can_be_online_only(&self) -> bool {
        !self.is_local() && self.pop.is_none()
    }

    /// Its mail kept on this computer only, with no mail server (the sample mailbox, a data file,
    /// a calendar-only account).
    pub fn is_local(&self) -> bool {
        self.imap.host.trim().is_empty() && self.jmap.is_none() && self.graph.is_none() && self.ews.is_none() && self.pop.is_none()
    }

    /// Calendars and contacts only (issue #61): no mail server, a CalDAV and CardDAV server it
    /// signs in to with its own user name and password. Its mail side is a local account's
    /// (nothing to sync or send), so only what concerns calendars and contacts has a server.
    pub fn is_calendar_only(&self) -> bool {
        self.is_local() && self.dav.as_ref().is_some_and(|d| !d.caldav.trim().is_empty() || !d.carddav.trim().is_empty())
    }

    /// Whether it has a server to connect to: its mail's, or a calendar-only account's.
    pub fn has_server(&self) -> bool {
        !self.is_local() || self.is_calendar_only()
    }

    /// Whether the account works through a web API (JMAP, Graph or EWS) rather than IMAP and
    /// SMTP.
    pub fn is_api(&self) -> bool {
        self.jmap.is_some() || self.graph.is_some() || self.ews.is_some()
    }
}

/// The longest an account can be asked to go between looking for new mail: a day.
pub const MAX_CHECK_MINUTES: u32 = 24 * 60;

#[cfg(test)]
mod tests {
    use super::*;

    /// An account signs in with its user name, or its address when it has none.
    #[test]
    fn the_login_name_is_the_user_name_else_the_address() {
        let mut a = AccountConfig { email: "zoe@studio.example".into(), ..Default::default() };
        assert_eq!(a.login_name(), "zoe@studio.example");
        a.username = "zoe".into();
        assert_eq!(a.login_name(), "zoe");
    }

    /// Issue #19's cause: a server whose name only ends in "gmail.com" was taken for Gmail's,
    /// and its account's sent mail kept nowhere.
    #[test]
    fn only_gmails_own_servers_file_sent_mail_themselves() {
        for h in ["smtp.gmail.com", "SMTP.Gmail.COM", " smtp.googlemail.com ", "smtp.gmail.com.", "gmail.com"] {
            assert!(files_sent_mail_itself(h), "{h}");
        }
        for h in ["smtp.notgmail.com", "mail.xgmail.com", "gmail.com.example", "smtp.googlemail.co", "", "."] {
            assert!(!files_sent_mail_itself(h), "{h}");
        }
    }

    #[test]
    fn microsofts_own_addresses_need_its_sign_in() {
        for e in ["a@outlook.com", "a@hotmail.com", "a@hotmail.co.uk", "a@live.nl", "a@msn.com", "A@Outlook.FR"] {
            assert!(microsoft_consumer(e), "{e}");
        }
        // An address's domain is read as the lookalike check reads it: `outlook.` is `outlook`.
        assert!(microsoft_consumer("a@Outlook.com."));
        for e in ["a@outlook-studio.example", "a@studio.example", "a@hotmail", "a@hotmail.", "hotmail.com", ""] {
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
    fn guesses_servers_from_the_domain() {
        let a = AccountConfig::guess("zoe@example.org");
        assert_eq!(a.imap.host, "mail.example.org");
        assert_eq!(a.imap.port, 993);
        assert_eq!(a.smtp.port, 465);
        assert_eq!(a.username, "zoe@example.org");
        let g = AccountConfig::guess("x@gmail.com");
        assert_eq!(g.imap.host, "imap.gmail.com");
        // A domain written with a dot at its end, or capitals beyond ASCII, is the same domain.
        assert_eq!(AccountConfig::guess("x@Gmail.com.").imap.host, "imap.gmail.com");
        assert_eq!(AccountConfig::guess("x@ÉCOLE.example").imap.host, "mail.école.example");
        assert_eq!(AccountConfig::guess("nonsense").imap.host, "");
    }
}
