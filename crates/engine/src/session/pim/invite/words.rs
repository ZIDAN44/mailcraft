//! The words of the meeting mail MailCraft writes (issue #235): invitations and their updates,
//! cancellations, answers, a meeting forwarded or handed on, a new time proposed, and the online
//! meeting's block in the notes, in the language the window is in (the window gives them; English
//! here, for the CLI and tests). Each is a template with `{name}`s filled in.

use mailcraft_model::ical::MeetingLabels;

/// What meeting mail says, by template.
#[derive(Clone, Debug)]
pub struct MailWords {
    pub invitation: String,
    pub updated: String,
    pub cancelled: String,
    pub cancelled_on: String,
    pub cancelled_body: String,
    pub cancelled_on_body: String,
    pub when: String,
    pub place: String,
    pub forward: String,
    pub forwarded_body: String,
    pub delegated_body: String,
    pub accepted: String,
    pub tentative: String,
    pub declined: String,
    pub delegated: String,
    pub accepted_body: String,
    pub tentative_body: String,
    pub declined_body: String,
    pub delegated_answer_body: String,
    pub proposed: String,
    pub proposed_body: String,
    pub proposal_declined: String,
    pub proposal_declined_body: String,
    /// The online meeting's block added to the notes.
    pub meeting: MeetingLabels,
    /// A date as MailCraft writes it (`Tue 2026-11-24 17:00`), in the window's language.
    pub date: fn(&str) -> String,
    /// The five categories a new profile starts with, by name (`organize::default_categories`).
    pub categories: [String; 5],
}

fn same(s: &str) -> String {
    s.to_string()
}

impl Default for MailWords {
    fn default() -> Self {
        let s = |x: &str| x.to_string();
        MailWords {
            invitation: s("Invitation: {summary}"),
            updated: s("Updated: {summary}"),
            cancelled: s("Cancelled: {summary}"),
            cancelled_on: s("Cancelled: {summary} ({when})"),
            cancelled_body: s("{summary} has been cancelled."),
            cancelled_on_body: s("{summary} on {when} has been cancelled."),
            when: s("When: {when}"),
            place: s("Where: {place}"),
            forward: s("Fwd: {summary}"),
            forwarded_body: s("{who} forwarded you this meeting, organized by {organizer}."),
            delegated_body: s("{who} asked you to go to this meeting in their place. It's organized by {organizer}."),
            accepted: s("Accepted: {summary}"),
            tentative: s("Tentative: {summary}"),
            declined: s("Declined: {summary}"),
            delegated: s("Delegated: {summary}"),
            accepted_body: s("{who} accepted the invitation to {summary}."),
            tentative_body: s("{who} tentatively accepted the invitation to {summary}."),
            declined_body: s("{who} declined the invitation to {summary}."),
            delegated_answer_body: s("{who} asked {delegate} to go to {summary} in their place."),
            proposed: s("New time proposed: {summary}"),
            proposed_body: s("{who} proposes a new time for {summary}: {when}."),
            proposal_declined: s("New time declined: {summary}"),
            proposal_declined_body: s("The proposed new time for {summary} was declined. The meeting stays as it was."),
            meeting: MeetingLabels::default(),
            date: same,
            categories: [s("Important"), s("Work"), s("Personal"), s("To do"), s("Later")],
        }
    }
}

impl MailWords {
    /// A template with its `{name}`s filled in (a value's own braces are left as they are).
    pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
        let mut out = String::with_capacity(template.len() + 64);
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(rest.get(..open).unwrap_or_default());
            let after = rest.get(open..).unwrap_or_default();
            match after.find('}').and_then(|close| Some((close, after.get(1..close)?))) {
                Some((close, name)) => match values.iter().find(|(k, _)| *k == name) {
                    Some((_, v)) => {
                        out.push_str(v);
                        rest = after.get(close + 1..).unwrap_or_default();
                    }
                    None => {
                        out.push('{');
                        rest = after.get(1..).unwrap_or_default();
                    }
                },
                None => {
                    out.push_str(after);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// A time as meeting mail writes it: MailCraft's way, in the window's language.
    pub fn time(&self, t: i64, clock: &mailcraft_model::dates::Clock) -> String {
        (self.date)(&mailcraft_model::dates::full_time(t, clock))
    }

    /// The `When:` line, and the `Where:` one when there's a place.
    pub fn when_where(&self, when: &str, place: &str) -> String {
        let mut out = MailWords::fill(&self.when, &[("when", when)]);
        if !place.is_empty() {
            out.push('\n');
            out.push_str(&MailWords::fill(&self.place, &[("place", place)]));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_filled_and_nothing_else() {
        assert_eq!(MailWords::fill("Invitation: {summary}", &[("summary", "Mix {v2}")]), "Invitation: Mix {v2}");
        assert_eq!(MailWords::fill("{who} asked {delegate}", &[("who", "Ann"), ("delegate", "Bo")]), "Ann asked Bo");
        assert_eq!(MailWords::fill("{unknown} {", &[("who", "x")]), "{unknown} {");
        assert_eq!(MailWords::fill("", &[]), "");
        let w = MailWords::default();
        assert_eq!(w.when_where("Tue 2026-11-24 17:00", ""), "When: Tue 2026-11-24 17:00");
        assert_eq!(w.when_where("x", "Room 2"), "When: x\nWhere: Room 2");
    }
}
