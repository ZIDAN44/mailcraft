//! Quick Steps, as Outlook's: several things done to the selected mail in one click (or with
//! Ctrl+Shift+1…9): mark it read, flag, pin or categorize it, then move, archive or delete it.

use mailcraft_model::organize::RuleAction;

use super::*;

/// The most Quick Steps kept, and actions in one.
pub const QUICK_STEPS_MAX: usize = 50;
const ACTIONS_MAX: usize = 10;
const NAME_MAX: usize = 100;

/// A named set of actions done to the selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickStep {
    pub name: String,
    /// In the order given, except that a move (to a folder, Archive, Deleted Items) comes last.
    pub actions: Vec<RuleAction>,
}

impl Session {
    fn quick_step_at(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        self.prefs.quick_steps.iter().position(|q| q.name.eq_ignore_ascii_case(name))
    }

    /// A Quick Step checked: a name not taken by another (`replacing`), actions that can be done
    /// (its categories and folders exist now), at most one move.
    fn checked_quick_step(&self, name: &str, actions: Vec<RuleAction>, replacing: Option<usize>) -> Result<QuickStep, CmdError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_control) {
            return Err(CmdError::Params(format!("a Quick Step needs a name (up to {NAME_MAX} characters)")));
        }
        if self.quick_step_at(name).is_some_and(|i| Some(i) != replacing) {
            return Err(CmdError::Params(format!("there's already a Quick Step named `{name}`")));
        }
        if actions.is_empty() || actions.len() > ACTIONS_MAX {
            return Err(CmdError::Params(format!("a Quick Step does 1 to {ACTIONS_MAX} things")));
        }
        let moves = actions.iter().filter(|a| matches!(a, RuleAction::MoveTo(_) | RuleAction::MoveToRole(_) | RuleAction::Delete)).count();
        if moves > 1 {
            return Err(CmdError::Params("a Quick Step moves mail to one place at most".into()));
        }
        for a in &actions {
            match a {
                RuleAction::Categorize(c) if self.category(c).is_none() => return Err(CmdError::Params(format!("no category `{c}`"))),
                RuleAction::MoveTo(f) if !self.accounts.iter().any(|a| a.folders.iter().any(|x| x.path == *f)) => {
                    return Err(CmdError::Params(format!("no folder `{f}`")));
                }
                RuleAction::MoveToRole(r) if !self.accounts.iter().any(|a| a.folder(*r).is_some()) => {
                    return Err(CmdError::Params("no account has that folder".into()));
                }
                _ => {}
            }
        }
        // The same action twice is done once.
        let mut once: Vec<RuleAction> = Vec::with_capacity(actions.len());
        for a in actions {
            if !once.contains(&a) {
                once.push(a);
            }
        }
        Ok(QuickStep { name: name.to_string(), actions: once })
    }

    pub fn create_quick_step(&mut self, name: &str, actions: Vec<RuleAction>) -> CmdResult {
        if self.prefs.quick_steps.len() >= QUICK_STEPS_MAX {
            return Err(CmdError::Failed(format!("there can be up to {QUICK_STEPS_MAX} Quick Steps")));
        }
        let q = self.checked_quick_step(name, actions, None)?;
        self.prefs.quick_steps.push(q.clone());
        Ok(serde_json::json!({ "name": q.name, "index": self.prefs.quick_steps.len() - 1 }))
    }

    pub fn edit_quick_step(&mut self, name: &str, new_name: Option<&str>, actions: Option<Vec<RuleAction>>) -> CmdResult {
        let at = self.quick_step_at(name).ok_or_else(|| CmdError::Params(format!("no Quick Step `{}`", name.trim())))?;
        let old = self.prefs.quick_steps.get(at).cloned().ok_or_else(|| CmdError::Params("no such Quick Step".into()))?;
        let q = self.checked_quick_step(new_name.unwrap_or(&old.name), actions.unwrap_or(old.actions), Some(at))?;
        if let Some(slot) = self.prefs.quick_steps.get_mut(at) {
            *slot = q.clone();
        }
        Ok(serde_json::json!({ "name": q.name }))
    }

    pub fn delete_quick_step(&mut self, name: &str) -> CmdResult {
        let at = self.quick_step_at(name).ok_or_else(|| CmdError::Params(format!("no Quick Step `{}`", name.trim())))?;
        let gone = self.prefs.quick_steps.remove(at);
        Ok(serde_json::json!({ "deleted": gone.name }))
    }

    /// Do a Quick Step (by name, or its place in the list) to the selection: its marks first,
    /// then its move, which Undo brings back.
    pub fn run_quick_step(&mut self, name: Option<&str>, index: Option<usize>) -> CmdResult {
        let at = match (name, index) {
            (Some(n), _) => self.quick_step_at(n).ok_or_else(|| CmdError::Params(format!("no Quick Step `{}`", n.trim())))?,
            (None, Some(i)) if i < self.prefs.quick_steps.len() => i,
            (None, Some(i)) => return Err(CmdError::Disabled(format!("there's no Quick Step {}", i + 1))),
            (None, None) => return Err(CmdError::Params("`name` or `index` is required".into())),
        };
        let q = self.prefs.quick_steps.get(at).cloned().ok_or_else(|| CmdError::Params("no such Quick Step".into()))?;
        if self.selection.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let mut target: Option<MoveTarget> = None;
        for a in &q.actions {
            match a {
                RuleAction::MarkRead => {
                    self.flag_selection(Flag::Seen, Some(true))?;
                }
                RuleAction::Flag => {
                    self.flag_selection(Flag::Flagged, Some(true))?;
                }
                RuleAction::Pin => {
                    self.pin_selection(Some(true))?;
                }
                // A category deleted since is passed over.
                RuleAction::Categorize(c) => {
                    if self.category(c).is_some() {
                        self.categorize_selection(c, Some(true))?;
                    }
                }
                RuleAction::MoveTo(f) => target = Some(MoveTarget::Folder(f.clone())),
                RuleAction::MoveToRole(r) => target = Some(MoveTarget::Role(*r)),
                RuleAction::Delete => target = Some(MoveTarget::Role(FolderRole::Trash)),
            }
        }
        let n = self.selection.len();
        if let Some(t) = target {
            self.move_selection(t)?;
        }
        Ok(serde_json::json!({ "name": q.name, "messages": n }))
    }
}
