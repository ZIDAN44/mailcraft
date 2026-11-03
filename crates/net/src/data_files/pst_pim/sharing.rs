//! A sharing invitation (`IPM.Sharing`, MS-OXSHARE): what was shared, read from its sharing
//! properties (PSETID_Sharing), as lines put before its text, so the message says it whatever
//! the text Outlook wrote with it says.

use super::{GuidKey, Named, OLE};
use crate::pst::Val;

const PSETID_SHARING: GuidKey = (0x0006_2040, 0, 0, OLE);
/// PidLidSharingRemoteName, PidLidSharingRemoteType, PidLidSharingLocalType, PidLidSharingRemotePath
/// (an internet calendar's address), PidLidSharingInitiatorName, PidLidSharingInitiatorSmtp
/// (MS-OXPROPS 2.247–2.270).
const REMOTE_NAME: u32 = 0x8A05;
const REMOTE_TYPE: u32 = 0x8A1D;
const LOCAL_TYPE: u32 = 0x8A14;
const REMOTE_PATH: u32 = 0x8A04;
const INITIATOR_NAME: u32 = 0x8A07;
const INITIATOR_SMTP: u32 = 0x8A08;
/// The longest value a line keeps, in characters.
const MAX_VALUE: usize = 512;

/// The lines saying what a sharing invitation shares (its folder's kind and name, who shares it,
/// an internet calendar's address); empty when its properties say none of it.
pub(crate) fn sharing_lines<'a>(p: &dyn Fn(u16) -> Option<Val<'a>>, named: &Named) -> Vec<String> {
    let text = |lid: u32| {
        let v = named.id(PSETID_SHARING, lid).and_then(p).and_then(Val::into_text).unwrap_or_default();
        let one: String = v.chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_VALUE).collect();
        one.trim().to_string()
    };
    let kind_of = |t: &str| {
        let t = t.to_ascii_lowercase();
        let kinds = [
            ("ipf.appointment", "calendar"),
            ("ipf.contact", "contacts"),
            ("ipf.task", "tasks"),
            ("ipf.stickynote", "notes"),
            ("ipf.journal", "journal"),
            ("ipf.note", "mail folder"),
        ];
        kinds.iter().find(|(c, _)| t.starts_with(c)).map(|(_, k)| *k)
    };
    let kind = [text(REMOTE_TYPE), text(LOCAL_TYPE)].iter().find_map(|t| kind_of(t));
    let name = text(REMOTE_NAME);
    let mut lines = Vec::new();
    match (kind, name.is_empty()) {
        (Some(kind), false) => lines.push(format!("Shared {kind}: {name}")),
        (Some(kind), true) => lines.push(format!("Shared {kind}")),
        (None, false) => lines.push(format!("Shared: {name}")),
        (None, true) => {}
    }
    let (who, address) = (text(INITIATOR_NAME), text(INITIATOR_SMTP));
    let address = if address.contains('@') { address } else { String::new() };
    match (who.is_empty(), address.is_empty()) {
        (false, false) => lines.push(format!("Shared by {who} <{address}>")),
        (false, true) => lines.push(format!("Shared by {who}")),
        (true, false) => lines.push(format!("Shared by {address}")),
        (true, true) => {}
    }
    let path = text(REMOTE_PATH);
    let web = ["http://", "https://", "webcal://", "webcals://"].iter().any(|s| path.get(..s.len()).is_some_and(|h| h.eq_ignore_ascii_case(s)));
    if web && !path.contains(char::is_whitespace) {
        lines.push(format!("Address: {path}"));
    }
    lines
}

/// `lines` put before a message's text (`body`) and its HTML, when it has some: the HTML is what
/// the reading pane shows, and the text what search and the preview read.
pub(crate) fn put_before(lines: &[String], body: &mut String, html: &mut Option<String>) {
    if lines.is_empty() {
        return;
    }
    let said = lines.join("\n");
    // A message with HTML alone has its text made from it: made here, so it keeps the HTML's.
    let own = if body.trim().is_empty() { html.as_deref().map(mailcraft_mime::html_to_text).unwrap_or_default() } else { std::mem::take(body) };
    *body = if own.trim().is_empty() { said } else { format!("{said}\n\n{own}") };
    if let Some(h) = html.as_mut().filter(|h| !h.trim().is_empty()) {
        let escaped: Vec<String> = lines.iter().map(|l| l.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")).collect();
        let block = format!("<p>{}</p>", escaped.join("<br>"));
        // After the body's tag when it has one (a tag in a comment or a value is not looked for:
        // the block then goes first, still shown).
        let lower = h.to_ascii_lowercase();
        let at = lower.find("<body").and_then(|b| lower.get(b..).and_then(|r| r.find('>')).map(|e| b + e + 1)).unwrap_or(0);
        if h.is_char_boundary(at) {
            h.insert_str(at, &block);
        }
    }
}
