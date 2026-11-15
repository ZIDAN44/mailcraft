//! The fake server's folders: `GetFolder`, `FindFolder` (shallow, deep, in a shared mailbox),
//! and folders made, moved, renamed, deleted and emptied.

use super::*;

/// How many pages of public folders [`Data::endless_public`] says more follow for: far more than
/// a client that stops asks for, and an end for one that doesn't.
pub const ENDLESS_PAGES: usize = 40;

impl Data {
    pub(super) fn get_folder(&self, op: &El) -> Vec<String> {
        let mut out = Vec::new();
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
        out
    }

    /// Shallow (public folders): the folders right under the one asked for. Deep: everything
    /// under the root, in pages, of the account's own mailbox or a shared one.
    pub(super) fn find_folder(&self, op: &El) -> Vec<String> {
        let parent = op.child("ParentFolderIds").and_then(|p| p.kids.first());
        if op.attr("Traversal") == Some("Shallow") {
            return self.find_folder_shallow(op, parent);
        }
        if parent.and_then(|f| self.shared_of(f)).is_some() {
            return self.find_folder_shared(op, parent);
        }
        // Deep: everything under the root (the mailbox's, not the public folders), in pages.
        let view = op.child("IndexedPageFolderView");
        let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
        let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000).min(3);
        let public = self.public_ids();
        let all: Vec<Fo> = self.folders.iter().filter(|f| f.id != "ROOT" && f.owner.is_empty() && !public.contains(&f.id)).cloned().collect();
        let page: Vec<String> = all.iter().skip(offset).take(max).map(|f| self.folder_xml(f)).collect();
        let last = offset + max >= all.len();
        vec![ok(format!(
            "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Folders>{}</t:Folders></m:RootFolder>",
            all.len(),
            page.concat()
        ))]
    }

    /// In pages of three at most, as the deep listings.
    fn find_folder_shallow(&self, op: &El, parent: Option<&El>) -> Vec<String> {
        let parent = parent.and_then(|f| self.folder_id(f)).unwrap_or_default();
        if self.unrouted_hierarchy(&parent) {
            return vec![err("ErrorFolderNotFound")];
        }
        if self.endless_public {
            // More always follow, until a test has had its answer (it counts the pages asked for).
            let last = self.calls.iter().filter(|c| *c == "FindFolder").count() > ENDLESS_PAGES;
            let page = "<t:Folder><t:DisplayName>No id</t:DisplayName><t:ChildFolderCount>0</t:ChildFolderCount></t:Folder>".repeat(3);
            return vec![ok(format!("<m:RootFolder IncludesLastItemInRange=\"{last}\"><t:Folders>{page}</t:Folders></m:RootFolder>"))];
        }
        let kids: Vec<String> = self
            .folders
            .iter()
            .filter(|f| f.parent == parent)
            .map(|f| {
                let n = self.folders.iter().filter(|x| x.parent == f.id).count();
                self.folder_xml(f).replace("<t:ChildFolderCount>0</t:ChildFolderCount>", &format!("<t:ChildFolderCount>{n}</t:ChildFolderCount>"))
            })
            .collect();
        let view = op.child("IndexedPageFolderView");
        let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
        let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000).min(3);
        let page: String = kids.iter().skip(offset).take(max).cloned().collect();
        let last = offset + max >= kids.len();
        vec![ok(format!("<m:RootFolder IncludesLastItemInRange=\"{last}\"><t:Folders>{page}</t:Folders></m:RootFolder>"))]
    }

    /// Deep, in a shared mailbox: refused without access to it; in pages, as the account's own.
    fn find_folder_shared(&self, op: &El, parent: Option<&El>) -> Vec<String> {
        let Some(Ok(owner)) = parent.and_then(|f| self.shared_of(f)) else {
            return vec![err("ErrorAccessDenied")];
        };
        let view = op.child("IndexedPageFolderView");
        let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
        let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000).min(3);
        let all: Vec<&Fo> = self.folders.iter().filter(|f| f.owner == owner && f.known.as_deref() != Some("msgfolderroot")).collect();
        let page: String = all.iter().skip(offset).take(max).map(|f| self.folder_xml(f)).collect();
        let last = offset + max >= all.len();
        vec![ok(format!("<m:RootFolder IncludesLastItemInRange=\"{last}\"><t:Folders>{page}</t:Folders></m:RootFolder>"))]
    }

    pub(super) fn create_folder(&mut self, op: &El) -> String {
        let parent = op.child("ParentFolderId").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_else(|| "ROOT".into());
        let name = op.find("DisplayName").map(|d| d.text.clone()).unwrap_or_default();
        let id = self.folder(&name, Some(&parent), None, "IPF.Note");
        ok(format!("<m:Folders><t:Folder><t:FolderId Id=\"{id}\"/></t:Folder></m:Folders>"))
    }

    pub(super) fn move_folder(&mut self, op: &El) -> Vec<String> {
        let to = op.child("ToFolderId").and_then(|t| t.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_else(|| "ROOT".into());
        let mut out = Vec::new();
        for id in ids(op.child("FolderIds")) {
            if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
                f.parent = to.clone();
            }
            out.push(ok(format!("<m:Folders><t:Folder><t:FolderId Id=\"{id}\"/></t:Folder></m:Folders>")));
        }
        out
    }

    /// `UpdateFolder`: a folder renamed.
    pub(super) fn update_folder(&mut self, op: &El) -> Vec<String> {
        let mut out = Vec::new();
        for ch in op.child("FolderChanges").map(|c| c.kids.clone()).unwrap_or_default() {
            let id = ch.child("FolderId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
            let name = ch.find("DisplayName").map(|d| d.text.clone()).unwrap_or_default();
            if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
                f.name = name;
            }
            out.push(ok(String::new()));
        }
        out
    }

    pub(super) fn delete_folder(&mut self, op: &El) -> Vec<String> {
        let mut out = Vec::new();
        for id in ids(op.child("FolderIds")) {
            self.folders.retain(|f| f.id != id);
            out.push(ok(String::new()));
        }
        out
    }

    pub(super) fn empty_folder(&mut self, op: &El) -> Vec<String> {
        let mut out = Vec::new();
        for id in ids(op.child("FolderIds")) {
            self.items.retain(|i| i.folder != id);
            out.push(ok(String::new()));
        }
        out
    }
}
