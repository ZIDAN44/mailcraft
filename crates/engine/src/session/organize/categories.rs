//! Categories, as Outlook's Color Categories: a list of names and colours, each kept on the
//! server as an IMAP keyword. Keywords for new categories are made as Thunderbird makes its
//! tags' (issue #28), a category's keyword can be seen and set (`$label6` to match SOGo), and a
//! keyword on a message that isn't in the list shows as a category too, named from the keyword,
//! until it's added to the list (Outlook's "not in the Master Category List").

use mailcraft_model::organize::keywords::{self, check_keyword, is_system_keyword};
use mailcraft_model::organize::{Category, RuleAction};

use super::*;

/// The most categories in the list.
const CATEGORIES_MAX: usize = 200;
/// The longest category name.
const NAME_MAX: usize = 60;

/// The colour a category gets from its name when none was chosen: the same every time, so an
/// event's category from the server and a keyword not in the list keep theirs.
pub(crate) fn color_from_name(name: &str) -> u8 {
    let h = name.trim().to_lowercase().bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    u8::try_from(h % 8).unwrap_or(0)
}

/// A name checked for the list: not empty, not too long, no control characters.
fn checked_name(name: &str) -> Result<&str, CmdError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_control) {
        return Err(CmdError::Params(format!("a category needs a name (up to {NAME_MAX} characters)")));
    }
    Ok(name)
}

impl Session {
    /// The category called `name` (any case).
    pub fn category(&self, name: &str) -> Option<&Category> {
        self.prefs.categories.iter().find(|c| c.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The category in the list with this keyword (any case).
    pub fn category_with_keyword(&self, keyword: &str) -> Option<&Category> {
        self.prefs.categories.iter().find(|c| c.keyword.eq_ignore_ascii_case(keyword))
    }

    /// What a keyword that isn't in the list shows as: a readable name made from it
    /// (`r=c3=bcckfrage` → `rückfrage`) and a colour from that name.
    pub fn unlisted_category(keyword: &str) -> Category {
        let name = keywords::readable(keyword);
        let color = color_from_name(&name);
        Category { name, keyword: keyword.to_string(), color }
    }

    /// The categories a message carries: those in the list, in the order the user keeps them,
    /// then its other keywords (not system ones) as [unlisted categories](Self::unlisted_category).
    pub fn categories_of(&self, m: &Summary) -> Vec<Category> {
        let mut out: Vec<Category> = self.prefs.categories.iter().filter(|c| m.flags.has(&c.keyword)).cloned().collect();
        for k in &m.flags.keywords {
            if !is_system_keyword(k) && self.category_with_keyword(k).is_none() {
                out.push(Self::unlisted_category(k));
            }
        }
        out
    }

    /// Whether a message carries a category, in the list or not.
    pub fn is_categorized(&self, m: &Summary) -> bool {
        m.flags.keywords.iter().any(|k| !is_system_keyword(k) || self.category_with_keyword(k).is_some())
    }

    /// Keywords on the messages in the list now that aren't categories in the list, with how
    /// many messages have each, most used first (at most 100).
    pub fn unlisted_keywords(&self) -> Vec<(String, usize)> {
        // Drawn every frame while the dialog is open, over a list that may be long: one pass,
        // each keyword looked at once.
        let listed: HashSet<String> = self.prefs.categories.iter().map(|c| c.keyword.to_ascii_lowercase()).collect();
        let mut seen: HashMap<String, (String, usize)> = HashMap::new();
        for m in &self.list {
            for k in &m.flags.keywords {
                let low = k.to_ascii_lowercase();
                if let Some((_, n)) = seen.get_mut(&low) {
                    *n += 1;
                } else if seen.len() < 1000 && !listed.contains(&low) && !is_system_keyword(k) {
                    seen.insert(low, (k.clone(), 1));
                }
            }
        }
        let mut out: Vec<(String, usize)> = seen.into_values().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase())));
        out.truncate(100);
        out
    }

    /// Add or remove a category on the selection, by its name in the list or (for one that
    /// isn't in the list) by its keyword.
    pub fn categorize_selection(&mut self, name: Option<&str>, keyword: Option<&str>, on: Option<bool>) -> CmdResult {
        let kw = match (name.filter(|n| !n.trim().is_empty()), keyword.filter(|k| !k.is_empty())) {
            (Some(name), _) => {
                self.category(name).map(|c| c.keyword.clone()).ok_or_else(|| CmdError::Params(format!("no category called {name}")))?
            }
            (None, Some(k)) => {
                if self.category_with_keyword(k).is_none() {
                    check_keyword(k).map_err(|e| CmdError::Params(e.into()))?;
                }
                k.to_string()
            }
            (None, None) => return Err(CmdError::Params("`category` or `keyword` is required".into())),
        };
        self.keyword_selection(&kw, on)
    }

    /// A keyword checked for a category: one that can be one, and no other category's
    /// (`except`: the category being changed).
    fn checked_keyword(&self, keyword: &str, except: Option<&str>) -> Result<String, CmdError> {
        let keyword = keyword.trim();
        check_keyword(keyword).map_err(|e| CmdError::Params(e.into()))?;
        if let Some(other) = self.category_with_keyword(keyword).filter(|c| except.is_none_or(|n| !c.name.eq_ignore_ascii_case(n))) {
            return Err(CmdError::Params(format!("the category {} has that keyword", other.name)));
        }
        Ok(keyword.to_string())
    }

    /// Add a category to the list. Its keyword is the one given (`$label6`, or one another
    /// program uses already) or one made from the name as Thunderbird makes a tag's.
    pub fn add_category(&mut self, name: &str, color: Option<u8>, keyword: Option<&str>) -> CmdResult {
        let name = checked_name(name)?;
        if self.category(name).is_some() {
            return Err(CmdError::Params(format!("there is already a category called {name}")));
        }
        if self.prefs.categories.len() >= CATEGORIES_MAX {
            return Err(CmdError::Params(format!("at most {CATEGORIES_MAX} categories")));
        }
        let keyword = match keyword.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => self.checked_keyword(k, None)?,
            None => keywords::new_keyword(name, &self.prefs.categories),
        };
        let color = color.map_or_else(|| u8::try_from(self.prefs.categories.len() % 8).unwrap_or(0), |c| c % 8);
        self.prefs.categories.push(Category { name: name.to_string(), keyword: keyword.clone(), color });
        Ok(serde_json::json!({ "created": name, "keyword": keyword, "color": color }))
    }

    /// Change a category: its name (rules and Quick Steps that use it follow), colour or
    /// keyword. A new keyword isn't put on mail that has the old one: that mail shows the old
    /// keyword as a category not in the list.
    pub fn edit_category(&mut self, name: &str, new_name: Option<&str>, color: Option<u8>, keyword: Option<&str>) -> CmdResult {
        let at = self
            .prefs
            .categories
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| CmdError::Params(format!("no category called {name}")))?;
        let old = self.prefs.categories.get(at).cloned().ok_or_else(|| CmdError::Params(format!("no category called {name}")))?;
        let new_name = match new_name {
            Some(n) => {
                let n = checked_name(n)?;
                if self.category(n).is_some_and(|c| c.name != old.name) {
                    return Err(CmdError::Params(format!("there is already a category called {n}")));
                }
                n.to_string()
            }
            None => old.name.clone(),
        };
        let keyword = match keyword.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) if k.eq_ignore_ascii_case(&old.keyword) => old.keyword.clone(),
            Some(k) => self.checked_keyword(k, Some(&old.name))?,
            None => old.keyword.clone(),
        };
        let color = color.map_or(old.color, |c| c % 8);
        if let Some(c) = self.prefs.categories.get_mut(at) {
            *c = Category { name: new_name.clone(), keyword: keyword.clone(), color };
        }
        let mut followed = 0usize;
        if new_name != old.name {
            let rename = |actions: &mut Vec<RuleAction>| {
                let mut n = 0;
                for a in actions.iter_mut() {
                    if let RuleAction::Categorize(c) = a
                        && c.eq_ignore_ascii_case(&old.name)
                    {
                        *c = new_name.clone();
                        n += 1;
                    }
                }
                n
            };
            for q in &mut self.prefs.quick_steps {
                followed += rename(&mut q.actions);
            }
            let mut rules = self.rules.clone();
            let mut in_rules = 0;
            for r in &mut rules {
                in_rules += rename(&mut r.actions);
            }
            if in_rules > 0 {
                self.set_rules(rules)?;
                followed += in_rules;
            }
        }
        Ok(serde_json::json!({ "name": new_name, "keyword": keyword, "color": color, "followed": followed }))
    }

    /// Forget a category. Messages keep the keyword (another client may still use it), and
    /// show it as a category not in the list.
    pub fn remove_category(&mut self, name: &str) -> CmdResult {
        let before = self.prefs.categories.len();
        self.prefs.categories.retain(|c| !c.name.eq_ignore_ascii_case(name.trim()));
        if self.prefs.categories.len() == before {
            return Err(CmdError::Params(format!("no category called {name}")));
        }
        Ok(serde_json::json!({ "removed": name }))
    }

    /// The category list as a file another computer (or a colleague) can import: each
    /// category's name, keyword and colour.
    pub fn categories_file(&self) -> Result<Vec<u8>, CmdError> {
        let doc = serde_json::json!({ "mailcraftCategories": 1, "categories": self.prefs.categories });
        serde_json::to_vec_pretty(&doc).map_err(|e| CmdError::Failed(e.to_string()))
    }

    /// Bring in the categories of a file made by [`Session::categories_file`], so a team uses
    /// the same keywords. A category here by the same name takes the file's colour and keyword
    /// (it was asked for); one whose keyword another category here has is left out and named.
    pub fn import_categories(&mut self, bytes: &[u8]) -> CmdResult {
        if bytes.len() > 1 << 20 {
            return Err(CmdError::Failed("that file is too big for a categories file".into()));
        }
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let doc: Value = serde_json::from_slice(bytes).map_err(|_| CmdError::Failed("not a MailCraft categories file".into()))?;
        let list = match (doc.get("mailcraftCategories"), doc.get("categories").and_then(Value::as_array)) {
            (Some(_), Some(list)) => list.clone(),
            _ => return Err(CmdError::Failed("not a MailCraft categories file".into())),
        };
        let (mut added, mut changed, mut skipped) = (0usize, 0usize, Vec::new());
        for v in list.into_iter().take(CATEGORIES_MAX) {
            let Ok(c) = serde_json::from_value::<Category>(v) else {
                skipped.push(String::new());
                continue;
            };
            let r = match self.category(&c.name).map(|h| (h.keyword.clone(), h.color)) {
                Some((k, col)) if k.eq_ignore_ascii_case(&c.keyword) && col == c.color % 8 => continue,
                Some(_) => self.edit_category(&c.name, None, Some(c.color), Some(&c.keyword)).map(|_| changed += 1),
                None => self.add_category(&c.name, Some(c.color), Some(&c.keyword)).map(|_| added += 1),
            };
            if r.is_err() {
                skipped.push(c.name.chars().take(NAME_MAX).collect());
            }
        }
        Ok(serde_json::json!({ "added": added, "changed": changed, "skipped": skipped, "categories": self.prefs.categories.len() }))
    }
}
