//! On-premises Exchange calendars and contacts through Exchange Web Services, synced into the
//! same calendars and address books as CalDAV and CardDAV ([`DavSync`], which the engine merges).
//!
//! The mailbox's Calendar and Contacts folders: appointments (with their recurrence, read as the
//! pattern and range Graph also uses, so [`crate::graph_pim::rrule_of`] makes the RRULE) and
//! contacts. What changed here goes first: new appointments and contacts are created, changed
//! appointments updated field by field, changed contacts made again, deletions done (no meeting
//! mail is ever sent); then both folders are read again.

use mailcraft_model::ical::{Attendee, CalEvent};
use mailcraft_model::pim::{BookInfo, CalendarInfo, DavSync, Note, Pim, StoredCard, StoredEvent, StoredNote};
use mailcraft_model::vcard::{Card, Typed};
use serde_json::{Value, json};

use crate::ews::{Client, El, esc};
use crate::graph_pim::{recurrence_of, rrule_of};
use crate::{NetError, Result};

/// The calendar and address book Exchange's folders go into.
pub const CALENDAR: &str = "ews:calendar";
pub const CONTACTS: &str = "ews:contacts";
/// Items read from a folder at most, and per `GetItem`.
const MAX_ITEMS: usize = 5000;
const BATCH: usize = 50;
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

fn utc(t: i64) -> String {
    let (y, m, d) = mailcraft_model::dates::civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

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
    email.contains('@').then(|| Attendee { email, name: text(mb, "Name"), partstat: String::new(), rsvp: false })
}

/// An appointment as an event (occurrences and changed instances are left to their series).
pub fn event_of(it: &El) -> Option<CalEvent> {
    if matches!(it.text_of("CalendarItemType"), Some("Occurrence" | "Exception")) {
        return None;
    }
    let all_day = it.text_of("IsAllDayEvent").is_some_and(|v| v == "true");
    let start = mailcraft_jmap::utc_date(&text(it, "Start"))?;
    let end = mailcraft_jmap::utc_date(&text(it, "End")).unwrap_or(start + 3600).max(start);
    // All-day times are local midnights: the nearest UTC midnight is the day.
    let day = |t: i64| (t + 43_200).div_euclid(86_400) * 86_400;
    let (start, end) = if all_day { (day(start), day(end).max(day(start) + 86_400)) } else { (start, end) };
    let body = it.child("Body");
    let content = body.map(|b| b.text.clone()).unwrap_or_default();
    let html = body.and_then(|b| b.attr("BodyType")).is_some_and(|t| t.eq_ignore_ascii_case("HTML"));
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
    Some(CalEvent {
        uid,
        summary: text(it, "Subject"),
        description: description.trim().to_string(),
        location: text(it, "Location"),
        start,
        end,
        all_day,
        rrule: it.child("Recurrence").and_then(recurrence_json).as_ref().and_then(rrule_of),
        organizer: it.child("Organizer").and_then(person),
        attendees,
        status: if it.text_of("IsCancelled") == Some("true") { "CANCELLED".into() } else { String::new() },
        categories: it
            .child("Categories")
            .map(|c| c.children("String").map(|s| s.text.trim().to_string()).filter(|s| !s.is_empty()).take(20).collect())
            .unwrap_or_default(),
        ..Default::default()
    })
}

/// The fields of an appointment, in the order EWS wants them.
fn event_fields(e: &CalEvent) -> Vec<(&'static str, String)> {
    let (start, end) = if e.all_day { (e.start, e.end.max(e.start + 86_400)) } else { (e.start, e.end.max(e.start)) };
    let mut f = vec![
        ("Subject", format!("<t:Subject>{}</t:Subject>", esc(&e.summary))),
        ("Body", format!("<t:Body BodyType=\"Text\">{}</t:Body>", esc(&e.description))),
    ];
    // Categories come between the item's fields and the appointment's (EWS keeps the schema's
    // order); none at all is left out (an empty list isn't allowed).
    if !e.categories.is_empty() {
        let list: String = e.categories.iter().map(|c| format!("<t:String>{}</t:String>", esc(c))).collect();
        f.push(("Categories", format!("<t:Categories>{list}</t:Categories>")));
    }
    f.extend([
        ("Start", format!("<t:Start>{}</t:Start>", utc(start))),
        ("End", format!("<t:End>{}</t:End>", utc(end))),
        ("IsAllDayEvent", format!("<t:IsAllDayEvent>{}</t:IsAllDayEvent>", e.all_day)),
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
    if !c.phones.is_empty() {
        x.push_str("<t:PhoneNumbers>");
        let mut used: Vec<&str> = Vec::new();
        for p in &c.phones {
            let key = match p.kind.as_str() {
                "cell" => "MobilePhone",
                "home" => "HomePhone",
                "fax" => "BusinessFax",
                _ => "BusinessPhone",
            };
            if !used.contains(&key) {
                used.push(key);
                x.push_str(&format!("<t:Entry Key=\"{key}\">{}</t:Entry>", esc(&p.value)));
            }
        }
        x.push_str("</t:PhoneNumbers>");
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
        .map(|(_, v)| Typed { value: v.trim_start_matches("SMTP:").to_string(), kind: String::new() })
        .collect();
    let phones = entries("PhoneNumbers")
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| {
            let kind = match k.as_str() {
                "MobilePhone" => "cell",
                "HomePhone" | "HomePhone2" => "home",
                "BusinessFax" | "HomeFax" => "fax",
                _ => "work",
            };
            Typed { value: v, kind: kind.into() }
        })
        .collect();
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
        extra: Vec::new(),
    })
}

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
        note: Note { uid: href, text: text.replace("\r\n", "\n"), color, modified },
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

/// Every item of a distinguished folder, with all its properties (bodies as text).
fn items(c: &Client, folder: &str) -> Result<Vec<El>> {
    items_in(c, &format!("<t:DistinguishedFolderId Id=\"{folder}\"/>"), if folder == "notes" { NOTE_COLOR } else { "" })
}

/// Every item of a folder (given as its EWS folder id element), with all its properties.
fn items_in(c: &Client, parent: &str, extra: &str) -> Result<Vec<El>> {
    let folder = parent;
    let mut ids: Vec<String> = Vec::new();
    let mut offset = 0;
    loop {
        let m = c.call_one(&format!(
            "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>\
             <m:IndexedPageItemView MaxEntriesReturned=\"500\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
             <m:ParentFolderIds>{folder}</m:ParentFolderIds></m:FindItem>"
        ))?;
        let Some(rf) = m.child("RootFolder") else { break };
        let page: Vec<String> =
            rf.child("Items").map(|i| i.kids.iter().filter_map(|k| k.child("ItemId")?.attr("Id").map(str::to_string)).collect()).unwrap_or_default();
        let n = page.len();
        ids.extend(page);
        offset += n;
        if n == 0 || rf.attr("IncludesLastItemInRange").is_none_or(|v| v == "true") || ids.len() >= MAX_ITEMS {
            break;
        }
    }
    let mut out = Vec::new();
    for chunk in ids.chunks(BATCH) {
        let list: String = chunk.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect();
        let doc = c.call(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>AllProperties</t:BaseShape><t:BodyType>Text</t:BodyType>\
             <t:AdditionalProperties><t:FieldURI FieldURI=\"calendar:Recurrence\"/>{}</t:AdditionalProperties></m:ItemShape>\
             <m:ItemIds>{list}</m:ItemIds></m:GetItem>",
            extra
        ))?;
        for m in crate::ews::messages(&doc) {
            if m.attr("ResponseClass") == Some("Error") {
                continue;
            }
            if let Some(items) = m.child("Items") {
                out.extend(items.kids.iter().cloned());
            }
        }
    }
    Ok(out)
}

fn new_id(m: &El) -> (String, String) {
    let id = m.find("ItemId");
    (id.and_then(|i| i.attr("Id")).unwrap_or("").to_string(), id.and_then(|i| i.attr("ChangeKey")).unwrap_or("").to_string())
}

fn delete(c: &Client, id: &str) -> Result<()> {
    c.call_one(&format!(
        "<m:DeleteItem DeleteType=\"MoveToDeletedItems\" SendMeetingCancellations=\"SendToNone\"><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:DeleteItem>",
        esc(id)
    ))
    .map(drop)
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

/// Whether this account may add and change items in a folder (its EffectiveRights). A folder's
/// `Modify` is about the folder itself (its name and permissions, the owner's alone), so an
/// editor is known by `CreateContents`.
fn may_edit(c: &Client, folder: &str) -> bool {
    let doc = c.call(&format!(
        "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
         <t:FieldURI FieldURI=\"folder:EffectiveRights\"/></t:AdditionalProperties></m:FolderShape>\
         <m:FolderIds>{folder}</m:FolderIds></m:GetFolder>"
    ));
    let Ok(doc) = doc else { return false };
    let right = |name: &str| doc.find(name).is_some_and(|e| e.text.trim() == "true");
    right("CreateContents")
}

/// A public folder's items. On Exchange 2013 and later its content is kept in a public folder
/// mailbox, and reading it is routed there (asked for by the GUID the folder names); a server
/// that answers without that (older ones, a single public folder mailbox) is asked plainly
/// first.
fn public_items(c: &Client, id: &str, replica: Option<&str>, total: u32) -> Result<Vec<El>> {
    let folder = format!("<t:FolderId Id=\"{}\"/>", esc(id));
    let plain = items_in(c, &folder, "");
    let Some(guid) = replica else { return plain };
    match plain {
        Ok(list) if !list.is_empty() || total == 0 => Ok(list),
        plain => match c.public_folder_mailbox(guid) {
            Ok(mailbox) => c.routed_to(&mailbox, || items_in(c, &folder, "")),
            Err(e) => {
                log::info!("public folder mailbox {guid}: {e}");
                plain
            }
        },
    }
}

/// Send what changed here, then read the calendar and contacts again.
pub fn sync(c: &Client, pim: &Pim, now: i64) -> Result<DavSync> {
    let mut out = DavSync { caldav_home: Some("ews".into()), carddav_home: Some("ews".into()), checked: now, ..Default::default() };
    for t in &pim.deleted {
        match delete(c, t.href.trim_start_matches("contact:").trim_start_matches("note:")) {
            Ok(()) | Err(NetError::No(_)) => out.deleted.push(t.href.clone()),
            Err(e) => out.errors.push(e.to_string()),
        }
    }
    for e in pim.events.iter().filter(|e| e.dirty) {
        let fields = event_fields(&e.event);
        let owner = mailcraft_model::pim::shared_calendar_owner(&e.calendar);
        let r = if e.href.is_empty() {
            let body: String = fields.iter().map(|(_, x)| x.as_str()).collect();
            c.call_one(&format!(
                "<m:CreateItem SendMeetingInvitations=\"SendToNone\"><m:SavedItemFolderId>{}</m:SavedItemFolderId>\
                 <m:Items><t:CalendarItem>{body}</t:CalendarItem></m:Items></m:CreateItem>",
                calendar_folder(owner)
            ))
            .map(|m| new_id(&m))
        } else {
            let mut updates: String = fields
                .iter()
                .map(|(name, x)| {
                    let uri = if matches!(*name, "Subject" | "Body" | "Categories") { format!("item:{name}") } else { format!("calendar:{name}") };
                    format!("<t:SetItemField><t:FieldURI FieldURI=\"{uri}\"/><t:CalendarItem>{x}</t:CalendarItem></t:SetItemField>")
                })
                .collect();
            if e.event.rrule.is_none() {
                updates.push_str("<t:DeleteItemField><t:FieldURI FieldURI=\"calendar:Recurrence\"/></t:DeleteItemField>");
            }
            c.call_one(&format!(
                "<m:UpdateItem ConflictResolution=\"AlwaysOverwrite\" SendMeetingInvitationsOrCancellations=\"SendToNone\"><m:ItemChanges>\
                 <t:ItemChange><t:ItemId Id=\"{}\"/><t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",
                esc(&e.href)
            ))
            .map(|m| {
                let (id, key) = new_id(&m);
                (if id.is_empty() { e.href.clone() } else { id }, key)
            })
        };
        match r {
            // Someone else's event stays in their calendar.
            Ok((href, etag)) => out.pushed_events.push(StoredEvent {
                calendar: if owner.is_some() { e.calendar.clone() } else { CALENDAR.into() },
                href,
                etag,
                event: e.event.clone(),
                dirty: false,
            }),
            Err(x) => out.errors.push(format!("{}: {x}", e.event.summary)),
        }
    }
    for k in pim.cards.iter().filter(|k| k.dirty) {
        // Made again: a contact's addresses and numbers are simplest replaced whole.
        if let Some(old) = k.href.strip_prefix("contact:")
            && let Err(e) = delete(c, old)
            && !matches!(e, NetError::No(_))
        {
            out.errors.push(format!("{}: {e}", k.card.display()));
            continue;
        }
        let r = c.call_one(&format!(
            "<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"contacts\"/></m:SavedItemFolderId><m:Items>{}</m:Items></m:CreateItem>",
            contact_xml(&k.card)
        ));
        match r {
            Ok(m) => {
                let (id, etag) = new_id(&m);
                out.pushed_cards.push(StoredCard { book: CONTACTS.into(), href: format!("contact:{id}"), etag, card: k.card.clone(), dirty: false });
            }
            Err(x) => out.errors.push(format!("{}: {x}", k.card.display())),
        }
    }
    // Then both folders, read again.
    for it in items(c, "calendar")? {
        if let Some(event) = event_of(&it) {
            let (href, etag) = it
                .child("ItemId")
                .map(|i| (i.attr("Id").unwrap_or("").to_string(), i.attr("ChangeKey").unwrap_or("").to_string()))
                .unwrap_or_default();
            out.events.push(StoredEvent { calendar: CALENDAR.into(), href, etag, event, dirty: false });
        }
    }
    let mut calendars = vec![CalendarInfo { id: CALENDAR.into(), name: "Calendar".into(), color: 0, read_only: false, ctag: String::new() }];
    out.synced_calendars.push(CALENDAR.into());
    // Other people's calendars opened here: their mailbox's calendar folder (the delegate
    // permissions they gave decide whether it can be read, and whether changed).
    for (n, email) in pim.shared.iter().enumerate() {
        let id = mailcraft_model::pim::shared_calendar_id(email);
        let parent = calendar_folder(Some(email));
        let can_edit = may_edit(c, &parent);
        match items_in(c, &parent, "") {
            Ok(list) => {
                for it in list {
                    if let Some(event) = event_of(&it) {
                        let (href, etag) = it
                            .child("ItemId")
                            .map(|i| (i.attr("Id").unwrap_or("").to_string(), i.attr("ChangeKey").unwrap_or("").to_string()))
                            .unwrap_or_default();
                        out.events.push(StoredEvent { calendar: id.clone(), href, etag, event, dirty: false });
                    }
                }
                out.synced_calendars.push(id.clone());
            }
            Err(e) => out.errors.push(format!("{email}'s calendar: {e}")),
        }
        calendars.push(CalendarInfo {
            id,
            name: email.clone(),
            color: u8::try_from((n + 1) % 8).unwrap_or(0),
            read_only: !can_edit,
            ctag: String::new(),
        });
    }
    out.calendars = Some(calendars);
    for it in items(c, "contacts")? {
        if let Some(card) = card_of(&it) {
            let etag = it.child("ItemId").and_then(|i| i.attr("ChangeKey")).unwrap_or("").to_string();
            out.cards.push(StoredCard { book: CONTACTS.into(), href: format!("contact:{}", card.uid), etag, card, dirty: false });
        }
    }
    let mut books = vec![BookInfo { id: CONTACTS.into(), name: "Contacts".into(), read_only: false, ctag: String::new() }];
    out.synced_books.push(CONTACTS.into());
    // The organisation's shared contacts and calendars kept as public folders (on-premises
    // Exchange): read-only, as Outlook shows them under Public Folders. A server without public
    // folders has none.
    match c.public_pim_folders() {
        Ok(list) => {
            for (n, f) in list.into_iter().take(MAX_PUBLIC_PIM).enumerate() {
                let (id, name, class) = (f.id, f.path, f.class);
                let key = format!("public:{id}");
                let found = match public_items(c, &id, f.replica.as_deref(), f.total) {
                    Ok(found) => found,
                    Err(e) => {
                        out.errors.push(format!("{name}: {e}"));
                        continue;
                    }
                };
                if class.starts_with("IPF.Contact") {
                    for it in found {
                        if let Some(card) = card_of(&it) {
                            out.cards.push(StoredCard {
                                book: key.clone(),
                                href: format!("public:{}", card.uid),
                                etag: String::new(),
                                card,
                                dirty: false,
                            });
                        }
                    }
                    books.push(BookInfo { id: key.clone(), name, read_only: true, ctag: String::new() });
                    out.synced_books.push(key);
                } else {
                    for it in found {
                        if let Some(event) = event_of(&it) {
                            let href = it.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or("").to_string();
                            out.events.push(StoredEvent { calendar: key.clone(), href, etag: String::new(), event, dirty: false });
                        }
                    }
                    if let Some(cals) = out.calendars.as_mut() {
                        cals.push(CalendarInfo {
                            id: key.clone(),
                            name,
                            color: u8::try_from((n + 3) % 8).unwrap_or(0),
                            read_only: true,
                            ctag: String::new(),
                        });
                    }
                    out.synced_calendars.push(key);
                }
            }
        }
        Err(e) => log::info!("no public contacts or calendars: {e}"),
    }
    out.books = Some(books);
    // Notes (Outlook's sticky notes): what changed here goes up (made again, as contacts are),
    // then the folder is read again. A mailbox without one keeps notes on this computer.
    for n in pim.notes.iter().filter(|n| n.dirty) {
        if let Some(old) = n.href.strip_prefix("note:")
            && let Err(e) = delete(c, old)
            && !matches!(e, NetError::No(_))
        {
            out.errors.push(format!("{}: {e}", n.note.title()));
            continue;
        }
        let r = c.call_one(&format!(
            "<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"notes\"/></m:SavedItemFolderId><m:Items>{}</m:Items></m:CreateItem>",
            note_xml(&n.note)
        ));
        match r {
            Ok(m) => {
                let (id, etag) = new_id(&m);
                out.pushed_notes.push(StoredNote { href: format!("note:{id}"), etag, note: n.note.clone(), dirty: false });
            }
            Err(x) => out.errors.push(format!("{}: {x}", n.note.title())),
        }
    }
    match items(c, "notes") {
        Ok(list) => out.notes = Some(list.iter().filter_map(note_of).collect()),
        Err(e) => log::info!("no Notes folder: {e}"),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
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
            phones: vec![Typed { value: "+1 555 0100".into(), kind: "cell".into() }],
            org: "Contoso & Co".into(),
            title: "Producer".into(),
            note: "Met at the fair".into(),
            extra: Vec::new(),
        };
        let xml = contact_xml(&card).replace("<t:Contact>", "<t:Contact><t:ItemId Id=\"C1\" ChangeKey=\"k\"/>");
        let back = card_of(parse(&xml).find("Contact").unwrap()).unwrap();
        assert_eq!(
            (back.name.as_str(), back.org.as_str(), back.note.as_str(), back.uid.as_str()),
            ("Ana Lima", "Contoso & Co", "Met at the fair", "C1")
        );
        assert_eq!(back.phones, card.phones);
        assert_eq!(back.emails, card.emails);
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
        assert_eq!(names.get(..4), Some(&["Subject", "Body", "Categories", "Start"][..]));
        assert!(event_fields(&e).iter().any(|(_, x)| x.contains("<t:String>Red category</t:String>")));
        let plain = CalEvent { categories: Vec::new(), ..e };
        assert!(event_fields(&plain).iter().all(|(n, _)| *n != "Categories"));
    }
}
