//! What others see of an event, and how it shows its time (issue #32): its classification
//! (`CLASS`: public, private, or confidential, which servers such as SOGo show as its time only)
//! and its "Show as" (`TRANSP`, and Microsoft's `X-MICROSOFT-CDO-BUSYSTATUS`, which Outlook
//! writes beside it for tentative, out of office and working elsewhere).
//!
//! Both stay among the event's own lines (`extra`) as they were written: an event edited here
//! keeps a classification set elsewhere, word for word, unless the user changes it.

use super::*;

/// Who sees what of an event (`CLASS`, RFC 5545 §3.8.1.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Privacy {
    /// Everyone who may see the calendar sees it (no `CLASS`, or `CLASS:PUBLIC`).
    #[default]
    Public,
    /// Outlook's Private: others see the time taken, not what it is.
    Private,
    /// "Show time only", as SOGo and Thunderbird have it.
    Confidential,
}

impl Privacy {
    pub const ALL: [Privacy; 3] = [Privacy::Public, Privacy::Private, Privacy::Confidential];

    /// Its name in commands: `public`, `private`, `confidential`.
    pub fn code(self) -> &'static str {
        match self {
            Privacy::Public => "public",
            Privacy::Private => "private",
            Privacy::Confidential => "confidential",
        }
    }

    /// From a command's word (Exchange's names too: `normal`, `personal`).
    pub fn from_code(s: &str) -> Option<Privacy> {
        match s.trim().to_ascii_lowercase().as_str() {
            "public" | "normal" => Some(Privacy::Public),
            "private" | "personal" => Some(Privacy::Private),
            "confidential" | "timeonly" | "showtimeonly" => Some(Privacy::Confidential),
            _ => None,
        }
    }
}

/// How an event shows its time to those planning around it (Outlook's Show As).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShowAs {
    #[serde(rename = "free")]
    Free,
    #[serde(rename = "workingElsewhere")]
    WorkingElsewhere,
    #[serde(rename = "tentative")]
    Tentative,
    /// What an event is when it says nothing.
    #[default]
    #[serde(rename = "busy")]
    Busy,
    #[serde(rename = "oof")]
    OutOfOffice,
}

impl ShowAs {
    /// In Outlook's order.
    pub const ALL: [ShowAs; 5] = [ShowAs::Free, ShowAs::WorkingElsewhere, ShowAs::Tentative, ShowAs::Busy, ShowAs::OutOfOffice];

    /// Its name in commands, as Microsoft Graph's `showAs` has it.
    pub fn code(self) -> &'static str {
        match self {
            ShowAs::Free => "free",
            ShowAs::WorkingElsewhere => "workingElsewhere",
            ShowAs::Tentative => "tentative",
            ShowAs::Busy => "busy",
            ShowAs::OutOfOffice => "oof",
        }
    }

    /// From a command's word, Graph's (`oof`) or EWS's (`OOF`, `WorkingElsewhere`) or the
    /// iCalendar one (`WORKINGELSEWHERE`), in any case.
    pub fn from_code(s: &str) -> Option<ShowAs> {
        match s.trim().to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
            "free" => Some(ShowAs::Free),
            "workingelsewhere" => Some(ShowAs::WorkingElsewhere),
            "tentative" => Some(ShowAs::Tentative),
            "busy" => Some(ShowAs::Busy),
            "oof" | "outofoffice" | "away" => Some(ShowAs::OutOfOffice),
            _ => None,
        }
    }

    /// Its `X-MICROSOFT-CDO-BUSYSTATUS` value.
    fn busy_status(self) -> &'static str {
        match self {
            ShowAs::Free => "FREE",
            ShowAs::WorkingElsewhere => "WORKINGELSEWHERE",
            ShowAs::Tentative => "TENTATIVE",
            ShowAs::Busy => "BUSY",
            ShowAs::OutOfOffice => "OOF",
        }
    }
}

const CLASS: &str = "CLASS";
const TRANSP: &str = "TRANSP";
const BUSY_STATUS: &str = "X-MICROSOFT-CDO-BUSYSTATUS";
const SECURE: &str = "X-SOGO-SECURE";

/// The event's own lines (not its alarms'): where each is, and the line.
fn own(e: &CalEvent) -> impl Iterator<Item = (usize, &str)> {
    let mut depth = 0usize;
    e.extra.iter().enumerate().filter_map(move |(i, raw)| {
        let head = raw.trim_start();
        if head.get(..6).is_some_and(|b| b.eq_ignore_ascii_case("BEGIN:")) {
            depth += 1;
            return None;
        }
        if head.get(..4).is_some_and(|b| b.eq_ignore_ascii_case("END:")) {
            depth = depth.saturating_sub(1);
            return None;
        }
        (depth == 0).then_some((i, head))
    })
}

/// Whether a line is named `name`: not one that starts the same (`CLASSIFICATION`).
fn named(head: &str, name: &str) -> bool {
    head.get(..name.len()).is_some_and(|n| n.eq_ignore_ascii_case(name)) && matches!(head.as_bytes().get(name.len()), Some(b':' | b';'))
}

/// A line's value: what follows its first `:` outside double quotes (as [`parse_line`] reads it).
fn value_of(head: &str) -> Option<&str> {
    let mut quoted = false;
    for (i, b) in head.bytes().enumerate() {
        match b {
            b'"' => quoted = !quoted,
            b':' if !quoted => return head.get(i + 1..),
            _ => {}
        }
    }
    None
}

/// The value of the event's first own line named each of `names`, trimmed, from one pass over
/// its kept lines, borrowed. (Each name was looked for alone, every line of it read into a new
/// line and its value upper-cased into another: an event's classification and Show as did that
/// four times for every item drawn: most of the time a calendar's items took to gather.)
fn first_own<'a, const N: usize>(e: &'a CalEvent, names: [&str; N]) -> [Option<&'a str>; N] {
    let mut out = [None; N];
    for (_, head) in own(e) {
        for (name, value) in names.iter().zip(out.iter_mut()) {
            if value.is_none() && named(head, name) {
                *value = value_of(head).map(str::trim);
            }
        }
        if out.iter().all(Option::is_some) {
            break;
        }
    }
    out
}

/// Whether a value is `word`, in any case.
fn is(value: Option<&str>, word: &str) -> bool {
    value.is_some_and(|v| v.eq_ignore_ascii_case(word))
}

/// Take the event's own lines named any of `names` out, and put `add` where its own lines are
/// (before its first alarm).
fn replace(e: &mut CalEvent, names: &[&str], add: Vec<String>) {
    let gone: Vec<usize> = own(e).filter(|(_, head)| names.iter().any(|n| named(head, n) && value_of(head).is_some())).map(|(i, _)| i).collect();
    for i in gone.into_iter().rev() {
        if i < e.extra.len() {
            e.extra.remove(i);
        }
    }
    let at = e.extra.iter().position(|l| l.trim_start().get(..6).is_some_and(|b| b.eq_ignore_ascii_case("BEGIN:"))).unwrap_or(e.extra.len());
    for (k, line) in add.into_iter().enumerate() {
        e.extra.insert(at + k, line);
    }
}

/// Whether the server gave only the event's time, its details hidden from this user: SOGo's
/// `X-SOGO-SECURE:YES` on a private or confidential event of someone else's calendar that the
/// user may see the date and time of only. Such a copy has SOGo's own words for a title
/// (`(Private event)`, in the owner's language), no `CLASS`, and a uid made up for it.
pub fn details_hidden(e: &CalEvent) -> bool {
    let [secure] = first_own(e, [SECURE]);
    is(secure, "YES")
}

/// An event's classification. One a calendar doesn't know (an `x-name`) counts as private, as
/// RFC 5545 asks; one whose details the server hid is private, though it says no `CLASS`.
pub fn privacy(e: &CalEvent) -> Privacy {
    let [class, secure] = first_own(e, [CLASS, SECURE]);
    privacy_of(class, secure)
}

/// An event's [`privacy`] and [`show_as`], from one pass over its kept lines: what a calendar
/// asks of every item it draws.
pub fn showing(e: &CalEvent) -> (Privacy, ShowAs) {
    let [class, secure, transp, status] = first_own(e, [CLASS, SECURE, TRANSP, BUSY_STATUS]);
    (privacy_of(class, secure), show_as_of(transp, status))
}

/// [`privacy`] from the event's first `CLASS` and `X-SOGO-SECURE`.
fn privacy_of(class: Option<&str>, secure: Option<&str>) -> Privacy {
    match class {
        None if is(secure, "YES") => Privacy::Private,
        None => Privacy::Public,
        Some(c) if c.is_empty() || c.eq_ignore_ascii_case("PUBLIC") => Privacy::Public,
        Some(c) if c.eq_ignore_ascii_case("CONFIDENTIAL") => Privacy::Confidential,
        Some(_) => Privacy::Private,
    }
}

/// Classify an event. Nothing changes when it already is so (a `CLASS` written elsewhere,
/// even one MailCraft doesn't know, stays as it was); public leaves `CLASS` out, as it's the
/// default.
pub fn set_privacy(e: &mut CalEvent, p: Privacy) {
    if privacy(e) == p {
        return;
    }
    let add = match p {
        Privacy::Public => Vec::new(),
        Privacy::Private => vec!["CLASS:PRIVATE".to_string()],
        Privacy::Confidential => vec!["CLASS:CONFIDENTIAL".to_string()],
    };
    replace(e, &[CLASS], add);
}

/// How an event shows its time: Microsoft's `X-MICROSOFT-CDO-BUSYSTATUS` when it has one (only
/// it tells tentative, out of office and working elsewhere apart), else `TRANSP` (transparent:
/// free; opaque or none: busy). When the two disagree about free and busy, `TRANSP` counts: it's
/// the standard one, and a calendar that changes only it (Thunderbird's Show Time As) leaves
/// the other as it was.
pub fn show_as(e: &CalEvent) -> ShowAs {
    let [transp, status] = first_own(e, [TRANSP, BUSY_STATUS]);
    show_as_of(transp, status)
}

/// [`show_as`] from the event's first `TRANSP` and busy status.
fn show_as_of(transp: Option<&str>, status: Option<&str>) -> ShowAs {
    const STATUSES: [(&str, ShowAs); 5] = [
        ("FREE", ShowAs::Free),
        ("WORKINGELSEWHERE", ShowAs::WorkingElsewhere),
        ("TENTATIVE", ShowAs::Tentative),
        ("BUSY", ShowAs::Busy),
        ("OOF", ShowAs::OutOfOffice),
    ];
    let status = status.and_then(|v| STATUSES.iter().find(|(w, _)| v.eq_ignore_ascii_case(w)).map(|(_, s)| *s));
    let (opaque, transparent) = (is(transp, "OPAQUE"), is(transp, "TRANSPARENT"));
    match status {
        Some(ShowAs::Free) if opaque => ShowAs::Busy,
        Some(s) if transparent && s != ShowAs::Free => ShowAs::Free,
        Some(s) => s,
        None if transparent => ShowAs::Free,
        None => ShowAs::Busy,
    }
}

/// Set how an event shows its time: `TRANSP` (free is transparent, the rest opaque) and
/// `X-MICROSOFT-CDO-BUSYSTATUS`, as Outlook writes them. Nothing changes when it already shows so.
pub fn set_show_as(e: &mut CalEvent, s: ShowAs) {
    if show_as(e) == s {
        return;
    }
    let transp = if s == ShowAs::Free { "TRANSPARENT" } else { "OPAQUE" };
    replace(e, &[TRANSP, BUSY_STATUS], vec![format!("TRANSP:{transp}"), format!("{BUSY_STATUS}:{}", s.busy_status())]);
}

/// The event without how it shows its time ([`set_show_as`]'s lines): what's the user's own.
pub(super) fn drop_show_as(e: &mut CalEvent) {
    replace(e, &[TRANSP, BUSY_STATUS], Vec::new());
}

/// What an invitation's organizer meant it to show as once it's accepted: Outlook sends the
/// guest's copy as tentative (`X-MICROSOFT-CDO-BUSYSTATUS:TENTATIVE`) and says this beside it
/// (`X-MICROSOFT-CDO-INTENDEDSTATUS`, usually busy).
pub fn intended_show_as(e: &CalEvent) -> Option<ShowAs> {
    let [intended] = first_own(e, [INTENDED_STATUS]);
    intended.and_then(ShowAs::from_code)
}

const INTENDED_STATUS: &str = "X-MICROSOFT-CDO-INTENDEDSTATUS";

/// The user answered an invitation (`ACCEPTED`, `TENTATIVE`…): its time shows as that answer
/// says, as Outlook's does. Accepted, it's what the organizer meant (else busy, when it was only
/// tentative); tentatively accepted, tentative (unless it takes no time: free stays free). Other
/// answers leave it.
pub fn show_as_answered(e: &mut CalEvent, partstat: &str) {
    let now = show_as(e);
    let to = match partstat.trim().to_ascii_uppercase().as_str() {
        "ACCEPTED" => match intended_show_as(e) {
            Some(s) => s,
            None if now == ShowAs::Tentative => ShowAs::Busy,
            None => now,
        },
        "TENTATIVE" if now != ShowAs::Free => ShowAs::Tentative,
        // Someone else goes in the user's place (issue #254): the time is theirs again.
        "DELEGATED" => ShowAs::Free,
        _ => now,
    };
    set_show_as(e, to);
}

/// Exchange's Personal (a sensitivity iCalendar has no word for): kept as `CLASS:X-PERSONAL`,
/// which counts as private here, so an item edited here goes back to Exchange still personal.
const PERSONAL: &str = "X-PERSONAL";

/// Whether an event is Exchange's Personal ([`set_personal`]).
pub fn personal(e: &CalEvent) -> bool {
    let [class] = first_own(e, [CLASS]);
    is(class, PERSONAL)
}

/// Mark an event as Exchange's Personal.
pub fn set_personal(e: &mut CalEvent) {
    if !personal(e) {
        replace(e, &[CLASS], vec![format!("{CLASS}:{PERSONAL}")]);
    }
}
