//! EWS: how much space the mailbox takes, and the most it may, as Outlook's Mailbox Settings
//! shows them.

use super::*;

/// `PR_MESSAGE_SIZE_EXTENDED`: a folder's size, in bytes.
const SIZE_TAG: u32 = 0x0E08;
/// Where the mailbox can no longer send, can no longer receive, and is warned
/// (`PR_PROHIBIT_SEND_QUOTA`, `PR_PROHIBIT_RECEIVE_QUOTA`, `PR_STORAGE_QUOTA_LIMIT`), in KB: the
/// first the server says is the limit shown.
const LIMIT_TAGS: [u32; 3] = [0x666E, 0x666A, 0x3FF5];
/// Most pages of folders whose sizes are added up (500 folders each).
const MAX_PAGES: usize = 40;

impl Client {
    /// The mailbox's size and the most it may hold, in bytes (#143): the sizes of its root folder
    /// and every folder under it added up, and its limit from its root folder; a limit of 0 when
    /// it has none. `None` when the server says neither.
    pub fn quota(&self) -> Result<Option<(u64, u64)>> {
        let prop = |tag: u32, kind: &str| format!("<t:ExtendedFieldURI PropertyTag=\"{tag:#06x}\" PropertyType=\"{kind}\"/>");
        let size = prop(SIZE_TAG, "Long");
        let limits: String = LIMIT_TAGS.iter().map(|t| prop(*t, "Integer")).collect();
        let root = self.call_one(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{size}{limits}</t:AdditionalProperties>\
             </m:FolderShape><m:FolderIds><t:DistinguishedFolderId Id=\"msgfolderroot\"/></m:FolderIds></m:GetFolder>"
        ))?;
        let Some(folder) = root.child("Folders").and_then(|f| f.kids.first()) else { return Ok(None) };
        let limit_kb = LIMIT_TAGS.iter().find_map(|t| extended(folder, *t).filter(|v| *v > 0));
        let mut used = extended(folder, SIZE_TAG);
        let mut offset = 0usize;
        for _ in 0..MAX_PAGES {
            let doc = self.call(&format!(
                "<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{size}\
                 </t:AdditionalProperties></m:FolderShape><m:IndexedPageFolderView MaxEntriesReturned=\"500\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
                 <m:ParentFolderIds><t:DistinguishedFolderId Id=\"msgfolderroot\"/></m:ParentFolderIds></m:FindFolder>"
            ))?;
            let Some(m) = messages(&doc).first().copied() else { break };
            check_message(m)?;
            let Some(rf) = m.child("RootFolder") else { break };
            let folders = rf.folders();
            let n = folders.len();
            for f in folders {
                if let Some(s) = extended(f, SIZE_TAG) {
                    used = Some(used.unwrap_or(0).saturating_add(s));
                }
            }
            offset = offset.saturating_add(n);
            if rf.attr("IncludesLastItemInRange").is_none_or(|v| v == "true") || n == 0 {
                break;
            }
        }
        Ok(match (used, limit_kb) {
            (None, None) => None,
            (used, limit) => Some((used.unwrap_or(0), limit.unwrap_or(0).saturating_mul(1024))),
        })
    }
}

/// A folder's extended property `tag`, as a number: `None` when it hasn't it, or it isn't one.
fn extended(folder: &El, tag: u32) -> Option<u64> {
    folder
        .children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).and_then(tag_number) == Some(tag))
        .and_then(|p| p.text_of("Value"))
        .and_then(|v| v.trim().parse::<i64>().ok())
        .and_then(|v| u64::try_from(v).ok())
}
