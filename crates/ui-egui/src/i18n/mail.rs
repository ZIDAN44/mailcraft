//! What the meeting mail MailCraft writes says, in the window's language (issue #235: invitations,
//! updates, cancellations, answers and forwards went in English, dates too): the engine's
//! templates, translated here, with the online meeting's block and the default categories.

use mailcraft_engine::MailWords;

/// The words for the current language.
pub fn mail_words() -> MailWords {
    MailWords {
        invitation: tl!("Invitation: {summary}").into(),
        updated: tl!("Updated: {summary}").into(),
        cancelled: tl!("Cancelled: {summary}").into(),
        cancelled_on: tl!("Cancelled: {summary} ({when})").into(),
        cancelled_body: tl!("{summary} has been cancelled.").into(),
        cancelled_on_body: tl!("{summary} on {when} has been cancelled.").into(),
        when: tl!("When: {when}").into(),
        place: tl!("Where: {place}").into(),
        forward: tl!("Fwd: {summary}").into(),
        forwarded_body: tl!("{who} forwarded you this meeting, organized by {organizer}.").into(),
        delegated_body: tl!("{who} asked you to go to this meeting in their place. It's organized by {organizer}.").into(),
        accepted: tl!("Accepted: {summary}").into(),
        tentative: tl!("Tentative: {summary}").into(),
        declined: tl!("Declined: {summary}").into(),
        delegated: tl!("Delegated: {summary}").into(),
        accepted_body: tl!("{who} accepted the invitation to {summary}.").into(),
        tentative_body: tl!("{who} tentatively accepted the invitation to {summary}.").into(),
        declined_body: tl!("{who} declined the invitation to {summary}.").into(),
        delegated_answer_body: tl!("{who} asked {delegate} to go to {summary} in their place.").into(),
        proposed: tl!("New time proposed: {summary}").into(),
        proposed_body: tl!("{who} proposes a new time for {summary}: {when}.").into(),
        proposal_declined: tl!("New time declined: {summary}").into(),
        proposal_declined_body: tl!("The proposed new time for {summary} was declined. The meeting stays as it was.").into(),
        meeting: crate::pim_dialogs::meeting_labels(),
        date: super::date,
        categories: [tl!("Important"), tl!("Work"), tl!("Personal"), tl!("To do"), tl!("Later")].map(String::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, set_current};

    /// Every template keeps the names it's filled with, in every language (a translation that
    /// lost `{summary}` would send a subject without the meeting's name).
    #[test]
    fn every_language_keeps_the_names_filled_in() {
        let english = MailWords::default();
        let pairs = |w: &MailWords| {
            vec![
                w.invitation.clone(),
                w.updated.clone(),
                w.cancelled.clone(),
                w.cancelled_on.clone(),
                w.cancelled_body.clone(),
                w.cancelled_on_body.clone(),
                w.when.clone(),
                w.place.clone(),
                w.forward.clone(),
                w.forwarded_body.clone(),
                w.delegated_body.clone(),
                w.accepted.clone(),
                w.tentative.clone(),
                w.declined.clone(),
                w.delegated.clone(),
                w.accepted_body.clone(),
                w.tentative_body.clone(),
                w.declined_body.clone(),
                w.delegated_answer_body.clone(),
                w.proposed.clone(),
                w.proposed_body.clone(),
                w.proposal_declined.clone(),
                w.proposal_declined_body.clone(),
            ]
        };
        let names = |s: &str| {
            let mut n: Vec<String> = s.split('{').skip(1).filter_map(|x| x.split_once('}').map(|(a, _)| a.to_string())).collect();
            n.sort();
            n
        };
        set_current(Lang::EN);
        let en = mail_words();
        assert_eq!(pairs(&en), pairs(&english), "English as the engine's own");
        for lang in Lang::for_picker() {
            set_current(lang);
            let w = mail_words();
            for (theirs, ours) in pairs(&w).iter().zip(pairs(&english)) {
                assert_eq!(names(theirs), names(&ours), "{}: {theirs}", lang.code());
            }
        }
        set_current(Lang::EN);
    }
}
