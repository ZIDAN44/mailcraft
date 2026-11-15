//! The Search Folders the dialog offers, and the one being made or changed.

/// The Search Folders offered, as Outlook's New Search Folder lists them: a name and what it
/// searches for.
pub const SEARCH_PRESETS: [(&str, PresetKind); 10] = [
    ("Unread mail", PresetKind::Fixed("is:unread")),
    ("Mail flagged for follow up", PresetKind::Fixed("is:flagged")),
    ("Important mail", PresetKind::Fixed("is:important")),
    ("Categorized mail", PresetKind::Fixed("is:categorized")),
    ("Mail with attachments", PresetKind::Fixed("has:attachment")),
    ("Large mail (over 100 KB)", PresetKind::Fixed("larger:100KB")),
    ("Pinned mail", PresetKind::Fixed("is:pinned")),
    // Not one of Outlook's: what you sent that nobody has answered (two to thirty days old).
    ("Sent mail waiting for a reply", PresetKind::Fixed("is:awaiting")),
    ("Mail from specific people", PresetKind::People),
    ("Custom", PresetKind::Custom),
];

/// What a Search Folder of [`SEARCH_PRESETS`] searches for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetKind {
    /// This search.
    Fixed(&'static str),
    /// Mail from the people the dialog asks for.
    People,
    /// The search typed in the dialog.
    Custom,
}

impl Default for PresetKind {
    /// The first offered.
    fn default() -> PresetKind {
        SEARCH_PRESETS.first().map_or(PresetKind::Custom, |p| p.1)
    }
}

impl PresetKind {
    /// Its name in the list (in English, to translate).
    pub fn label(self) -> &'static str {
        SEARCH_PRESETS.iter().find(|p| p.1 == self).map_or("Custom", |p| p.0)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SearchFolderForm {
    /// The name it had (`None`: a new one).
    pub original: Option<String>,
    /// Which of [`SEARCH_PRESETS`].
    pub preset: PresetKind,
    pub name: String,
    /// For "Mail from specific people": addresses or names, separated by commas.
    pub people: String,
    /// For "Custom": the search, as typed in the search box.
    pub query: String,
    pub error: Option<String>,
}

impl SearchFolderForm {
    pub fn new() -> SearchFolderForm {
        SearchFolderForm { name: tl!("Unread mail").to_string(), ..Default::default() }
    }

    /// An existing one, to change: its search shown as written.
    pub fn edit(f: &mailcraft_engine::SearchFolder) -> SearchFolderForm {
        let preset = SEARCH_PRESETS.iter().map(|p| p.1).find(|k| matches!(k, PresetKind::Fixed(q) if *q == f.query)).unwrap_or(PresetKind::Custom);
        SearchFolderForm { original: Some(f.name.clone()), preset, name: f.name.clone(), query: f.query.clone(), ..Default::default() }
    }

    /// Another kind picked: its name follows, unless one was typed.
    pub fn apply_preset(&mut self, before: PresetKind) {
        if self.name.trim().is_empty() || self.name == tl!(before.label()) {
            self.name = tl!(self.preset.label()).to_string();
        }
        // Custom starts from the search it was, to change.
        if let (PresetKind::Custom, PresetKind::Fixed(was)) = (self.preset, before)
            && self.query.trim().is_empty()
        {
            self.query = was.to_string();
        }
    }

    /// What it searches for: `from:ana,"zoe park"` for people.
    pub fn final_query(&self) -> String {
        match self.preset {
            PresetKind::People => {
                let people: Vec<String> = self
                    .people
                    .split([',', ';'])
                    .map(|p| p.trim().replace('"', ""))
                    .filter(|p| !p.is_empty())
                    .map(|p| if p.contains(char::is_whitespace) { format!("\"{p}\"") } else { p })
                    .collect();
                if people.is_empty() { String::new() } else { format!("from:{}", people.join(",")) }
            }
            PresetKind::Custom => self.query.trim().to_string(),
            PresetKind::Fixed(query) => query.to_string(),
        }
    }
}
