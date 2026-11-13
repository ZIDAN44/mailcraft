//! Reading a folder's items whole: the appointments of a calendar, the people of a contacts
//! folder, the notes.

use super::*;

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
        let list: String = chunk.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect();
        let doc = c.call(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>AllProperties</t:BaseShape><t:BodyType>Text</t:BodyType>\
             <t:AdditionalProperties><t:FieldURI FieldURI=\"item:EffectiveRights\"/>{extra}</t:AdditionalProperties></m:ItemShape>\
             <m:ItemIds>{list}</m:ItemIds></m:GetItem>"
        ))?;
        for m in crate::ews::messages(&doc) {
            if m.attr("ResponseClass") == Some("Error") {
                // One deleted since the folder was listed is simply gone.
                let code = m.text_of("ResponseCode").unwrap_or("");
                if code != "ErrorItemNotFound" {
                    if read.refused == 0 {
                        read.why = m.text_of("MessageText").unwrap_or(code).trim().chars().take(200).collect();
                    }
                    read.refused += 1;
                }
                continue;
            }
            if let Some(items) = m.child("Items") {
                read.items.extend(items.kids.iter().cloned());
            }
        }
    }
    Ok(read)
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
