//! EWS: the mailbox (its folders, their messages, sending, automatic replies).

use super::*;

impl Client {
    /// The account's mail folders, with their roles and counts.
    pub fn mailboxes(&self) -> Result<Vec<Mailbox>> {
        let roles = SPECIAL;
        let hidden = ["msgfolderroot", "outbox"];
        let ids: String = roles.iter().map(|(n, _)| *n).chain(hidden).map(|n| format!("<t:DistinguishedFolderId Id=\"{n}\"/>")).collect();
        let doc = self.call(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds>{ids}</m:FolderIds></m:GetFolder>"
        ))?;
        // One answer per asked-for folder, in order; missing ones are errors.
        let found = folder_ids(&doc);
        let mut role_of: HashMap<String, FolderRole> = roles.iter().zip(&found).filter_map(|((_, r), id)| Some((id.clone()?, *r))).collect();
        let root = found.get(roles.len()).cloned().flatten();
        let mut skip: Vec<String> = found.iter().skip(roles.len() + 1).flatten().cloned().collect();
        for (name, id) in self.newer_ids("") {
            if name == "archive" {
                role_of.entry(id).or_insert(FolderRole::Archive);
            } else {
                skip.push(id);
            }
        }

        let mut out = self.mail_folders("<t:DistinguishedFolderId Id=\"msgfolderroot\"/>", &skip, MAX_FOLDERS)?;
        for m in &mut out {
            // Those at the top have none.
            if m.parent == root {
                m.parent = None;
            }
            m.role = role_of.get(&m.id).copied().unwrap_or(FolderRole::Other);
        }
        keep_rooted(&mut out);
        // Exchange's public folders, under a folder of their own (a server without them, or
        // one that won't show them, just has none).
        match self.public_folders() {
            Ok(pf) => out.extend(pf),
            Err(e) => log::info!("no public folders: {e}"),
        }
        // Shared mailboxes, each under a folder of its own (named so it can't be taken for one
        // of the account's folders).
        let mut open = Vec::new();
        self.shared_roles.borrow_mut().clear();
        // Kept for the calendars and contacts read next.
        let found = self.shared_mailboxes();
        *self.shared_listed.borrow_mut() = Listed::now(found.clone());
        for sm in found {
            let taken = |n: &str| out.iter().any(|m| m.parent.is_none() && m.name.eq_ignore_ascii_case(n));
            let top = if taken(&sm.name) { format!("{} ({})", sm.name, sm.address) } else { sm.name.clone() };
            match self.shared_folders(&sm, &top) {
                Ok(f) if !f.is_empty() => {
                    log::info!("shared mailbox {}: {} folders", sm.address, f.len().saturating_sub(1));
                    open.push(sm.address.clone());
                    out.extend(f);
                }
                Ok(_) => log::info!("shared mailbox {}: no folders", sm.address),
                Err(e) => log::warn!("shared mailbox {}: {e}", sm.address),
            }
        }
        *self.shared_open.borrow_mut() = open;
        Ok(out)
    }

    /// The ids of [`NEWER`]'s folders in the account's mailbox, or in another's (`mb`, the
    /// `Mailbox` element naming it), by name: asked for together, and one by one when the server
    /// refuses that, as Exchange 2010's schema does (#149). One the server refuses alone isn't
    /// asked again on this connection; one it hasn't (no Archive folder made yet) is left out.
    pub(super) fn newer_ids(&self, mb: &str) -> Vec<(&'static str, String)> {
        let ask = |names: &[&'static str]| -> Result<Vec<(&'static str, String)>> {
            let ids: String = names.iter().map(|n| format!("<t:DistinguishedFolderId Id=\"{n}\">{mb}</t:DistinguishedFolderId>")).collect();
            let doc = self.call(&format!(
                "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds>{ids}</m:FolderIds></m:GetFolder>"
            ))?;
            Ok(names.iter().zip(folder_ids(&doc)).filter_map(|(n, id)| Some((*n, id?))).collect())
        };
        let names: Vec<&'static str> = NEWER.into_iter().filter(|n| !self.unknown_ids.borrow().contains(n)).collect();
        if names.is_empty() {
            return Vec::new();
        }
        match ask(&names) {
            Ok(found) => return found,
            Err(e) => log::info!("the folders Exchange 2013 brought, asked one by one: {e}"),
        }
        let mut found = Vec::new();
        for n in names {
            match ask(&[n]) {
                Ok(f) => found.extend(f),
                // Refused by what it is: not asked again. A connection lost isn't that.
                Err(e @ (NetError::Protocol(_) | NetError::No(_) | NetError::Bad(_))) => {
                    log::info!("the server doesn't know the {n} folder: {e}");
                    self.unknown_ids.borrow_mut().push(n);
                }
                Err(e) => log::info!("the {n} folder: {e}"),
            }
        }
        found
    }

    /// The mail folders anywhere under a mailbox's root (`root`, the folder id element naming
    /// it), a page at a time, up to `max`: their names, parents and counts, each with no role
    /// yet. Folders of other classes (calendars, contacts), those in `skip` (its Outbox and the
    /// like) and those flagged hidden are left out; what's in them, the caller's
    /// [`keep_rooted`] drops.
    pub(super) fn mail_folders(&self, root: &str, skip: &[String], max: usize) -> Result<Vec<Mailbox>> {
        let mut out: Vec<Mailbox> = Vec::new();
        let mut offset = 0;
        loop {
            let doc = self.call(&format!(
                "<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:ParentFolderId\"/>\
                 <t:FieldURI FieldURI=\"folder:TotalCount\"/><t:FieldURI FieldURI=\"folder:UnreadCount\"/>\
                 <t:FieldURI FieldURI=\"folder:FolderClass\"/>{HIDDEN}</t:AdditionalProperties></m:FolderShape>\
                 <m:IndexedPageFolderView MaxEntriesReturned=\"500\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
                 <m:ParentFolderIds>{root}</m:ParentFolderIds></m:FindFolder>"
            ))?;
            let Some(m) = messages(&doc).first().copied() else { break };
            check_message(m)?;
            let Some(rf) = m.child("RootFolder") else { break };
            let folders = rf.folders();
            let n = folders.len();
            for f in folders {
                let Some(id) = f.child("FolderId").and_then(|i| i.attr("Id")).map(str::to_string) else { continue };
                // Skip anything flagged PidTagAttributeHidden: server bookkeeping, not mail.
                if !is_mail_class(f.text_of("FolderClass").unwrap_or("")) || skip.contains(&id) || is_hidden(f) || out.len() >= max {
                    continue;
                }
                out.push(Mailbox {
                    name: f.text_of("DisplayName").unwrap_or_default().chars().take(255).collect(),
                    parent: f.child("ParentFolderId").and_then(|p| p.attr("Id")).map(str::to_string),
                    role: FolderRole::Other,
                    total: count_of(f, "TotalCount"),
                    unread: count_of(f, "UnreadCount"),
                    id,
                });
            }
            let last = rf.attr("IncludesLastItemInRange").is_none_or(|v| v == "true");
            offset += n;
            if last || n == 0 || out.len() >= max {
                break;
            }
        }
        Ok(out)
    }

    /// A folder's (total, unread).
    pub(super) fn counts(&self, folder: &str) -> Result<(u32, u32)> {
        let m = self.call_one(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
             <t:FieldURI FieldURI=\"folder:TotalCount\"/><t:FieldURI FieldURI=\"folder:UnreadCount\"/></t:AdditionalProperties></m:FolderShape>\
             <m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:GetFolder>",
            esc(folder)
        ))?;
        let f = m.folders().into_iter().next();
        let count = |k: &str| f.map_or(0, |f| count_of(f, k));
        Ok((count("TotalCount"), count("UnreadCount")))
    }

    /// The newest `limit` messages' ids and flags, a page of up to a thousand at a time (EWS
    /// pages by offset; it says when the last page is in).
    pub(super) fn newest(&self, folder: &str, limit: usize) -> Result<Vec<(String, Flags)>> {
        let limit = limit.clamp(1, MAX_KEPT);
        let mut out: Vec<(String, Flags)> = Vec::new();
        while out.len() < limit {
            let page = (limit - out.len()).min(1000);
            let m = self.call_one(&format!(
                "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{FLAG_FIELDS}</t:AdditionalProperties></m:ItemShape>\
                 <m:IndexedPageItemView MaxEntriesReturned=\"{}\" Offset=\"{}\" BasePoint=\"Beginning\"/>\
                 <m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder>\
                 <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindItem>",
                page,
                out.len(),
                esc(folder)
            ))?;
            let last = m.find("RootFolder").and_then(|r| r.attr("IncludesLastItemInRange")) != Some("false");
            let Some(items) = m.find("Items") else { break };
            let got: Vec<(String, Flags)> =
                items.kids.iter().filter_map(|it| Some((it.child("ItemId")?.attr("Id")?.to_string(), flags(it)))).collect();
            let n = got.len();
            out.extend(got);
            if last || n == 0 || n < page {
                break;
            }
        }
        Ok(out)
    }

    /// The newest `limit` ids of a folder's messages a search finds (one page: `limit` is at most
    /// a few hundred).
    pub(super) fn find(&self, folder: &str, q: &WordQuery, limit: usize) -> Result<Vec<String>> {
        let (restriction, words) = if q.text.is_empty() {
            (restriction(q), String::new())
        } else {
            (String::new(), format!("<m:QueryString>{}</m:QueryString>", esc(&q.text)))
        };
        let m = self.call_one(&format!(
            "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{FLAG_FIELDS}\
             <t:FieldURI FieldURI=\"item:HasAttachments\"/></t:AdditionalProperties></m:ItemShape>\
             <m:IndexedPageItemView MaxEntriesReturned=\"{}\" Offset=\"0\" BasePoint=\"Beginning\"/>{restriction}\
             <m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder>\
             <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds>{words}</m:FindItem>",
            limit.clamp(1, 1000),
            esc(folder)
        ))?;
        let Some(items) = m.find("Items") else { return Ok(Vec::new()) };
        Ok(items
            .kids
            .iter()
            .filter(|it| q.admits(&flags(it), it.text_of("HasAttachments").is_some_and(|v| v == "true")))
            .filter_map(|it| Some(it.child("ItemId")?.attr("Id")?.to_string()))
            .take(limit)
            .collect())
    }

    /// Mark every message in the folder `folder` read (issue #197): at once with
    /// `MarkAllItemsAsRead`, which Exchange 2013 brought; on a server without it (Exchange
    /// 2010), its unread ones found and marked a page at a time, as a message is marked. At once,
    /// no read receipts are sent, as Outlook sends none for a folder marked read.
    pub(super) fn mark_read(&self, folder: &str) -> Result<()> {
        let all = self.call_2013(&format!(
            "<m:MarkAllItemsAsRead><m:ReadFlag>true</m:ReadFlag><m:SuppressReadReceipts>true</m:SuppressReadReceipts>\
             <m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:MarkAllItemsAsRead>",
            esc(folder)
        ));
        match all.and_then(|doc| messages(&doc).into_iter().try_for_each(check_message)) {
            Ok(()) => return Ok(()),
            // Refused for what it is: asked the older way. A connection lost isn't that.
            Err(e @ (NetError::Protocol(_) | NetError::No(_) | NetError::Bad(_))) => {
                log::info!("the folder marked read a page at a time: {e}");
            }
            Err(e) => return Err(e),
        }
        let unread = WordQuery { unread: Some(true), ..WordQuery::default() };
        for _ in 0..1000 {
            let ids = self.find(folder, &unread, 500)?;
            if ids.is_empty() {
                return Ok(());
            }
            for chunk in ids.chunks(BATCH) {
                let changes: String = chunk
                    .iter()
                    .map(|id| {
                        format!(
                            "<t:ItemChange><t:ItemId Id=\"{}\"/><t:Updates><t:SetItemField><t:FieldURI FieldURI=\"message:IsRead\"/>\
                             <t:Message><t:IsRead>true</t:IsRead></t:Message></t:SetItemField></t:Updates></t:ItemChange>",
                            esc(id)
                        )
                    })
                    .collect();
                self.call_all(&format!(
                    // Without `SuppressReadReceipts`: Exchange 2013 brought that too, and 2010's
                    // schema refuses it.
                    "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\">\
                     <m:ItemChanges>{changes}</m:ItemChanges></m:UpdateItem>"
                ))?;
            }
        }
        Err(NetError::No("the folder's mail didn't all get marked read".into()))
    }

    /// Of these messages, those still in a folder, with their flags.
    pub(super) fn still_in(&self, folder: &str, ids: &[String]) -> Result<Vec<(String, Flags)>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(BATCH) {
            let list = item_ids(chunk);
            let doc = self.call(&format!(
                "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{FLAG_FIELDS}\
                 <t:FieldURI FieldURI=\"item:ParentFolderId\"/></t:AdditionalProperties></m:ItemShape><m:ItemIds>{list}</m:ItemIds></m:GetItem>"
            ))?;
            for m in messages(&doc).into_iter().filter(|m| m.attr("ResponseClass") != Some("Error")) {
                for it in m.child("Items").map(|i| i.kids.as_slice()).unwrap_or_default() {
                    let Some(id) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
                    if it.child("ParentFolderId").and_then(|p| p.attr("Id")).is_none_or(|p| p == folder) {
                        out.push((id.to_string(), flags(it)));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Messages' details, for the list.
    pub(super) fn headers(&self, ids: &[String]) -> Result<Vec<El>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(BATCH) {
            let list = item_ids(chunk);
            let doc = self.call(&format!(
                "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{HEADER_FIELDS}{FLAG_FIELDS}</t:AdditionalProperties></m:ItemShape>\
                 <m:ItemIds>{list}</m:ItemIds></m:GetItem>"
            ))?;
            for m in into_messages(doc) {
                if m.attr("ResponseClass") == Some("Error") {
                    continue;
                }
                if let Some(items) = m.kids.into_iter().find(|k| k.name == "Items") {
                    out.extend(items.kids);
                }
            }
        }
        Ok(out)
    }

    /// A message's MIME source.
    pub fn download(&self, id: &str) -> Result<Vec<u8>> {
        let m = self.call_one(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:IncludeMimeContent>true</t:IncludeMimeContent></m:ItemShape>\
             <m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:GetItem>",
            esc(id)
        ))?;
        let mime = m.find("MimeContent").ok_or_else(|| NetError::Protocol("the message has no source".into()))?;
        unb64(&mime.text).ok_or_else(|| NetError::Protocol("a damaged message source".into()))
    }

    /// Store a MIME message in a folder (as a draft when it's one); its id.
    pub fn create(&self, raw: &[u8], folder: &str, draft: bool) -> Result<String> {
        self.store(raw, folder, draft.then_some(8))
    }

    /// Store a MIME message in a folder as mail received and read (not a draft): issue #38.
    pub fn create_received(&self, raw: &[u8], folder: &str) -> Result<String> {
        self.store(raw, folder, Some(1))
    }

    /// Store a MIME message in a folder with its message flags (`PR_MESSAGE_FLAGS`: 8 a draft, 1
    /// read) when given, else as the server sets them; its id.
    fn store(&self, raw: &[u8], folder: &str, flags: Option<u8>) -> Result<String> {
        let flags = flags
            .map(|f| {
                format!(
                    "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x0E07\" PropertyType=\"Integer\"/><t:Value>{f}</t:Value></t:ExtendedProperty>"
                )
            })
            .unwrap_or_default();
        let m = self.in_folder(folder, || {
            self.call_one(&format!(
                "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:FolderId Id=\"{}\"/></m:SavedItemFolderId>\
                 <m:Items><t:Message><t:MimeContent CharacterSet=\"UTF-8\">{}</t:MimeContent>{flags}</t:Message></m:Items></m:CreateItem>",
                esc(folder),
                base64::encode(raw)
            ))
        })?;
        Ok(m.find("ItemId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string())
    }

    /// Send a MIME message, keeping a copy in Sent Items.
    pub fn send_mime(&self, raw: &[u8]) -> Result<()> {
        self.call_one(&format!(
            "<m:CreateItem MessageDisposition=\"SendAndSaveCopy\"><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"sentitems\"/></m:SavedItemFolderId>\
             <m:Items><t:Message><t:MimeContent CharacterSet=\"UTF-8\">{}</t:MimeContent></t:Message></m:Items></m:CreateItem>",
            base64::encode(raw)
        ))
        .map(drop)
    }

    /// The automatic replies.
    pub fn vacation(&self) -> Result<mailcraft_model::job::Vacation> {
        let doc = self.call(&format!(
            "<m:GetUserOofSettingsRequest><t:Mailbox><t:Address>{}</t:Address></t:Mailbox></m:GetUserOofSettingsRequest>",
            esc(&self.email)
        ))?;
        if let Some(rm) = doc.find("ResponseMessage") {
            check_message(rm)?;
        }
        let s = doc.find("OofSettings").ok_or_else(|| NetError::Protocol("no automatic reply settings".into()))?;
        let state = s.text_of("OofState").unwrap_or("Disabled");
        let day = |name: &str, end: bool| -> Option<String> {
            let t = s.find(name)?.text.clone();
            let secs = jmap::utc_date(&t)?;
            // The end is the moment after the last day.
            let secs = if end { secs.saturating_sub(1) } else { secs };
            Some(mailcraft_model::dates::day_string(secs.div_euclid(86_400)))
        };
        let scheduled = state == "Scheduled";
        let reply = |which: &str| {
            let m = s.find(which).and_then(|r| r.child("Message")).map(|m| m.text.clone()).unwrap_or_default();
            crate::exchange::reply_text(&m)
        };
        let (text, external) = (reply("InternalReply"), reply("ExternalReply"));
        let outside = match s.text_of("ExternalAudience").unwrap_or("All") {
            "None" => "none",
            "Known" => "known",
            _ => "",
        };
        Ok(mailcraft_model::job::Vacation {
            enabled: state != "Disabled",
            subject: String::new(),
            // Outside, the same unless it says something else (an empty one is the same).
            external: (!external.is_empty() && external != text).then_some(external),
            outside: outside.to_string(),
            text,
            from: if scheduled { day("StartTime", false) } else { None },
            until: if scheduled { day("EndTime", true) } else { None },
            ..Default::default()
        })
    }

    pub fn set_vacation(&self, v: &mailcraft_model::job::Vacation) -> Result<()> {
        let at = |day: &Option<String>, end: bool| -> Option<String> {
            let d = day.as_deref().filter(|d| mailcraft_model::job::is_day(d))?;
            Some(if end { format!("{d}T23:59:59Z") } else { format!("{d}T00:00:00Z") })
        };
        let (start, end) = (at(&v.from, false), at(&v.until, true));
        let state = match (v.enabled, start.is_some() || end.is_some()) {
            (false, _) => "Disabled",
            (true, true) => "Scheduled",
            (true, false) => "Enabled",
        };
        let duration = format!(
            "<t:Duration><t:StartTime>{}</t:StartTime><t:EndTime>{}</t:EndTime></t:Duration>",
            start.unwrap_or_else(|| "2000-01-01T00:00:00Z".into()),
            end.unwrap_or_else(|| "2100-01-01T00:00:00Z".into())
        );
        // HTML, then escaped for the XML it goes in.
        let text = esc(&crate::exchange::reply_html(&v.text));
        let external = esc(&crate::exchange::reply_html(v.external_text()));
        let audience = match v.outside.as_str() {
            "none" => "None",
            "known" => "Known",
            _ => "All",
        };
        let doc = self.call(&format!(
            "<m:SetUserOofSettingsRequest><t:Mailbox><t:Address>{}</t:Address></t:Mailbox><t:UserOofSettings>\
             <t:OofState>{state}</t:OofState><t:ExternalAudience>{audience}</t:ExternalAudience>{duration}\
             <t:InternalReply><t:Message>{text}</t:Message></t:InternalReply><t:ExternalReply><t:Message>{external}</t:Message></t:ExternalReply>\
             </t:UserOofSettings></m:SetUserOofSettingsRequest>",
            esc(&self.email)
        ))?;
        match doc.find("ResponseMessage") {
            Some(rm) => check_message(rm),
            None => Ok(()),
        }
    }
}
