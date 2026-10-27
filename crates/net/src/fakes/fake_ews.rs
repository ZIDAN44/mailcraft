//! An in-memory Exchange Web Services server on localhost, for tests: folders (distinguished
//! ids, a deep tree), messages (find, get with their MIME source, update, move with a new id,
//! delete), creating from MIME (drafts and sending), folder changes and automatic replies, signed
//! in with NTLM on a kept-open connection (or Basic, when set to).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use crate::ews::{El, esc, parse, unb64};

pub const USER: &str = "riley";
pub const DOMAIN: &str = "CONTOSO";
pub const PASSWORD: &str = "pw";

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
    pub draft: bool,
    pub received: String,
}

pub struct Data {
    pub folders: Vec<Fo>,
    pub items: Vec<It>,
    /// Appointments and contacts: (id, folder, the element as created).
    pub pim: Vec<(String, String, El)>,
    /// MIME sources sent.
    pub sent: Vec<Vec<u8>>,
    /// (state, start, end, message)
    /// State, start, end, inside reply, outside reply, outside audience.
    pub oof: (String, String, String, String, String, String),
    /// Sign in with Basic instead of NTLM.
    pub basic: bool,
    /// Operations asked for, in order.
    pub calls: Vec<String>,
    /// NTLM handshakes done (one per connection).
    pub handshakes: usize,
    /// The EWS address, for Autodiscover's answer.
    pub url: String,
    /// People whose calendars this account may change (delegates with editor rights).
    pub editors: Vec<String>,
    /// The folder's rights say nothing of that (as some servers answer for another mailbox's
    /// calendar): only each item's rights do.
    pub item_rights_only: bool,
    /// Listing a mail folder's messages takes this long (a big mailbox's first sync).
    pub hold: Option<std::time::Duration>,
    /// Ids of folders listed with `PidTagAttributeHidden` true.
    pub flagged_hidden: Vec<String>,
    /// Messages whose source can't be had (the server fails on it).
    pub unreadable: Vec<String>,
    /// The public folder mailbox holding the public folders' content (Exchange 2013 and
    /// later): reading inside them is only answered when routed there.
    pub pf_mailbox: Option<String>,
    /// The public folder hierarchy mailbox (Exchange 2013 and later): listing public folders is
    /// only answered when routed there.
    pub pf_hierarchy: Option<String>,
    /// The request's `X-PublicFolderMailbox` header.
    routed: String,
    /// Other mailboxes on the server.
    pub shared: Vec<SharedBox>,
    /// The `X-AnchorMailbox` header of each request about a shared mailbox's folders.
    pub anchors: Vec<String>,
    /// The organisation's directory (Active Directory): (name, address).
    pub directory: Vec<(String, String)>,
    /// A server whose directory answers name each mailbox by its alias (the address's user part,
    /// issue #9): the full name is only in the contact, when one is asked for.
    pub directory_aliases: bool,
    /// The `User-Agent` of each request.
    pub agents: Vec<String>,
    /// Refuse (HTTP 403) every request naming another `User-Agent` (issue #9: a server that lets
    /// in only the clients it knows).
    pub only_agent: Option<String>,
    /// Signed in by a front in front of it (a Kerberos one): no sign-in asked here.
    pub open: bool,
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
            oof: ("Disabled".into(), String::new(), String::new(), String::new(), String::new(), "All".into()),
            basic: false,
            calls: Vec::new(),
            handshakes: 0,
            url: String::new(),
            editors: Vec::new(),
            item_rights_only: false,
            hold: None,
            flagged_hidden: Vec::new(),
            unreadable: Vec::new(),
            pf_mailbox: None,
            pf_hierarchy: None,
            routed: String::new(),
            shared: Vec::new(),
            anchors: Vec::new(),
            directory: Vec::new(),
            directory_aliases: false,
            agents: Vec::new(),
            only_agent: None,
            open: false,
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
                self.pim.iter().any(|(_, f, _)| *f == id).then_some(id)
            }
            "DistinguishedFolderId" => {
                let known = el.attr("Id")?;
                self.folders.iter().find(|f| f.owner.is_empty() && f.known.as_deref() == Some(known)).map(|f| f.id.clone())
            }
            _ => None,
        }
    }

    fn folder_xml(&self, f: &Fo) -> String {
        let total = self.items.iter().filter(|i| i.folder == f.id).count() + self.pim.iter().filter(|(_, x, _)| *x == f.id).count();
        let unread = self.items.iter().filter(|i| i.folder == f.id && !i.read).count();
        // Public folders name the mailbox holding their content (a GUID, as ASCII, NUL-ended).
        let replica = if self.pf_mailbox.is_some() && self.public_ids().contains(&f.id) {
            format!(
                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x6698\" PropertyType=\"Binary\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
                crate::imap::base64(format!("{PF_GUID}\0").as_bytes())
            )
        } else {
            String::new()
        };
        let replica = if self.flagged_hidden.contains(&f.id) {
            format!(
                "{replica}<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x10f4\" PropertyType=\"Boolean\"/><t:Value>true</t:Value></t:ExtendedProperty>"
            )
        } else {
            replica
        };
        // As Exchange answers: each folder in an element of its kind (EWS's folder array).
        let kind = match f.class.as_str() {
            c if c.starts_with("IPF.Contact") => "ContactsFolder",
            c if c.starts_with("IPF.Appointment") => "CalendarFolder",
            c if c.starts_with("IPF.Task") => "TasksFolder",
            _ => "Folder",
        };
        format!(
            "<t:{kind}><t:FolderId Id=\"{}\"/><t:ParentFolderId Id=\"{}\"/><t:FolderClass>{}</t:FolderClass><t:DisplayName>{}</t:DisplayName><t:TotalCount>{total}</t:TotalCount><t:ChildFolderCount>0</t:ChildFolderCount><t:UnreadCount>{unread}</t:UnreadCount>{replica}</t:{kind}>",
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
            x.push_str(&format!("<t:MimeContent CharacterSet=\"UTF-8\">{}</t:MimeContent>", crate::imap::base64(&it.raw)));
        }
        x.push_str(&format!(
            "<t:Subject>{}</t:Subject><t:DateTimeReceived>{}</t:DateTimeReceived><t:Size>{}</t:Size><t:HasAttachments>false</t:HasAttachments>",
            esc(&p.subject),
            it.received,
            it.raw.len()
        ));
        if it.flagged {
            x.push_str(&ext("0x1090", 2));
        }
        x.push_str(&ext("0x0E07", if it.draft { 8 } else { 1 }));
        x.push_str(&format!("<t:ToRecipients>{}</t:ToRecipients>", p.to.iter().map(mb).collect::<String>()));
        if !p.cc.is_empty() {
            x.push_str(&format!("<t:CcRecipients>{}</t:CcRecipients>", p.cc.iter().map(mb).collect::<String>()));
        }
        x.push_str(&format!("<t:From>{}</t:From>", mb(&p.from)));
        x.push_str(&format!("<t:InternetMessageId>&lt;{}&gt;</t:InternetMessageId><t:IsRead>{}</t:IsRead></t:Message>", esc(&p.message_id), it.read));
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
        match self.pim.iter().find(|(i, _, _)| i == id).and_then(|(_, f, _)| f.strip_prefix("mailbox:")) {
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
        let ok = |inner: String| format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>");
        let err =
            |code: &str| format!("<m:R ResponseClass=\"Error\"><m:MessageText>{code}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>");
        let ids = |el: Option<&El>| -> Vec<String> {
            el.map(|e| e.kids.iter().filter_map(|k| k.attr("Id").map(str::to_string)).collect()).unwrap_or_default()
        };
        let mut out = Vec::new();
        match op.name.as_str() {
            "GetFolder" => {
                for f in op.child("FolderIds").map(|f| f.kids.clone()).unwrap_or_default() {
                    // Someone else's calendar: what this account may do there.
                    if let Some(who) = self.folder_id(&f).and_then(|id| id.strip_prefix("mailbox:").map(str::to_string)) {
                        let edit = self.editors.contains(&who) && !self.item_rights_only;
                        out.push(ok(format!(
                            "<m:Folders><t:CalendarFolder><t:FolderId Id=\"mailbox:{who}\"/><t:EffectiveRights><t:CreateAssociated>false</t:CreateAssociated>\
                             <t:CreateContents>{edit}</t:CreateContents><t:CreateHierarchy>false</t:CreateHierarchy><t:Delete>{edit}</t:Delete>\
                             <t:Modify>false</t:Modify><t:Read>true</t:Read></t:EffectiveRights></t:CalendarFolder></m:Folders>"
                        )));
                        continue;
                    }
                    if matches!(self.shared_of(&f), Some(Err(()))) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    let root = f.attr("Id") == Some("publicfoldersroot");
                    match self.folder_id(&f).and_then(|id| self.folders.iter().find(|x| x.id == id).cloned()) {
                        // The root is the hierarchy's; any other public folder, its content's.
                        Some(fo) if root && self.unrouted_hierarchy(&fo.id) => out.push(err("ErrorFolderNotFound")),
                        Some(fo) if !root && self.unrouted(&fo.id) => out.push(err("ErrorFolderNotFound")),
                        Some(fo) => out.push(ok(format!("<m:Folders>{}</m:Folders>", self.folder_xml(&fo)))),
                        None => out.push(err("ErrorFolderNotFound")),
                    }
                }
            }
            // Shallow (public folders): the folders right under the one asked for.
            // The directory: every name or address with a word starting with the search, up to
            // 100, as Exchange's ambiguous name resolution; several are a warning.
            "ResolveNames" if op.attr("SearchScope") == Some("ActiveDirectory") => {
                out.push(self.directory_answer(op));
            }
            "ResolveNames" => {
                let entry = op.text_of("UnresolvedEntry").unwrap_or("").to_ascii_lowercase();
                match self.shared.iter().find(|b| legacy_dn(&b.address).eq_ignore_ascii_case(&entry) || b.address.eq_ignore_ascii_case(&entry)) {
                    Some(b) => out.push(ok(format!(
                        "<m:ResolutionSet TotalItemsInView=\"1\" IncludesLastItemInRange=\"true\"><t:Resolution><t:Mailbox><t:Name>{}</t:Name>\
                         <t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox></t:Resolution></m:ResolutionSet>",
                        esc(&b.name),
                        esc(&b.address)
                    ))),
                    None => out.push(err("ErrorNameResolutionNoResults")),
                }
            }
            "FindFolder" if op.attr("Traversal") == Some("Shallow") => {
                let parent = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                if self.unrouted_hierarchy(&parent) {
                    out.push(err("ErrorFolderNotFound"));
                    return ("FindFolderResponse".into(), out);
                }
                let kids: Vec<String> = self
                    .folders
                    .iter()
                    .filter(|f| f.parent == parent)
                    .map(|f| {
                        let n = self.folders.iter().filter(|x| x.parent == f.id).count();
                        self.folder_xml(f)
                            .replace("<t:ChildFolderCount>0</t:ChildFolderCount>", &format!("<t:ChildFolderCount>{n}</t:ChildFolderCount>"))
                    })
                    .collect();
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Folders>{}</t:Folders></m:RootFolder>", kids.concat())));
            }
            "FindFolder" if op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.shared_of(f)).is_some() => {
                // Deep, in a shared mailbox: refused without access to it.
                let Some(Ok(owner)) = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.shared_of(f)) else {
                    out.push(err("ErrorAccessDenied"));
                    return ("FindFolderResponse".into(), out);
                };
                let all: Vec<String> = self
                    .folders
                    .iter()
                    .filter(|f| f.owner == owner && f.known.as_deref() != Some("msgfolderroot"))
                    .map(|f| self.folder_xml(f))
                    .collect();
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Folders>{}</t:Folders></m:RootFolder>", all.concat())));
            }
            "FindFolder" => {
                // Deep: everything under the root (the mailbox's, not the public folders), in pages.
                let view = op.child("IndexedPageFolderView");
                let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
                let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000).min(3);
                let public = self.public_ids();
                let all: Vec<Fo> = self.folders.iter().filter(|f| f.id != "ROOT" && f.owner.is_empty() && !public.contains(&f.id)).cloned().collect();
                let page: Vec<String> = all.iter().skip(offset).take(max).map(|f| self.folder_xml(f)).collect();
                let last = offset + max >= all.len();
                out.push(ok(format!(
                    "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Folders>{}</t:Folders></m:RootFolder>",
                    all.len(),
                    page.concat()
                )));
            }
            "FindItem" if self.pim_folder(op.child("ParentFolderIds").and_then(|p| p.kids.first())) => {
                let folder = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                if self.unrouted(&folder) {
                    out.push(err("ErrorFolderNotFound"));
                    return ("FindItemResponse".into(), out);
                }
                let items: String =
                    self.pim.iter().filter(|(_, f, _)| *f == folder).map(|(id, _, _)| format!("<t:Item><t:ItemId Id=\"{id}\"/></t:Item>")).collect();
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Items>{items}</t:Items></m:RootFolder>")));
            }
            "FindItem" => {
                // A folder the mailbox hasn't (a Notes folder, say): as Exchange answers.
                let Some(folder) = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)) else {
                    out.push(err("ErrorFolderNotFound"));
                    return ("FindItemResponse".into(), out);
                };
                if self.unrouted(&folder) {
                    out.push(err("ErrorFolderNotFound"));
                    return ("FindItemResponse".into(), out);
                }
                let view = op.child("IndexedPageItemView");
                let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000);
                // Pages by offset, as Exchange does (up to 1000 at a time).
                let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
                let max = max.min(1000);
                let mut list: Vec<&It> = self.items.iter().filter(|i| i.folder == folder).collect();
                list.sort_by(|a, b| b.received.cmp(&a.received));
                // A search: AQS words, or a restriction on the read and flag state.
                if let Some(q) = op.child("QueryString") {
                    list.retain(|i| crate::fake_graph::words_match(&q.text, &i.raw));
                }
                if let Some(r) = op.child("Restriction") {
                    let mut all = Vec::new();
                    r.all("IsEqualTo", &mut all);
                    for cond in all {
                        let value = cond.find("Constant").and_then(|c| c.attr("Value")).unwrap_or("");
                        if cond.find("FieldURI").and_then(|f| f.attr("FieldURI")) == Some("message:IsRead") {
                            let want = value == "true";
                            list.retain(|i| i.read == want);
                        } else if cond.find("ExtendedFieldURI").is_some() {
                            let negated = r.find("Not").is_some();
                            list.retain(|i| i.flagged != negated);
                        }
                    }
                }
                let last = offset + max >= list.len();
                let items: String = list
                    .iter()
                    .skip(offset)
                    .take(max)
                    .map(|i| {
                        format!(
                            "<t:Message><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/><t:IsRead>{}</t:IsRead>{}</t:Message>",
                            i.id,
                            i.read,
                            if i.flagged {
                                "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty>"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect();
                out.push(ok(format!(
                    "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Items>{items}</t:Items></m:RootFolder>",
                    list.len()
                )));
            }
            "GetItem" => {
                let mime = op.find("IncludeMimeContent").is_some_and(|m| m.text == "true");
                for id in ids(op.child("ItemIds")) {
                    if self.pim.iter().any(|(i, f, _)| *i == id && self.unrouted(f)) {
                        out.push(err("ErrorItemNotFound"));
                        continue;
                    }
                    if let Some((_, _, el)) = self.pim.iter().find(|(i, _, _)| *i == id) {
                        let mut el = el.clone();
                        el.kids.insert(
                            0,
                            El {
                                name: "ItemId".into(),
                                attrs: vec![("Id".into(), id.clone()), ("ChangeKey".into(), "ck".into())],
                                ..Default::default()
                            },
                        );
                        // Each item's own rights (`item:EffectiveRights`), when asked for.
                        let mut fields = Vec::new();
                        op.all("FieldURI", &mut fields);
                        if fields.iter().any(|f| f.attr("FieldURI") == Some("item:EffectiveRights")) {
                            let edit = self.may_change(&id);
                            let rights = parse(&format!(
                                "<t:EffectiveRights><t:CreateAssociated>false</t:CreateAssociated><t:CreateContents>false</t:CreateContents>\
                                 <t:CreateHierarchy>false</t:CreateHierarchy><t:Delete>{edit}</t:Delete><t:Modify>{edit}</t:Modify><t:Read>true</t:Read></t:EffectiveRights>"
                            ));
                            el.kids.extend(rights.kids);
                        }
                        out.push(ok(format!("<m:Items>{}</m:Items>", xml(&el))));
                        continue;
                    }
                    match self.items.iter().find(|i| i.id == id) {
                        Some(i) if self.unrouted(&i.folder) => out.push(err("ErrorItemNotFound")),
                        Some(i) if mime && self.unreadable.contains(&i.id) => out.push(err("ErrorInternalServerError")),
                        Some(i) => out.push(ok(format!("<m:Items>{}</m:Items>", self.item_xml(i, mime)))),
                        None => out.push(err("ErrorItemNotFound")),
                    }
                }
            }
            "CreateItem" => {
                let send = op.attr("MessageDisposition") == Some("SendAndSaveCopy");
                let folder = op.child("SavedItemFolderId").and_then(|s| s.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                for m in op.child("Items").map(|i| i.kids.clone()).unwrap_or_default() {
                    let sticky = m.text_of("ItemClass").is_some_and(|c| c.starts_with("IPM.StickyNote"));
                    if folder.strip_prefix("mailbox:").is_some_and(|who| !self.editors.iter().any(|e| e == who)) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    if m.name == "CalendarItem" || m.name == "Contact" || sticky {
                        let id = self.id("P");
                        out.push(ok(format!("<m:Items><t:{0}><t:ItemId Id=\"{id}\" ChangeKey=\"ck\"/></t:{0}></m:Items>", m.name)));
                        self.pim.push((id, folder.clone(), m));
                        continue;
                    }
                    let raw = m.child("MimeContent").and_then(|c| unb64(&c.text)).unwrap_or_default();
                    if send && String::from_utf8_lossy(&raw).contains("nobody@") {
                        out.push(err("ErrorInvalidRecipients"));
                        continue;
                    }
                    let draft = m.children("ExtendedProperty").any(|p| p.text_of("Value") == Some("8"));
                    let id = self.id("I");
                    if send {
                        self.sent.push(raw.clone());
                    }
                    self.items.push(It {
                        id: id.clone(),
                        folder: folder.clone(),
                        raw,
                        read: true,
                        flagged: false,
                        draft,
                        received: "2026-10-15T12:00:00Z".into(),
                    });
                    out.push(ok(if send { String::new() } else { format!("<m:Items><t:Message><t:ItemId Id=\"{id}\"/></t:Message></m:Items>") }));
                }
            }
            "UpdateItem" => {
                for ch in op.child("ItemChanges").map(|c| c.kids.clone()).unwrap_or_default() {
                    let id = ch.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
                    if !self.may_change(&id) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    if let Some((_, _, el)) = self.pim.iter_mut().find(|(i, _, _)| *i == id) {
                        for u in ch.child("Updates").map(|u| u.kids.clone()).unwrap_or_default() {
                            let field =
                                u.find("FieldURI").and_then(|f| f.attr("FieldURI")).and_then(|f| f.split(':').nth(1)).unwrap_or("").to_string();
                            el.kids.retain(|k| k.name != field);
                            if u.name == "SetItemField"
                                && let Some(new) = u.kids.iter().find(|k| k.name != "FieldURI").and_then(|w| w.kids.first())
                            {
                                el.kids.push(new.clone());
                            }
                        }
                        out.push(ok(format!("<m:Items><t:CalendarItem><t:ItemId Id=\"{id}\" ChangeKey=\"ck2\"/></t:CalendarItem></m:Items>")));
                        continue;
                    }
                    let Some(it) = self.items.iter_mut().find(|i| i.id == id) else {
                        out.push(err("ErrorItemNotFound"));
                        continue;
                    };
                    for u in ch.child("Updates").map(|u| u.kids.clone()).unwrap_or_default() {
                        if let Some(r) = u.find("IsRead") {
                            it.read = r.text == "true";
                        }
                        if u.find("ExtendedFieldURI").and_then(|e| e.attr("PropertyTag")) == Some("0x1090") {
                            it.flagged = u.name == "SetItemField";
                        }
                    }
                    out.push(ok(String::new()));
                }
            }
            "MoveItem" => {
                let to = op.child("ToFolderId").and_then(|t| t.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                for id in ids(op.child("ItemIds")) {
                    let new = self.id("I");
                    match self.items.iter_mut().find(|i| i.id == id) {
                        Some(it) => {
                            // Exchange gives a moved item a new id.
                            it.id = new.clone();
                            it.folder = to.clone();
                            out.push(ok(format!("<m:Items><t:Message><t:ItemId Id=\"{new}\"/></t:Message></m:Items>")));
                        }
                        None => out.push(err("ErrorItemNotFound")),
                    }
                }
            }
            "DeleteItem" => {
                for id in ids(op.child("ItemIds")) {
                    if !self.may_change(&id) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    self.items.retain(|i| i.id != id);
                    self.pim.retain(|(i, _, _)| *i != id);
                    out.push(ok(String::new()));
                }
            }
            "CreateFolder" => {
                let parent = op.child("ParentFolderId").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_else(|| "ROOT".into());
                let name = op.find("DisplayName").map(|d| d.text.clone()).unwrap_or_default();
                let id = self.folder(&name, Some(&parent), None, "IPF.Note");
                out.push(ok(format!("<m:Folders><t:Folder><t:FolderId Id=\"{id}\"/></t:Folder></m:Folders>")));
            }
            "MoveFolder" => {
                let to = op.child("ToFolderId").and_then(|t| t.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_else(|| "ROOT".into());
                for id in ids(op.child("FolderIds")) {
                    if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
                        f.parent = to.clone();
                    }
                    out.push(ok(format!("<m:Folders><t:Folder><t:FolderId Id=\"{id}\"/></t:Folder></m:Folders>")));
                }
            }
            "UpdateFolder" => {
                for ch in op.child("FolderChanges").map(|c| c.kids.clone()).unwrap_or_default() {
                    let id = ch.child("FolderId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
                    let name = ch.find("DisplayName").map(|d| d.text.clone()).unwrap_or_default();
                    if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
                        f.name = name;
                    }
                    out.push(ok(String::new()));
                }
            }
            "DeleteFolder" => {
                for id in ids(op.child("FolderIds")) {
                    self.folders.retain(|f| f.id != id);
                    out.push(ok(String::new()));
                }
            }
            "EmptyFolder" => {
                for id in ids(op.child("FolderIds")) {
                    self.items.retain(|i| i.folder != id);
                    out.push(ok(String::new()));
                }
            }
            "GetUserOofSettingsRequest" => {
                let (state, start, end, msg, outside, audience) = self.oof.clone();
                return (
                    "GetUserOofSettingsResponse".into(),
                    vec![format!(
                        "<ResponseMessage ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode></ResponseMessage>\
                         <OofSettings><OofState>{state}</OofState><ExternalAudience>{audience}</ExternalAudience>\
                         <Duration><StartTime>{start}</StartTime><EndTime>{end}</EndTime></Duration>\
                         <InternalReply><Message>{}</Message></InternalReply><ExternalReply><Message>{}</Message></ExternalReply></OofSettings>",
                        esc(&msg),
                        esc(&outside)
                    )],
                );
            }
            "SetUserOofSettingsRequest" => {
                let s = op.child("UserOofSettings");
                let get = |n: &str| s.and_then(|s| s.find(n)).map(|e| e.text.clone()).unwrap_or_default();
                let msg = s.and_then(|s| s.find("InternalReply")).and_then(|r| r.child("Message")).map(|m| m.text.clone()).unwrap_or_default();
                if msg == "refuse" {
                    let refused = "<ResponseMessage ResponseClass=\"Error\"><MessageText>refused</MessageText><ResponseCode>ErrorInvalidParameter</ResponseCode></ResponseMessage>";
                    return ("SetUserOofSettingsResponse".into(), vec![refused.into()]);
                }
                let outside = s.and_then(|s| s.find("ExternalReply")).and_then(|r| r.child("Message")).map(|m| m.text.clone()).unwrap_or_default();
                self.oof = (get("OofState"), get("StartTime"), get("EndTime"), msg, outside, get("ExternalAudience"));
                return (
                    "SetUserOofSettingsResponse".into(),
                    vec!["<ResponseMessage ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode></ResponseMessage>".into()],
                );
            }
            other => {
                out.push(err(&format!("ErrorInvalidOperation:{other}")));
            }
        }
        (format!("{}Response", op.name), out)
    }
}

pub struct Server {
    pub url: String,
    pub data: Arc<Mutex<Data>>,
}

fn respond(s: &mut impl Write, status: u16, headers: &[(&str, String)], body: &[u8]) {
    let mut head = format!("HTTP/1.1 {status} X\r\nContent-Type: text/xml; charset=utf-8\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// Start the server; it runs until the test process ends.
pub fn serve(data: Data) -> Server {
    serve_at(data, "127.0.0.1:0")
}

/// Start the server on an address of its own.
fn serve_at(data: Data, at: &str) -> Server {
    let l = TcpListener::bind(at).unwrap();
    let url = format!("http://127.0.0.1:{}/EWS/Exchange.asmx", l.local_addr().unwrap().port());
    let mut data = data;
    data.url = url.clone();
    let data = Arc::new(Mutex::new(data));
    let shared = data.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(s) = conn else { continue };
            let shared = shared.clone();
            std::thread::spawn(move || serve_connection(s, &shared));
        }
    });
    Server { url, data }
}

/// Requests on one kept-open connection: NTLM signs it in once.
fn serve_connection(mut s: std::net::TcpStream, shared: &Arc<Mutex<Data>>) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut signed_in = false;
    let mut challenge: Option<crate::ntlm::Challenge> = None;
    loop {
        let mut first = String::new();
        if reader.read_line(&mut first).unwrap_or(0) == 0 {
            return;
        }
        let (mut len, mut auth, mut routed, mut anchor, mut agent) = (0usize, String::new(), String::new(), String::new(), String::new());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if k.eq_ignore_ascii_case("authorization") {
                    auth = v.trim().to_string();
                }
                if k.eq_ignore_ascii_case("x-publicfoldermailbox") {
                    routed = v.trim().to_string();
                }
                if k.eq_ignore_ascii_case("x-anchormailbox") {
                    anchor = v.trim().to_string();
                }
                if k.eq_ignore_ascii_case("user-agent") {
                    agent = v.trim().to_string();
                }
            }
        }
        let mut body = vec![0u8; len];
        let _ = reader.read_exact(&mut body);
        let only = {
            let mut d = shared.lock().unwrap();
            d.agents.push(agent.clone());
            d.only_agent.clone()
        };
        if only.is_some_and(|o| o != agent) {
            respond(&mut s, 403, &[], b"Forbidden");
            continue;
        }
        let (basic, open) = {
            let d = shared.lock().unwrap();
            (d.basic, d.open)
        };
        if basic && !open {
            let expected = format!("Basic {}", crate::imap::base64(format!("{DOMAIN}\\{USER}:{PASSWORD}").as_bytes()));
            if auth != expected {
                respond(&mut s, 401, &[("WWW-Authenticate", "Basic realm=\"x\"".into())], b"");
                continue;
            }
        } else if !signed_in && !open {
            match auth.strip_prefix("NTLM ").and_then(unb64) {
                Some(m) if m.get(8) == Some(&1) => {
                    let c = crate::ntlm::make_challenge([9; 8], &[]);
                    challenge = crate::ntlm::challenge(&c).ok();
                    respond(&mut s, 401, &[("WWW-Authenticate", format!("NTLM {}", crate::imap::base64(&c)))], b"");
                    continue;
                }
                Some(m) if m.get(8) == Some(&3) => {
                    let ok = challenge.as_ref().is_some_and(|c| crate::ntlm::verify(c, &m, USER, DOMAIN, PASSWORD));
                    if !ok {
                        respond(&mut s, 401, &[("WWW-Authenticate", "NTLM".into()), ("WWW-Authenticate", "Negotiate".into())], b"");
                        continue;
                    }
                    signed_in = true;
                    shared.lock().unwrap().handshakes += 1;
                }
                _ => {
                    respond(&mut s, 401, &[("WWW-Authenticate", "Negotiate".into()), ("WWW-Authenticate", "NTLM".into())], b"");
                    continue;
                }
            }
        }
        let doc = parse(&String::from_utf8_lossy(&body));
        // Autodiscover: this server's own address.
        if doc.find("Autodiscover").is_some_and(|a| a.find("EMailAddress").is_some()) {
            let (url, pf, hierarchy) = {
                let d = shared.lock().unwrap();
                (d.url.clone(), d.pf_mailbox.clone(), d.pf_hierarchy.clone())
            };
            // The public folder mailbox, asked for by its GUID: its address.
            let asked = doc.find("EMailAddress").map(|e| e.text.clone()).unwrap_or_default();
            if let Some(pf) = pf.filter(|_| asked.starts_with(PF_GUID)) {
                let answer = format!(
                    "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Public folders</DisplayName>\
                     <AutoDiscoverSMTPAddress>{pf}</AutoDiscoverSMTPAddress></User><Account><AccountType>email</AccountType><Action>settings</Action>\
                     <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol></Account></Response></Autodiscover>"
                );
                respond(&mut s, 200, &[], answer.as_bytes());
                continue;
            }
            // The account's: its server, and its public folder hierarchy mailbox.
            let public =
                hierarchy.map(|h| format!("<PublicFolderInformation><SmtpAddress>{h}</SmtpAddress></PublicFolderInformation>")).unwrap_or_default();
            // Mailboxes mapped to the account (full access, automapped), as Outlook is told.
            let mapped: String = shared
                .lock()
                .unwrap()
                .shared
                .iter()
                .filter(|b| b.automap)
                .map(|b| {
                    let id = if b.legacy {
                        format!("<LegacyDN>{}</LegacyDN><Server>mbx.contoso.example</Server>", legacy_dn(&b.address))
                    } else {
                        format!("<SmtpAddress>{}</SmtpAddress><OwnerSmtpAddress>{}</OwnerSmtpAddress>", b.address, b.address)
                    };
                    format!("<AlternativeMailbox><Type>Delegate</Type><DisplayName>{}</DisplayName>{id}</AlternativeMailbox>", esc(&b.name))
                })
                .collect();
            let public = format!(
                "{public}{mapped}<AlternativeMailbox><Type>Archive</Type><DisplayName>Online Archive</DisplayName><SmtpAddress>archive@contoso.example</SmtpAddress></AlternativeMailbox>"
            );
            let answer = format!(
                "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Riley</DisplayName></User><Account><AccountType>email</AccountType><Action>settings</Action>\
                 <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol>{public}</Account></Response></Autodiscover>"
            );
            respond(&mut s, 200, &[], answer.as_bytes());
            continue;
        }
        let Some(op) = doc.find("Body").and_then(|b| b.kids.first()).cloned() else {
            respond(
                &mut s,
                500,
                &[],
                b"<soap:Envelope><soap:Body><soap:Fault><faultstring>no operation</faultstring></soap:Fault></soap:Body></soap:Envelope>",
            );
            continue;
        };
        // A slow server: listing a mail folder takes its time (without holding up the others).
        let hold = {
            let d = shared.lock().unwrap();
            d.hold.filter(|_| op.name == "FindItem" && !d.pim_folder(op.child("ParentFolderIds").and_then(|p| p.kids.first())))
        };
        if let Some(h) = hold {
            std::thread::sleep(h);
        }
        let mut d = shared.lock().unwrap();
        d.calls.push(op.name.clone());
        d.routed = routed;
        if !anchor.is_empty() && d.anchors.len() < 1000 {
            d.anchors.push(anchor);
        }
        let (name, parts) = d.route(&op);
        drop(d);
        let inner = if name.starts_with("GetUserOof") || name.starts_with("SetUserOof") {
            parts.concat()
        } else {
            let tag = name.trim_end_matches("Response").to_string() + "ResponseMessage";
            format!(
                "<m:ResponseMessages>{}</m:ResponseMessages>",
                parts.concat().replace("<m:R ", &format!("<m:{tag} ")).replace("</m:R>", &format!("</m:{tag}>"))
            )
        };
        let xml = format!(
            "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><m:{name} xmlns:m=\"m\" xmlns:t=\"t\">{inner}</m:{name}></s:Body></s:Envelope>"
        );
        respond(&mut s, 200, &[], xml.as_bytes());
    }
}

/// An element as XML again (for items kept as they were created).
fn xml(e: &El) -> String {
    let attrs: String = e.attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect();
    let inner: String = e.kids.iter().map(xml).collect();
    format!("<t:{0}{attrs}>{1}{inner}</t:{0}>", e.name, esc(&e.text))
}

mod directory;
#[cfg(test)]
mod tests;
