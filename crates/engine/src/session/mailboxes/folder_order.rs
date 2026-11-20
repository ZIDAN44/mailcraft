//! Folders in an order of the user's own (issue #183), as Outlook lets a folder be dragged to its
//! place or moved up and down: kept by account (`Prefs::folder_order`, paths in that order),
//! special folders keeping theirs, and A to Z again when asked. It's the folder pane's order; a
//! server keeps none.

use super::*;
use mailcraft_model::{is_orderable, pane_order};

impl Session {
    /// An account's folders in the folder pane's order: the user's own, else A to Z.
    pub fn pane_folders<'a>(&self, a: &'a AccountState) -> Vec<&'a Folder> {
        match self.prefs.folder_order.get(&a.config.id).filter(|o| !o.is_empty()) {
            Some(order) => pane_order(&a.folders, order).into_iter().filter_map(|i| a.folders.get(i)).collect(),
            None => a.folders.iter().collect(),
        }
    }

    /// Whether an account's folders are in an order of the user's own.
    pub fn folders_ordered(&self, account: &str) -> bool {
        self.prefs.folder_order.get(account).is_some_and(|o| !o.is_empty())
    }

    /// The user's folders in `folder`'s place: those beside it, in the pane's order.
    fn siblings(&self, account: &str, folder: &str) -> Vec<String> {
        let Some(a) = self.account(account) else { return Vec::new() };
        let d = a.folders.iter().find_map(|f| f.delimiter);
        let up = Self::parent_of(folder, d);
        self.pane_folders(a).into_iter().filter(|f| is_orderable(f) && Self::parent_of(&f.path, d) == up).map(|f| f.path.clone()).collect()
    }

    /// Put `folder` before (or `after`) `target`: beside it when they're in the same folder, else
    /// moved into `target`'s folder first, and put there.
    pub fn order_folder(&mut self, account: &str, folder: &str, target: &str, after: bool) -> CmdResult {
        let f = self.user_folder(account, folder)?;
        let t = self.user_folder(account, target)?;
        if f.path == t.path {
            return Ok(serde_json::json!({ "folder": folder }));
        }
        let d = f.delimiter.or(self.delimiter(account));
        if mailcraft_model::is_within(&t.path, &f.path, d) {
            return Err(CmdError::Params("a folder can't go inside itself".into()));
        }
        // Into the target's folder first, when it's in another.
        let mut path = f.path.clone();
        let (up, there) = (Self::parent_of(&f.path, d), Self::parent_of(&t.path, d));
        if up != there {
            let moved = self.move_folder(account, &f.path, there)?;
            path = moved.get("moved").and_then(serde_json::Value::as_str).unwrap_or(&f.path).to_string();
        }
        // The pane's order, the folder taken out and put back beside the target.
        let a = self.account(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        let mut order: Vec<String> = self.pane_folders(a).into_iter().filter(|x| is_orderable(x)).map(|x| x.path.clone()).collect();
        order.retain(|p| *p != path);
        let at = order.iter().position(|p| *p == t.path).ok_or_else(|| CmdError::Params(format!("no folder `{target}`")))?;
        order.insert(if after { at + 1 } else { at }, path.clone());
        self.prefs.folder_order.insert(account.to_string(), order);
        Ok(serde_json::json!({ "folder": path, "before": if after { serde_json::Value::Null } else { target.into() } }))
    }

    /// Move `folder` up (`by` < 0) or down among those beside it.
    pub fn reorder_folder(&mut self, account: &str, folder: &str, by: i64) -> CmdResult {
        self.user_folder(account, folder)?;
        let around = self.siblings(account, folder);
        let at = around.iter().position(|p| p == folder).ok_or_else(|| CmdError::Params(format!("no folder `{folder}`")))?;
        let to = if by < 0 { at.checked_sub(1) } else { Some(at + 1).filter(|i| *i < around.len()) };
        let Some(other) = to.and_then(|i| around.get(i)).cloned() else {
            return Err(CmdError::Disabled(if by < 0 { "it's first already".into() } else { "it's last already".into() }));
        };
        self.order_folder(account, folder, &other, by > 0)
    }

    /// Whether `folder` can move up (`by` < 0) or down among those beside it.
    pub fn can_reorder_folder(&self, account: &str, folder: &str, by: i64) -> bool {
        let around = self.siblings(account, folder);
        around.iter().position(|p| p == folder).is_some_and(|at| if by < 0 { at > 0 } else { at + 1 < around.len() })
    }

    /// An account's folders A to Z again.
    pub fn folders_a_to_z(&mut self, account: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        self.prefs.folder_order.remove(account);
        Ok(serde_json::json!({ "account": account }))
    }

    /// A folder renamed or moved (`to`), or deleted (none): the order follows it.
    pub(in crate::session) fn order_follows_folder(&mut self, account: &str, from: &str, to: Option<&str>, d: Option<char>) {
        let Some(order) = self.prefs.folder_order.get_mut(account) else { return };
        match to {
            Some(to) => order.iter_mut().for_each(|p| {
                if let Some(new) = mailcraft_model::rebase_path(p, from, to, d) {
                    *p = new;
                }
            }),
            None => order.retain(|p| !mailcraft_model::is_within(p, from, d)),
        }
    }
}
