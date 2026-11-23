//! The organisation's signature and disclaimer (issue #245). The signature is a template:
//! `{name}`, `{title}`, `{department}`, `{phone}`, `{mobile}`, `{office}`, `{company}` and
//! `{email}`, filled from the person's details (theirs to give, in File › Info, and their
//! organisation's directory's on Exchange and Microsoft 365). A line of it whose details are
//! missing goes. Locked, it's in every account of its domain as the organisation wrote it, the
//! one for new messages and replies, and can't be changed or removed; otherwise it's put in once
//! and again only when the organisation's changes. The disclaimer goes under every message sent
//! from the domain's accounts, as it's sent: before it's signed or encrypted, and never in the
//! message being written, where it could be taken out.

use std::collections::BTreeMap;

use mailcraft_model::{DirectoryPerson, Signature};
use serde_json::{Value, json};

use super::*;
use crate::policy::SignatureRule;

/// The person's details a signature takes, in the order File › Info asks for them.
pub const DETAILS: [&str; 7] = ["name", "title", "department", "phone", "mobile", "office", "company"];
/// The longest a detail is kept.
const DETAIL_MAX: usize = 200;

/// HTML's own characters written as text.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `template` with each `{detail}` filled from `values` (escaped for HTML when `html`). In
/// text, a line whose details are all missing goes: no "Phone: " with nothing after it.
pub fn fill(template: &str, values: &BTreeMap<String, String>, html: bool) -> String {
    let put = |line: &str| -> (String, bool, bool) {
        let mut out = String::with_capacity(line.len());
        let (mut asked, mut given) = (false, false);
        let mut rest = line;
        while let Some(open) = rest.find('{') {
            out.push_str(rest.get(..open).unwrap_or_default());
            let after = rest.get(open + 1..).unwrap_or_default();
            match after.find('}').map(|close| (after.get(..close).unwrap_or_default(), close)) {
                Some((key, close)) if DETAILS.contains(&key) || key == "email" => {
                    asked = true;
                    let v = values.get(key).map(|v| v.trim()).unwrap_or_default();
                    given |= !v.is_empty();
                    out.push_str(&if html { escape(v) } else { v.to_string() });
                    rest = after.get(close + 1..).unwrap_or_default();
                }
                _ => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        (out, asked, given)
    };
    if html {
        return put(template).0;
    }
    template
        .lines()
        .filter_map(|line| {
            let (out, asked, given) = put(line);
            (!asked || given).then_some(out)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Session {
    /// An account's details as a signature takes them: theirs, with its own name and address
    /// where they gave none.
    fn details_of(c: &AccountConfig) -> BTreeMap<String, String> {
        let mut d = c.details.clone();
        d.entry("name".into()).or_insert_with(|| c.name.clone());
        d.insert("email".into(), c.email.clone());
        d
    }

    /// The organisation's signature for account `c`, filled in, and its rule.
    pub fn org_signature(&self, c: &AccountConfig) -> Option<(Signature, SignatureRule)> {
        let rule = self.organisation.signature_for(&c.email)?.clone();
        let details = Self::details_of(c);
        let text = fill(&rule.text, &details, false);
        let html = fill(&rule.html, &details, true);
        let mut v = json!({"signature": text});
        if !html.trim().is_empty() {
            v["code"] = json!(html);
        }
        let mut sig = crate::cmd::signature_param(&v, rule.name.clone()).ok()?;
        // Its text is the organisation's own text, where it gave one (not the HTML's words).
        if !text.trim().is_empty() {
            sig.text = text;
        }
        Some((sig, rule))
    }

    /// The organisation's signature put into `c` as its rule says; whether `c` changed. `before`
    /// is `c` before its details changed: an unlocked signature still as they filled it is
    /// filled again from the new ones.
    pub(crate) fn impose_org_signature(&self, c: &mut AccountConfig, before: Option<&AccountConfig>) -> bool {
        let Some((sig, rule)) = self.org_signature(c) else { return false };
        // Its template is what's kept, not its filled-in text: an unlocked one the person
        // changed or removed stays so when their details or name change.
        let made = format!("{}\n{}", rule.text, rule.html);
        let there = c.signature_named(&rule.name).cloned();
        let due = rule.locked && there.as_ref() != Some(&sig) || !rule.locked && c.org_signature != made;
        let refill = !rule.locked
            && there.as_ref().is_some_and(|t| *t != sig)
            && before.and_then(|b| self.org_signature(b)).is_some_and(|(old, _)| there.as_ref() == Some(&old));
        let mut changed = false;
        if due || refill {
            changed |= c.keep_signature(there.as_ref().map(|s| s.name.as_str()), sig.clone()).is_ok();
            c.org_signature = made;
        }
        // Locked, it's the one for new messages and replies, as the organisation says; else it
        // becomes so as it's put in, where the account has none (the person may choose none).
        for (wanted, slot) in [(rule.new_messages, &mut c.signature_new), (rule.replies, &mut c.signature_reply)] {
            if wanted && (rule.locked || due && slot.is_none()) && !slot.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&rule.name)) {
                *slot = Some(rule.name.clone());
                changed = true;
            }
        }
        changed
    }

    /// Each account's organisation signature, as the profile was set or details changed.
    pub(crate) fn apply_org_signatures(&mut self) {
        let mut changed = false;
        let mut configs: Vec<AccountConfig> = self.accounts.iter().map(|a| a.config.clone()).collect();
        for c in &mut configs {
            if self.impose_org_signature(c, None) {
                changed = true;
                if let Some(a) = self.accounts.iter_mut().find(|a| a.config.id == c.id) {
                    a.config = c.clone();
                }
            }
        }
        if changed {
            self.save_accounts();
        }
    }

    /// Whether signature `name` of account `c` is the organisation's, locked: it isn't changed,
    /// removed, or put aside as the one for new messages or replies.
    pub fn org_signature_locked(&self, c: &AccountConfig, name: &str) -> bool {
        self.organisation.signature_for(&c.email).is_some_and(|r| r.locked && r.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Account `account`'s details for its organisation's signature (`account.details`): each
    /// of [`DETAILS`] given, set (empty: taken out); the signature filled in again.
    pub fn set_details(&mut self, account: &str, v: &Value) -> CmdResult {
        let mut c = self.config_of(account)?;
        let before = c.clone();
        for key in DETAILS {
            match v.get(key) {
                None => {}
                Some(Value::String(s)) if s.trim().is_empty() => {
                    c.details.remove(key);
                }
                Some(Value::String(s)) => {
                    let s: String = s.trim().chars().filter(|ch| !ch.is_control()).take(DETAIL_MAX).collect();
                    c.details.insert(key.into(), s);
                }
                Some(_) => return Err(CmdError::Params(format!("`{key}` is text"))),
            }
        }
        self.impose_org_signature(&mut c, Some(&before));
        let details = c.details.clone();
        self.update_account(c, None)?;
        Ok(json!({"details": details}))
    }

    /// What an organisation's directory says of the person themselves, into their details
    /// where they gave none (Exchange, Microsoft 365: name, title, department).
    pub(crate) fn details_from_directory(&mut self, account: &str, people: &[DirectoryPerson]) {
        let Some(mut c) = self.account(account).map(|a| a.config.clone()) else { return };
        let before = c.clone();
        if self.organisation.signature_for(&c.email).is_none() {
            return;
        }
        let Some(me) = people.iter().find(|p| p.email.eq_ignore_ascii_case(&c.email)) else { return };
        let mut changed = false;
        for (key, v) in [("name", &me.name), ("title", &me.title), ("department", &me.department)] {
            if !v.trim().is_empty() && !c.details.contains_key(key) {
                c.details.insert(key.into(), v.trim().chars().take(DETAIL_MAX).collect());
                changed = true;
            }
        }
        if changed {
            self.impose_org_signature(&mut c, Some(&before));
            let _ = self.update_account(c, None);
        }
    }

    /// The organisation's directory asked about the person themselves, for the details of a
    /// signature that names some ([`Session::details_from_directory`] takes the answer).
    /// Each account's own, once it's online: asked before (as it's added, as MailCraft starts),
    /// its directory only answers that it's offline.
    pub(crate) fn ask_own_details(&mut self) {
        let directory = self.directory_accounts();
        let asks: Vec<(String, String)> = self
            .accounts
            .iter()
            .filter(|a| self.organisation.signature_for(&a.config.email).is_some() && !a.config.details.contains_key("title"))
            .filter(|a| matches!(a.status, SyncStatus::Online { .. }) && directory.contains(&a.config.id))
            .map(|a| (a.config.id.clone(), a.config.email.clone()))
            .collect();
        for (account, email) in asks {
            self.ask_directory_once(&account, &email);
        }
    }

    /// The organisation's disclaimer under a message sent from account `account`'s address, as
    /// it's sent: in its text, and in its HTML when it has some.
    pub(crate) fn add_disclaimer(&self, email: &str, d: &mut mime::Draft) {
        let Some(disclaimer) = self.organisation.disclaimer_for(email) else { return };
        if ends_with_disclaimer(&d.body, &disclaimer.text) {
            return;
        }
        if !disclaimer.text.is_empty() {
            d.body = format!("{}\n\n{}", d.body.trim_end(), disclaimer.text);
        }
        if let Some(html) = &mut d.html {
            let part = if disclaimer.html.is_empty() {
                format!("<p>{}</p>", escape(&disclaimer.text).replace('\n', "<br>"))
            } else {
                mailcraft_mime::signature_code(&disclaimer.html)
            };
            match html.rfind("</body>") {
                Some(at) => html.insert_str(at, &part),
                None => html.push_str(&part),
            }
        }
    }

    /// The organisation's disclaimer taken from under message `c` from `email`'s account, as it
    /// opens again to be changed (from the Outbox): it went under it as it was sent, and goes
    /// again then.
    pub(crate) fn strip_disclaimer(&self, email: &str, c: &mut Compose) {
        let Some(disclaimer) = self.organisation.disclaimer_for(email) else { return };
        let text = disclaimer.text.trim();
        if !ends_with_disclaimer(&c.body, text) {
            return;
        }
        let body = c.body.trim_end();
        if let Some(head) = body.get(..body.len().saturating_sub(text.len())) {
            c.body = head.trim_end().to_string();
            c.styles.fit(&c.body);
        }
    }
}

/// Whether `body` ends with `disclaimer` on lines of its own: under it already, not in a reply's
/// quote of a message that had it (`> This message…`).
fn ends_with_disclaimer(body: &str, disclaimer: &str) -> bool {
    let disclaimer = disclaimer.trim();
    let body = body.trim_end();
    !disclaimer.is_empty()
        && body.ends_with(disclaimer)
        && body.get(..body.len().saturating_sub(disclaimer.len())).is_some_and(|head| head.is_empty() || head.ends_with('\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_filled_and_lines_with_nothing_go() {
        let values: BTreeMap<String, String> =
            [("name", "Zoe <Park>"), ("title", "Editor"), ("phone", " ")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let text = fill("{name}\n{title}, {department}\nPhone: {phone}\n{nonsense} and { braces }\nThanks", &values, false);
        assert_eq!(text, "Zoe <Park>\nEditor, \n{nonsense} and { braces }\nThanks");
        assert_eq!(fill("<b>{name}</b>{", &values, true), "<b>Zoe &lt;Park&gt;</b>{");
    }
}
