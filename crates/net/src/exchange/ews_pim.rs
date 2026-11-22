//! On-premises Exchange calendars and contacts through Exchange Web Services, synced into the
//! same calendars and address books as CalDAV and CardDAV ([`DavSync`], which the engine merges).
//!
//! The mailbox's Calendar and Contacts folders: appointments (with their recurrence, read as the
//! pattern and range Graph also uses, so [`crate::graph_pim::rrule_of`] makes the RRULE) and
//! contacts. What changed here goes first: new appointments and contacts are created, changed
//! appointments updated field by field, changed contacts made again, deletions done (no meeting
//! mail is ever sent); then both folders are read again.

use mailcraft_model::dates::utc_stamp;
use mailcraft_model::ical::{Attendee, CalEvent, Privacy, ShowAs, personal, privacy, set_personal, set_privacy, set_show_as, show_as};
use mailcraft_model::pim::{BookInfo, CalendarInfo, DavSync, Note, Pim, StoredCard, StoredEvent, StoredNote};
use mailcraft_model::vcard::{Card, PostalAddress, Typed};
use serde_json::{Value, json};

use crate::ews::{Client, El, esc};
use crate::graph_pim::{recurrence_of, rrule_of};
use crate::{NetError, Result};

/// The calendar and address book Exchange's folders go into.
pub const CALENDAR: &str = "ews:calendar";
pub const CONTACTS: &str = "ews:contacts";
/// Items read from a folder at most, and per `GetItem`.
const MAX_ITEMS: usize = 5000;
/// A series' occurrences taken out, and its changed ones, kept at most.
const MAX_EXDATES: usize = 1000;
const BATCH: usize = 50;
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

fn text(e: &El, name: &str) -> String {
    e.text_of(name).unwrap_or_default().trim().to_string()
}

/// An EWS `Recurrence` element as Graph's pattern and range.
pub fn recurrence_json(r: &El) -> Option<Value> {
    let p = r.kids.iter().find(|k| k.name.ends_with("Recurrence") && !k.name.contains("End") && !k.name.contains("Numbered"))?;
    let kind = match p.name.as_str() {
        "DailyRecurrence" => "daily",
        "WeeklyRecurrence" => "weekly",
        "AbsoluteMonthlyRecurrence" => "absoluteMonthly",
        "RelativeMonthlyRecurrence" => "relativeMonthly",
        "AbsoluteYearlyRecurrence" => "absoluteYearly",
        "RelativeYearlyRecurrence" => "relativeYearly",
        _ => return None,
    };
    let num = |n: &str| p.text_of(n).and_then(|v| v.trim().parse::<u64>().ok());
    let days: Vec<String> = p.text_of("DaysOfWeek").unwrap_or("").split_whitespace().map(str::to_ascii_lowercase).collect();
    let month = p.text_of("Month").and_then(|m| MONTHS.iter().position(|x| x.eq_ignore_ascii_case(m.trim()))).map(|i| i + 1);
    let pattern = json!({
        "type": kind,
        "interval": num("Interval").unwrap_or(1),
        "daysOfWeek": days,
        "index": p.text_of("DayOfWeekIndex").unwrap_or("first").trim().to_ascii_lowercase(),
        "month": month.unwrap_or(0),
        "dayOfMonth": num("DayOfMonth").unwrap_or(0),
    });
    let date = |e: &El, n: &str| e.text_of(n).map(|d| d.trim().get(..10).unwrap_or("").to_string());
    let range = if let Some(e) = r.child("EndDateRecurrence") {
        json!({"type": "endDate", "startDate": date(e, "StartDate"), "endDate": date(e, "EndDate")})
    } else if let Some(e) = r.child("NumberedRecurrence") {
        json!({"type": "numbered", "startDate": date(e, "StartDate"), "numberOfOccurrences": e.text_of("NumberOfOccurrences").and_then(|n| n.trim().parse::<u64>().ok()).unwrap_or(1)})
    } else {
        json!({"type": "noEnd"})
    };
    Some(json!({"pattern": pattern, "range": range}))
}

/// An RRULE as an EWS `Recurrence` element (starting on the event's day).
pub fn recurrence_xml(rule: &str, start: i64) -> Option<String> {
    let r = recurrence_of(rule, start)?;
    let p = &r["pattern"];
    let interval = p["interval"].as_u64().unwrap_or(1).max(1);
    let day = |d: &Value| d.as_str().and_then(|n| DAYS.iter().find(|x| x.eq_ignore_ascii_case(n))).copied();
    let days = p["daysOfWeek"].as_array().map(|a| a.iter().filter_map(day).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    let index = p["index"].as_str().map(|i| {
        let mut c = i.chars();
        c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
    });
    let month = p["month"].as_u64().and_then(|m| MONTHS.get(usize::try_from(m).ok()?.checked_sub(1)?)).copied().unwrap_or("January");
    let mday = p["dayOfMonth"].as_u64().unwrap_or(1);
    let pattern = match p["type"].as_str()? {
        "daily" => format!("<t:DailyRecurrence><t:Interval>{interval}</t:Interval></t:DailyRecurrence>"),
        "weekly" => format!("<t:WeeklyRecurrence><t:Interval>{interval}</t:Interval><t:DaysOfWeek>{days}</t:DaysOfWeek></t:WeeklyRecurrence>"),
        "absoluteMonthly" => format!(
            "<t:AbsoluteMonthlyRecurrence><t:Interval>{interval}</t:Interval><t:DayOfMonth>{mday}</t:DayOfMonth></t:AbsoluteMonthlyRecurrence>"
        ),
        "relativeMonthly" => format!(
            "<t:RelativeMonthlyRecurrence><t:Interval>{interval}</t:Interval><t:DaysOfWeek>{days}</t:DaysOfWeek><t:DayOfWeekIndex>{}</t:DayOfWeekIndex></t:RelativeMonthlyRecurrence>",
            index.unwrap_or_else(|| "First".into())
        ),
        "absoluteYearly" => {
            format!("<t:AbsoluteYearlyRecurrence><t:DayOfMonth>{mday}</t:DayOfMonth><t:Month>{month}</t:Month></t:AbsoluteYearlyRecurrence>")
        }
        "relativeYearly" => format!(
            "<t:RelativeYearlyRecurrence><t:DaysOfWeek>{days}</t:DaysOfWeek><t:DayOfWeekIndex>{}</t:DayOfWeekIndex><t:Month>{month}</t:Month></t:RelativeYearlyRecurrence>",
            index.unwrap_or_else(|| "First".into())
        ),
        _ => return None,
    };
    let rg = &r["range"];
    let first = rg["startDate"].as_str().unwrap_or("");
    let range = match rg["type"].as_str() {
        Some("endDate") => format!(
            "<t:EndDateRecurrence><t:StartDate>{first}</t:StartDate><t:EndDate>{}</t:EndDate></t:EndDateRecurrence>",
            rg["endDate"].as_str().unwrap_or(first)
        ),
        Some("numbered") => format!(
            "<t:NumberedRecurrence><t:StartDate>{first}</t:StartDate><t:NumberOfOccurrences>{}</t:NumberOfOccurrences></t:NumberedRecurrence>",
            rg["numberOfOccurrences"].as_u64().unwrap_or(1)
        ),
        _ => format!("<t:NoEndRecurrence><t:StartDate>{first}</t:StartDate></t:NoEndRecurrence>"),
    };
    Some(format!("<t:Recurrence>{pattern}{range}</t:Recurrence>"))
}

fn person(m: &El) -> Option<Attendee> {
    let mb = if m.name == "Mailbox" { m } else { m.find("Mailbox")? };
    let email = text(mb, "EmailAddress");
    email.contains('@').then(|| Attendee { email, name: text(mb, "Name"), partstat: String::new(), rsvp: false, ..Default::default() })
}

/// The answer the mailbox's owner (`me`: their addresses) gave a meeting, from the appointment's
/// own `MyResponseType`, put on their place among its guests, as Outlook shows it. That place
/// keeps the answer the invitation came with ("Unknown", or tentative) and isn't changed when the
/// owner answers: some accepted series showed as tentative or not answered (#144). The place is
/// under whichever of their addresses the meeting was sent to, not only the account's own: under
/// another, an accepted series still waited for an answer.
fn own_answer(event: &mut CalEvent, it: &El, me: &[&str]) {
    let partstat = match it.text_of("MyResponseType") {
        Some("Accept") => "ACCEPTED",
        Some("Tentative") => "TENTATIVE",
        Some("Decline") => "DECLINED",
        _ => return,
    };
    for a in event.attendees.iter_mut().filter(|a| me.iter().any(|m| !m.is_empty() && a.email.eq_ignore_ascii_case(m))) {
        a.partstat = partstat.to_string();
    }
}

/// An appointment as an event (occurrences and changed instances are left to their series).
pub fn event_of(it: &El) -> Option<CalEvent> {
    // A plain occurrence is its series'. A changed one (an exception: moved, cancelled, its own
    // subject or answer) is the occurrence it stands for, by the start its series gave it
    // (`OriginalStart`, which [`reading::with_changed_occurrences`] puts on it from the series'
    // list): it was dropped, and the series kept its usual time there (issue #204).
    let original = match it.text_of("CalendarItemType") {
        Some("Occurrence") => return None,
        Some("Exception") => Some(mailcraft_jmap::utc_date(&text(it, "OriginalStart"))?),
        _ => None,
    };
    let all_day = it.text_of("IsAllDayEvent").is_some_and(|v| v == "true");
    let start = mailcraft_jmap::utc_date(&text(it, "Start"))?;
    let end = mailcraft_jmap::utc_date(&text(it, "End")).unwrap_or(start + 3600).max(start);
    // All-day times are local midnights: the nearest UTC midnight is the day.
    let day = |t: i64| (t + 43_200).div_euclid(86_400) * 86_400;
    let (start, end) = if all_day { (day(start), day(end).max(day(start) + 86_400)) } else { (start, end) };
    let body = it.child("Body");
    let content = body.map(|b| b.text.clone()).unwrap_or_default();
    let html = body.and_then(|b| b.attr("BodyType")).is_some_and(|t| t.eq_ignore_ascii_case("HTML"));
    // A Teams or Zoom invitation's join link, whole, from the HTML (its text wraps long ones).
    let join_url = if html {
        mailcraft_mime::html_links(&content).into_iter().find(|l| mailcraft_model::ical::is_meeting_link(l)).unwrap_or_default()
    } else {
        String::new()
    };
    let description = if html { mailcraft_mime::html_to_text(&content) } else { content };
    let mut attendees = Vec::new();
    for list in ["RequiredAttendees", "OptionalAttendees"] {
        if let Some(l) = it.child(list) {
            for a in l.children("Attendee").take(500) {
                if let Some(mut p) = person(a) {
                    p.partstat = match a.text_of("ResponseType").unwrap_or("") {
                        "Accept" | "Organizer" => "ACCEPTED",
                        "Decline" => "DECLINED",
                        "Tentative" => "TENTATIVE",
                        _ => "NEEDS-ACTION",
                    }
                    .to_string();
                    p.rsvp = true;
                    attendees.push(p);
                }
            }
        }
    }
    let id = it.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or("").to_string();
    let uid = [text(it, "UID"), id].into_iter().find(|u| !u.is_empty())?;
    let mut event = CalEvent {
        uid,
        summary: text(it, "Subject"),
        join_url,
        description: description.trim().to_string(),
        location: text(it, "Location"),
        start,
        end,
        all_day,
        rrule: if original.is_some() { None } else { it.child("Recurrence").and_then(recurrence_json).as_ref().and_then(rrule_of) },
        recurrence_id: original.map(|t| if all_day { day(t) } else { t }),
        // The occurrences taken out of a series (issue #204): each by its start.
        exdates: it
            .child("DeletedOccurrences")
            .map(|d| {
                d.children("DeletedOccurrence")
                    .filter_map(|o| mailcraft_jmap::utc_date(&text(o, "Start")))
                    .map(|t| if all_day { day(t) } else { t })
                    .take(MAX_EXDATES)
                    .collect()
            })
            .unwrap_or_default(),
        organizer: it.child("Organizer").and_then(person),
        attendees,
        status: if it.text_of("IsCancelled") == Some("true") { "CANCELLED".into() } else { String::new() },
        categories: it
            .child("Categories")
            .map(|c| c.children("String").map(|s| s.text.trim().to_string()).filter(|s| !s.is_empty()).take(20).collect())
            .unwrap_or_default(),
        ..Default::default()
    };
    // Exchange's reminder, kept as the event's alarm.
    if it.text_of("ReminderIsSet") == Some("true") {
        let minutes = it.text_of("ReminderMinutesBeforeStart").and_then(|m| m.trim().parse::<i64>().ok()).unwrap_or(15);
        mailcraft_model::ical::set_reminder(&mut event, Some(minutes));
    }
    // Private and Show as (issue #32), as its CLASS and TRANSP (with Outlook's busy status).
    match it.text_of("Sensitivity") {
        Some(s) if s.trim().eq_ignore_ascii_case("Personal") => set_personal(&mut event),
        Some(s) => {
            if let Some(p) = Privacy::from_code(s) {
                set_privacy(&mut event, p);
            }
        }
        None => {}
    }
    if let Some(s) = it.text_of("LegacyFreeBusyStatus").and_then(ShowAs::from_code) {
        set_show_as(&mut event, s);
    }
    Some(event)
}

/// An event's classification as EWS's `Sensitivity`.
fn sensitivity(e: &CalEvent) -> &'static str {
    match privacy(e) {
        _ if personal(e) => "Personal",
        Privacy::Public => "Normal",
        Privacy::Private => "Private",
        Privacy::Confidential => "Confidential",
    }
}

/// How an event shows its time as EWS's `LegacyFreeBusyStatus`.
fn free_busy(e: &CalEvent) -> &'static str {
    match show_as(e) {
        ShowAs::Free => "Free",
        ShowAs::WorkingElsewhere => "WorkingElsewhere",
        ShowAs::Tentative => "Tentative",
        ShowAs::Busy => "Busy",
        ShowAs::OutOfOffice => "OOF",
    }
}

/// The fields of an appointment, in the order EWS wants them.
fn event_fields(e: &CalEvent) -> Vec<(&'static str, String)> {
    let (start, end) = if e.all_day { (e.start, e.end.max(e.start + 86_400)) } else { (e.start, e.end.max(e.start)) };
    let mut f = vec![
        ("Subject", format!("<t:Subject>{}</t:Subject>", esc(&e.summary))),
        ("Sensitivity", format!("<t:Sensitivity>{}</t:Sensitivity>", sensitivity(e))),
        ("Body", format!("<t:Body BodyType=\"Text\">{}</t:Body>", esc(&e.description))),
    ];
    // Categories come between the item's fields and the appointment's (EWS keeps the schema's
    // order); none at all is left out (an empty list isn't allowed).
    if !e.categories.is_empty() {
        let list: String = e.categories.iter().map(|c| format!("<t:String>{}</t:String>", esc(c))).collect();
        f.push(("Categories", format!("<t:Categories>{list}</t:Categories>")));
    }
    // Its reminder (after the item's other fields, before the appointment's).
    let reminder = mailcraft_model::ical::reminder_minutes(e);
    f.push(("ReminderIsSet", format!("<t:ReminderIsSet>{}</t:ReminderIsSet>", reminder.is_some())));
    if let Some(m) = reminder {
        f.push(("ReminderMinutesBeforeStart", format!("<t:ReminderMinutesBeforeStart>{m}</t:ReminderMinutesBeforeStart>")));
    }
    f.extend([
        ("Start", format!("<t:Start>{}</t:Start>", utc_stamp(start))),
        ("End", format!("<t:End>{}</t:End>", utc_stamp(end))),
        ("IsAllDayEvent", format!("<t:IsAllDayEvent>{}</t:IsAllDayEvent>", e.all_day)),
        ("LegacyFreeBusyStatus", format!("<t:LegacyFreeBusyStatus>{}</t:LegacyFreeBusyStatus>", free_busy(e))),
        ("Location", format!("<t:Location>{}</t:Location>", esc(&e.location))),
    ]);
    if let Some(r) = e.rrule.as_deref().and_then(|r| recurrence_xml(r, e.start)) {
        f.push(("Recurrence", r));
    }
    f
}

/// A person, in the order EWS wants a contact's fields.
fn contact_xml(c: &Card) -> String {
    let mut x = format!("<t:Contact><t:Body BodyType=\"Text\">{}</t:Body>", esc(&c.note));
    x.push_str(&format!("<t:DisplayName>{}</t:DisplayName>", esc(&c.display())));
    if !c.given.is_empty() {
        x.push_str(&format!("<t:GivenName>{}</t:GivenName>", esc(&c.given)));
    }
    if !c.org.is_empty() {
        x.push_str(&format!("<t:CompanyName>{}</t:CompanyName>", esc(&c.org)));
    }
    if !c.emails.is_empty() {
        x.push_str("<t:EmailAddresses>");
        for (i, e) in c.emails.iter().take(3).enumerate() {
            x.push_str(&format!("<t:Entry Key=\"EmailAddress{}\">{}</t:Entry>", i + 1, esc(&e.value)));
        }
        x.push_str("</t:EmailAddresses>");
    }
    // Its addresses (Exchange keeps one each of business, home and other).
    let addresses: Vec<(&str, &PostalAddress)> = ["Business", "Home", "Other"]
        .into_iter()
        .filter_map(|key| {
            let kind = match key {
                "Business" => "work",
                "Home" => "home",
                _ => "",
            };
            c.addresses
                .iter()
                .find(|a| !a.is_empty() && (a.kind == kind || (key == "Other" && !matches!(a.kind.as_str(), "work" | "home"))))
                .map(|a| (key, a))
        })
        .collect();
    if !addresses.is_empty() {
        x.push_str("<t:PhysicalAddresses>");
        for (key, a) in addresses {
            x.push_str(&format!("<t:Entry Key=\"{key}\">"));
            for (name, v) in
                [("Street", &a.street), ("City", &a.city), ("State", &a.region), ("CountryOrRegion", &a.country), ("PostalCode", &a.postcode)]
            {
                if !v.trim().is_empty() {
                    x.push_str(&format!("<t:{name}>{}</t:{name}>", esc(v.trim())));
                }
            }
            x.push_str("</t:Entry>");
        }
        x.push_str("</t:PhysicalAddresses>");
    }
    if !c.phones.is_empty() {
        x.push_str("<t:PhoneNumbers>");
        let mut used: Vec<&str> = Vec::new();
        for p in &c.phones {
            let kind = match p.kind.as_str() {
                "cell" | "mobile" => "cell",
                "home" => "home",
                "fax" => "fax",
                "pager" => "pager",
                "work" => "work",
                _ => "",
            };
            // The first free entry for its kind (another number of a kind already full goes to
            // Other).
            let key = PHONE_KEYS
                .iter()
                .find(|(k, kd)| *kd == kind && !used.contains(k))
                .or_else(|| PHONE_KEYS.iter().find(|(k, _)| *k == "OtherTelephone" && !used.contains(k)))
                .map(|(k, _)| *k);
            if let Some(key) = key {
                used.push(key);
                x.push_str(&format!("<t:Entry Key=\"{key}\">{}</t:Entry>", esc(&p.value)));
            }
        }
        x.push_str("</t:PhoneNumbers>");
    }
    // A birthday at midday UTC, so it's the same day everywhere.
    if let Some(b) = mailcraft_model::vcard::birthday_for_exchange(&c.birthday) {
        x.push_str(&format!("<t:Birthday>{b}T12:00:00Z</t:Birthday>"));
    }
    if !c.title.is_empty() {
        x.push_str(&format!("<t:JobTitle>{}</t:JobTitle>", esc(&c.title)));
    }
    if !c.family.is_empty() {
        x.push_str(&format!("<t:Surname>{}</t:Surname>", esc(&c.family)));
    }
    x.push_str("</t:Contact>");
    x
}

/// A contact as a card.
pub fn card_of(it: &El) -> Option<Card> {
    let id = it.child("ItemId").and_then(|i| i.attr("Id"))?.to_string();
    let entries = |list: &str| -> Vec<(String, String)> {
        it.child(list)
            .map(|l| l.children("Entry").map(|e| (e.attr("Key").unwrap_or("").to_string(), e.text.trim().to_string())).collect())
            .unwrap_or_default()
    };
    let emails = entries("EmailAddresses")
        .into_iter()
        .filter(|(_, v)| v.contains('@'))
        .map(|(_, v)| Typed { value: v.trim_start_matches("SMTP:").trim_start_matches("smtp:").to_string(), kind: String::new() })
        .collect();
    let mut phones: Vec<Typed> = entries("PhoneNumbers")
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| {
            let kind = match k.as_str() {
                "MobilePhone" => "cell",
                "HomePhone" | "HomePhone2" => "home",
                "BusinessFax" | "HomeFax" | "OtherFax" => "fax",
                "BusinessPhone" | "BusinessPhone2" | "CompanyMainPhone" => "work",
                "Pager" => "pager",
                _ => "",
            };
            Typed { value: v, kind: kind.into() }
        })
        .collect();
    // As Outlook lists them: business, home, mobile, then the rest.
    phones.sort_by_key(|p| match p.kind.as_str() {
        "work" => 0,
        "home" => 1,
        "cell" => 2,
        _ => 3,
    });
    let addresses = it
        .child("PhysicalAddresses")
        .map(|l| {
            l.children("Entry")
                .map(|e| PostalAddress {
                    kind: match e.attr("Key").unwrap_or("") {
                        "Business" => "work",
                        "Home" => "home",
                        _ => "",
                    }
                    .into(),
                    street: e.text_of("Street").unwrap_or("").trim().replace("\r\n", "\n"),
                    city: e.text_of("City").unwrap_or("").trim().to_string(),
                    region: e.text_of("State").unwrap_or("").trim().to_string(),
                    postcode: e.text_of("PostalCode").unwrap_or("").trim().to_string(),
                    country: e.text_of("CountryOrRegion").unwrap_or("").trim().to_string(),
                })
                .filter(|a| !a.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let birthday = it.text_of("Birthday").map(crate::exchange::birthday).unwrap_or_default();
    let body = it.child("Body").map(|b| b.text.trim().to_string()).unwrap_or_default();
    Some(Card {
        uid: id,
        name: text(it, "DisplayName"),
        given: text(it, "GivenName"),
        family: text(it, "Surname"),
        emails,
        phones,
        org: text(it, "CompanyName"),
        title: text(it, "JobTitle"),
        note: body,
        addresses,
        birthday,
        ..Default::default()
    })
}

/// Exchange's phone entries, by the kind each takes (two of business and home).
const PHONE_KEYS: [(&str, &str); 9] = [
    ("BusinessPhone", "work"),
    ("BusinessPhone2", "work"),
    ("HomePhone", "home"),
    ("HomePhone2", "home"),
    ("MobilePhone", "cell"),
    ("BusinessFax", "fax"),
    ("HomeFax", "fax"),
    ("Pager", "pager"),
    ("OtherTelephone", ""),
];

/// A sticky note's colour (MS-OXONOTE PidLidNoteColor), as EWS asks for it.
const NOTE_COLOR: &str = r#"<t:ExtendedFieldURI PropertySetId="0006200E-0000-0000-C000-000000000046" PropertyId="35584" PropertyType="Integer"/>"#;

/// A note in Exchange's Notes folder (`IPM.StickyNote`).
pub fn note_of(it: &El) -> Option<StoredNote> {
    if !it.text_of("ItemClass").is_some_and(|c| c.starts_with("IPM.StickyNote")) {
        return None;
    }
    let id = it.child("ItemId")?;
    let href = format!("note:{}", id.attr("Id")?);
    let text = it.text_of("Body").map(str::to_string).filter(|b| !b.trim().is_empty()).unwrap_or_else(|| text(it, "Subject"));
    let color = it
        .children("ExtendedProperty")
        .find(|p| p.find("ExtendedFieldURI").and_then(|f| f.attr("PropertyId")) == Some("35584"))
        .and_then(|p| p.text_of("Value"))
        .and_then(|v| v.trim().parse::<u8>().ok())
        .filter(|c| *c < mailcraft_model::pim::NOTE_COLORS)
        .unwrap_or(mailcraft_model::pim::NOTE_YELLOW);
    let modified = it.text_of("LastModifiedTime").and_then(mailcraft_jmap::utc_date).unwrap_or(0);
    Some(StoredNote {
        href: href.clone(),
        etag: id.attr("ChangeKey").unwrap_or("").to_string(),
        note: Note { uid: href, text: text.replace("\r\n", "\n"), color, modified, files: Vec::new() },
        dirty: false,
    })
}

/// A note as EWS creates it: a message of class `IPM.StickyNote`, its first line the subject.
fn note_xml(n: &Note) -> String {
    let subject: String = n.title().chars().take(255).collect();
    format!(
        "<t:Message><t:ItemClass>IPM.StickyNote</t:ItemClass><t:Subject>{}</t:Subject><t:Body BodyType=\"Text\">{}</t:Body>\
         <t:ExtendedProperty>{NOTE_COLOR}<t:Value>{}</t:Value></t:ExtendedProperty></t:Message>",
        esc(&subject),
        esc(&n.text),
        n.color
    )
}

/// Public contact folders and calendars read, at most.
const MAX_PUBLIC_PIM: usize = 20;

/// An item's id and change key (its href and etag here), from its `ItemId`: empty when it has
/// none.
fn id_and_key(id: Option<&El>) -> (String, String) {
    let attr = |name: &str| id.and_then(|i| i.attr(name)).unwrap_or("").to_string();
    (attr("Id"), attr("ChangeKey"))
}

/// The events among a calendar's items, as kept in `calendar`; `me`, the addresses of the mailbox
/// it's in (none for a public folder's), whose answers to meetings the items say.
fn stored_events<'a>(items: Vec<El>, calendar: &'a str, me: &'a [&'a str]) -> impl Iterator<Item = StoredEvent> + 'a {
    items.into_iter().filter_map(move |it| {
        let mut event = event_of(&it)?;
        own_answer(&mut event, &it, me);
        let (href, etag) = id_and_key(it.child("ItemId"));
        Some(StoredEvent { calendar: calendar.to_string(), href, etag, event, dirty: false })
    })
}

/// The name the server gives one of the account's own special folders (`calendar`, `contacts`):
/// the mailbox's language's ("Agenda", "Contactpersonen"). `None` when it doesn't say.
fn folder_name(c: &Client, distinguished: &str) -> Option<String> {
    let m = c
        .call_one(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
             <t:FieldURI FieldURI=\"folder:DisplayName\"/></t:AdditionalProperties></m:FolderShape>\
             <m:FolderIds><t:DistinguishedFolderId Id=\"{distinguished}\"/></m:FolderIds></m:GetFolder>"
        ))
        .ok()?;
    let name: String = m.find("DisplayName")?.text.trim().chars().take(120).collect();
    (!name.is_empty()).then_some(name)
}

/// Run `f` in the mailbox whose calendar `calendar` is, when it's someone else's
/// ([`Client::in_mailbox`]).
fn in_calendar<T>(c: &Client, calendar: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match mailcraft_model::pim::shared_calendar_owner(calendar) {
        Some(owner) => c.in_mailbox(owner, f),
        None => f(),
    }
}

/// The calendar folder: this mailbox's, or someone else's by their address.
fn calendar_folder(owner: Option<&str>) -> String {
    match owner {
        Some(email) => format!(
            "<t:DistinguishedFolderId Id=\"calendar\"><t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox></t:DistinguishedFolderId>",
            esc(email)
        ),
        None => "<t:DistinguishedFolderId Id=\"calendar\"/>".into(),
    }
}

/// Whether this account may add items to a folder (its EffectiveRights): `false` when the server
/// doesn't say. A folder's `Modify` is about the folder itself (its name and permissions, the
/// owner's alone), so an editor is known by `CreateContents`.
fn may_edit(c: &Client, folder: &str) -> Result<bool> {
    let m = c.call_one(&format!(
        "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
         <t:FieldURI FieldURI=\"folder:EffectiveRights\"/></t:AdditionalProperties></m:FolderShape>\
         <m:FolderIds>{folder}</m:FolderIds></m:GetFolder>"
    ))?;
    Ok(m.find("CreateContents").is_some_and(|e| e.text.trim().eq_ignore_ascii_case("true")))
}

/// Whether an item says this account may change it (its own EffectiveRights).
fn item_may_change(it: &El) -> bool {
    it.child("EffectiveRights").and_then(|r| r.child("Modify")).is_some_and(|m| m.text.trim().eq_ignore_ascii_case("true"))
}

mod pass;
#[cfg(test)]
pub use pass::sync;
pub use pass::{Pass, read_people};
mod pushing;
mod reading;
mod reminders;
mod tasks;
use pushing::{push_cards, push_deletions, push_events, push_notes};
use reading::{read_calendar, read_contacts, read_notes, read_public, read_shared_calendar};
use tasks::{TASK_FIELDS, push_tasks, read_tasks};

#[cfg(test)]
mod tests;
