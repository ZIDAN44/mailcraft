//! The session: an Exchange mailbox's Inbox rules on its server (issue #139), which the server
//! keeps and runs: asked for when the Rules dialog opens, changed by the `rules.server…`
//! commands, and shown as the server has them once it says.

use mailcraft_model::organize::{Rule, ServerRule, ServerRuleOp, ServerRules};

use super::*;

/// What is known of an account's rules on its server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ServerRulesState {
    /// Not asked for (or the account keeps none there).
    #[default]
    Unknown,
    /// Asked for, or a change on its way.
    Loading,
    /// As the server has them; `error`: why the last change wasn't made.
    Loaded { rules: ServerRules, error: Option<String> },
    /// The server gave none (it offers none, or reading them failed).
    Failed(String),
}

/// Why a change isn't made while Outlook's own copy of the rules is beside them, until it's
/// asked to remove it.
pub const OUTLOOK_LIST: &str =
    "Outlook keeps its own copy of these rules: changing them here removes it, with Outlook's rules that are turned off or run only on its computer";

impl Session {
    /// The accounts whose rules are on their server: Exchange's (EWS).
    pub fn server_rule_accounts(&self) -> Vec<String> {
        self.accounts.iter().filter(|a| a.config.ews.is_some()).map(|a| a.config.id.clone()).collect()
    }

    /// What is known of an account's rules on its server.
    pub fn server_rules(&self, account: &str) -> &ServerRulesState {
        static NONE: ServerRulesState = ServerRulesState::Unknown;
        self.account(account).map_or(&NONE, |a| &a.server_rules)
    }

    /// Ask an Exchange account's server for its rules.
    pub fn load_server_rules(&mut self, account: &str) -> CmdResult {
        let a = self.account_mut(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if a.config.ews.is_none() {
            return Err(CmdError::Params("only an Exchange account keeps rules on its server that MailCraft can show".into()));
        }
        a.server_rules = ServerRulesState::Loading;
        self.jobs.push(Job::GetServerRules { account: account.to_string() });
        Ok(serde_json::json!({"pending": true}))
    }

    /// The rules as the server has them, or why there are none ([`Event::ServerRules`]).
    pub(crate) fn server_rules_read(&mut self, account: &str, rules: Option<ServerRules>, error: Option<String>) {
        let Some(a) = self.account_mut(account) else { return };
        a.server_rules = match (rules, error) {
            (Some(mut rules), error) => {
                for r in rules.rules.iter_mut().filter_map(|r| r.rule.as_mut()) {
                    // A rule on the server runs on this account's mail alone.
                    r.account = Some(account.to_string());
                }
                ServerRulesState::Loaded { rules, error }
            }
            (None, error) => ServerRulesState::Failed(error.unwrap_or_else(|| "the server gave no rules".into())),
        };
    }

    /// The rules as last read, ready to be changed: not while a change is on its way, nor
    /// beside Outlook's own copy of them unless `drop_outlook_list` says it goes.
    fn rules_to_change(&self, account: &str, drop_outlook_list: bool) -> std::result::Result<&ServerRules, CmdError> {
        match self.server_rules(account) {
            ServerRulesState::Loaded { rules, .. } if rules.outlook_list && !drop_outlook_list => Err(CmdError::Failed(OUTLOOK_LIST.into())),
            ServerRulesState::Loaded { rules, .. } => Ok(rules),
            ServerRulesState::Loading => Err(CmdError::Failed("the rules are being read from the server".into())),
            _ => Err(CmdError::Params("read the rules from the server first (rules.server)".into())),
        }
    }

    fn server_rule(rules: &ServerRules, id: &str) -> std::result::Result<ServerRule, CmdError> {
        rules.rules.iter().find(|r| r.id == id).cloned().ok_or_else(|| CmdError::Params(format!("no rule `{id}` on the server")))
    }

    /// Send changes to the server; the rules are shown as it has them once it says.
    fn change_server_rules(&mut self, account: &str, ops: Vec<ServerRuleOp>, drop_outlook_list: bool) -> CmdResult {
        if let Some(a) = self.account_mut(account) {
            a.server_rules = ServerRulesState::Loading;
        }
        let n = ops.len();
        self.jobs.push(Job::SetServerRules { account: account.to_string(), ops, drop_outlook_list });
        Ok(serde_json::json!({"changes": n, "pending": true}))
    }

    /// A new rule on the server, last in the order, or the rule `id` made to do what `rule`
    /// says (its name and switch too).
    pub fn save_server_rule(&mut self, account: &str, id: Option<&str>, rule: Rule, drop_outlook_list: bool) -> CmdResult {
        if rule.name.trim().is_empty() || rule.conditions.is_empty() || rule.actions.is_empty() {
            return Err(CmdError::Params("a rule needs a name, a condition and an action".into()));
        }
        let rules = self.rules_to_change(account, drop_outlook_list)?;
        let op = match id {
            Some(id) => {
                let was = Self::server_rule(rules, id)?;
                if was.not_supported || was.rule.is_none() {
                    return Err(CmdError::Params("this rule was made in another program: it can be turned on or off, moved or deleted here".into()));
                }
                let now = ServerRule { name: rule.name.trim().to_string(), enabled: rule.enabled, rule: Some(rule), ..was.clone() };
                ServerRuleOp::Set { was, now }
            }
            None => {
                let priority = rules.rules.iter().map(|r| r.priority).filter(|p| *p < u32::MAX).max().unwrap_or(0).saturating_add(1);
                let now = ServerRule { name: rule.name.trim().to_string(), priority, enabled: rule.enabled, rule: Some(rule), ..Default::default() };
                ServerRuleOp::Create { now }
            }
        };
        self.change_server_rules(account, vec![op], drop_outlook_list)
    }

    /// Turn the rule `id` on or off.
    pub fn switch_server_rule(&mut self, account: &str, id: &str, on: bool, drop_outlook_list: bool) -> CmdResult {
        let was = Self::server_rule(self.rules_to_change(account, drop_outlook_list)?, id)?;
        let now = ServerRule { enabled: on, rule: was.rule.clone().map(|r| Rule { enabled: on, ..r }), ..was.clone() };
        self.change_server_rules(account, vec![ServerRuleOp::Set { was, now }], drop_outlook_list)
    }

    /// Move the rule `id` one up (or down) the order: it and its neighbour change places.
    pub fn move_server_rule(&mut self, account: &str, id: &str, up: bool, drop_outlook_list: bool) -> CmdResult {
        let rules = self.rules_to_change(account, drop_outlook_list)?;
        let at = rules.rules.iter().position(|r| r.id == id).ok_or_else(|| CmdError::Params(format!("no rule `{id}` on the server")))?;
        let other = if up { at.checked_sub(1) } else { Some(at + 1).filter(|o| *o < rules.rules.len()) };
        let Some(other) = other else { return Ok(serde_json::json!({"changes": 0})) };
        let (Some(a), Some(b)) = (rules.rules.get(at).cloned(), rules.rules.get(other).cloned()) else {
            return Ok(serde_json::json!({"changes": 0}));
        };
        // Each takes the other's place (their numbers apart when the server gave them alike).
        let (pa, pb) = if a.priority == b.priority {
            let low = (at.min(other) + 1) as u32;
            if at < other { (low + 1, low) } else { (low, low + 1) }
        } else {
            (b.priority, a.priority)
        };
        let ops = vec![
            ServerRuleOp::Set { now: ServerRule { priority: pa, ..a.clone() }, was: a },
            ServerRuleOp::Set { now: ServerRule { priority: pb, ..b.clone() }, was: b },
        ];
        self.change_server_rules(account, ops, drop_outlook_list)
    }

    /// Delete the rule `id` from the server.
    pub fn delete_server_rule(&mut self, account: &str, id: &str, drop_outlook_list: bool) -> CmdResult {
        let was = Self::server_rule(self.rules_to_change(account, drop_outlook_list)?, id)?;
        if was.not_supported {
            return Err(CmdError::Params("this rule was made in a program that can do more than the server says: delete it there".into()));
        }
        self.change_server_rules(account, vec![ServerRuleOp::Delete { was }], drop_outlook_list)
    }
}
