//! EWS: other people's mailboxes (shared, found with Autodiscover) and public folders.

use super::*;

impl Client {
    /// The shared mailboxes added to the account by hand.
    pub fn set_shared(&self, addresses: &[String]) {
        *self.shared_added.borrow_mut() = addresses.iter().map(|a| a.trim().to_string()).filter(|a| a.contains('@')).take(MAX_SHARED).collect();
    }

    /// The account's Autodiscover answer, asked once.
    pub(super) fn discovery(&self) -> Option<String> {
        if let Some(known) = self.discovered.borrow().clone() {
            return known;
        }
        let found = self.ask_autodiscover(&self.email).ok();
        *self.discovered.borrow_mut() = Some(found.clone());
        found
    }

    /// The mailboxes this account opens besides its own: those added by hand, then those the
    /// server maps to it (Autodiscover's `AlternativeMailbox`, the mailboxes it has full access
    /// to, as Outlook opens them by itself). Its own address isn't one; each is there once.
    pub fn shared_mailboxes(&self) -> Vec<SharedMailbox> {
        let mut out: Vec<SharedMailbox> = Vec::new();
        let add = |out: &mut Vec<SharedMailbox>, address: String, name: String| {
            let a = address.trim().to_string();
            if a.contains('@')
                && !a.eq_ignore_ascii_case(&self.email)
                && !out.iter().any(|x| x.address.eq_ignore_ascii_case(&a))
                && out.len() < MAX_SHARED
            {
                let name = if name.trim().is_empty() { a.clone() } else { name.trim().chars().take(120).collect() };
                out.push(SharedMailbox { address: a, name });
            }
        };
        // Added by hand: named as the server knows it (its address when it doesn't say).
        for a in self.shared_added.borrow().clone() {
            let name = self.resolve(&a).map(|(_, name)| name).unwrap_or_default();
            add(&mut out, a, name);
        }
        if let Some(xml) = self.discovery() {
            for alt in alternative_mailboxes(&xml) {
                // An older server names it by its Exchange address only: asked for its email address.
                let address = alt.smtp.or_else(|| alt.legacy_dn.as_deref().and_then(|dn| self.resolve(dn)).map(|(a, _)| a));
                match address {
                    Some(a) => add(&mut out, a, alt.name),
                    None => log::info!("shared mailbox {}: no address for it", alt.name),
                }
            }
        }
        out
    }

    /// A mailbox the server knows by an address (an email address, or an Exchange address
    /// `/o=…`): its email address and name.
    pub(super) fn resolve(&self, entry: &str) -> Option<(String, String)> {
        let m = self
            .call_one(&format!(
                "<m:ResolveNames ReturnFullContactData=\"false\"><m:UnresolvedEntry>{}</m:UnresolvedEntry></m:ResolveNames>",
                esc(entry)
            ))
            .ok()?;
        let b = m.find("Mailbox")?;
        let address = b.text_of("EmailAddress").map(|a| a.trim().to_string()).filter(|a| a.contains('@'))?;
        Some((address, b.text_of("Name").unwrap_or("").trim().to_string()))
    }

    /// Run `f` with every request routed to another mailbox (a shared one).
    pub(super) fn anchored_to<T>(&self, mailbox: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let before = self.anchor.replace(Some(mailbox.to_string()));
        let r = f();
        *self.anchor.borrow_mut() = before;
        r
    }

    /// A shared mailbox's mail folders, under a folder named for it (`top`): asked for in that
    /// mailbox (its root named with the `Mailbox` element), routed there. Its special folders'
    /// roles there are noted (`shared_roles`), for their icons and places.
    pub(super) fn shared_folders(&self, sm: &SharedMailbox, top: &str) -> Result<Vec<Mailbox>> {
        let mb = format!("<t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox>", esc(&sm.address));
        self.anchored_to(&sm.address, || {
            let root = self.call_one(&format!(
                "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape>\
                 <m:FolderIds><t:DistinguishedFolderId Id=\"msgfolderroot\">{mb}</t:DistinguishedFolderId></m:FolderIds></m:GetFolder>"
            ))?;
            let Some(root_id) = root.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string) else { return Ok(Vec::new()) };
            // Its special folders, and its Outbox and the like, which stay out as the account's
            // own do; one it hasn't is an error in its place.
            let hidden = ["outbox", "conversationhistory", "syncissues"];
            let ids: String = SPECIAL
                .iter()
                .map(|(n, _)| *n)
                .chain(hidden)
                .map(|n| format!("<t:DistinguishedFolderId Id=\"{n}\">{mb}</t:DistinguishedFolderId>"))
                .collect();
            let doc = self.call(&format!(
                "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds>{ids}</m:FolderIds></m:GetFolder>"
            ))?;
            let found: Vec<Option<String>> = messages(&doc)
                .iter()
                .map(|m| if m.attr("ResponseClass") == Some("Error") { None } else { m.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string) })
                .collect();
            let skip: Vec<String> = found.iter().skip(SPECIAL.len()).flatten().cloned().collect();
            {
                let mut roles = self.shared_roles.borrow_mut();
                roles.insert(root_id.clone(), (sm.address.clone(), FolderRole::Other));
                for ((_, role), id) in SPECIAL.iter().zip(&found) {
                    if let Some(id) = id {
                        roles.insert(id.clone(), (sm.address.clone(), *role));
                    }
                }
            }
            let mut out = vec![Mailbox { id: root_id.clone(), name: top.to_string(), parent: None, role: FolderRole::Other, total: 0, unread: 0 }];
            let mut offset = 0;
            loop {
                let m = self.call_one(&format!(
                    "<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
                     <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:ParentFolderId\"/>\
                     <t:FieldURI FieldURI=\"folder:TotalCount\"/><t:FieldURI FieldURI=\"folder:UnreadCount\"/>\
                     <t:FieldURI FieldURI=\"folder:FolderClass\"/>{HIDDEN}</t:AdditionalProperties></m:FolderShape>\
                     <m:IndexedPageFolderView MaxEntriesReturned=\"500\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
                     <m:ParentFolderIds><t:DistinguishedFolderId Id=\"msgfolderroot\">{mb}</t:DistinguishedFolderId></m:ParentFolderIds></m:FindFolder>"
                ))?;
                let Some(rf) = m.child("RootFolder") else { break };
                let folders = rf.folders();
                let n = folders.len();
                for f in folders {
                    let Some(id) = f.child("FolderId").and_then(|i| i.attr("Id")).map(str::to_string) else { continue };
                    let class = f.text_of("FolderClass").unwrap_or("");
                    if !(class.is_empty() || class == "IPF.Note" || class.starts_with("IPF.Note.")) || skip.contains(&id) || is_hidden(f) || out.len() >= MAX_FOLDERS {
                        continue;
                    }
                    let parent = f.child("ParentFolderId").and_then(|p| p.attr("Id")).map(str::to_string).or_else(|| Some(root_id.clone()));
                    let count = |k: &str| f.text_of(k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
                    self.shared_roles.borrow_mut().entry(id.clone()).or_insert_with(|| (sm.address.clone(), FolderRole::Other));
                    out.push(Mailbox {
                        name: f.text_of("DisplayName").unwrap_or_default().chars().take(255).collect(),
                        parent,
                        role: FolderRole::Other,
                        total: count("TotalCount"),
                        unread: count("UnreadCount"),
                        id,
                    });
                }
                let last = rf.attr("IncludesLastItemInRange").is_none_or(|v| v == "true");
                offset += n;
                if last || n == 0 || out.len() >= MAX_FOLDERS {
                    break;
                }
            }
            keep_rooted(&mut out);
            Ok(out)
        })
    }

    /// Ask Autodiscover about an address (at the Exchange server's own Autodiscover first, then
    /// the domain's): the first answer.
    pub(super) fn ask_autodiscover(&self, address: &str) -> Result<String> {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><Autodiscover xmlns=\"http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006\">\
             <Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema></Request></Autodiscover>",
            esc(address)
        );
        let mut urls = Vec::new();
        if let Ok(u) = http::Url::parse(&self.url)
            && let Some(origin) = self.url.len().checked_sub(u.path.len()).and_then(|end| self.url.get(..end))
        {
            urls.push(format!("{origin}/autodiscover/autodiscover.xml"));
        }
        urls.extend(autodiscover_urls(address));
        let mut last = NetError::No("Autodiscover didn't answer".into());
        for url in urls {
            let c = match Client::new(&url, &self.user, &self.password, &self.email) {
                Ok(c) => c,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            match c.post(body.clone().into_bytes()) {
                Ok(resp) if resp.ok() => return Ok(String::from_utf8_lossy(&resp.body).into_owned()),
                Ok(resp) => last = NetError::Protocol(format!("Autodiscover answered HTTP {}", resp.status)),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// The account's public folder hierarchy mailbox, from Autodiscover (asked once): where
    /// listing public folders is routed on Exchange 2013 and later.
    pub(crate) fn public_hierarchy_mailbox(&self) -> Option<String> {
        if let Some(known) = self.pf_hierarchy.borrow().clone() {
            return known;
        }
        let found = self.discovery().as_deref().and_then(autodiscover_public);
        *self.pf_hierarchy.borrow_mut() = Some(found.clone());
        found
    }

    /// Run `f` with every request routed to a public folder mailbox.
    pub(crate) fn routed_to<T>(&self, mailbox: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let before = self.route.replace(Some(mailbox.to_string()));
        let r = f();
        *self.route.borrow_mut() = before;
        r
    }

    /// The address of the public folder mailbox with this GUID, from Autodiscover (asked at
    /// the Exchange server's own Autodiscover first, then the domain's), kept once found.
    pub(crate) fn public_folder_mailbox(&self, guid: &str) -> Result<String> {
        if let Some(m) = self.pf_mailboxes.borrow().get(guid) {
            return Ok(m.clone());
        }
        let domain = self.email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default();
        if domain.is_empty() {
            return Err(NetError::No("no domain to ask about public folders".into()));
        }
        let answer = self.ask_autodiscover(&format!("{guid}@{domain}"))?;
        let found = autodiscover_smtp(&answer).ok_or_else(|| NetError::No("Autodiscover didn't find the public folder mailbox".into()))?;
        self.pf_mailboxes.borrow_mut().insert(guid.to_string(), found.clone());
        Ok(found)
    }

    /// Exchange's public folders (the organization's shared folders): their mail folders, under
    /// a "Public Folders" folder. Public folders only list one level at a time, so the tree is
    /// walked, a few levels deep and up to a few hundred folders.
    pub(super) fn public_folders(&self) -> Result<Vec<Mailbox>> {
        let (root_id, all) = self.public_tree()?;
        // Calendars and contacts kept as public folders aren't mail; a folder holding others is
        // kept for them.
        let mut out: Vec<Mailbox> = all
            .into_iter()
            .filter(|t| t.class.is_empty() || t.class == "IPF.Note" || t.class.starts_with("IPF.Note.") || t.children)
            .map(|t| t.mailbox)
            .collect();
        if out.is_empty() {
            return Ok(out);
        }
        out.insert(0, Mailbox { id: root_id, name: PUBLIC_FOLDERS.into(), parent: None, role: FolderRole::Other, total: 0, unread: 0 });
        Ok(out)
    }

    /// Exchange's public contact folders and calendars (shared address books and calendars).
    pub(crate) fn public_pim_folders(&self) -> Result<Vec<PublicFolder>> {
        let (_, all) = self.public_tree()?;
        let path = |m: &Mailbox| {
            let mut parts = vec![m.name.clone()];
            let mut up = m.parent.clone();
            while let Some(p) = up.take() {
                if let Some(t) = all.iter().find(|t| t.mailbox.id == p)
                    && parts.len() < PUBLIC_DEPTH + 1
                {
                    parts.push(t.mailbox.name.clone());
                    up = t.mailbox.parent.clone();
                }
            }
            parts.reverse();
            parts.join(" › ")
        };
        Ok(all
            .iter()
            .filter(|t| t.class.starts_with("IPF.Contact") || t.class.starts_with("IPF.Appointment"))
            .map(|t| PublicFolder {
                id: t.mailbox.id.clone(),
                path: path(&t.mailbox),
                class: t.class.clone(),
                total: t.mailbox.total,
                replica: t.replica.clone(),
            })
            .collect())
    }

    /// Every public folder: asked plainly (older servers, one public folder mailbox), else
    /// routed to the account's public folder hierarchy mailbox, as Exchange 2013 and later want.
    pub(super) fn public_tree(&self) -> Result<(String, Vec<TreeFolder>)> {
        let plain = self.public_tree_walk();
        if plain.as_ref().is_ok_and(|(_, all)| !all.is_empty()) {
            return plain;
        }
        match self.public_hierarchy_mailbox() {
            Some(mailbox) => match self.routed_to(&mailbox, || self.public_tree_walk()) {
                Ok(t) => Ok(t),
                Err(e) => Err(NetError::No(format!("public folders ({mailbox}): {e}"))),
            },
            None => plain,
        }
    }

    /// Walk the public folders a level at a time, a few levels deep: the root's id, and each
    /// folder with its class, whether it holds others and where its content is kept. A folder
    /// that won't list its own is passed over.
    pub(super) fn public_tree_walk(&self) -> Result<(String, Vec<TreeFolder>)> {
        let root = self.call_one(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape>\
             <m:FolderIds><t:DistinguishedFolderId Id=\"publicfoldersroot\"/></m:FolderIds></m:GetFolder>",
        )?;
        let Some(root_id) = root.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string) else { return Ok((String::new(), Vec::new())) };
        let mut out: Vec<TreeFolder> = Vec::new();
        let mut queue: std::collections::VecDeque<(String, usize)> = std::collections::VecDeque::from([(root_id.clone(), 0)]);
        while let Some((parent, depth)) = queue.pop_front() {
            if depth >= PUBLIC_DEPTH || out.len() >= MAX_PUBLIC {
                break;
            }
            let m = self.call_one(&format!(
                "<m:FindFolder Traversal=\"Shallow\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:TotalCount\"/>\
                 <t:FieldURI FieldURI=\"folder:UnreadCount\"/><t:FieldURI FieldURI=\"folder:FolderClass\"/>\
                 <t:FieldURI FieldURI=\"folder:ChildFolderCount\"/>{REPLICA_LIST}</t:AdditionalProperties></m:FolderShape>\
                 <m:IndexedPageFolderView MaxEntriesReturned=\"200\" Offset=\"0\" BasePoint=\"Beginning\"/>\
                 <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindFolder>",
                esc(&parent)
            ));
            let m = match m {
                Ok(m) => m,
                Err(e) if depth == 0 => return Err(e),
                Err(e) => {
                    log::info!("a public folder's folders: {e}");
                    continue;
                }
            };
            let folders = m.child("RootFolder").map(El::folders).unwrap_or_default();
            for f in folders {
                let Some(id) = f.child("FolderId").and_then(|i| i.attr("Id")).map(str::to_string) else { continue };
                let count = |k: &str| f.text_of(k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
                let class = f.text_of("FolderClass").unwrap_or("").to_string();
                let replica = f
                    .children("ExtendedProperty")
                    .find(|p| {
                        p.child("ExtendedFieldURI")
                            .and_then(|u| u.attr("PropertyTag"))
                            .is_some_and(|t| t.eq_ignore_ascii_case("0x6698") || t == "26264")
                    })
                    .and_then(|p| p.text_of("Value"))
                    .and_then(replica_guid);
                if let Some(g) = &replica {
                    let mut known = self.pf_replicas.borrow_mut();
                    if !known.contains(g) && known.len() < 20 {
                        known.push(g.clone());
                    }
                }
                out.push(TreeFolder {
                    mailbox: Mailbox {
                        name: f.text_of("DisplayName").unwrap_or_default().chars().take(255).collect(),
                        parent: Some(parent.clone()),
                        role: FolderRole::Other,
                        total: count("TotalCount"),
                        unread: count("UnreadCount"),
                        id: id.clone(),
                    },
                    class,
                    children: count("ChildFolderCount") > 0,
                    replica,
                });
                if count("ChildFolderCount") > 0 {
                    queue.push_back((id, depth + 1));
                }
                if out.len() >= MAX_PUBLIC {
                    break;
                }
            }
        }
        Ok((root_id, out))
    }
}
