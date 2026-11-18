//! The fake server's Inbox rules (issue #139): `GetInboxRules` and `UpdateInboxRules`, with
//! Outlook's own copy of the rules that stops changes until it's removed, and rules saved again
//! by Outlook (each with a new id).

use super::*;

impl Data {
    /// A rule as another program made it (its XML, `<Rule>` and all, in the types namespace):
    /// given an id, last in the order.
    pub fn add_rule(&mut self, xml: &str) -> String {
        let mut rule = crate::ews::parse(xml).kids.into_iter().next().unwrap_or_default();
        let id = self.id("RULE");
        rule.kids.retain(|k| k.name != "RuleId");
        rule.kids.insert(0, El { name: "RuleId".into(), text: id.clone(), ..Default::default() });
        self.rules.push(rule);
        id
    }

    /// Outlook saving the rules: each comes back with a new id.
    pub fn rules_saved_elsewhere(&mut self) {
        let fresh: Vec<String> = (0..self.rules.len()).map(|_| self.id("RULE")).collect();
        for (r, id) in self.rules.iter_mut().zip(fresh) {
            if let Some(k) = r.kids.iter_mut().find(|k| k.name == "RuleId") {
                k.text = id;
            }
        }
    }

    pub(super) fn inbox_rules_answer(&self) -> String {
        let rules: String = self.rules.iter().map(|r| r.write("t")).collect();
        format!(
            "<m:ResponseCode>NoError</m:ResponseCode><m:OutlookRuleBlobExists>{}</m:OutlookRuleBlobExists><m:InboxRules>{rules}</m:InboxRules>",
            self.outlook_rules
        )
    }

    pub(super) fn update_inbox_rules(&mut self, op: &El) -> String {
        let drop_list = op.text_of("RemoveOutlookRuleBlob") == Some("true");
        if self.outlook_rules && !drop_list {
            return "<m:ResponseCode>ErrorOutlookRuleBlobExists</m:ResponseCode><m:MessageText>Outlook rules exist</m:MessageText>".into();
        }
        let id_of = |r: &El| r.text_of("RuleId").unwrap_or("").to_string();
        let mut rules = self.rules.clone();
        for (i, o) in op.child("Operations").map(|o| o.kids.clone()).unwrap_or_default().into_iter().enumerate() {
            let rule = o.child("Rule").cloned();
            let wanted = rule.as_ref().map(id_of).or_else(|| o.text_of("RuleId").map(str::to_string)).unwrap_or_default();
            let at = rules.iter().position(|r| id_of(r) == wanted);
            match (o.name.as_str(), rule, at) {
                ("CreateRuleOperation", Some(mut r), _) => {
                    let id = self.id("RULE");
                    r.kids.retain(|k| k.name != "RuleId");
                    r.kids.insert(0, El { name: "RuleId".into(), text: id, ..Default::default() });
                    rules.push(r);
                }
                ("SetRuleOperation", Some(r), Some(at)) => rules[at] = r,
                ("DeleteRuleOperation", _, Some(at)) => {
                    rules.remove(at);
                }
                _ => {
                    return format!(
                        "<m:ResponseCode>ErrorInboxRulesValidationError</m:ResponseCode><m:MessageText>validation</m:MessageText><m:RuleOperationErrors>\
                         <t:RuleOperationError><t:OperationIndex>{i}</t:OperationIndex><t:ValidationErrors><t:RuleValidationError>\
                         <t:FieldURI>RuleId</t:FieldURI><t:ErrorCode>RuleNotFound</t:ErrorCode><t:ErrorMessage>The rule wasn't found.</t:ErrorMessage>\
                         </t:RuleValidationError></t:ValidationErrors></t:RuleOperationError></m:RuleOperationErrors>"
                    );
                }
            }
        }
        self.rules = rules;
        self.outlook_rules = false;
        self.rule_updates += 1;
        "<m:ResponseCode>NoError</m:ResponseCode>".into()
    }
}
