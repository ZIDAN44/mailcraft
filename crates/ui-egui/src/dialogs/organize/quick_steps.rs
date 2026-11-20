//! The Quick Step form: what a step does to the mail chosen, and where it moves it.

use super::*;

/// Where a Quick Step moves mail: nowhere, a folder with a role, or a folder by its path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StepMove {
    #[default]
    Stay,
    Role(FolderRole),
    Folder(String),
}

#[derive(Clone, Debug, Default)]
pub struct QuickStepForm {
    /// The name it had (`None`: a new one).
    pub original: Option<String>,
    pub name: String,
    pub read: bool,
    pub flag: bool,
    pub pin: bool,
    /// A category to add (empty: none).
    pub category: String,
    pub move_to: StepMove,
    /// The folders to offer: each account's own, by path, once.
    pub folders: Vec<String>,
    pub error: Option<String>,
}

impl QuickStepForm {
    pub(crate) fn folders(s: &Session) -> Vec<String> {
        let mut v: Vec<String> = s
            .accounts
            .iter()
            .flat_map(|a| a.folders.iter())
            .filter(|f| matches!(f.role, FolderRole::Other | FolderRole::Inbox) && f.mailbox.is_none())
            .map(|f| f.path.clone())
            .collect();
        v.sort();
        v.dedup();
        v.truncate(500);
        v
    }

    /// A new one: marked read and moved to Archive, as Outlook's "Done".
    pub fn new(s: &Session) -> QuickStepForm {
        QuickStepForm { read: true, move_to: StepMove::Role(FolderRole::Archive), folders: Self::folders(s), ..Default::default() }
    }

    pub fn edit(s: &Session, q: &mailcraft_engine::QuickStep) -> QuickStepForm {
        use mailcraft_engine::model::organize::RuleAction;
        let mut f = QuickStepForm { original: Some(q.name.clone()), name: q.name.clone(), folders: Self::folders(s), ..Default::default() };
        for a in &q.actions {
            match a {
                RuleAction::MarkRead => f.read = true,
                RuleAction::Flag => f.flag = true,
                RuleAction::Pin => f.pin = true,
                RuleAction::Categorize(c) => f.category = c.clone(),
                RuleAction::MoveTo(p) => f.move_to = StepMove::Folder(p.clone()),
                RuleAction::MoveToRole(r) => f.move_to = StepMove::Role(*r),
                RuleAction::Delete => f.move_to = StepMove::Role(FolderRole::Trash),
            }
        }
        f
    }

    /// Its actions as `quickStep.create` takes them.
    pub fn actions(&self) -> Vec<serde_json::Value> {
        let mut v = Vec::new();
        if self.read {
            v.push(json!({"do": "markRead"}));
        }
        if self.flag {
            v.push(json!({"do": "flag"}));
        }
        if self.pin {
            v.push(json!({"do": "pin"}));
        }
        if !self.category.is_empty() {
            v.push(json!({"do": "categorize", "value": self.category}));
        }
        match &self.move_to {
            StepMove::Stay => {}
            StepMove::Role(FolderRole::Trash) => v.push(json!({"do": "delete"})),
            StepMove::Role(r) => v.push(json!({"do": "moveToRole", "value": r.code()})),
            StepMove::Folder(p) => v.push(json!({"do": "moveTo", "value": p})),
        }
        v
    }

    pub(crate) fn move_label(m: &StepMove) -> String {
        match m {
            StepMove::Stay => tl!("Don't move").to_string(),
            StepMove::Role(FolderRole::Archive) => tl!("Archive").to_string(),
            StepMove::Role(FolderRole::Trash) => tl!("Deleted Items").to_string(),
            StepMove::Role(FolderRole::Junk) => tl!("Junk Email").to_string(),
            StepMove::Role(r) => r.label().unwrap_or("").to_string(),
            StepMove::Folder(p) => p.clone(),
        }
    }
}
