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
    next: u32,
}

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
            next: 0,
        };
        d.folders.push(Fo {
            id: "ROOT".into(),
            name: "Top of Information Store".into(),
            parent: String::new(),
            class: "IPF.Note".into(),
            known: Some("msgfolderroot".into()),
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
        });
        id
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
            "DistinguishedFolderId" if el.child("Mailbox").is_some() => {
                let who = el.find("EmailAddress").map(|e| e.text.to_ascii_lowercase())?;
                let id = format!("mailbox:{who}");
                self.pim.iter().any(|(_, f, _)| *f == id).then_some(id)
            }
            "DistinguishedFolderId" => {
                let known = el.attr("Id")?;
                self.folders.iter().find(|f| f.known.as_deref() == Some(known)).map(|f| f.id.clone())
            }
            _ => None,
        }
    }

    fn folder_xml(&self, f: &Fo) -> String {
        let total = self.items.iter().filter(|i| i.folder == f.id).count();
        let unread = self.items.iter().filter(|i| i.folder == f.id && !i.read).count();
        format!(
            "<t:Folder><t:FolderId Id=\"{}\"/><t:ParentFolderId Id=\"{}\"/><t:FolderClass>{}</t:FolderClass><t:DisplayName>{}</t:DisplayName><t:TotalCount>{total}</t:TotalCount><t:ChildFolderCount>0</t:ChildFolderCount><t:UnreadCount>{unread}</t:UnreadCount></t:Folder>",
            f.id,
            f.parent,
            f.class,
            esc(&f.name)
        )
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
                        let edit = self.editors.contains(&who);
                        out.push(ok(format!(
                            "<m:Folders><t:CalendarFolder><t:FolderId Id=\"mailbox:{who}\"/><t:EffectiveRights><t:CreateAssociated>false</t:CreateAssociated>\
                             <t:CreateContents>{edit}</t:CreateContents><t:CreateHierarchy>false</t:CreateHierarchy><t:Delete>{edit}</t:Delete>\
                             <t:Modify>{edit}</t:Modify><t:Read>true</t:Read></t:EffectiveRights></t:CalendarFolder></m:Folders>"
                        )));
                        continue;
                    }
                    match self.folder_id(&f).and_then(|id| self.folders.iter().find(|x| x.id == id).cloned()) {
                        Some(fo) => out.push(ok(format!("<m:Folders>{}</m:Folders>", self.folder_xml(&fo)))),
                        None => out.push(err("ErrorFolderNotFound")),
                    }
                }
            }
            // Shallow (public folders): the folders right under the one asked for.
            "FindFolder" if op.attr("Traversal") == Some("Shallow") => {
                let parent = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
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
            "FindFolder" => {
                // Deep: everything under the root (the mailbox's, not the public folders), in pages.
                let view = op.child("IndexedPageFolderView");
                let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
                let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000).min(3);
                let public = self.public_ids();
                let all: Vec<Fo> = self.folders.iter().filter(|f| f.id != "ROOT" && !public.contains(&f.id)).cloned().collect();
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
                let max: usize =
                    op.child("IndexedPageItemView").and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000);
                let mut list: Vec<&It> = self.items.iter().filter(|i| i.folder == folder).collect();
                list.sort_by(|a, b| b.received.cmp(&a.received));
                let items: String = list
                    .iter()
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
                    "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"true\"><t:Items>{items}</t:Items></m:RootFolder>",
                    list.len()
                )));
            }
            "GetItem" => {
                let mime = op.find("IncludeMimeContent").is_some_and(|m| m.text == "true");
                for id in ids(op.child("ItemIds")) {
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
                        out.push(ok(format!("<m:Items>{}</m:Items>", xml(&el))));
                        continue;
                    }
                    match self.items.iter().find(|i| i.id == id) {
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
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
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
        let (mut len, mut auth) = (0usize, String::new());
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
            }
        }
        let mut body = vec![0u8; len];
        let _ = reader.read_exact(&mut body);
        let basic = shared.lock().unwrap().basic;
        if basic {
            let expected = format!("Basic {}", crate::imap::base64(format!("{DOMAIN}\\{USER}:{PASSWORD}").as_bytes()));
            if auth != expected {
                respond(&mut s, 401, &[("WWW-Authenticate", "Basic realm=\"x\"".into())], b"");
                continue;
            }
        } else if !signed_in {
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
            let url = shared.lock().unwrap().url.clone();
            let answer = format!(
                "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Riley</DisplayName></User><Account><AccountType>email</AccountType><Action>settings</Action>\
                 <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol></Account></Response></Autodiscover>"
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
        let mut d = shared.lock().unwrap();
        d.calls.push(op.name.clone());
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use mailcraft_model::job::Secret;
    use mailcraft_model::{AccountConfig, Event, Flag, Job, Op, SyncStatus};
    use mailcraft_store::Store;

    use super::*;

    fn until(pool: &crate::worker::Pool, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
        let end = Instant::now() + Duration::from_secs(20);
        let mut seen = Vec::new();
        while Instant::now() < end {
            if let Some(e) = pool.wait(Duration::from_millis(100)) {
                let hit = f(&e);
                seen.push(e);
                if hit {
                    return seen;
                }
            }
        }
        panic!("timed out waiting for {what}; saw {seen:?}");
    }

    fn eventually(what: &str, mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {what}");
    }

    fn mail(subject: &str, id: &str) -> Vec<u8> {
        format!("From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
            .into_bytes()
    }

    #[test]
    fn an_on_premises_exchange_account_works_through_ews() {
        let mut d = Data::default();
        let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Drafts", None, Some("drafts"), "IPF.Note");
        let sent = d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
        let trash = d.folder("Deleted Items", None, Some("deleteditems"), "IPF.Note");
        d.folder("Outbox", None, Some("outbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        let projects = d.folder("Projects", None, None, "IPF.Note");
        d.folder("2024", Some(&projects), None, "IPF.Note");
        let first = d.deliver(&inbox, &mail("Storyboard", "m1"), true, "2024-05-06T09:00:00Z");
        let second = d.deliver(&inbox, &mail("Sound mix", "m2"), false, "2024-05-06T10:00:00Z");
        d.deliver(&projects, &mail("Plan", "m3"), true, "2024-05-01T10:00:00Z");
        let server = serve(d);
        let data = server.data.clone();

        let dir = std::env::temp_dir().join(format!("mailcraft-ews-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            name: "Riley".into(),
            email: "riley@contoso.example".into(),
            username: format!("{DOMAIN}\\{USER}"),
            ews: Some(server.url.clone()),
            ..Default::default()
        };
        assert!(!account.is_local() && account.is_api());

        // A wrong password means signing in again.
        pool.submit(Job::Start { account: account.clone(), password: Some(Secret("wrong".into())) });
        until(&pool, "sign in again", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

        pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
        let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        let folders = seen
            .iter()
            .rev()
            .find_map(|e| match e {
                Event::Folders { folders, .. } => Some(folders.clone()),
                _ => None,
            })
            .unwrap();
        let mut paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(paths, ["Deleted Items", "Drafts", "INBOX", "Projects", "Projects/2024", "Sent Items"], "pages followed; no Outbox, no calendar");
        let inbox_folder = folders.iter().find(|f| f.path == "INBOX").unwrap();
        assert_eq!((inbox_folder.total, inbox_folder.unread), (2, 1));
        let st = store.load_folder("a1", "INBOX").unwrap();
        let subjects: Vec<&str> = st.messages.iter().map(|m| m.subject.as_str()).collect();
        assert_eq!(subjects, ["Sound mix", "Storyboard"], "newest first");
        assert!(st.messages[1].flags.seen && !st.messages[0].flags.seen);
        assert_eq!(st.messages[0].from.email, "zoe@contoso.example");
        assert!(store.has_body(&st.messages[0].key), "the newest sources are downloaded");
        assert!(data.lock().unwrap().handshakes <= 2, "NTLM signs in the connection once, not every request");

        // Open a message whose source isn't cached.
        let key = st.messages[0].key.clone();
        store.remove_body(&key);
        pool.submit(Job::FetchBody { key: key.clone() });
        until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

        // Read, flag, move (Exchange gives the moved message a new id).
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
        eventually("read on the server", || data.lock().unwrap().items.iter().any(|i| i.id == second && i.read));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Flagged, on: true } });
        eventually("flagged on the server", || data.lock().unwrap().items.iter().any(|i| i.id == second && i.flagged));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: vec![key.clone()], to: "Projects".into() } });
        eventually("moved on the server", || {
            data.lock().unwrap().items.iter().any(|i| i.folder == projects && String::from_utf8_lossy(&i.raw).contains("Sound mix"))
        });
        until(&pool, "the move shows in Projects", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Projects"));
        eventually("in Projects here, flagged", || {
            store.load_folder("a1", "Projects").unwrap().messages.iter().any(|m| m.subject == "Sound mix" && m.flags.flagged)
        });

        // Folders: create, rename and move to the top, delete.
        pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Projects/Caf&AOk-".into() } });
        eventually("created", || data.lock().unwrap().by_name("Café").is_some_and(|f| f.parent == projects));
        until(&pool, "folder list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Projects/Caf&AOk-")));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Projects/Caf&AOk-".into(), to: "Clients".into() } });
        eventually("renamed", || data.lock().unwrap().by_name("Clients").is_some_and(|f| f.parent == "ROOT"));
        until(&pool, "renamed in the list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Clients")));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteFolder { path: "Clients".into() } });
        eventually("deleted", || data.lock().unwrap().by_name("Clients").is_none());

        // Automatic replies.
        pool.submit(Job::GetVacation { account: "a1".into() });
        until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
        let away = mailcraft_model::job::Vacation {
            enabled: true,
            subject: String::new(),
            text: "Back on Monday".into(),
            from: Some("2026-10-15".into()),
            until: Some("2026-10-20".into()),
            ..Default::default()
        };
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
        until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
        assert_eq!(data.lock().unwrap().oof.0, "Scheduled");
        // Another reply for people outside the organisation, to the user's contacts only.
        let split = mailcraft_model::job::Vacation { external: Some("Out of office".into()), outside: "known".into(), ..away.clone() };
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: split.clone() });
        until(&pool, "two replies", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == split));
        let oof = data.lock().unwrap().oof.clone();
        assert_eq!((oof.4.as_str(), oof.5.as_str()), ("Out of office", "Known"));
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
        until(&pool, "one reply again", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: mailcraft_model::job::Vacation { text: "refuse".into(), ..away.clone() } });
        until(&pool, "a refused change", |e| matches!(e, Event::Vacation { vacation: Some(v), error: Some(_), .. } if *v == away));

        // Empty Deleted Items; delete for good.
        for i in 0..3 {
            data.lock().unwrap().deliver(&trash, &mail(&format!("Old {i}"), &format!("t{i}")), true, "2024-01-01T00:00:00Z");
        }
        pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Deleted Items".into() } });
        eventually("emptied", || !data.lock().unwrap().items.iter().any(|i| i.folder == trash));
        let gone = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "Storyboard").unwrap().key.clone();
        pool.submit(Job::Apply { account: "a1".into(), op: Op::Expunge { keys: vec![gone] } });
        eventually("deleted for good", || !data.lock().unwrap().items.iter().any(|i| i.id == first));

        // Send: the Bcc goes in the MIME for Exchange; Sent Items has the copy.
        let raw = b"From: riley@contoso.example\r\nTo: zoe@contoso.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n".to_vec();
        let send = |id: u64, to: Vec<String>| Job::Send {
            account: "a1".into(),
            id,
            raw: raw.clone(),
            from: "riley@contoso.example".into(),
            recipients: to,
            sent_folder: Some("Sent Items".into()),
        };
        pool.submit(send(7, vec!["zoe@contoso.example".into(), "boss@contoso.example".into()]));
        until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
        let out = String::from_utf8(data.lock().unwrap().sent.last().unwrap().clone()).unwrap();
        assert!(out.starts_with("Bcc: boss@contoso.example\r\nFrom:"), "{out}");
        assert!(data.lock().unwrap().items.iter().any(|i| i.folder == sent));
        pool.submit(send(8, vec!["nobody@contoso.example".into()]));
        until(&pool, "refused", |e| matches!(e, Event::SendFailed { id: 8, .. }));

        // A draft replaces its earlier copy.
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft one", "d1"), folder: "Drafts".into(), replaces: None });
        until(&pool, "draft", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Drafts"));
        let draft = store.load_folder("a1", "Drafts").unwrap().messages.iter().find(|m| m.subject == "Draft one").unwrap().key.clone();
        assert!(store.load_folder("a1", "Drafts").unwrap().messages.iter().any(|m| m.flags.draft));
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft two", "d2"), folder: "Drafts".into(), replaces: Some(draft) });
        eventually("one draft left", || data.lock().unwrap().items.iter().filter(|i| i.draft).count() == 1);
        pool.submit(Job::Remove { account: "a1".into() });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn calendars_and_contacts_sync_and_autodiscover_finds_the_server() {
        use mailcraft_model::ical::CalEvent;
        use mailcraft_model::pim::{LOCAL, Pim, StoredCard, StoredEvent, Tombstone};
        use mailcraft_model::vcard::{Card, Typed};
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let standup = parse(
            "<t:CalendarItem><t:Subject>Stand-up</t:Subject><t:Body BodyType=\"Text\">Daily</t:Body><t:Start>2026-10-05T09:00:00Z</t:Start>\
             <t:End>2026-10-05T09:15:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent><t:Location>Room 2</t:Location><t:UID>standup-1</t:UID>\
             <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday Wednesday Friday</t:DaysOfWeek></t:WeeklyRecurrence>\
             <t:NoEndRecurrence><t:StartDate>2026-10-05Z</t:StartDate></t:NoEndRecurrence></t:Recurrence></t:CalendarItem>",
        );
        d.pim.push((
            "OLD".into(),
            cal.clone(),
            parse("<t:CalendarItem><t:Subject>Gone</t:Subject><t:Start>2026-10-01T09:00:00Z</t:Start></t:CalendarItem>").kids[0].clone(),
        ));
        d.pim.push(("E1".into(), cal.clone(), standup.kids[0].clone()));
        let server = serve(d);
        let user = format!("{DOMAIN}\\{USER}");
        assert_eq!(crate::ews::autodiscover("riley@contoso.example", &user, PASSWORD, Some(&server.url)).unwrap(), server.url);
        let c = crate::ews::Client::connect(&server.url, &user, PASSWORD, "riley@contoso.example").unwrap();
        let mut pim = Pim::default();
        pim.events.push(StoredEvent {
            calendar: LOCAL.into(),
            event: CalEvent {
                uid: "local-1".into(),
                summary: "Poster review".into(),
                start: 1_792_000_000,
                end: 1_792_003_600,
                rrule: Some("FREQ=DAILY;COUNT=3".into()),
                ..Default::default()
            },
            dirty: true,
            ..Default::default()
        });
        pim.cards.push(StoredCard {
            book: LOCAL.into(),
            card: Card {
                uid: "p1".into(),
                name: "Ben Ito".into(),
                emails: vec![Typed { value: "ben@contoso.example".into(), kind: String::new() }],
                ..Default::default()
            },
            dirty: true,
            ..Default::default()
        });
        pim.deleted.push(Tombstone { href: "OLD".into(), etag: String::new(), ..Default::default() });
        let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
        assert!(sync.errors.is_empty(), "{:?}", sync.errors);
        assert_eq!(sync.deleted, ["OLD"]);
        let standup = sync.events.iter().find(|e| e.event.summary == "Stand-up").unwrap();
        assert_eq!(standup.event.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"));
        assert_eq!(
            (standup.event.uid.as_str(), standup.event.location.as_str(), standup.event.end - standup.event.start),
            ("standup-1", "Room 2", 900)
        );
        let poster = sync.events.iter().find(|e| e.event.summary == "Poster review").unwrap();
        assert_eq!(poster.event.rrule.as_deref(), Some("FREQ=DAILY;COUNT=3"), "made on the server with its recurrence");
        assert!(sync.cards.iter().any(|k| k.card.name == "Ben Ito" && k.card.emails[0].value == "ben@contoso.example"));
        assert_eq!(sync.pushed_events.len(), 1);
        // A change goes field by field; the series keeps its id.
        let mut changed = pim.clone();
        changed.deleted.clear();
        changed.cards.clear();
        changed.events = vec![StoredEvent {
            href: standup.href.clone(),
            event: CalEvent { summary: "Stand-up (moved)".into(), rrule: None, ..standup.event.clone() },
            dirty: true,
            ..standup.clone()
        }];
        let sync = crate::ews_pim::sync(&c, &changed, 1_792_000_000).unwrap();
        let moved = sync.events.iter().find(|e| e.href == standup.href).unwrap();
        assert_eq!((moved.event.summary.as_str(), moved.event.rrule.as_deref()), ("Stand-up (moved)", None));
        // No Notes folder on this server: notes stay on this computer.
        assert!(sync.notes.is_none());
    }

    #[test]
    fn sticky_notes_sync_with_the_notes_folder() {
        use mailcraft_model::pim::{Note, Pim, StoredNote};
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let notes = d.folder("Notes", None, Some("notes"), "IPF.StickyNote");
        let old = parse(
            "<t:Message><t:ItemClass>IPM.StickyNote</t:ItemClass><t:Subject>Call Ana</t:Subject><t:Body BodyType=\"Text\">Call Ana\r\nabout the mix</t:Body>\
             <t:ExtendedProperty><t:ExtendedFieldURI PropertySetId=\"0006200E-0000-0000-C000-000000000046\" PropertyId=\"35584\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty></t:Message>",
        );
        d.pim.push(("N1".into(), notes, old.kids[0].clone()));
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let mut pim = Pim::default();
        pim.notes.push(StoredNote {
            note: Note { uid: "n-local".into(), text: "Buy paper\n170 gsm".into(), color: 1, modified: 5 },
            dirty: true,
            ..Default::default()
        });
        let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
        assert!(sync.errors.is_empty(), "{:?}", sync.errors);
        let list = sync.notes.unwrap();
        let ana = list.iter().find(|n| n.href == "note:N1").unwrap();
        assert_eq!((ana.note.text.as_str(), ana.note.color), ("Call Ana\nabout the mix", 2));
        assert!(list.iter().any(|n| n.note.text == "Buy paper\n170 gsm" && n.note.color == 1), "made in the Notes folder, its colour kept");
        assert_eq!(sync.pushed_notes.len(), 1);
        assert!(sync.pushed_notes[0].href.starts_with("note:"));
    }

    #[test]
    fn someone_elses_calendar_opens_read_only() {
        use mailcraft_model::pim::{Pim, shared_calendar_id};
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let review = parse(
            "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>",
        );
        d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), review.kids[0].clone()));
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let pim = Pim { shared: vec!["ana@contoso.example".into(), "nobody@contoso.example".into()], ..Default::default() };
        let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
        let id = shared_calendar_id("ana@contoso.example");
        let cals = sync.calendars.clone().unwrap();
        assert!(cals.iter().any(|x| x.id == id && x.read_only && x.name == "ana@contoso.example"));
        assert!(sync.events.iter().any(|e| e.calendar == id && e.event.summary == "Budget review"));
        assert!(sync.synced_calendars.contains(&id));
        // One that can't be read says so, and the rest still syncs.
        assert!(sync.errors.iter().any(|e| e.starts_with("nobody@contoso.example's calendar")), "{:?}", sync.errors);
        assert!(!sync.synced_calendars.contains(&shared_calendar_id("nobody@contoso.example")));
    }

    /// Someone who made this account an editor of their calendar: events go in, change and go
    /// there; someone who didn't keeps theirs read-only.
    #[test]
    fn someone_elses_calendar_can_be_changed_by_an_editor() {
        use mailcraft_model::ical::CalEvent;
        use mailcraft_model::pim::{Pim, StoredEvent, Tombstone, shared_calendar_id};
        let mut d = Data { editors: vec!["ana@contoso.example".into()], ..Default::default() };
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let item = |subject: &str, uid: &str| {
            parse(&format!(
                "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
                 <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>{uid}</t:UID></t:CalendarItem>"
            ))
            .kids[0]
                .clone()
        };
        d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), item("Budget review", "budget-1")));
        d.pim.push(("S2".into(), "mailbox:ana@contoso.example".into(), item("Team lunch", "lunch-1")));
        d.pim.push(("B1".into(), "mailbox:ben@contoso.example".into(), item("Ben's call", "call-1")));
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let (ana, ben) = (shared_calendar_id("ana@contoso.example"), shared_calendar_id("ben@contoso.example"));
        let mut pim = Pim { shared: vec!["ana@contoso.example".into(), "ben@contoso.example".into()], ..Default::default() };
        let first = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
        let cals = first.calendars.clone().unwrap();
        let ro = |id: &str| cals.iter().find(|x| x.id == id).map(|x| x.read_only);
        assert_eq!((ro(&ana), ro(&ben)), (Some(false), Some(true)));
        let mut review = first.events.iter().find(|e| e.event.summary == "Budget review").cloned().unwrap();
        let lunch = first.events.iter().find(|e| e.event.summary == "Team lunch").cloned().unwrap();
        review.event.summary = "Budget review (moved)".into();
        review.dirty = true;
        pim.events = vec![
            review,
            StoredEvent {
                calendar: ana.clone(),
                event: CalEvent {
                    uid: "new-1".into(),
                    summary: "Poster check".into(),
                    start: 1_792_000_000,
                    end: 1_792_003_600,
                    ..Default::default()
                },
                dirty: true,
                ..Default::default()
            },
        ];
        pim.deleted = vec![Tombstone { href: lunch.href.clone(), etag: String::new(), calendar: ana.clone() }];
        let second = crate::ews_pim::sync(&c, &pim, 1_792_000_100).unwrap();
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert!(second.pushed_events.iter().all(|e| e.calendar == ana), "they stay in Ana's calendar");
        let d = server.data.lock().unwrap();
        let in_ana: Vec<String> = d
            .pim
            .iter()
            .filter(|(_, f, _)| f == "mailbox:ana@contoso.example")
            .filter_map(|(_, _, el)| el.text_of("Subject").map(str::to_string))
            .collect();
        assert!(in_ana.contains(&"Budget review (moved)".to_string()) && in_ana.contains(&"Poster check".to_string()), "{in_ana:?}");
        assert!(!in_ana.contains(&"Team lunch".to_string()), "deleted");
        assert!(!d.pim.iter().any(|(_, f, el)| !f.starts_with("mailbox:") && el.text_of("Subject") == Some("Poster check")), "not in this mailbox");
    }

    #[test]
    fn public_folders_list_under_their_own_folder_and_sync() {
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
        let team = d.folder("Team", Some(&root), None, "IPF.Note");
        let news = d.folder("Announcements", Some(&team), None, "IPF.Note");
        let rooms = d.folder("Rooms", Some(&root), None, "IPF.Appointment");
        let shared = d.folder("Company contacts", Some(&team), None, "IPF.Contact");
        d.pim.push((
            "PC1".into(),
            shared,
            parse("<t:Contact><t:ItemId Id=\"PC1\" ChangeKey=\"k\"/><t:DisplayName>Front desk</t:DisplayName><t:EmailAddresses><t:Entry Key=\"EmailAddress1\">desk@contoso.example</t:Entry></t:EmailAddresses></t:Contact>")
                .kids[0]
                .clone(),
        ));
        d.pim.push((
            "PE1".into(),
            rooms,
            parse("<t:CalendarItem><t:Subject>Room 2 booked</t:Subject><t:Start>2026-10-16T09:00:00Z</t:Start><t:End>2026-10-16T10:00:00Z</t:End><t:UID>room-1</t:UID></t:CalendarItem>")
                .kids[0]
                .clone(),
        ));
        d.deliver(
            &news,
            b"From: hr@contoso.example\r\nSubject: Holiday hours\r\nMessage-ID: <h@x>\r\n\r\nClosed Friday.\r\n",
            false,
            "2026-10-14T09:00:00Z",
        );
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let boxes = c.mailboxes().unwrap();
        let names: Vec<&str> = boxes.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Inbox", "Public Folders", "Team", "Announcements"], "the room calendar isn't mail");
        let pf = boxes.iter().find(|m| m.name == "Public Folders").unwrap();
        let team_box = boxes.iter().find(|m| m.name == "Team").unwrap();
        assert_eq!((pf.parent.as_deref(), team_box.parent.as_deref()), (None, Some(pf.id.as_str())));
        assert_eq!(boxes.iter().find(|m| m.name == "Announcements").unwrap().parent.as_deref(), Some(team_box.id.as_str()));
        // Their mail syncs as any folder's.
        let store = crate::fake::tests_store("public-folders");
        let mut st = crate::jmap::State::default();
        let folders = crate::ews::list_folders(&c, &mut st, "a1").unwrap();
        let news_path = folders.iter().find(|f| f.path.ends_with("Announcements")).map(|f| f.path.clone()).unwrap();
        crate::ews::sync_folder(&c, &mut st, &store, "a1", &news_path, &crate::sync::SyncOptions::default()).unwrap();
        let fs = store.load_folder("a1", &news_path).unwrap();
        assert_eq!(fs.messages.len(), 1);
        assert_eq!(fs.messages[0].subject, "Holiday hours");
        let _ = std::fs::remove_dir_all(store.root());
        // Issue #1: the organisation's shared contacts and calendars come in, read-only.
        let sync = crate::ews_pim::sync(&c, &mailcraft_model::pim::Pim::default(), 0).unwrap();
        let books = sync.books.clone().unwrap_or_default();
        let book = books.iter().find(|b| b.id.starts_with("public:")).unwrap();
        assert_eq!((book.name.as_str(), book.read_only), ("Team › Company contacts", true));
        assert!(sync.cards.iter().any(|k| k.book == book.id && k.card.display() == "Front desk"));
        let cals = sync.calendars.clone().unwrap_or_default();
        let room = cals.iter().find(|c| c.id.starts_with("public:")).unwrap();
        assert_eq!((room.name.as_str(), room.read_only), ("Rooms", true));
        assert!(sync.events.iter().any(|e| e.calendar == room.id && e.event.summary == "Room 2 booked"));
        // A server without public folders lists the mailbox's alone.
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        assert_eq!(c.mailboxes().unwrap().len(), 1);
    }

    #[test]
    fn basic_sign_in_when_that_is_what_the_server_offers() {
        let mut d = Data { basic: true, ..Default::default() };
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        assert_eq!(c.mailboxes().unwrap().len(), 1);
        assert!(matches!(crate::ews::Client::connect(&server.url, "x", "y", "e"), Err(crate::NetError::Auth(_))));
    }
}
