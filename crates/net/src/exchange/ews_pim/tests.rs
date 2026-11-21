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
        ..Default::default()
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

/// Issue #144: a meeting's answer is the mailbox owner's, as the appointment's `MyResponseType`
/// says it, not the one their place among the guests kept from the invitation (an accepted
/// weekly meeting showed as tentative); the other guests' answers stay as they are, and one
/// with no answer of the owner's (not answered yet, or the organizer's own) is left as it is.
#[test]
fn a_meetings_answer_is_the_owners_own() {
    let item = |mine: &str| {
        let xml = format!(
            "<t:CalendarItem><t:ItemId Id=\"E1\"/><t:Subject>Weekly review</t:Subject>\
             <t:Start>2026-10-16T10:00:00Z</t:Start><t:End>2026-10-16T11:00:00Z</t:End>\
             <t:CalendarItemType>RecurringMaster</t:CalendarItemType><t:MyResponseType>{mine}</t:MyResponseType>\
             <t:RequiredAttendees>\
             <t:Attendee><t:Mailbox><t:Name>Riley</t:Name><t:EmailAddress>Riley@Contoso.example</t:EmailAddress></t:Mailbox><t:ResponseType>Tentative</t:ResponseType></t:Attendee>\
             <t:Attendee><t:Mailbox><t:Name>Ana</t:Name><t:EmailAddress>ana@contoso.example</t:EmailAddress></t:Mailbox><t:ResponseType>Decline</t:ResponseType></t:Attendee>\
             </t:RequiredAttendees></t:CalendarItem>"
        );
        let e = stored_events(vec![parse(&xml).find("CalendarItem").unwrap().clone()], "cal", &["riley@contoso.example"]).next().unwrap().event;
        e.attendees.iter().map(|a| a.partstat.clone()).collect::<Vec<_>>()
    };
    assert_eq!(item("Accept"), ["ACCEPTED", "DECLINED"]);
    assert_eq!(item("Decline"), ["DECLINED", "DECLINED"]);
    assert_eq!(item("Tentative"), ["TENTATIVE", "DECLINED"]);
    for none in ["NoResponseReceived", "Unknown", "Organizer"] {
        assert_eq!(item(none), ["TENTATIVE", "DECLINED"], "{none}");
    }
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
    // Personal (iCalendar has no word for it) is private here, and goes back personal through
    // an edit; made confidential here, it's that.
    let mut mine = item("Personal", "Busy");
    mine.summary = "Doctor (moved)".into();
    mailcraft_model::ical::set_privacy(&mut mine, Privacy::Private);
    assert!(event_fields(&mine).iter().any(|(_, x)| x == "<t:Sensitivity>Personal</t:Sensitivity>"));
    mailcraft_model::ical::set_privacy(&mut mine, Privacy::Confidential);
    assert!(event_fields(&mine).iter().any(|(_, x)| x == "<t:Sensitivity>Confidential</t:Sensitivity>"));
}

/// Issue #204: a series' occurrences taken out come as its exceptions (`EXDATE`), and a changed
/// one (moved, its own subject) as the occurrence its series' start names (`RECURRENCE-ID`),
/// with no rule of its own: the series stayed at its usual time on both days, as if nothing had
/// changed. A changed one with no start it stands for is left out, as before.
#[test]
fn a_series_changed_and_cancelled_occurrences() {
    let t = |s: &str| mailcraft_jmap::utc_date(s).unwrap();
    let master = parse(
        "<t:CalendarItem><t:ItemId Id=\"E1\"/><t:Subject>Weekly review</t:Subject><t:UID>weekly-1</t:UID>\
         <t:Start>2026-10-05T09:00:00Z</t:Start><t:End>2026-10-05T10:00:00Z</t:End>\
         <t:CalendarItemType>RecurringMaster</t:CalendarItemType>\
         <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
         <t:NoEndRecurrence><t:StartDate>2026-10-05Z</t:StartDate></t:NoEndRecurrence></t:Recurrence>\
         <t:ModifiedOccurrences><t:Occurrence><t:ItemId Id=\"X1\"/><t:Start>2026-10-13T14:00:00Z</t:Start>\
         <t:End>2026-10-13T15:00:00Z</t:End><t:OriginalStart>2026-10-12T09:00:00Z</t:OriginalStart></t:Occurrence></t:ModifiedOccurrences>\
         <t:DeletedOccurrences><t:DeletedOccurrence><t:Start>2026-10-19T09:00:00Z</t:Start></t:DeletedOccurrence></t:DeletedOccurrences>\
         </t:CalendarItem>",
    );
    let e = event_of(master.find("CalendarItem").unwrap()).unwrap();
    assert_eq!(e.exdates, [t("2026-10-19T09:00:00Z")]);
    assert!(e.rrule.is_some() && e.recurrence_id.is_none());

    let moved = |original: &str| {
        parse(&format!(
            "<t:CalendarItem><t:ItemId Id=\"X1\"/><t:Subject>Weekly review (moved)</t:Subject><t:UID>weekly-1</t:UID>\
             <t:Start>2026-10-13T14:00:00Z</t:Start><t:End>2026-10-13T15:00:00Z</t:End>\
             <t:CalendarItemType>Exception</t:CalendarItemType>{original}\
             <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
             <t:NoEndRecurrence><t:StartDate>2026-10-05Z</t:StartDate></t:NoEndRecurrence></t:Recurrence></t:CalendarItem>"
        ))
    };
    let x = moved("<t:OriginalStart>2026-10-12T09:00:00Z</t:OriginalStart>");
    let x = event_of(x.find("CalendarItem").unwrap()).unwrap();
    assert_eq!((x.recurrence_id, x.rrule.as_deref(), x.start), (Some(t("2026-10-12T09:00:00Z")), None, t("2026-10-13T14:00:00Z")));
    assert!(event_of(moved("").find("CalendarItem").unwrap()).is_none(), "no start it stands for");

    // Drawn: the 12th's is the moved one on the 13th, the 19th's is gone, the 26th's is there.
    let seen: Vec<i64> =
        mailcraft_model::pim::ical::occurrences(&[e, x], t("2026-10-05T00:00:00Z"), t("2026-11-01T00:00:00Z")).iter().map(|o| o.start).collect();
    assert_eq!(seen, [t("2026-10-05T09:00:00Z"), t("2026-10-13T14:00:00Z"), t("2026-10-26T09:00:00Z")]);
}

/// Issue #204: a changed occurrence edited here isn't asked to lose a rule it hasn't; an
/// appointment that stopped repeating is, as before.
#[test]
fn a_changed_occurrence_keeps_its_series() {
    let mut e = CalEvent { summary: "Weekly review".into(), start: 1_791_900_000, end: 1_791_903_600, ..Default::default() };
    let rule = "DeleteItemField><t:FieldURI FieldURI=\"calendar:Recurrence\"";
    assert!(pushing::event_updates(&e, &event_fields(&e)).contains(rule), "a single appointment");
    e.recurrence_id = Some(1_791_795_600);
    let updates = pushing::event_updates(&e, &event_fields(&e));
    assert!(!updates.contains(rule) && updates.contains("calendar:Start"), "{updates}");
}
