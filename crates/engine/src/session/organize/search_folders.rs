//! Search Folders, as Outlook's: a saved search shown as a folder (Unread Mail, Large Mail, mail
//! from certain people…), its messages found in every folder of every account, except Deleted
//! Items and Junk, whenever it's opened.

use mailcraft_model::search::Query;

use super::*;

/// The most Search Folders kept.
pub const SEARCH_FOLDERS_MAX: usize = 100;
/// The longest name and query.
const NAME_MAX: usize = 100;
const QUERY_MAX: usize = 1000;

/// A saved search shown as a folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFolder {
    pub name: String,
    /// In the search box's language (`is:unread`, `from:ana larger:1MB`).
    pub query: String,
}

impl Session {
    fn search_folder_at(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        self.prefs.search_folders.iter().position(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// A Search Folder checked: a name not taken by another (`replacing`), a query that searches.
    fn checked_search_folder(&self, name: &str, query: &str, replacing: Option<usize>) -> Result<SearchFolder, CmdError> {
        let name = name.trim();
        let query = query.trim();
        if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_control) {
            return Err(CmdError::Params(format!("a Search Folder needs a name (up to {NAME_MAX} characters)")));
        }
        if self.search_folder_at(name).is_some_and(|i| Some(i) != replacing) {
            return Err(CmdError::Params(format!("there's already a Search Folder named `{name}`")));
        }
        if query.is_empty() || query.chars().count() > QUERY_MAX || Query::parse(query).is_empty() {
            return Err(CmdError::Params("a Search Folder needs something to search for".into()));
        }
        Ok(SearchFolder { name: name.to_string(), query: query.to_string() })
    }

    pub fn create_search_folder(&mut self, name: &str, query: &str) -> CmdResult {
        if self.prefs.search_folders.len() >= SEARCH_FOLDERS_MAX {
            return Err(CmdError::Failed(format!("there can be up to {SEARCH_FOLDERS_MAX} Search Folders")));
        }
        let f = self.checked_search_folder(name, query, None)?;
        self.prefs.search_folders.push(f.clone());
        self.open_search_folder(&f.name)
    }

    /// Rename it or change what it searches for (each optional).
    pub fn edit_search_folder(&mut self, name: &str, new_name: Option<&str>, query: Option<&str>) -> CmdResult {
        let at = self.search_folder_at(name).ok_or_else(|| CmdError::Params(format!("no Search Folder `{}`", name.trim())))?;
        let old = self.prefs.search_folders.get(at).cloned().ok_or_else(|| CmdError::Params("no such Search Folder".into()))?;
        let f = self.checked_search_folder(new_name.unwrap_or(&old.name), query.unwrap_or(&old.query), Some(at))?;
        if let Some(slot) = self.prefs.search_folders.get_mut(at) {
            *slot = f.clone();
        }
        // Open, it shows what it now finds.
        if self.search_folder.as_deref() == Some(old.name.as_str()) {
            return self.open_search_folder(&f.name);
        }
        Ok(serde_json::json!({ "name": f.name, "query": f.query }))
    }

    pub fn delete_search_folder(&mut self, name: &str) -> CmdResult {
        let at = self.search_folder_at(name).ok_or_else(|| CmdError::Params(format!("no Search Folder `{}`", name.trim())))?;
        let gone = self.prefs.search_folders.remove(at);
        // Only the search goes: the messages it found stay where they are.
        if self.search_folder.as_deref() == Some(gone.name.as_str()) {
            self.run_search("", SearchScope::All);
        }
        Ok(serde_json::json!({ "deleted": gone.name }))
    }

    /// Show what a Search Folder finds now.
    pub fn open_search_folder(&mut self, name: &str) -> CmdResult {
        let at = self.search_folder_at(name).ok_or_else(|| CmdError::Params(format!("no Search Folder `{}`", name.trim())))?;
        let f = self.prefs.search_folders.get(at).cloned().ok_or_else(|| CmdError::Params("no such Search Folder".into()))?;
        let n = self.search_as(&f.query, SearchScope::All, Some(f.name.clone()));
        Ok(serde_json::json!({ "name": f.name, "query": f.query, "messages": n }))
    }

    /// The Search Folder on screen, if the search is still its own.
    pub fn open_search_folder_name(&self) -> Option<&str> {
        let name = self.search_folder.as_deref()?;
        let f = self.prefs.search_folders.iter().find(|f| f.name == name)?;
        (self.search.as_ref().map(|(q, _)| q.as_str()) == Some(f.query.as_str())).then_some(name)
    }
}
