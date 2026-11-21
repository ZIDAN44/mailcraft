//! The fake server's items: messages and calendar, contacts and notes items found, read, made,
//! changed, moved and deleted.

use super::*;

impl Data {
    /// `FindItem`: a calendar, contacts or notes folder's items, or a mail folder's messages.
    pub(super) fn find_item(&self, op: &El) -> Vec<String> {
        let parent = op.child("ParentFolderIds").and_then(|p| p.kids.first());
        if self.pim_folder(parent) {
            let folder = parent.and_then(|f| self.folder_id(f)).unwrap_or_default();
            if self.unrouted(&folder) || self.refuses_folder(&folder) {
                return vec![err(if self.unrouted(&folder) { "ErrorFolderNotFound" } else { "ErrorAccessDenied" })];
            }
            // A series' changed occurrences aren't listed, as Exchange's listing (with no
            // calendar view) gives the series alone: they're read by the ids it names.
            let items: String = self
                .pim
                .iter()
                .filter(|p| p.folder == folder && p.el.text_of("CalendarItemType") != Some("Exception"))
                .map(|p| format!("<t:Item><t:ItemId Id=\"{}\"/></t:Item>", p.id))
                .collect();
            return vec![ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Items>{items}</t:Items></m:RootFolder>"))];
        }
        // A folder the mailbox hasn't (a Notes folder, say): as Exchange answers.
        let Some(folder) = parent.and_then(|f| self.folder_id(f)) else {
            return vec![err("ErrorFolderNotFound")];
        };
        if self.unrouted(&folder) {
            return vec![err("ErrorFolderNotFound")];
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
            // When it came (issue #179): its time as Exchange writes it compares as text.
            for (kind, keep) in [("IsGreaterThanOrEqualTo", true), ("IsLessThan", false)] {
                let mut those = Vec::new();
                r.all(kind, &mut those);
                for cond in those {
                    let at = cond.find("Constant").and_then(|c| c.attr("Value")).unwrap_or("").to_string();
                    list.retain(|i| (i.received.as_str() >= at.as_str()) == keep);
                }
            }
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
                    super::follow_up_xml(i)
                )
            })
            .collect();
        vec![ok(format!(
            "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Items>{items}</t:Items></m:RootFolder>",
            list.len()
        ))]
    }

    /// `GetItem`: each item asked for, a message with its MIME source when asked; a calendar or
    /// contacts item as it was made, with its own rights when asked.
    pub(super) fn get_item(&self, op: &El) -> Vec<String> {
        let mime = op.find("IncludeMimeContent").is_some_and(|m| m.text == "true");
        let mut out = Vec::new();
        for id in ids(op.child("ItemIds")) {
            if self.pim.iter().any(|p| p.id == id && self.unrouted(&p.folder)) {
                out.push(err("ErrorItemNotFound"));
                continue;
            }
            if let Some(PimItem { el, .. }) = self.pim.iter().find(|p| p.id == id) {
                if let Some(code) = self.refuses_item(op, &id, el) {
                    out.push(err(code));
                    continue;
                }
                let mut el = el.clone();
                el.kids.insert(
                    0,
                    El { name: "ItemId".into(), attrs: vec![("Id".into(), id.clone()), ("ChangeKey".into(), "ck".into())], ..Default::default() },
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
                Some(i) => {
                    let mut x = self.item_xml(i, mime);
                    if asks_for(op, "item:Body") {
                        x = x.replace("</t:Message>", &format!("{}</t:Message>", body_and_files(&i.raw)));
                    }
                    out.push(ok(format!("<m:Items>{x}</m:Items>")));
                }
                None => out.push(err("ErrorItemNotFound")),
            }
        }
        out
    }

    /// `CreateItem`: a calendar, contacts, notes or tasks item kept as made; a message from its MIME
    /// source, saved (a draft) or sent and saved.
    pub(super) fn create_item(&mut self, op: &El) -> Vec<String> {
        let send = op.attr("MessageDisposition") == Some("SendAndSaveCopy");
        let folder = op.child("SavedItemFolderId").and_then(|s| s.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
        let mut out = Vec::new();
        for m in op.child("Items").map(|i| i.kids.clone()).unwrap_or_default() {
            let sticky = m.text_of("ItemClass").is_some_and(|c| c.starts_with("IPM.StickyNote"));
            if folder.strip_prefix("mailbox:").is_some_and(|who| !self.editors.iter().any(|e| e == who)) || self.unrouted(&folder) {
                out.push(err(if self.unrouted(&folder) { "ErrorFolderNotFound" } else { "ErrorAccessDenied" }));
                continue;
            }
            if m.name == "CalendarItem" || m.name == "Contact" || m.name == "Task" || sticky {
                let id = self.id("P");
                out.push(ok(format!("<m:Items><t:{0}><t:ItemId Id=\"{id}\" ChangeKey=\"ck\"/></t:{0}></m:Items>", m.name)));
                self.pim.push(PimItem { id, folder: folder.clone(), el: m });
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
                complete: false,
                due: None,
                reminder_set: false,
                reminder_signal: None,
                reminder_time: None,
            });
            out.push(ok(if send { String::new() } else { format!("<m:Items><t:Message><t:ItemId Id=\"{id}\"/></t:Message></m:Items>") }));
        }
        out
    }

    /// `UpdateItem`: a calendar or contacts item's fields set or taken off; a message's read and
    /// flag state.
    pub(super) fn update_item(&mut self, op: &El) -> Vec<String> {
        let mut out = Vec::new();
        for ch in op.child("ItemChanges").map(|c| c.kids.clone()).unwrap_or_default() {
            let id = ch.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
            if !self.may_change(&id) {
                out.push(err("ErrorAccessDenied"));
                continue;
            }
            if let Some(PimItem { el, .. }) = self.pim.iter_mut().find(|p| p.id == id) {
                for u in ch.child("Updates").map(|u| u.kids.clone()).unwrap_or_default() {
                    let field = u.find("FieldURI").and_then(|f| f.attr("FieldURI")).and_then(|f| f.split(':').nth(1)).unwrap_or("").to_string();
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
                let uri = u.find("ExtendedFieldURI");
                if uri.and_then(|e| e.attr("PropertyTag")) == Some("0x1090") {
                    // 2 flagged, 1 complete; taken off, neither.
                    let value = u.find("Value").map(|v| v.text.trim().to_string());
                    let set = u.name == "SetItemField";
                    it.flagged = set && value.as_deref() != Some("1");
                    it.complete = set && value.as_deref() == Some("1");
                }
                // A follow-up's due day (`PidLidTaskDueDate`), and its reminder's properties.
                let value = if u.name == "SetItemField" { u.find("Value").map(|v| v.text.trim().to_string()) } else { None };
                match uri.and_then(|e| e.attr("PropertyId")) {
                    Some("33029") => it.due = value,
                    Some("34051") => it.reminder_set = value.as_deref() == Some("true"),
                    Some("34144") => it.reminder_signal = value,
                    Some("34050") => it.reminder_time = value,
                    _ => {}
                }
            }
            out.push(ok(String::new()));
        }
        out
    }

    /// `MoveItem`: each message into the folder, said by its new id.
    pub(super) fn move_item(&mut self, op: &El) -> Vec<String> {
        let to = op.child("ToFolderId").and_then(|t| t.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
        let mut out = Vec::new();
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
        out
    }

    pub(super) fn delete_item(&mut self, op: &El) -> Vec<String> {
        let mut out = Vec::new();
        for id in ids(op.child("ItemIds")) {
            // Not an id of Exchange's at all: one starting `MALFORMED`, or none.
            if id.is_empty() || id.starts_with("MALFORMED") {
                out.push(err(if id.is_empty() { "ErrorInvalidIdEmpty" } else { "ErrorInvalidIdMalformed" }));
                continue;
            }
            if !self.may_change(&id) {
                out.push(err("ErrorAccessDenied"));
                continue;
            }
            if !self.items.iter().any(|i| i.id == id) && !self.pim.iter().any(|p| p.id == id) {
                out.push(err("ErrorItemNotFound"));
                continue;
            }
            // A task is deleted only with `AffectedTaskOccurrences` said, as Exchange wants it
            // (issue #230); other items take it too.
            if op.attr("AffectedTaskOccurrences").is_none() && self.pim.iter().any(|p| p.id == id && p.el.name == "Task") {
                out.push(err("ErrorInvalidRequest"));
                continue;
            }
            self.items.retain(|i| i.id != id);
            self.pim.retain(|p| p.id != id);
            out.push(ok(String::new()));
        }
        out
    }
}

/// Whether an operation asks for the field `uri`.
fn asks_for(op: &El, uri: &str) -> bool {
    let mut fields = Vec::new();
    op.all("FieldURI", &mut fields);
    fields.iter().any(|f| f.attr("FieldURI") == Some(uri))
}

/// A message's body and the list of its files, without their bytes, as Exchange gives them when
/// asked for (`item:Body`, `item:Attachments`): what its text first is made of (issue #24).
fn body_and_files(raw: &[u8]) -> String {
    let Some(p) = mailcraft_mime::parse(raw) else { return String::new() };
    let mut x = format!("<t:Body BodyType=\"Text\">{}</t:Body>", esc(&p.text));
    if !p.attachments.is_empty() {
        x.push_str("<t:Attachments>");
        for a in &p.attachments {
            let id = a.content_id.as_deref().map(|c| format!("<t:ContentId>{}</t:ContentId>", esc(c))).unwrap_or_default();
            x.push_str(&format!(
                "<t:FileAttachment><t:Name>{}</t:Name><t:ContentType>{}</t:ContentType>{id}<t:Size>{}</t:Size><t:IsInline>{}</t:IsInline></t:FileAttachment>",
                esc(&a.name),
                esc(&a.mime),
                a.size,
                a.inline
            ));
        }
        x.push_str("</t:Attachments>");
    }
    x
}
