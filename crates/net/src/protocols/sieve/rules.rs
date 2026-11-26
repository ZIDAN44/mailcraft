//! MailCraft's rules run by an IMAP server (issue #243): each written as Sieve (RFC 5228) in
//! MailCraft's own script, beside its automatic replies, so mail is sorted as it arrives even
//! while MailCraft is closed or another device reads it first.
//!
//! A rule says what MailCraft's does, as Sieve can say it: its tests on the message's headers
//! (case-insensitive, as MailCraft's are), its flags and categories with `imap4flags` (RFC 5232),
//! a folder with `fileinto`. What Sieve can't say (a rule for mail the user sends; attachments
//! on a server without the `mime` extension, RFC 5703) is refused, and the rule stays MailCraft's.
//! The rules are kept as MailCraft wrote them in a line of the script ([`KeptRules`]), never read
//! back out of Sieve.

use serde::{Deserialize, Serialize};

use mailcraft_model::FolderRole;
use mailcraft_model::organize::{Condition, Rule, RuleAction};

use super::sieve_str;

/// MailCraft's rules on the server as it keeps them in its script: the rules, in the order they
/// run, and the Sieve they were written as, with the extensions it needs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptRules {
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub require: Vec<String>,
    #[serde(default)]
    pub sieve: String,
}

/// What writing a rule needs to know of the account: its folders by role (for Archive, Delete
/// and the like), its categories' keywords by name, and what its server's Sieve can do.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub roles: Vec<(FolderRole, String)>,
    pub keywords: Vec<(String, String)>,
    pub extensions: Vec<String>,
}

impl Context {
    fn has(&self, ext: &str) -> bool {
        self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }

    fn folder(&self, role: FolderRole) -> Result<&str, String> {
        self.roles.iter().find(|(r, _)| *r == role).map(|(_, p)| p.as_str()).ok_or_else(|| format!("this account has no {role:?} folder"))
    }

    /// A category's keyword: the one it was made with, else the one a new category of that name
    /// gets.
    fn keyword(&self, name: &str) -> String {
        self.keywords
            .iter()
            .find(|(n, _)| n.trim().eq_ignore_ascii_case(name.trim()))
            .map_or_else(|| mailcraft_model::keyword_for(name), |(_, k)| k.clone())
    }
}

/// `rules` written as Sieve, those turned on in their order: the script's part and the
/// extensions it needs; or why one can't run on the server.
pub fn written(rules: &[Rule], cx: &Context) -> Result<KeptRules, String> {
    let mut require: Vec<&str> = Vec::new();
    let mut sieve = String::new();
    for rule in rules {
        // Checked even when off: it may be turned on without being written again.
        let (test, actions, needs) = rule_parts(rule, cx).map_err(|e| format!("{}: {e}", rule.name))?;
        for n in needs {
            if !cx.has(n) {
                return Err(format!("{}: this server's Sieve can't {}", rule.name, what_needs(n)));
            }
            if !require.contains(&n) {
                require.push(n);
            }
        }
        if rule.enabled {
            sieve.push_str(&format!("# {}\r\nif {test} {{\r\n{actions}}}\r\n", one_line(&rule.name)));
        }
    }
    Ok(KeptRules { rules: rules.to_vec(), require: require.into_iter().map(str::to_string).collect(), sieve })
}

/// What an extension lets a rule do, for when the server lacks it.
fn what_needs(ext: &str) -> &'static str {
    match ext {
        "fileinto" => "file mail into a folder (fileinto)",
        "imap4flags" => "mark, flag or categorize mail (imap4flags)",
        "mime" => "tell whether mail has attachments (mime)",
        _ => "run this rule",
    }
}

/// A rule's name in a comment line: on one line.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// A rule's test, its actions (indented lines), and the extensions they need.
fn rule_parts(rule: &Rule, cx: &Context) -> Result<(String, String, Vec<&'static str>), String> {
    if rule.sent {
        return Err("a rule for mail you send runs in MailCraft, not on the server".into());
    }
    if rule.conditions.is_empty() || rule.actions.is_empty() {
        return Err("a rule needs a condition and an action".into());
    }
    let mut needs = Vec::new();
    let tests: Vec<String> = rule.conditions.iter().map(|c| test(c, cx, &mut needs)).collect::<Result<_, _>>()?;
    let test = match tests.as_slice() {
        [one] => one.clone(),
        _ if rule.match_all => format!("allof({})", tests.join(", ")),
        _ => format!("anyof({})", tests.join(", ")),
    };
    // Flags first: the folder (or the Inbox) takes the message with them.
    let mut flags = Vec::new();
    let mut folder: Option<String> = None;
    for a in &rule.actions {
        match a {
            RuleAction::MarkRead => flags.push("\\Seen".to_string()),
            RuleAction::Flag => flags.push("\\Flagged".to_string()),
            RuleAction::Pin => flags.push(mailcraft_model::PINNED.to_string()),
            RuleAction::Categorize(name) => flags.push(cx.keyword(name)),
            RuleAction::MoveTo(path) => {
                folder.get_or_insert_with(|| path.clone());
            }
            RuleAction::MoveToRole(role) => {
                let p = cx.folder(*role)?.to_string();
                folder.get_or_insert(p);
            }
            RuleAction::Delete => {
                let p = cx.folder(FolderRole::Trash)?.to_string();
                folder.get_or_insert(p);
            }
            RuleAction::MoveToAccount { .. } => return Err("a rule on the server can't move mail into another account".into()),
        }
    }
    let mut actions = String::new();
    if !flags.is_empty() {
        needs.push("imap4flags");
        let list: Vec<String> = flags.iter().map(|f| sieve_str(f)).collect();
        actions.push_str(&format!("    addflag [{}];\r\n", list.join(", ")));
    }
    if let Some(path) = folder {
        needs.push("fileinto");
        // A folder's name as Sieve takes it: in UTF-8, where IMAP names it in modified UTF-7.
        actions.push_str(&format!("    fileinto {};\r\n", sieve_str(&mailcraft_model::utf7::decode(&path))));
    }
    if rule.stop {
        actions.push_str("    stop;\r\n");
    }
    Ok((test, actions, needs))
}

/// One condition as a Sieve test. Text is looked for as MailCraft does, anywhere in the header,
/// in any case; none at all matches nothing (MailCraft's empty text matches no mail, Sieve's
/// would match all).
fn test(c: &Condition, cx: &Context, needs: &mut Vec<&'static str>) -> Result<String, String> {
    let contains = |headers: &[&str], t: &str| -> String {
        if t.trim().is_empty() {
            return "false".into();
        }
        let names: Vec<String> = headers.iter().map(|h| sieve_str(h)).collect();
        let names = if names.len() == 1 { names.concat() } else { format!("[{}]", names.join(", ")) };
        format!("header :contains {names} {}", sieve_str(t.trim()))
    };
    Ok(match c {
        Condition::From(t) => contains(&["from"], t),
        Condition::To(t) => contains(&["to", "cc"], t),
        Condition::Subject(t) => contains(&["subject"], t),
        Condition::AnyText(t) => contains(&["from", "to", "cc", "subject"], t),
        Condition::ListId(t) => contains(&["list-id"], t),
        // As MailCraft tells bulk mail (`mime::is_bulk`): a list's headers, a precedence of bulk,
        // list or junk, mail sent automatically, or from a no-reply address.
        Condition::Bulk => "anyof(exists \"list-id\", exists \"list-unsubscribe\", header :is \"precedence\" [\"bulk\", \"list\", \"junk\"], \
             allof(exists \"auto-submitted\", not header :is \"auto-submitted\" \"no\"), \
             address :localpart :is \"from\" [\"noreply\", \"no-reply\", \"donotreply\"])"
            .into(),
        Condition::HasAttachment => {
            if !cx.has("mime") {
                return Err("this server's Sieve can't tell whether mail has attachments (mime)".into());
            }
            needs.push("mime");
            "header :mime :anychild :contains \"content-disposition\" \"attachment\"".into()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx() -> Context {
        Context {
            roles: vec![(FolderRole::Trash, "Trash".into()), (FolderRole::Archive, "Archive".into())],
            keywords: vec![("Important".into(), "$label1".into())],
            extensions: ["fileinto", "imap4flags", "include", "vacation"].map(String::from).to_vec(),
        }
    }

    fn rule(name: &str, conditions: Vec<Condition>, actions: Vec<RuleAction>) -> Rule {
        Rule { name: name.into(), enabled: true, account: None, conditions, match_all: true, actions, stop: false, sent: false }
    }

    #[test]
    fn rules_are_written_as_sieve() {
        let invoices = Rule {
            stop: true,
            ..rule(
                "Invoices",
                vec![Condition::From("billing@shop.example".into()), Condition::Subject("Invoice".into())],
                vec![RuleAction::MoveTo("Bills/&AMk-t&AOk-".into()), RuleAction::MarkRead, RuleAction::Categorize("Important".into())],
            )
        };
        let lists = Rule {
            match_all: false,
            ..rule("Lists", vec![Condition::Bulk, Condition::To("team@".into())], vec![RuleAction::MoveToRole(FolderRole::Archive)])
        };
        let off = Rule { enabled: false, ..rule("Off", vec![Condition::Subject("x".into())], vec![RuleAction::Delete]) };
        let k = written(&[invoices.clone(), lists, off], &cx()).unwrap();
        assert_eq!(k.require, ["imap4flags", "fileinto"]);
        let s = &k.sieve;
        assert!(s.contains("if allof(header :contains \"from\" \"billing@shop.example\", header :contains \"subject\" \"Invoice\") {"), "{s}");
        assert!(s.contains("    addflag [\"\\\\Seen\", \"$label1\"];\r\n    fileinto \"Bills/Été\";\r\n    stop;\r\n}"), "{s}");
        assert!(s.contains("if anyof(anyof(exists \"list-id\""), "{s}");
        // A list by its List-Id (issue #288).
        let debian = rule("Debian", vec![Condition::ListId("debian-devel.lists.debian.org".into())], vec![RuleAction::MarkRead]);
        let listed = written(&[debian], &cx()).unwrap().sieve;
        assert!(listed.contains("if header :contains \"list-id\" \"debian-devel.lists.debian.org\" {"), "{listed}");
        assert!(s.contains("header :contains [\"to\", \"cc\"] \"team@\") {\r\n    fileinto \"Archive\";"), "{s}");
        assert!(!s.contains("Off"), "a rule turned off isn't written: {s}");
        assert_eq!(k.rules.len(), 3, "but kept");
    }

    #[test]
    fn what_sieve_cant_say_is_refused() {
        let sent = Rule { sent: true, ..rule("Sent", vec![Condition::Subject("x".into())], vec![RuleAction::Flag]) };
        assert!(written(&[sent], &cx()).unwrap_err().contains("mail you send"));
        let attached = rule("Files", vec![Condition::HasAttachment], vec![RuleAction::Flag]);
        assert!(written(std::slice::from_ref(&attached), &cx()).unwrap_err().contains("attachments"));
        let mut with_mime = cx();
        with_mime.extensions.push("mime".into());
        let k = written(&[attached], &with_mime).unwrap();
        assert!(k.require.contains(&"mime".to_string()) && k.sieve.contains(":mime :anychild"));
        let mut bare = cx();
        bare.extensions.retain(|e| e != "imap4flags");
        assert!(written(&[rule("Flag", vec![Condition::Subject("x".into())], vec![RuleAction::Flag])], &bare).unwrap_err().contains("imap4flags"));
        let mut no_trash = cx();
        no_trash.roles.clear();
        assert!(written(&[rule("Bin", vec![Condition::Subject("x".into())], vec![RuleAction::Delete])], &no_trash).is_err());
    }

    /// What's typed can't break out of its string or comment: quotes, backslashes and line ends
    /// stay inside; empty text matches nothing.
    #[test]
    fn typed_text_stays_in_its_string() {
        let evil = rule(
            "Name\r\n}\r\ndiscard;\r\n#",
            vec![Condition::Subject("\"; discard; \\".into()), Condition::From("  ".into())],
            vec![RuleAction::Flag],
        );
        let k = written(&[evil], &cx()).unwrap();
        assert!(k.sieve.starts_with("# Name  }  discard;  #\r\n"), "{}", k.sieve);
        assert!(k.sieve.contains(r#"header :contains "subject" "\"; discard; \\""#), "{}", k.sieve);
        assert!(k.sieve.contains("false"), "empty text matches nothing");
        assert_eq!(k.sieve.matches("discard;").count(), 2, "both inside strings or the comment: {}", k.sieve);
    }
}
