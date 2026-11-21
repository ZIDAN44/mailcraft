//! Reading a folder's items whole: the appointments of a calendar, the people of a contacts
//! folder, the notes; and each of the folders a sync reads again: the account's calendar,
//! the shared calendars, its contacts, the public folders' and its notes.

use super::*;

/// The account's calendar, read again: its events, and how it's named.
pub(super) fn read_calendar(c: &Client, out: &mut DavSync) -> Result<CalendarInfo> {
    match items(c, "calendar") {
        Ok(read) => {
            read.say(out, "Calendar");
            out.events.extend(stored_events(read.items, CALENDAR, &c.own_addresses()));
            out.synced_calendars.push(CALENDAR.into());
        }
        // A mailbox without the folder has no calendar. One the server won't give is said, not
        // only logged: a calendar that stays empty says why (issue #80).
        Err(NetError::No(e)) if e.starts_with(crate::ews::GONE) => log::info!("calendar: {e}"),
        Err(NetError::No(e)) => out.errors.push(format!("Calendar: {e}")),
        Err(e) => return Err(e),
    }
    // Named as the server names it, in the mailbox's language ("Agenda"), as Outlook shows it.
    let own = folder_name(c, "calendar").unwrap_or_else(|| "Calendar".into());
    Ok(CalendarInfo { id: CALENDAR.into(), name: own, color: 0, read_only: false, ctag: String::new(), ..Default::default() })
}

/// A calendar of someone else's to read: their address, its name, and whether it was opened by
/// hand (Open shared calendar) rather than with their shared mailbox.
pub(super) type SharedCalendar = (String, String, bool);

/// Other people's calendars opened here (Open shared calendar), then the calendars of the
/// shared mailboxes the account opens (Outlook lists a shared mailbox's calendar with its
/// mail). The shared mailboxes are those the folder listing before found (it's asked again
/// when there was none).
pub(super) fn shared_calendars(c: &Client, pim: &Pim) -> Vec<SharedCalendar> {
    let opened: Vec<SharedCalendar> = pim.shared.iter().map(|e| (e.clone(), e.clone(), true)).collect();
    let mailboxes: Vec<SharedCalendar> = c
        .listed_shared_mailboxes()
        .into_iter()
        .filter(|m| !pim.shared.iter().any(|e| e.eq_ignore_ascii_case(&m.address)))
        .map(|m| (m.address, m.name, false))
        .collect();
    opened.into_iter().chain(mailboxes).collect()
}

/// One of [`shared_calendars`] (the `n`th: its colour), read: their mailbox's calendar folder,
/// the delegate permissions they gave deciding whether it can be read, and whether changed.
pub(super) fn read_shared_calendar(c: &Client, (email, name, asked): SharedCalendar, n: usize, out: &mut DavSync, calendars: &mut Vec<CalendarInfo>) {
    let id = mailcraft_model::pim::shared_calendar_id(&email);
    let parent = calendar_folder(Some(&email));
    // Editable when the folder's rights say so, or (some servers say nothing useful about
    // another mailbox's folder) when its events do, as Outlook finds by trying.
    let folder_rights = c.in_mailbox(&email, || may_edit(c, &parent));
    let mut can_edit = *folder_rights.as_ref().unwrap_or(&false);
    let mut free_busy = false;
    match c.in_mailbox(&email, || items_in(c, &parent, RECURRENCE)) {
        Ok(read) => {
            read.say(out, &name);
            can_edit |= read.items.iter().any(item_may_change);
            if !can_edit && let Err(e) = &folder_rights {
                log::info!("{email}'s calendar rights: {e}");
            }
            out.events.extend(stored_events(read.items, &id, &[email.as_str()]));
            out.synced_calendars.push(id.clone());
        }
        // One asked for by hand says why it's missing; a shared mailbox whose calendar
        // isn't shared with the account just has none here. Exchange says it isn't found when
        // it's shared for free/busy time alone, as a company's usually are (issue #138), and
        // refuses it when it's shared for subject and place too (#140): it's shown by those
        // times, asked for apart, and says why if there are none.
        Err(e) if asked && (crate::ews::gone(&e) || crate::ews::denied(&e)) => {
            log::info!("{email}'s calendar: free/busy time alone ({e})");
            free_busy = true;
            out.synced_calendars.push(id.clone());
        }
        Err(e) if asked => out.errors.push(format!("{email}'s calendar: {e}")),
        Err(e) => {
            log::info!("shared mailbox {email}: no calendar ({e})");
            return;
        }
    }
    // One opened by hand by their address: their name, as the directory gives it (issue #222).
    let name = if asked && name.eq_ignore_ascii_case(&email) { c.directory_name_of(&email).unwrap_or(name) } else { name };
    calendars.push(CalendarInfo {
        id,
        name,
        color: u8::try_from((n + 1) % 8).unwrap_or(0),
        read_only: !can_edit || free_busy,
        ctag: String::new(),
        free_busy,
        ..Default::default()
    });
}

/// The account's contacts, read again: its people, and how the folder's named.
pub(super) fn read_contacts(c: &Client, out: &mut DavSync) -> Result<BookInfo> {
    match items(c, "contacts") {
        Ok(read) => {
            read.say(out, "Contacts");
            for it in read.items {
                if let Some(card) = card_of(&it) {
                    let etag = it.child("ItemId").and_then(|i| i.attr("ChangeKey")).unwrap_or("").to_string();
                    out.cards.push(StoredCard { book: CONTACTS.into(), href: format!("contact:{}", card.uid), etag, card, dirty: false });
                }
            }
            out.synced_books.push(CONTACTS.into());
        }
        Err(NetError::No(e)) if e.starts_with(crate::ews::GONE) => log::info!("contacts: {e}"),
        Err(NetError::No(e)) => out.errors.push(format!("Contacts: {e}")),
        Err(e) => return Err(e),
    }
    let own = folder_name(c, "contacts").unwrap_or_else(|| "Contacts".into());
    Ok(BookInfo { id: CONTACTS.into(), name: own, read_only: false, ctag: String::new() })
}

/// The organisation's shared contacts and calendars kept as public folders (on-premises
/// Exchange): read-only, as Outlook shows them under Public Folders. A server without public
/// folders has none. Found in the tree the folder listing before walked, when there was one.
pub(super) fn read_public(c: &Client, out: &mut DavSync, calendars: &mut Vec<CalendarInfo>, books: &mut Vec<BookInfo>) {
    let list = match c.public_pim_folders() {
        Ok(list) => list,
        // A server with public folders that won't list them says why; one without has none.
        Err(e) => {
            if c.public_hierarchy_mailbox().is_some() {
                out.errors.push(format!("Public folders: {e}"));
            } else {
                log::info!("no public contacts or calendars: {e}");
            }
            return;
        }
    };
    // Said in the log, found or not: what a "where are my shared contacts?" needs.
    let contacts = list.iter().filter(|f| f.class.starts_with("IPF.Contact")).count();
    log::info!("public folders: {contacts} address books, {} calendars", list.len() - contacts);
    for (n, f) in list.into_iter().take(MAX_PUBLIC_PIM).enumerate() {
        let (id, name, class) = (f.id, f.path, f.class);
        let key = format!("public:{id}");
        let extra = if class.starts_with("IPF.Contact") { "" } else { RECURRENCE };
        let found = match public_items(c, &id, f.replica.as_deref(), f.total, extra) {
            Ok(read) => {
                read.say(out, &name);
                read.items
            }
            Err(e) => {
                out.errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        if class.starts_with("IPF.Contact") {
            for it in found {
                if let Some(card) = card_of(&it) {
                    out.cards.push(StoredCard { book: key.clone(), href: format!("public:{}", card.uid), etag: String::new(), card, dirty: false });
                }
            }
            books.push(BookInfo { id: key.clone(), name, read_only: true, ctag: String::new() });
            out.synced_books.push(key);
        } else {
            out.events.extend(stored_events(found, &key, &[]).map(|e| StoredEvent { etag: String::new(), ..e }));
            calendars.push(CalendarInfo {
                id: key.clone(),
                name,
                color: u8::try_from((n + 3) % 8).unwrap_or(0),
                read_only: true,
                ctag: String::new(),
                foreign: false,
                free_busy: false,
            });
            out.synced_calendars.push(key);
        }
    }
}

/// The notes (Outlook's sticky notes), read again. A mailbox without a Notes folder keeps notes
/// on this computer.
pub(super) fn read_notes(c: &Client, out: &mut DavSync) {
    match items(c, "notes") {
        Ok(read) => out.notes = Some(read.items.iter().filter_map(note_of).collect()),
        Err(e) => log::info!("no Notes folder: {e}"),
    }
}

/// An appointment's recurrence, asked for by name: only of a calendar's items. Asked of a
/// contact or a note, Exchange answers that the property isn't one of theirs, for each of them.
pub(super) const RECURRENCE: &str = "<t:FieldURI FieldURI=\"calendar:Recurrence\"/>";

/// A folder's items as read, and what the server said of those it wouldn't give.
pub(super) struct Read {
    pub(super) items: Vec<El>,
    /// How many the server refused (not those gone since they were listed), and why the first.
    refused: usize,
    why: String,
}

impl Read {
    /// Say, with the sync's other errors, that some of `folder`'s items couldn't be read: they
    /// were left out without a word, and a calendar could come out empty that way (issue #80).
    pub(super) fn say(&self, out: &mut DavSync, folder: &str) {
        if self.refused > 0 {
            log::warn!("{folder}: {} items not read: {}", self.refused, self.why);
            out.errors.push(format!("{folder}: {} items couldn't be read ({})", self.refused, self.why));
        }
    }
}

/// Every item of a distinguished folder, with all its properties (bodies as text).
pub(super) fn items(c: &Client, folder: &str) -> Result<Read> {
    let extra = match folder {
        "calendar" => RECURRENCE,
        "notes" => NOTE_COLOR,
        "tasks" => TASK_FIELDS,
        _ => "",
    };
    items_in(c, &format!("<t:DistinguishedFolderId Id=\"{folder}\"/>"), extra)
}

/// Every item of a folder (given as its EWS folder id element), with all its properties and
/// those `extra` names (field URIs).
pub(super) fn items_in(c: &Client, parent: &str, extra: &str) -> Result<Read> {
    let folder = parent;
    let mut ids: Vec<String> = Vec::new();
    let mut offset = 0;
    loop {
        let m = c.call_one(&format!(
            "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>\
             <m:IndexedPageItemView MaxEntriesReturned=\"500\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
             <m:ParentFolderIds>{folder}</m:ParentFolderIds></m:FindItem>"
        ))?;
        let Some(rf) = m.child("RootFolder") else { break };
        let page: Vec<String> =
            rf.child("Items").map(|i| i.kids.iter().filter_map(|k| k.child("ItemId")?.attr("Id").map(str::to_string)).collect()).unwrap_or_default();
        let n = page.len();
        ids.extend(page);
        offset += n;
        if n == 0 || rf.attr("IncludesLastItemInRange").is_none_or(|v| v == "true") || ids.len() >= MAX_ITEMS {
            break;
        }
    }
    let mut read = Read { items: Vec::new(), refused: 0, why: String::new() };
    for chunk in ids.chunks(BATCH) {
        let list = crate::ews::item_ids(chunk);
        let doc = c.call(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>AllProperties</t:BaseShape><t:BodyType>Text</t:BodyType>\
             <t:AdditionalProperties><t:FieldURI FieldURI=\"item:EffectiveRights\"/>{extra}</t:AdditionalProperties></m:ItemShape>\
             <m:ItemIds>{list}</m:ItemIds></m:GetItem>"
        ))?;
        for m in crate::ews::into_messages(doc) {
            if m.attr("ResponseClass") == Some("Error") {
                // One deleted since the folder was listed is simply gone; a series whose every
                // occurrence was deleted, which Exchange won't give, has nothing to show (issue
                // #229: it was said at every start).
                let code = m.text_of("ResponseCode").unwrap_or("");
                if code == "ErrorRecurrenceHasNoOccurrence" {
                    log::info!("a series with no occurrences left, not shown");
                } else if code != "ErrorItemNotFound" {
                    if read.refused == 0 {
                        read.why = m.text_of("MessageText").unwrap_or(code).trim().chars().take(200).collect();
                    }
                    read.refused += 1;
                }
                continue;
            }
            if let Some(items) = m.kids.into_iter().find(|k| k.name == "Items") {
                read.items.extend(items.kids);
            }
        }
    }
    // A calendar's: its series' changed occurrences too (issue #204).
    if extra.contains(RECURRENCE) {
        with_changed_occurrences(c, &mut read.items, extra)?;
    }
    Ok(read)
}

/// The changed occurrences of the series among `items` (moved, cancelled, given a subject or an
/// answer of their own), read and put with them, each with the start its series gave it
/// (`OriginalStart`, from the series' `ModifiedOccurrences`), which names it. A listing gives a
/// series alone, and the changes made to one occurrence were lost: it stayed at the series'
/// usual time, a cancelled one too (issue #204). One the server won't give is left to its
/// series, as it was.
pub(super) fn with_changed_occurrences(c: &Client, items: &mut Vec<El>, extra: &str) -> Result<()> {
    // One a server listed with its series is read once.
    let listed: std::collections::HashSet<&str> = items.iter().filter_map(|it| it.child("ItemId")?.attr("Id")).collect();
    let mut original: Vec<(String, String)> = Vec::new();
    for it in items.iter().filter(|it| it.text_of("CalendarItemType") == Some("RecurringMaster")) {
        let Some(changed) = it.child("ModifiedOccurrences") else { continue };
        for o in changed.children("Occurrence").take(MAX_EXDATES) {
            if let (Some(id), Some(start)) = (o.child("ItemId").and_then(|i| i.attr("Id")), o.text_of("OriginalStart"))
                && !listed.contains(id)
            {
                original.push((id.to_string(), start.trim().to_string()));
            }
        }
        if original.len() >= MAX_ITEMS {
            break;
        }
    }
    original.truncate(MAX_ITEMS);
    for chunk in original.chunks(BATCH) {
        let ids: Vec<String> = chunk.iter().map(|(id, _)| id.clone()).collect();
        let asked = c.call(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>AllProperties</t:BaseShape><t:BodyType>Text</t:BodyType>\
             <t:AdditionalProperties><t:FieldURI FieldURI=\"item:EffectiveRights\"/>{extra}</t:AdditionalProperties></m:ItemShape>\
             <m:ItemIds>{}</m:ItemIds></m:GetItem>",
            crate::ews::item_ids(&ids)
        ));
        // Refused as a whole: the series keep their usual times, as before; the calendar is
        // read all the same. A connection lost is the calendar's failure.
        let doc = match asked {
            Ok(doc) => doc,
            Err(e @ (NetError::Protocol(_) | NetError::No(_) | NetError::Bad(_))) => {
                log::info!("changed occurrences not read: {e}");
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        for (m, (_, start)) in crate::ews::into_messages(doc).into_iter().zip(chunk) {
            if m.attr("ResponseClass") == Some("Error") {
                log::info!("a changed occurrence not read: {}", m.text_of("MessageText").unwrap_or("").trim());
                continue;
            }
            let Some(list) = m.kids.into_iter().find(|k| k.name == "Items") else { continue };
            for mut it in list.kids.into_iter().filter(|it| it.text_of("CalendarItemType") == Some("Exception")) {
                if it.child("OriginalStart").is_none() {
                    it.kids.push(El { name: "OriginalStart".into(), text: start.clone(), ..Default::default() });
                }
                items.push(it);
            }
        }
    }
    Ok(())
}

/// A public folder's items. On Exchange 2013 and later its content is kept in a public folder
/// mailbox, and reading it is routed there (asked for by the GUID the folder names); a server
/// that answers without that (older ones, a single public folder mailbox) is asked plainly
/// first.
pub(super) fn public_items(c: &Client, id: &str, replica: Option<&str>, total: u32, extra: &str) -> Result<Read> {
    let folder = format!("<t:FolderId Id=\"{}\"/>", esc(id));
    let plain = items_in(c, &folder, extra);
    let Some(guid) = replica else { return plain };
    match plain {
        Ok(read) if !read.items.is_empty() || total == 0 => Ok(read),
        plain => match c.public_folder_mailbox(guid) {
            Ok(mailbox) => c.routed_to(&mailbox, || items_in(c, &folder, extra)),
            Err(e) => {
                log::info!("public folder mailbox {guid}: {e}");
                plain
            }
        },
    }
}
