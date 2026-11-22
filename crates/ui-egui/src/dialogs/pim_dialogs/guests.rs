//! A meeting's guests in its form: who they are, with their answers, and whom a guest handed
//! their place to (issue #254).

use super::*;

impl crate::Dialog {
    /// The suggestions of a dialog's boxes of people, made for what's typed in them (before it's
    /// drawn: drawing can't change the session).
    pub(crate) fn refresh_people(&mut self, s: &mut Session, _acts: &mut Vec<Action>) {
        if let crate::Dialog::PassMeeting(f) = self {
            f.people.refresh(s, &f.to);
        }
    }
}

/// The guests as the form lists them: each with their answer, or whom they handed their place to.
pub(super) fn listed(attendees: &[(String, String, String)]) -> String {
    attendees
        .iter()
        .map(|(who, answer, to)| match answer.as_str() {
            "DELEGATED" if !to.is_empty() => tf!("{name} (delegated to {delegate})", name = who, delegate = to),
            _ => format!("{who} ({})", tl!(partstat_label(answer))),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
