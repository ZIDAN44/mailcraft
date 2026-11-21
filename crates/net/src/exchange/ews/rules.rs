//! EWS: the mailbox's Inbox rules, kept and run by the server (issue #139; `GetInboxRules` and
//! `UpdateInboxRules`, MS-OXWSRULES): read as MailCraft's own rules where all of a rule can be
//! said as one, and changed without losing what MailCraft can't say of the others.

use mailcraft_model::organize::{Condition, Rule, RuleAction, ServerRule, ServerRuleOp, ServerRules};

use super::*;

/// Rules read at most.
const MAX_RULES: usize = 1000;
/// The most of a rule's conditions, exceptions and actions kept to send back as they came.
const MAX_RAW: usize = 64 * 1024;

/// How a rule's folder is told: by its id on the server, or by the name of a special folder.
pub enum RuleFolder<'a> {
    Id(&'a str),
    Distinguished(&'a str),
}

/// Why a rule set or deleted here isn't: another program changed the rules since they were read.
pub const CHANGED_ELSEWHERE: &str = "the rules were changed in another program meanwhile: they're read again";

impl Client {
    /// The mailbox's rules on the server; `folder_path` says the path here of a folder a rule
    /// names (`None`: one MailCraft doesn't list, and the rule is kept as it is).
    pub fn inbox_rules(&self, folder_path: &dyn Fn(RuleFolder<'_>) -> Option<String>) -> Result<ServerRules> {
        rules_in(&self.call("<m:GetInboxRules/>")?, folder_path)
    }

    /// Change the mailbox's rules on the server. A rule set or deleted is first checked against
    /// what the server has now: another program (Outlook saving its rules gives each a new id)
    /// may have changed it since it was read, and it's then left alone ([`CHANGED_ELSEWHERE`]).
    /// `folder_id` gives the server's id of a folder here (its path); `drop_outlook_list`
    /// removes Outlook's own copy of the rules, which the server won't change them beside.
    pub fn update_inbox_rules(
        &self,
        ops: &[ServerRuleOp],
        drop_outlook_list: bool,
        folder_path: &dyn Fn(RuleFolder<'_>) -> Option<String>,
        folder_id: &dyn Fn(&str) -> Option<String>,
    ) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        let now = self.inbox_rules(folder_path)?;
        let mut xml = String::new();
        for op in ops {
            if let ServerRuleOp::Set { was, .. } | ServerRuleOp::Delete { was } = op
                && !now.rules.iter().any(|r| r.id == was.id && r.raw == was.raw)
            {
                return Err(NetError::No(CHANGED_ELSEWHERE.into()));
            }
            xml.push_str(&match op {
                ServerRuleOp::Create { now } => format!("<t:CreateRuleOperation>{}</t:CreateRuleOperation>", rule_xml(None, now, None, folder_id)?),
                ServerRuleOp::Set { was, now } => {
                    format!("<t:SetRuleOperation>{}</t:SetRuleOperation>", rule_xml(Some(&was.id), now, Some(was), folder_id)?)
                }
                ServerRuleOp::Delete { was } => format!("<t:DeleteRuleOperation><t:RuleId>{}</t:RuleId></t:DeleteRuleOperation>", esc(&was.id)),
            });
        }
        let doc = self.call(&format!(
            "<m:UpdateInboxRules><m:RemoveOutlookRuleBlob>{drop_outlook_list}</m:RemoveOutlookRuleBlob><m:Operations>{xml}</m:Operations></m:UpdateInboxRules>"
        ))?;
        let answer = doc.find("UpdateInboxRulesResponse").ok_or_else(|| NetError::Protocol("no UpdateInboxRulesResponse".into()))?;
        if answer.attr("ResponseClass") == Some("Error") {
            let code = answer.text_of("ResponseCode").unwrap_or("");
            // What was wrong with the rule, as the server says it.
            let why = answer.find("RuleValidationError").and_then(|e| e.text_of("ErrorMessage")).or(answer.text_of("MessageText")).unwrap_or(code);
            return Err(NetError::No(match code {
                "ErrorOutlookRuleBlobExists" => "Outlook keeps its own copy of these rules: it has to be removed to change them here".into(),
                _ => why.to_string(),
            }));
        }
        Ok(())
    }
}

/// The rules in an answer to `GetInboxRules`.
fn rules_in(doc: &El, folder_path: &dyn Fn(RuleFolder<'_>) -> Option<String>) -> Result<ServerRules> {
    let answer = doc.find("GetInboxRulesResponse").ok_or_else(|| NetError::Protocol("no GetInboxRulesResponse".into()))?;
    check_message(answer)?;
    let outlook_list = answer.text_of("OutlookRuleBlobExists") == Some("true");
    let mut rules: Vec<ServerRule> = answer
        .child("InboxRules")
        .map(|l| l.children("Rule").take(MAX_RULES).filter_map(|r| read_rule(r, folder_path)).collect())
        .unwrap_or_default();
    rules.sort_by_key(|r| r.priority);
    Ok(ServerRules { outlook_list, rules })
}

/// The rules in an answer to `GetInboxRules` (its XML) as MailCraft reads them, each folder
/// named by its id (a special one by `@` and its name): what the fuzz target reads.
pub fn read_rules_answer(xml: &str) -> Result<ServerRules> {
    rules_in(&parse(xml), &|f| {
        Some(match f {
            RuleFolder::Id(id) => id.to_string(),
            RuleFolder::Distinguished(name) => format!("@{name}"),
        })
    })
}

/// A rule's conditions and actions as they're written to the server, each folder named by its
/// path: the fuzz target's round trip.
pub fn rule_written(rule: &Rule) -> Result<String> {
    what_it_does(rule, &|p| Some(p.to_string()))
}

/// A rule as the server gave it; `None` when it has no id, or holds more than is kept.
fn read_rule(el: &El, folder_path: &dyn Fn(RuleFolder<'_>) -> Option<String>) -> Option<ServerRule> {
    let id = el.text_of("RuleId").filter(|i| !i.is_empty())?.to_string();
    let flag = |name: &str| el.text_of(name) == Some("true");
    let raw: String = ["Conditions", "Exceptions", "Actions"].iter().filter_map(|n| el.child(n)).map(|p| p.write("t")).collect();
    if raw.len() > MAX_RAW {
        log::info!("an Inbox rule on the server holds more than MailCraft keeps: left out");
        return None;
    }
    let name: String = el.text_of("DisplayName").unwrap_or("").chars().take(256).collect();
    let rule = as_rule(el, &name, folder_path);
    Some(ServerRule {
        id,
        name,
        priority: el.text_of("Priority").and_then(|p| p.trim().parse().ok()).unwrap_or(u32::MAX),
        enabled: flag("IsEnabled"),
        in_error: flag("IsInError"),
        not_supported: flag("IsNotSupported"),
        rule,
        raw,
    })
}

/// The one string of a list of strings, when it holds exactly one.
fn one_string(el: &El) -> Option<String> {
    let mut strings = el.children("String");
    let s = strings.next()?.text.trim().to_string();
    (strings.next().is_none() && !s.is_empty()).then_some(s)
}

/// A rule as MailCraft's own, when all it does can be said so: each of its conditions one
/// MailCraft has (a single text), no exceptions, and each action one MailCraft has.
fn as_rule(el: &El, name: &str, folder_path: &dyn Fn(RuleFolder<'_>) -> Option<String>) -> Option<Rule> {
    if el.child("Exceptions").is_some_and(|e| !e.kids.is_empty()) {
        return None;
    }
    let mut conditions = Vec::new();
    for c in el.child("Conditions").map(|c| c.kids.as_slice()).unwrap_or_default() {
        conditions.push(match c.name.as_str() {
            "ContainsSenderStrings" => Condition::From(one_string(c)?),
            "ContainsRecipientStrings" => Condition::To(one_string(c)?),
            "ContainsSubjectStrings" => Condition::Subject(one_string(c)?),
            "HasAttachments" if c.text.trim() == "true" => Condition::HasAttachment,
            _ => return None,
        });
    }
    let mut actions = Vec::new();
    let mut stop = false;
    for a in el.child("Actions").map(|a| a.kids.as_slice()).unwrap_or_default() {
        let on = a.text.trim() == "true";
        match a.name.as_str() {
            "StopProcessingRules" if on => stop = true,
            "Delete" if on => actions.push(RuleAction::Delete),
            "MarkAsRead" if on => actions.push(RuleAction::MarkRead),
            "AssignCategories" => actions.push(RuleAction::Categorize(one_string(a)?)),
            "MoveToFolder" => {
                let f = a.kids.first()?;
                let path = match (f.name.as_str(), f.attr("Id")) {
                    ("FolderId", Some(id)) => folder_path(RuleFolder::Id(id)),
                    ("DistinguishedFolderId", Some(id)) => folder_path(RuleFolder::Distinguished(id)),
                    _ => None,
                }?;
                actions.push(RuleAction::MoveTo(path));
            }
            _ => return None,
        }
    }
    if conditions.is_empty() || actions.is_empty() {
        return None;
    }
    Some(Rule {
        name: name.to_string(),
        enabled: el.text_of("IsEnabled") == Some("true"),
        account: None,
        conditions,
        match_all: true,
        actions,
        stop,
        sent: false,
    })
}

/// A rule's XML: its id (`None` for a new one), name, order and switch from `now`; what it does
/// from `now.rule` when it was changed here (or is new), else `was`'s as the server wrote it.
fn rule_xml(id: Option<&str>, now: &ServerRule, was: Option<&ServerRule>, folder_id: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let body = match (was, &now.rule) {
        (Some(was), rule) if same_doing(rule.as_ref(), was.rule.as_ref()) => was.raw.clone(),
        (_, Some(rule)) => what_it_does(rule, folder_id)?,
        (_, None) => return Err(NetError::No("a new rule on the server needs a condition and an action".into())),
    };
    let id = id.map(|i| format!("<t:RuleId>{}</t:RuleId>", esc(i))).unwrap_or_default();
    Ok(format!(
        "<t:Rule>{id}<t:DisplayName>{}</t:DisplayName><t:Priority>{}</t:Priority><t:IsEnabled>{}</t:IsEnabled>{body}</t:Rule>",
        esc(now.name.trim()),
        now.priority.max(1),
        now.enabled
    ))
}

/// Whether two rules do the same (their name and switch aside, which the rule's own fields say).
fn same_doing(a: Option<&Rule>, b: Option<&Rule>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (&a.conditions, a.match_all, &a.actions, a.stop) == (&b.conditions, b.match_all, &b.actions, b.stop),
        (None, None) => true,
        _ => false,
    }
}

/// MailCraft's rule as a server rule's conditions and actions, in the order the schema has them.
/// What a server rule can't do (several texts of a kind, "any" of them, list mail, flagging,
/// pinning) is refused: the editor doesn't offer it for one.
fn what_it_does(rule: &Rule, folder_id: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let cant = |what: &str| NetError::No(format!("a rule on the server can't {what}"));
    if !rule.match_all && rule.conditions.len() > 1 {
        return Err(cant("match any one of several conditions"));
    }
    let strings = |s: &str| format!("<t:String>{}</t:String>", esc(s));
    let (mut to, mut from, mut subject, mut attachments) = (None, None, None, false);
    for c in &rule.conditions {
        let (slot, t) = match c {
            Condition::To(t) => (&mut to, t),
            Condition::From(t) => (&mut from, t),
            Condition::Subject(t) => (&mut subject, t),
            Condition::HasAttachment => {
                attachments = true;
                continue;
            }
            Condition::AnyText(_) => return Err(cant("look at the sender, recipients and subject together")),
            Condition::Bulk => return Err(cant("tell list mail")),
        };
        if slot.replace(strings(t)).is_some() {
            return Err(cant("have two conditions of one kind"));
        }
    }
    let mut conditions = String::new();
    for (name, v) in [("ContainsRecipientStrings", to), ("ContainsSenderStrings", from), ("ContainsSubjectStrings", subject)] {
        if let Some(v) = v {
            conditions.push_str(&format!("<t:{name}>{v}</t:{name}>"));
        }
    }
    if attachments {
        conditions.push_str("<t:HasAttachments>true</t:HasAttachments>");
    }
    if conditions.is_empty() {
        return Err(cant("run without a condition"));
    }
    let (mut category, mut delete, mut read, mut move_to) = (None, false, false, None);
    for a in &rule.actions {
        match a {
            RuleAction::Categorize(c) => {
                if category.replace(strings(c)).is_some() {
                    return Err(cant("give two categories"));
                }
            }
            RuleAction::Delete => delete = true,
            RuleAction::MarkRead => read = true,
            RuleAction::MoveTo(path) => {
                let id = folder_id(path).ok_or_else(|| NetError::No(format!("there is no folder {path} on the server")))?;
                if move_to.replace(format!("<t:FolderId Id=\"{}\"/>", esc(&id))).is_some() {
                    return Err(cant("move mail to two folders"));
                }
            }
            RuleAction::MoveToRole(role) => {
                let known = SPECIAL.iter().find(|(_, r)| r == role).map(|(n, _)| *n).or(matches!(role, FolderRole::Archive).then_some("archive"));
                let known = known.ok_or_else(|| cant("move mail to that folder"))?;
                if move_to.replace(format!("<t:DistinguishedFolderId Id=\"{known}\"/>")).is_some() {
                    return Err(cant("move mail to two folders"));
                }
            }
            RuleAction::Flag => return Err(cant("flag mail")),
            RuleAction::Pin => return Err(cant("pin mail")),
        }
    }
    let mut actions = String::new();
    if let Some(c) = category {
        actions.push_str(&format!("<t:AssignCategories>{c}</t:AssignCategories>"));
    }
    if delete {
        actions.push_str("<t:Delete>true</t:Delete>");
    }
    if read {
        actions.push_str("<t:MarkAsRead>true</t:MarkAsRead>");
    }
    if let Some(m) = move_to {
        actions.push_str(&format!("<t:MoveToFolder>{m}</t:MoveToFolder>"));
    }
    if rule.stop {
        actions.push_str("<t:StopProcessingRules>true</t:StopProcessingRules>");
    }
    if actions.is_empty() || actions == "<t:StopProcessingRules>true</t:StopProcessingRules>" {
        return Err(cant("run without an action"));
    }
    Ok(format!("<t:Conditions>{conditions}</t:Conditions><t:Actions>{actions}</t:Actions>"))
}
