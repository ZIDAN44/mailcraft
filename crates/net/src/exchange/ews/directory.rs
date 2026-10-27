//! The organisation's directory (Outlook's Global Address List): asked each time, never kept
//! here, so people added on the server are found as soon as it knows them (issue #9).

use super::*;

/// Most people one search returns (Exchange itself stops at 100).
pub const DIRECTORY_MAX: usize = 100;
/// Longest search sent.
const MAX_QUERY: usize = 256;

impl Client {
    /// Everyone in the directory matching `query` (a name's or an address's start, as Exchange's
    /// ambiguous name resolution matches): (name, address). None found is an empty list, not an
    /// error.
    pub fn search_directory(&self, query: &str) -> Result<Vec<(String, String)>> {
        let q: String = query.trim().chars().filter(|c| !c.is_control()).take(MAX_QUERY).collect();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let doc = self.call(&format!(
            "<m:ResolveNames ReturnFullContactData=\"true\" SearchScope=\"ActiveDirectory\"><m:UnresolvedEntry>{}</m:UnresolvedEntry></m:ResolveNames>",
            esc(&q)
        ))?;
        let Some(m) = messages(&doc).first().copied() else { return Err(NetError::Protocol("an empty answer".into())) };
        // Several found is a warning with the list; none, an error that only says so.
        if m.attr("ResponseClass") == Some("Error") {
            return match m.text_of("ResponseCode") {
                Some("ErrorNameResolutionNoResults") => Ok(Vec::new()),
                _ => check_message(m).map(|()| Vec::new()),
            };
        }
        let mut found = Vec::new();
        m.all("Resolution", &mut found);
        let mut out: Vec<(String, String)> = Vec::new();
        for r in found {
            let Some(b) = r.child("Mailbox") else { continue };
            let address = b.text_of("EmailAddress").map(str::trim).unwrap_or("");
            // An Exchange address (`/o=…`) can't be written to from here.
            if !address.contains('@') || address.starts_with('/') || address.len() > 320 {
                continue;
            }
            if out.iter().any(|(_, a)| a.eq_ignore_ascii_case(address)) {
                continue;
            }
            out.push((directory_name(r), address.to_string()));
            if out.len() >= DIRECTORY_MAX {
                break;
            }
        }
        Ok(out)
    }
}

/// A person's name as the directory keeps it: the contact's display name, or its given name and
/// surname, before the mailbox's name, which some servers fill with the alias (`john.doe`, issue
/// #9). The contact comes with the full data asked for; without one, the mailbox's name.
pub(crate) fn directory_name(resolution: &El) -> String {
    let text = |el: Option<&El>, name: &str| el.and_then(|e| e.text_of(name)).map(str::trim).unwrap_or("").to_string();
    let contact = resolution.child("Contact");
    let mut name = text(contact, "DisplayName");
    if name.is_empty() {
        name = [text(contact, "GivenName"), text(contact, "Surname")].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
    }
    if name.is_empty() {
        name = text(resolution.child("Mailbox"), "Name");
    }
    name.chars().filter(|c| !c.is_control()).take(256).collect()
}
