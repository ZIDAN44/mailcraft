//! The EWS calendar, contacts and notes mapping, both ways.

use super::*;
use crate::ews::parse;

#[test]
fn recurrences_both_ways() {
    let start = mailcraft_jmap::utc_date("2026-10-05T09:00:00Z").unwrap_or(0);
    for rule in [
        "FREQ=WEEKLY;BYDAY=MO,WE;INTERVAL=2;UNTIL=20261231T235959Z",
        "FREQ=MONTHLY;BYDAY=-1FR;COUNT=6",
        "FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=14",
        "FREQ=DAILY",
    ] {
        let xml = recurrence_xml(rule, start).unwrap();
        let el = parse(&xml);
        let back = el.find("Recurrence").and_then(recurrence_json).as_ref().and_then(rrule_of).unwrap();
        assert_eq!(back, rule, "{xml}");
    }
    assert!(recurrence_xml("FREQ=HOURLY", start).is_none());
    let junk = parse("<t:Recurrence><t:WeeklyRecurrence><t:Interval>x</t:Interval></t:WeeklyRecurrence></t:Recurrence>");
    assert!(junk.find("Recurrence").and_then(recurrence_json).is_some(), "a bad number falls back");
}

#[test]
fn contacts_both_ways() {
    let card = Card {
        uid: "x".into(),
        name: "Ana Lima".into(),
        given: "Ana".into(),
        family: "Lima".into(),
        emails: vec![Typed { value: "ana@contoso.example".into(), kind: String::new() }],
        // Issue #1: each number keeps what it is, two of a kind too.
        phones: vec![
            Typed { value: "+1 555 0101".into(), kind: "work".into() },
            Typed { value: "+1 555 0102".into(), kind: "work".into() },
            Typed { value: "+1 555 0103".into(), kind: "home".into() },
            Typed { value: "+1 555 0100".into(), kind: "cell".into() },
            Typed { value: "+1 555 0104".into(), kind: "fax".into() },
        ],
        org: "Contoso & Co".into(),
        title: "Producer".into(),
        note: "Met at the fair".into(),
        addresses: vec![
            PostalAddress {
                kind: "work".into(),
                street: "An street 1".into(),
                city: "Brussel".into(),
                postcode: "1000".into(),
                country: "Belgium".into(),
                ..Default::default()
            },
            PostalAddress { kind: "home".into(), street: "Rue Haute 9".into(), city: "Gent".into(), ..Default::default() },
        ],
        birthday: "1990-07-12".into(),
        extra: Vec::new(),
    };
    let xml = contact_xml(&card).replace("<t:Contact>", "<t:Contact><t:ItemId Id=\"C1\" ChangeKey=\"k\"/>");
    // In the schema's order: addresses between emails and phones, the birthday after phones.
    let at = |s: &str| xml.find(s).unwrap();
    assert!(at("<t:EmailAddresses>") < at("<t:PhysicalAddresses>") && at("<t:PhysicalAddresses>") < at("<t:PhoneNumbers>"));
    assert!(at("<t:PhoneNumbers>") < at("<t:Birthday>") && at("<t:Birthday>") < at("<t:JobTitle>"));
    assert!(xml.contains("<t:Entry Key=\"BusinessPhone2\">+1 555 0102</t:Entry>"));
    let back = card_of(parse(&xml).find("Contact").unwrap()).unwrap();
    assert_eq!((back.name.as_str(), back.org.as_str(), back.note.as_str(), back.uid.as_str()), ("Ana Lima", "Contoso & Co", "Met at the fair", "C1"));
    assert_eq!(back.phones, card.phones);
    assert_eq!(back.emails, card.emails);
    assert_eq!(back.addresses, card.addresses);
    assert_eq!(back.birthday, "1990-07-12");
    // Exchange's own: a birthday kept as the start of that day in Brussels, in UTC.
    let theirs = "<t:Contact><t:ItemId Id=\"C2\"/><t:Birthday>1990-07-11T22:00:00Z</t:Birthday></t:Contact>";
    assert_eq!(card_of(parse(theirs).find("Contact").unwrap()).unwrap().birthday, "1990-07-12");
}

/// An appointment's categories come in, and go out between its body and its start (the
/// schema's order); none at all aren't written.
#[test]
fn categories_both_ways() {
    let xml = "<t:CalendarItem><t:ItemId Id=\"E1\"/><t:Subject>Pitch</t:Subject><t:Categories><t:String>Clients</t:String><t:String> </t:String><t:String>Red category</t:String></t:Categories>\
               <t:Start>2026-10-16T10:00:00Z</t:Start><t:End>2026-10-16T11:00:00Z</t:End></t:CalendarItem>";
    let e = event_of(parse(xml).find("CalendarItem").unwrap()).unwrap();
    assert_eq!(e.categories, ["Clients", "Red category"]);
    let names: Vec<&str> = event_fields(&e).iter().map(|(n, _)| *n).collect();
    assert_eq!(names.get(..6), Some(&["Subject", "Sensitivity", "Body", "Categories", "ReminderIsSet", "Start"][..]));
    assert!(event_fields(&e).iter().any(|(_, x)| x.contains("<t:String>Red category</t:String>")));
    let plain = CalEvent { categories: Vec::new(), ..e };
    assert!(event_fields(&plain).iter().all(|(n, _)| *n != "Categories"));
}

/// Issue #32: Private and Show as come in as Exchange keeps them (Sensitivity, and the
/// LegacyFreeBusyStatus Outlook's Show As sets), and go out in the schema's order: Sensitivity
/// after the subject, the status between the all-day flag and the location.
#[test]
fn private_and_show_as_both_ways() {
    use mailcraft_model::ical::{Privacy, ShowAs};
    let item = |sensitivity: &str, status: &str| {
        let xml = format!(
            "<t:CalendarItem><t:ItemId Id=\"E1\"/><t:Subject>Doctor</t:Subject><t:Sensitivity>{sensitivity}</t:Sensitivity>             <t:Start>2026-10-16T10:00:00Z</t:Start><t:End>2026-10-16T11:00:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent>             <t:LegacyFreeBusyStatus>{status}</t:LegacyFreeBusyStatus></t:CalendarItem>"
        );
        event_of(parse(&xml).find("CalendarItem").unwrap()).unwrap()
    };
    for (sens, want) in
        [("Normal", Privacy::Public), ("Personal", Privacy::Private), ("Private", Privacy::Private), ("Confidential", Privacy::Confidential)]
    {
        assert_eq!(privacy(&item(sens, "Busy")), want, "{sens}");
    }
    for (status, want) in [
        ("Free", ShowAs::Free),
        ("Tentative", ShowAs::Tentative),
        ("Busy", ShowAs::Busy),
        ("OOF", ShowAs::OutOfOffice),
        ("WorkingElsewhere", ShowAs::WorkingElsewhere),
        ("NoData", ShowAs::Busy),
        ("junk", ShowAs::Busy),
    ] {
        let e = item("Normal", status);
        assert_eq!(show_as(&e), want, "{status}");
        let fields = event_fields(&e);
        let back = fields.iter().find(|(n, _)| *n == "LegacyFreeBusyStatus").map(|(_, x)| x.clone()).unwrap();
        let written = match want {
            ShowAs::Free => "Free",
            ShowAs::WorkingElsewhere => "WorkingElsewhere",
            ShowAs::Tentative => "Tentative",
            ShowAs::Busy => "Busy",
            ShowAs::OutOfOffice => "OOF",
        };
        assert_eq!(back, format!("<t:LegacyFreeBusyStatus>{written}</t:LegacyFreeBusyStatus>"));
    }
    let e = item("Confidential", "OOF");
    let fields = event_fields(&e);
    let names: Vec<&str> = fields.iter().map(|(n, _)| *n).collect();
    let at = |n: &str| names.iter().position(|x| *x == n).unwrap();
    assert!(at("Subject") < at("Sensitivity") && at("Sensitivity") < at("Body"));
    assert!(at("IsAllDayEvent") < at("LegacyFreeBusyStatus") && at("LegacyFreeBusyStatus") < at("Location"));
    assert!(fields.iter().any(|(_, x)| x == "<t:Sensitivity>Confidential</t:Sensitivity>"));
    // A plain event is Normal and Busy, said outright (an update sets every field).
    let plain = CalEvent { uid: "p".into(), start: 0, end: 3600, ..Default::default() };
    let fields = event_fields(&plain);
    assert!(fields.iter().any(|(_, x)| x == "<t:Sensitivity>Normal</t:Sensitivity>"));
    assert!(fields.iter().any(|(_, x)| x == "<t:LegacyFreeBusyStatus>Busy</t:LegacyFreeBusyStatus>"));
}
