//! The profile as named values: what Group Policy writes under `Software\Policies\MailCraft`
//! (each of the ADMX template's policies), and the keys of a macOS configuration profile. The
//! same names on both: `Profile` (the whole profile, as JSON text), `ProfileFile` (where a
//! profile file is), and one value each for what's set most often.

use serde_json::{Value, json};

/// A named value as the system keeps it.
#[derive(Clone, Debug, PartialEq)]
pub enum Named {
    Text(String),
    Number(i64),
    Bool(bool),
}

impl Named {
    fn text(&self) -> Option<&str> {
        match self {
            Named::Text(s) => Some(s.trim()),
            _ => None,
        }
    }

    /// A switch, as a number (`0`, `1`), a boolean, or text (`true`, `1`…).
    fn on(&self) -> Option<bool> {
        match self {
            Named::Number(n) => Some(*n != 0),
            Named::Bool(b) => Some(*b),
            Named::Text(s) => match s.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Some(true),
                "0" | "false" | "no" | "off" => Some(false),
                _ => None,
            },
        }
    }
}

/// The names read, each with what it sets.
pub const NAMED: [(&str, &str); 9] = [
    ("Profile", "the whole profile, as JSON"),
    ("ProfileFile", "the full path of a profile file"),
    ("Organisation", "the organisation's name"),
    ("Contact", "who to ask about a locked setting"),
    ("Updates", "on, stable or off, locked"),
    ("RequireEncryption", "1: no connection without encryption"),
    ("CertificateExceptions", "0: no certificate trusted by hand"),
    ("WebPictures", "0: pictures from the web never load by themselves, locked"),
    ("AIAgents", "off, read or ask: what AI agents may do, locked"),
];

/// What named values set: the profile file's path, the profile's JSON text, and the rest as a
/// profile; and what of them couldn't be read. `values` are (name, value), names compared
/// without regard to case.
pub fn from_named(values: &[(String, Named)], at: &str) -> (Option<String>, Option<String>, Value, Vec<String>) {
    let mut file = None;
    let mut profile = None;
    let mut set = json!({});
    let mut ignored = Vec::new();
    let mut settings = serde_json::Map::new();
    for (name, v) in values {
        let bad = |ignored: &mut Vec<String>| ignored.push(format!("{at}\\{name}: not a value it takes"));
        match name.to_ascii_lowercase().as_str() {
            "profile" => match v.text() {
                Some(t) if !t.is_empty() => profile = Some(t.to_string()),
                _ => bad(&mut ignored),
            },
            "profilefile" => match v.text() {
                Some(t) if !t.is_empty() => file = Some(t.to_string()),
                _ => bad(&mut ignored),
            },
            "organisation" | "organization" => match v.text() {
                Some(t) if !t.is_empty() => set["organisation"] = json!(t),
                _ => bad(&mut ignored),
            },
            "contact" => match v.text() {
                Some(t) if !t.is_empty() => set["contact"] = json!(t),
                _ => bad(&mut ignored),
            },
            "updates" => match v.text().filter(|t| super::super::Updates::parse(t).is_some()) {
                Some(t) => set["updates"] = json!({"value": t, "locked": true}),
                None => bad(&mut ignored),
            },
            "requireencryption" => match v.on() {
                Some(on) => set["requireEncryption"] = json!(on),
                None => bad(&mut ignored),
            },
            "certificateexceptions" => match v.on() {
                Some(on) => set["certificateExceptions"] = json!(on),
                None => bad(&mut ignored),
            },
            "webpictures" => match v.on() {
                Some(false) => {
                    settings.insert("autoPictures".into(), json!({"value": false, "locked": true}));
                }
                Some(true) => {}
                None => bad(&mut ignored),
            },
            "aiagents" => match v.text().map(str::to_ascii_lowercase).as_deref() {
                Some("off") => {
                    settings.insert("agents".into(), json!({"value": false, "locked": true}));
                }
                Some("read") => {
                    settings.insert("agentsRun".into(), json!({"value": false, "locked": true}));
                }
                // Asked before each command: none may be let run without asking.
                Some("ask") => {
                    settings.insert("agentsAlways".into(), json!({"value": [], "locked": true}));
                }
                _ => bad(&mut ignored),
            },
            // The configuration profile's own keys, and others' values under the same key.
            n if n.starts_with("payload") || n == "pfm_version" => {}
            _ => ignored.push(format!("{at}\\{name}: not one MailCraft reads")),
        }
    }
    if !settings.is_empty() {
        set["settings"] = Value::Object(settings);
    }
    (file, profile, set, ignored)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Group Policy template writes only names MailCraft reads, and every one it reads.
    #[test]
    fn the_group_policy_template_names_what_mailcraft_reads() {
        let admx = include_str!("../../../../../packaging/windows/policy/MailCraft.admx");
        let mut written: Vec<&str> = admx.split("valueName=\"").skip(1).filter_map(|r| r.split('"').next()).collect();
        written.sort_unstable();
        written.dedup();
        let mut read: Vec<&str> = NAMED.iter().map(|(n, _)| *n).collect();
        read.sort_unstable();
        assert_eq!(written, read);
    }

    #[test]
    fn named_values_are_the_profile_they_say() {
        let values = vec![
            ("RequireEncryption".to_string(), Named::Number(1)),
            ("Updates".to_string(), Named::Text("off".into())),
            ("WebPictures".to_string(), Named::Number(0)),
            ("AIAgents".to_string(), Named::Text("read".into())),
            ("organization".to_string(), Named::Text(" Contoso ".into())),
            ("ProfileFile".to_string(), Named::Text(r"\\server\share\mail.json".into())),
            ("Updates2".to_string(), Named::Number(1)),
            ("CertificateExceptions".to_string(), Named::Text("maybe".into())),
        ];
        let (file, profile, set, ignored) = from_named(&values, "HKLM");
        assert_eq!(file.as_deref(), Some(r"\\server\share\mail.json"));
        assert_eq!(profile, None);
        assert_eq!(set["requireEncryption"], true);
        assert_eq!(set["updates"], json!({"value": "off", "locked": true}));
        assert_eq!(set["settings"]["autoPictures"], json!({"value": false, "locked": true}));
        assert_eq!(set["settings"]["agentsRun"], json!({"value": false, "locked": true}));
        assert_eq!(set["organisation"], "Contoso");
        assert_eq!(ignored.len(), 2, "{ignored:?}");
        assert!(set.get("certificateExceptions").is_none());
        // Asking before each command: no command let run without asking.
        let (_, _, set, _) = from_named(&[("AIAgents".to_string(), Named::Text("Ask".into()))], "HKLM");
        assert_eq!(set["settings"]["agentsAlways"], json!({"value": [], "locked": true}));
    }
}
