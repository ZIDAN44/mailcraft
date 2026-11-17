//! Project a contact's standard fields for the Android system contacts editor.

use crate::Action;

pub(super) fn add_contact_action(card: &mailcraft_engine::model::vcard::Card) -> Action {
    // Keep photos and unrelated raw vCard properties out of an Android intent.
    let contact = mailcraft_engine::model::vcard::Card {
        name: card.name.clone(),
        given: card.given.clone(),
        family: card.family.clone(),
        emails: card.emails.clone(),
        phones: card.phones.clone(),
        org: card.org.clone(),
        title: card.title.clone(),
        note: card.note.clone(),
        addresses: card.addresses.clone(),
        birthday: card.birthday.clone(),
        extra: card
            .extra
            .iter()
            .filter(|line| {
                contact_property_name(line)
                    .is_some_and(|name| ["URL", "NICKNAME", "ANNIVERSARY"].iter().any(|supported| name.eq_ignore_ascii_case(supported)))
            })
            .cloned()
            .collect(),
        ..Default::default()
    };
    Action::AddToSystemContacts { contact }
}

fn contact_property_name(line: &str) -> Option<&str> {
    let mut quoted = false;
    let colon = line
        .char_indices()
        .find(|(_, character)| {
            if *character == '"' {
                quoted = !quoted;
            }
            *character == ':' && !quoted
        })
        .map(|(index, _)| index)?;
    line.get(..colon)?.split(';').next()?.rsplit('.').next()
}
