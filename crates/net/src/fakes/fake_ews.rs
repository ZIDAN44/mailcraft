//! An in-memory Exchange Web Services server on localhost, for tests: folders (distinguished
//! ids, a deep tree), messages (find, get with their MIME source, update, move with a new id,
//! delete), creating from MIME (drafts and sending), folder changes and automatic replies, signed
//! in with NTLM on a kept-open connection (or Basic, when set to).

use std::sync::{Arc, Mutex};

use crate::ews::{El, esc, parse, unb64};
pub use oof::Oof;
pub use reminders::Listed;
use serving::serve_at;

pub const USER: &str = "riley";
pub const DOMAIN: &str = "CONTOSO";
pub const PASSWORD: &str = "pw";

/// The account these tests sign in with (`id`, Riley, `CONTOSO\riley`), on the EWS server at
/// `url`.
pub fn account(id: &str, url: &str) -> mailcraft_model::AccountConfig {
    mailcraft_model::AccountConfig {
        id: id.into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(url.into()),
        ..Default::default()
    }
}

#[derive(Clone, Debug)]
pub struct Fo {
    pub id: String,
    pub name: String,
    pub parent: String,
    pub class: String,
    /// The distinguished id (`inbox`, `sentitems`…).
    pub known: Option<String>,
    /// The other mailbox it belongs to (a shared one), or empty for the account's own.
    pub owner: String,
}

/// Another mailbox on the server (a shared one) and what the account may do with it.
#[derive(Clone, Debug)]
pub struct SharedBox {
    pub address: String,
    pub name: String,
    /// Autodiscover maps it to the account (full access with automapping), as Outlook opens it.
    pub automap: bool,
    /// Autodiscover names it by its Exchange address only (older servers): ResolveNames finds it.
    pub legacy: bool,
    /// The account has access to it.
    pub granted: bool,
}

#[derive(Clone, Debug)]
pub struct It {
    pub id: String,
    pub folder: String,
    pub raw: Vec<u8>,
    pub read: bool,
    pub flagged: bool,
    /// Its follow-up marked complete (`PidTagFlagStatus` 1).
    pub complete: bool,
    /// The day its follow-up is due (`PidLidTaskDueDate`), as written.
    pub due: Option<String>,
    /// Its flag's reminder: set (`PidLidReminderSet`), and its signal time and time
    /// (`PidLidReminderSignalTime`, `PidLidReminderTime`), as written.
    pub reminder_set: bool,
    pub reminder_signal: Option<String>,
    pub reminder_time: Option<String>,
    pub draft: bool,
    pub received: String,
}

/// An appointment, contact or the like: kept as the element it was created from.
#[derive(Clone, Debug)]
pub struct PimItem {
    pub id: String,
    /// Its folder's id, or `mailbox:<address>` for a shared mailbox's calendar.
    pub folder: String,
    pub el: El,
}

impl PimItem {
    pub fn new(id: impl Into<String>, folder: impl Into<String>, el: El) -> PimItem {
        PimItem { id: id.into(), folder: folder.into(), el }
    }
}

pub struct Data {
    pub folders: Vec<Fo>,
    pub items: Vec<It>,
    /// Appointments, contacts and the like.
    pub pim: Vec<PimItem>,
    /// MIME sources sent.
    pub sent: Vec<Vec<u8>>,
    /// The automatic replies.
    pub oof: Oof,
    /// Sign in with Basic instead of NTLM.
    pub basic: bool,
    /// Operations asked for, in order.
    pub calls: Vec<String>,
    /// NTLM handshakes done (one per connection).
    pub handshakes: usize,
    /// A connection signed in is asked to sign in again, once, after a request answered on it:
    /// as behind a proxy that dropped the sign-in it held, its TCP connection still open.
    pub expire_ntlm_once: bool,
    /// Handshakes done again on a connection signed in before ([`Data::expire_ntlm_once`]).
    pub ntlm_reauthentications: usize,
    /// The EWS address, for Autodiscover's answer.
    pub url: String,
    /// People whose calendars this account may change (delegates with editor rights).
    pub editors: Vec<String>,
    /// The folder's rights say nothing of that (as some servers answer for another mailbox's
    /// calendar): only each item's rights do.
    pub item_rights_only: bool,
    /// Listing a mail folder's messages waits while this is set, until the test lets it go (a big
    /// mailbox's first sync, as long as the test likes).
    pub hold: bool,
    /// Listings of a mail folder's messages waiting now, held.
    pub held: usize,
    /// Ids of folders listed with `PidTagAttributeHidden` true.
    pub flagged_hidden: Vec<String>,
    /// Messages whose source can't be had (the server fails on it); appointments and contacts
    /// that can't be read, and calendar or contacts folders that can't be listed.
    pub unreadable: Vec<String>,
    /// The public folder mailbox holding the public folders' content (Exchange 2013 and
    /// later): reading inside them is only answered when routed there.
    pub pf_mailbox: Option<String>,
    /// The public folder hierarchy mailbox (Exchange 2013 and later): listing public folders is
    /// only answered when routed there.
    pub pf_hierarchy: Option<String>,
    /// A public folder's folders are listed without ids, each page saying more follow (a server
    /// gone wrong).
    pub endless_public: bool,
    /// The request's `X-PublicFolderMailbox` header.
    routed: String,
    /// Other mailboxes on the server.
    pub shared: Vec<SharedBox>,
    /// The `X-AnchorMailbox` header of each request about a shared mailbox's folders.
    pub anchors: Vec<String>,
    /// The organisation's directory (Active Directory): (name, address).
    pub directory: Vec<(String, String)>,
    /// Some of them's job titles and departments (issue #213): (address, title, department).
    pub directory_titles: Vec<(String, String, String)>,
    /// A server whose directory answers name each mailbox by its alias (the address's user part,
    /// issue #9): the full name is only in the contact, when one is asked for.
    pub directory_aliases: bool,
    /// People's photos (issue #223): (address, the picture's bytes).
    pub photos: Vec<(String, Vec<u8>)>,
    /// The `User-Agent` of each request.
    pub agents: Vec<String>,
    /// Refuse (HTTP 403) every request naming another `User-Agent` (issue #9: a server that lets
    /// in only the clients it knows).
    pub only_agent: Option<String>,
    /// Signed in by a front in front of it (a Kerberos one): no sign-in asked here.
    pub open: bool,
    /// Names its sign-in schemes in lower case (`ntlm <challenge>`), as a proxy may.
    pub lower_case_schemes: bool,
    /// Its reminders (issue #40).
    pub reminders: reminders::Reminders,
    /// Exchange 2010's schema (#149): a request naming a distinguished folder it doesn't know
    /// (`archive`, `conversationhistory`, `syncissues`: 2013 brought them) fails validation as a
    /// whole (HTTP 500).
    pub exchange_2010: bool,
    /// Requests refused so ([`Data::exchange_2010`]).
    pub schema_refusals: usize,
    /// Fields its schema doesn't know besides those none does (`schema.rs`): a request naming
    /// one is refused whole, as one naming a field no Exchange knows is (issue #191).
    pub refused_fields: Vec<String>,
    /// Requests refused for a field (`schema.rs`), by the field.
    pub field_refusals: Vec<String>,
    /// The mailbox's limits, in KB (where it can't send, can't receive, and is warned), said on
    /// its root folder, with every folder's size; `None`: neither said (issue #143).
    pub quota_kb: Option<[i64; 3]>,
    /// The mailbox's Inbox rules (each a `Rule` element as the server keeps it; issue #139).
    pub rules: Vec<El>,
    /// Outlook's own copy of the rules is beside them: they're not changed until it's removed.
    pub outlook_rules: bool,
    /// Changes to the rules taken.
    pub rule_updates: usize,
    /// People's times taken, as the availability service gives them (issue #140).
    pub free_busy: Vec<availability::FakeBusy>,
    /// People who don't share their free/busy times with the account.
    pub free_busy_refused: Vec<String>,
    /// Someone's `WorkingHours`, as the service writes it beside their times (issue #185).
    pub working_hours: Vec<(String, String)>,
    /// The free/busy times asked for, in order.
    pub availability_asks: Vec<availability::AvailabilityAsk>,
    /// Pull subscriptions to the mailbox's folders (issue #145).
    pub subscriptions: Vec<watching::FakeSub>,
    /// A subscription to every folder at once is refused (Exchange 2010 before its first
    /// service pack): one naming the folders is taken.
    pub subscribe_all_refused: bool,
    /// Two mailbox servers behind a load balancer (issue #145): a subscription is kept by the one
    /// it was made on, which a look reaches only with the cookie it gave.
    pub balanced: bool,
    balance_turn: bool,
    /// The folders a `GetEvents` answer says at most, the rest said as more (`None`: all).
    pub events_per_answer: Option<usize>,
    /// `GetEvents` asked.
    pub get_events_asked: usize,
    /// Listing a calendar's or address book's items takes this long (a big shared calendar).
    pub pim_slow: Option<std::time::Duration>,
    /// What came to it and went out, for a test's wait to tell where it stopped
    /// ([`Data::where_it_stopped`]).
    traffic: serving::Traffic,
    next: u32,
}

/// A mailbox's Exchange address (its LegacyDN), as older servers name it.
fn legacy_dn(address: &str) -> String {
    format!("/o=Contoso/ou=Exchange Administrative Group/cn=Recipients/cn={}", address.split('@').next().unwrap_or(address))
}

/// The public folder content mailbox's GUID, as `PR_REPLICA_LIST` names it.
pub const PF_GUID: &str = "1ec2a236-ed93-4f88-b9c6-33e63fa4aa44";

impl Default for Data {
    fn default() -> Data {
        let mut d = Data {
            folders: Vec::new(),
            items: Vec::new(),
            pim: Vec::new(),
            sent: Vec::new(),
            oof: Oof::default(),
            basic: false,
            calls: Vec::new(),
            handshakes: 0,
            expire_ntlm_once: false,
            ntlm_reauthentications: 0,
            url: String::new(),
            editors: Vec::new(),
            item_rights_only: false,
            hold: false,
            held: 0,
            flagged_hidden: Vec::new(),
            unreadable: Vec::new(),
            pf_mailbox: None,
            pf_hierarchy: None,
            endless_public: false,
            routed: String::new(),
            shared: Vec::new(),
            anchors: Vec::new(),
            directory: Vec::new(),
            directory_titles: Vec::new(),
            directory_aliases: false,
            photos: Vec::new(),
            agents: Vec::new(),
            only_agent: None,
            open: false,
            lower_case_schemes: false,
            reminders: Default::default(),
            exchange_2010: false,
            schema_refusals: 0,
            refused_fields: Vec::new(),
            field_refusals: Vec::new(),
            quota_kb: None,
            rules: Vec::new(),
            outlook_rules: false,
            rule_updates: 0,
            free_busy: Vec::new(),
            free_busy_refused: Vec::new(),
            working_hours: Vec::new(),
            availability_asks: Vec::new(),
            subscriptions: Vec::new(),
            subscribe_all_refused: false,
            balanced: false,
            balance_turn: false,
            events_per_answer: None,
            get_events_asked: 0,
            pim_slow: None,
            traffic: Default::default(),
            next: 0,
        };
        d.folders.push(Fo {
            id: "ROOT".into(),
            name: "Top of Information Store".into(),
            parent: String::new(),
            class: "IPF.Note".into(),
            known: Some("msgfolderroot".into()),
            owner: String::new(),
        });
        d
    }
}

impl Data {
    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    pub fn folder(&mut self, name: &str, parent: Option<&str>, known: Option<&str>, class: &str) -> String {
        let id = self.id("F");
        self.folders.push(Fo {
            id: id.clone(),
            name: name.into(),
            parent: parent.unwrap_or("ROOT").into(),
            class: class.into(),
            known: known.map(str::to_string),
            owner: String::new(),
        });
        id
    }

    /// Another mailbox on the server with its root, Inbox and Outbox: its Inbox's id.
    pub fn shared_mailbox(&mut self, b: SharedBox) -> String {
        let owner = b.address.to_ascii_lowercase();
        let make = |d: &mut Data, name: &str, parent: &str, known: Option<&str>| {
            let id = d.id("S");
            d.folders.push(Fo {
                id: id.clone(),
                name: name.into(),
                parent: parent.into(),
                class: "IPF.Note".into(),
                known: known.map(str::to_string),
                owner: owner.clone(),
            });
            id
        };
        let root = make(self, "Top of Information Store", "", Some("msgfolderroot"));
        let inbox = make(self, "Inbox", &root, Some("inbox"));
        make(self, "Outbox", &root, Some("outbox"));
        self.shared.push(b);
        inbox
    }

    /// A folder in a shared mailbox, under `parent`.
    pub fn shared_folder(&mut self, owner: &str, name: &str, parent: &str) -> String {
        let id = self.id("S");
        self.folders.push(Fo {
            id: id.clone(),
            name: name.into(),
            parent: parent.into(),
            class: "IPF.Note".into(),
            known: None,
            owner: owner.to_ascii_lowercase(),
        });
        id
    }

    /// The shared mailbox an element names (`<t:Mailbox>`), if it's one; `Err` when the account
    /// has no access to it.
    fn shared_of(&self, el: &El) -> Option<Result<String, ()>> {
        let who = el.find("Mailbox").and_then(|m| m.text_of("EmailAddress"))?.to_ascii_lowercase();
        let b = self.shared.iter().find(|b| b.address.eq_ignore_ascii_case(&who))?;
        Some(if b.granted { Ok(who) } else { Err(()) })
    }

    pub fn deliver(&mut self, folder: &str, raw: &[u8], read: bool, received: &str) -> String {
        let id = self.id("I");
        self.items.push(It {
            id: id.clone(),
            folder: folder.into(),
            raw: raw.to_vec(),
            read,
            flagged: false,
            draft: false,
            received: received.into(),
            complete: false,
            due: None,
            reminder_set: false,
            reminder_signal: None,
            reminder_time: None,
        });
        id
    }

    pub fn by_name(&self, name: &str) -> Option<&Fo> {
        self.folders.iter().find(|f| f.name == name)
    }

    fn folder_id(&self, el: &El) -> Option<String> {
        match el.name.as_str() {
            "FolderId" => el.attr("Id").map(str::to_string),
            // Another mailbox's folder: its items are kept as `mailbox:<address>`.
            "DistinguishedFolderId" if el.child("Mailbox").is_some() && self.shared_of(el).is_some() => {
                let owner = self.shared_of(el)?.ok()?;
                let known = el.attr("Id")?;
                self.folders.iter().find(|f| f.owner == owner && f.known.as_deref() == Some(known)).map(|f| f.id.clone())
            }
            "DistinguishedFolderId" if el.child("Mailbox").is_some() => {
                let who = el.find("EmailAddress").map(|e| e.text.to_ascii_lowercase())?;
                let id = format!("mailbox:{who}");
                self.pim.iter().any(|p| p.folder == id).then_some(id)
            }
            "DistinguishedFolderId" => {
                let known = el.attr("Id")?;
                self.folders.iter().find(|f| f.owner.is_empty() && f.known.as_deref() == Some(known)).map(|f| f.id.clone())
            }
            _ => None,
        }
    }

    fn folder_xml(&self, f: &Fo) -> String {
        let total = self.items.iter().filter(|i| i.folder == f.id).count() + self.pim.iter().filter(|p| p.folder == f.id).count();
        let unread = self.items.iter().filter(|i| i.folder == f.id && !i.read).count();
        // Its extended properties: public folders name the mailbox holding their content (a
        // GUID, as ASCII, NUL-ended); a folder may be marked hidden.
        let mut extended = String::new();
        if self.pf_mailbox.is_some() && self.public_ids().contains(&f.id) {
            extended.push_str(&format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x6698\" PropertyType=\"Binary\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
                mailcraft_model::base64::encode(format!("{PF_GUID}\0").as_bytes())
            ));
        }
        // Its size, and the mailbox's limits on its root, as Exchange says them when asked.
        if let Some(limits) = self.quota_kb {
            let size: usize = self.items.iter().filter(|i| i.folder == f.id).map(|i| i.raw.len()).sum();
            extended.push_str(&format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0xe08\" PropertyType=\"Long\"/><t:Value>{size}</t:Value></t:ExtendedProperty>"
            ));
            if f.known.as_deref() == Some("msgfolderroot") {
                for (tag, kb) in ["0x666e", "0x666a", "0x3ff5"].into_iter().zip(limits) {
                    extended.push_str(&format!(
                        "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"Integer\"/><t:Value>{kb}</t:Value></t:ExtendedProperty>"
                    ));
                }
            }
        }
        if self.flagged_hidden.contains(&f.id) {
            extended.push_str(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x10f4\" PropertyType=\"Boolean\"/><t:Value>true</t:Value></t:ExtendedProperty>",
            );
        }
        // As Exchange answers: each folder in an element of its kind (EWS's folder array).
        let kind = match f.class.as_str() {
            c if c.starts_with("IPF.Contact") => "ContactsFolder",
            c if c.starts_with("IPF.Appointment") => "CalendarFolder",
            c if c.starts_with("IPF.Task") => "TasksFolder",
            _ => "Folder",
        };
        format!(
            "<t:{kind}><t:FolderId Id=\"{}\"/><t:ParentFolderId Id=\"{}\"/><t:FolderClass>{}</t:FolderClass><t:DisplayName>{}</t:DisplayName><t:TotalCount>{total}</t:TotalCount><t:ChildFolderCount>0</t:ChildFolderCount><t:UnreadCount>{unread}</t:UnreadCount>{extended}</t:{kind}>",
            f.id,
            f.parent,
            f.class,
            esc(&f.name)
        )
    }

    /// Whether listing under this folder goes unanswered: the public folders, asked without
    /// being routed to the hierarchy mailbox.
    fn unrouted_hierarchy(&self, folder: &str) -> bool {
        self.pf_hierarchy.as_ref().is_some_and(|m| *m != self.routed) && self.public_ids().iter().any(|p| p == folder)
    }

    /// Whether a request for this folder's content goes unanswered: a public folder, asked
    /// without being routed to the mailbox holding it.
    fn unrouted(&self, folder: &str) -> bool {
        self.pf_mailbox.as_ref().is_some_and(|m| *m != self.routed) && self.public_ids().iter().any(|p| p == folder)
    }

    fn item_xml(&self, it: &It, mime_content: bool) -> String {
        let p = mailcraft_mime::parse(&it.raw).unwrap_or_default();
        let mb = |a: &mailcraft_model::EmailAddress| {
            format!(
                "<t:Mailbox><t:Name>{}</t:Name><t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox>",
                esc(&a.name),
                esc(&a.email)
            )
        };
        let ext = |tag: &str, v: i64| {
            format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"Integer\"/><t:Value>{v}</t:Value></t:ExtendedProperty>"
            )
        };
        let mut x = format!("<t:Message><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/>", it.id);
        if mime_content {
            x.push_str(&format!("<t:MimeContent CharacterSet=\"UTF-8\">{}</t:MimeContent>", mailcraft_model::base64::encode(&it.raw)));
        }
        x.push_str(&format!(
            "<t:Subject>{}</t:Subject><t:DateTimeReceived>{}</t:DateTimeReceived><t:Size>{}</t:Size><t:HasAttachments>{}</t:HasAttachments>",
            esc(&p.subject),
            it.received,
            it.raw.len(),
            // As Exchange: files shown in the text don't count.
            p.attachments.iter().any(|a| !a.inline)
        ));
        x.push_str(&follow_up_xml(it));
        x.push_str(&ext("0x0E07", if it.draft { 8 } else { 1 }));
        x.push_str(&format!("<t:ToRecipients>{}</t:ToRecipients>", p.to.iter().map(mb).collect::<String>()));
        if !p.cc.is_empty() {
            x.push_str(&format!("<t:CcRecipients>{}</t:CcRecipients>", p.cc.iter().map(mb).collect::<String>()));
        }
        x.push_str(&format!("<t:From>{}</t:From>", mb(&p.from)));
        x.push_str(&format!("<t:InternetMessageId>&lt;{}&gt;</t:InternetMessageId>", esc(&p.message_id)));
        if !p.in_reply_to.is_empty() {
            x.push_str(&format!("<t:InReplyTo>&lt;{}&gt;</t:InReplyTo>", esc(&p.in_reply_to)));
        }
        if !p.references.is_empty() {
            let refs: Vec<String> = p.references.iter().map(|r| format!("&lt;{}&gt;", esc(r))).collect();
            x.push_str(&format!("<t:References>{}</t:References>", refs.join(" ")));
        }
        x.push_str(&format!("<t:IsRead>{}</t:IsRead></t:Message>", it.read));
        x
    }

    /// The public folders root and everything under it.
    fn public_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.folders.iter().filter(|f| f.known.as_deref() == Some("publicfoldersroot")).map(|f| f.id.clone()).collect();
        let mut i = 0;
        while let Some(id) = ids.get(i).cloned() {
            ids.extend(self.folders.iter().filter(|f| f.parent == id).map(|f| f.id.clone()));
            i += 1;
        }
        ids
    }

    /// Whether this account may change an item: anything of its own, and someone else's only where
    /// they made it an editor.
    fn may_change(&self, id: &str) -> bool {
        match self.pim.iter().find(|p| p.id == *id).and_then(|p| p.folder.strip_prefix("mailbox:")) {
            Some(who) => self.editors.iter().any(|e| e == who),
            None => true,
        }
    }

    fn pim_folder(&self, f: Option<&El>) -> bool {
        let Some(id) = f.and_then(|f| self.folder_id(f)) else { return false };
        id.starts_with("mailbox:") || self.folders.iter().any(|x| x.id == id && x.class != "IPF.Note")
    }

    /// Answer one operation: (response element name, response messages).
    fn route(&mut self, op: &El) -> (String, Vec<String>) {
        let out = match op.name.as_str() {
            "GetFolder" => self.get_folder(op),
            "GetReminders" | "PerformReminderAction" => vec![self.reminders_answer(op)],
            "GetInboxRules" => vec![self.inbox_rules_answer()],
            "UpdateInboxRules" => vec![self.update_inbox_rules(op)],
            "Subscribe" => vec![self.subscribe(op)],
            "GetEvents" => vec![self.get_events(op)],
            // The directory: every name or address with a word starting with the search, up to
            // 100, as Exchange's ambiguous name resolution; several are a warning.
            "ResolveNames" if op.attr("SearchScope") == Some("ActiveDirectory") => vec![self.directory_answer(op)],
            "ResolveNames" => vec![self.resolve_mailbox(op)],
            "FindFolder" => self.find_folder(op),
            "FindItem" => self.find_item(op),
            "GetItem" => self.get_item(op),
            "CreateItem" => self.create_item(op),
            "UpdateItem" => self.update_item(op),
            "MoveItem" => self.move_item(op),
            "DeleteItem" => self.delete_item(op),
            "CreateFolder" => vec![self.create_folder(op)],
            "MoveFolder" => self.move_folder(op),
            "UpdateFolder" => self.update_folder(op),
            "DeleteFolder" => self.delete_folder(op),
            "EmptyFolder" => self.empty_folder(op),
            "MarkAllItemsAsRead" => self.mark_all_read(op),
            "GetUserOofSettingsRequest" => return ("GetUserOofSettingsResponse".into(), vec![self.oof_answer()]),
            "SetUserOofSettingsRequest" => return ("SetUserOofSettingsResponse".into(), vec![self.set_oof(op)]),
            "GetUserAvailabilityRequest" => return ("GetUserAvailabilityResponse".into(), vec![self.availability(op)]),
            other => vec![err(&format!("ErrorInvalidOperation:{other}"))],
        };
        (format!("{}Response", op.name), out)
    }
}

/// A response message that succeeded, with what's inside it (`<m:R>`, renamed for the
/// operation as the envelope is made).
fn ok(inner: String) -> String {
    format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>")
}

/// A response message that failed, with its code.
fn err(code: &str) -> String {
    format!("<m:R ResponseClass=\"Error\"><m:MessageText>{code}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>")
}

/// The ids an element lists (`ItemIds`, `FolderIds`).
fn ids(el: Option<&El>) -> Vec<String> {
    el.map(|e| e.kids.iter().filter_map(|k| k.attr("Id").map(str::to_string)).collect()).unwrap_or_default()
}

pub struct Server {
    pub url: String,
    pub data: Arc<Mutex<Data>>,
}

/// Start the server; it runs until the test process ends.
pub fn serve(data: Data) -> Server {
    serve_at(data, "127.0.0.1:0")
}

/// An element as XML again (for items kept as they were created).
fn xml(e: &El) -> String {
    let attrs: String = e.attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect();
    let inner: String = e.kids.iter().map(xml).collect();
    format!("<t:{0}{attrs}>{1}{inner}</t:{0}>", e.name, esc(&e.text))
}

mod availability;
mod directory;
mod folders;
mod items;
mod oof;
mod refusing;
mod reminders;
mod rules;
mod schema;
mod serving;
#[cfg(test)]
mod tests;
mod watching;

/// A message's flag status, the day its follow-up is due and its reminder, as Exchange gives
/// them (naming the property sets it knows).
fn follow_up_xml(it: &It) -> String {
    let status = if it.flagged {
        Some(2)
    } else if it.complete {
        Some(1)
    } else {
        None
    };
    let mut x = status
        .map(|v| {
            format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>{v}</t:Value></t:ExtendedProperty>"
            )
        })
        .unwrap_or_default();
    if let Some(d) = &it.due {
        x.push_str(&format!(
            "<t:ExtendedProperty><t:ExtendedFieldURI DistinguishedPropertySetId=\"Task\" PropertyId=\"33029\" PropertyType=\"SystemTime\"/><t:Value>{d}</t:Value></t:ExtendedProperty>"
        ));
    }
    let common = |id: &str, kind: &str, v: &str| {
        format!(
            "<t:ExtendedProperty><t:ExtendedFieldURI DistinguishedPropertySetId=\"Common\" PropertyId=\"{id}\" PropertyType=\"{kind}\"/><t:Value>{v}</t:Value></t:ExtendedProperty>"
        )
    };
    if it.reminder_set || it.reminder_signal.is_some() || it.reminder_time.is_some() {
        x.push_str(&common("34051", "Boolean", if it.reminder_set { "true" } else { "false" }));
    }
    for (id, v) in [("34144", &it.reminder_signal), ("34050", &it.reminder_time)] {
        if let Some(v) = v {
            x.push_str(&common(id, "SystemTime", v));
        }
    }
    x
}
