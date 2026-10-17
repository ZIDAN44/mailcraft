//! Exchange Online calendars and contacts through Microsoft Graph, synced into the same calendars
//! and address books as CalDAV and CardDAV ([`DavSync`], which the engine merges).
//!
//! Events and contacts are Graph's JSON resources mapped to and from MailCraft's model: times in
//! UTC (`Prefer: outlook.timezone="UTC"`), recurrence patterns as RRULEs (RFC 5545), attendees
//! with their responses. Changes made here go first (new, changed, deleted), then every calendar
//! and the contacts are read again.

use mailcraft_model::ical::{Attendee, CalEvent};
use mailcraft_model::pim::{BookInfo, CalendarInfo, DavSync, LOCAL, Pim, StoredCard, StoredEvent};
use mailcraft_model::vcard::{Card, Typed};
use serde_json::{Value, json};

use crate::graph::Client;
use crate::{NetError, Result};

/// Most events read from one calendar, and contacts in all.
const MAX_ITEMS: usize = 5000;
/// The address book Graph's contacts go into.
pub const CONTACTS: &str = "graph:contacts";
const EVENT_FIELDS: &str = "id,iCalUId,subject,body,start,end,isAllDay,location,attendees,organizer,recurrence,changeKey,isCancelled,type";
const CONTACT_FIELDS: &str =
    "id,changeKey,displayName,givenName,surname,emailAddresses,businessPhones,homePhones,mobilePhone,companyName,jobTitle,personalNotes";

fn seg(id: &str) -> String {
    mailcraft_jmap::pct(id)
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// `{"dateTime": "2026-10-15T09:00:00.0000000", "timeZone": "UTC"}` in Unix seconds.
fn when(v: Option<&Value>) -> Option<i64> {
    mailcraft_jmap::utc_date(v?.get("dateTime")?.as_str()?)
}

fn date_time(t: i64) -> Value {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let (y, m, d) = mailcraft_model::dates::civil_from_days(days);
    json!({"dateTime": format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60), "timeZone": "UTC"})
}

fn ymd(t: i64) -> String {
    let (y, m, d) = mailcraft_model::dates::civil_from_days(t.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

const DAYS: [(&str, &str); 7] =
    [("sunday", "SU"), ("monday", "MO"), ("tuesday", "TU"), ("wednesday", "WE"), ("thursday", "TH"), ("friday", "FR"), ("saturday", "SA")];
const INDEX: [(&str, i32); 5] = [("first", 1), ("second", 2), ("third", 3), ("fourth", 4), ("last", -1)];

fn day_code(name: &str) -> Option<&'static str> {
    DAYS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, c)| *c)
}

fn day_name(code: &str) -> Option<&'static str> {
    DAYS.iter().find(|(_, c)| c.eq_ignore_ascii_case(code)).map(|(n, _)| *n)
}

/// A Graph recurrence (pattern and range) as an RRULE.
pub fn rrule_of(r: &Value) -> Option<String> {
    let p = r.get("pattern")?;
    let interval = p.get("interval").and_then(Value::as_u64).unwrap_or(1).max(1);
    let days: Vec<&str> =
        p.get("daysOfWeek").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).filter_map(day_code).collect()).unwrap_or_default();
    let index = INDEX.iter().find(|(n, _)| *n == s(p, "index")).map_or(1, |(_, i)| *i);
    let month = p.get("month").and_then(Value::as_u64).unwrap_or(0);
    let mday = p.get("dayOfMonth").and_then(Value::as_u64).unwrap_or(0);
    let by_index = days.iter().map(|d| format!("{index}{d}")).collect::<Vec<_>>().join(",");
    let mut rule = match s(p, "type").as_str() {
        "daily" => "FREQ=DAILY".to_string(),
        "weekly" if !days.is_empty() => format!("FREQ=WEEKLY;BYDAY={}", days.join(",")),
        "weekly" => "FREQ=WEEKLY".to_string(),
        "absoluteMonthly" => format!("FREQ=MONTHLY;BYMONTHDAY={mday}"),
        "relativeMonthly" => format!("FREQ=MONTHLY;BYDAY={by_index}"),
        "absoluteYearly" => format!("FREQ=YEARLY;BYMONTH={month};BYMONTHDAY={mday}"),
        "relativeYearly" => format!("FREQ=YEARLY;BYMONTH={month};BYDAY={by_index}"),
        _ => return None,
    };
    if interval > 1 {
        rule.push_str(&format!(";INTERVAL={interval}"));
    }
    let range = r.get("range").cloned().unwrap_or(Value::Null);
    match s(&range, "type").as_str() {
        "endDate" => {
            let end = s(&range, "endDate").replace('-', "");
            if end.len() == 8 && end.chars().all(|c| c.is_ascii_digit()) {
                rule.push_str(&format!(";UNTIL={end}T235959Z"));
            }
        }
        "numbered" => {
            if let Some(n) = range.get("numberOfOccurrences").and_then(Value::as_u64).filter(|n| *n > 0) {
                rule.push_str(&format!(";COUNT={n}"));
            }
        }
        _ => {}
    }
    Some(rule)
}

/// An RRULE as a Graph recurrence, starting on `start`'s day; `None` for what Graph can't say.
pub fn recurrence_of(rule: &str, start: i64) -> Option<Value> {
    let parts: Vec<(String, String)> =
        rule.split(';').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().to_ascii_uppercase())).collect();
    let get = |k: &str| parts.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str());
    let interval: u64 = get("INTERVAL").and_then(|v| v.parse().ok()).unwrap_or(1);
    let byday: Vec<(i32, &str)> = get("BYDAY")
        .map(|v| {
            v.split(',')
                .filter_map(|d| {
                    let code = d.get(d.len().saturating_sub(2)..)?;
                    let n = d.get(..d.len().saturating_sub(2)).and_then(|n| if n.is_empty() { Some(0) } else { n.parse().ok() })?;
                    Some((n, day_name(code)?))
                })
                .collect()
        })
        .unwrap_or_default();
    let (y, m, d) = mailcraft_model::dates::civil_from_days(start.div_euclid(86_400));
    let index = |n: i32| INDEX.iter().find(|(_, i)| *i == n).map_or("first", |(name, _)| *name);
    let days: Vec<&str> = byday.iter().map(|(_, d)| *d).collect();
    let pattern = match get("FREQ")? {
        "DAILY" => json!({"type": "daily", "interval": interval}),
        "WEEKLY" => {
            let start_day = DAYS.get(usize::try_from((start.div_euclid(86_400) + 4).rem_euclid(7)).unwrap_or(0)).map_or("monday", |(n, _)| *n);
            json!({"type": "weekly", "interval": interval, "daysOfWeek": if days.is_empty() { vec![start_day] } else { days }})
        }
        "MONTHLY" => match byday.first() {
            Some((n, _)) => json!({"type": "relativeMonthly", "interval": interval, "daysOfWeek": days, "index": index(*n)}),
            None => {
                json!({"type": "absoluteMonthly", "interval": interval, "dayOfMonth": get("BYMONTHDAY").and_then(|v| v.parse::<u32>().ok()).unwrap_or(d)})
            }
        },
        "YEARLY" => {
            let month = get("BYMONTH").and_then(|v| v.parse::<u32>().ok()).unwrap_or(m);
            match byday.first() {
                Some((n, _)) => json!({"type": "relativeYearly", "interval": interval, "month": month, "daysOfWeek": days, "index": index(*n)}),
                None => {
                    json!({"type": "absoluteYearly", "interval": interval, "month": month, "dayOfMonth": get("BYMONTHDAY").and_then(|v| v.parse::<u32>().ok()).unwrap_or(d)})
                }
            }
        }
        _ => return None,
    };
    let start_date = format!("{y:04}-{m:02}-{d:02}");
    let range = if let Some(u) = get("UNTIL").filter(|u| u.len() >= 8) {
        json!({"type": "endDate", "startDate": start_date, "endDate": format!("{}-{}-{}", &u[0..4], &u[4..6], &u[6..8])})
    } else if let Some(n) = get("COUNT").and_then(|v| v.parse::<u64>().ok()) {
        json!({"type": "numbered", "startDate": start_date, "numberOfOccurrences": n})
    } else {
        json!({"type": "noEnd", "startDate": start_date})
    };
    Some(json!({"pattern": pattern, "range": range}))
}

fn partstat(response: &str) -> &'static str {
    match response {
        "accepted" | "organizer" => "ACCEPTED",
        "declined" => "DECLINED",
        "tentativelyAccepted" => "TENTATIVE",
        _ => "NEEDS-ACTION",
    }
}

fn person(v: &Value) -> Option<Attendee> {
    let e = v.get("emailAddress")?;
    let email = s(e, "address");
    (!email.is_empty()).then(|| Attendee { email, name: s(e, "name"), partstat: String::new(), rsvp: false })
}

/// A Graph event as an event.
pub fn event_of(v: &Value) -> Option<CalEvent> {
    let all_day = v.get("isAllDay").and_then(Value::as_bool).unwrap_or(false);
    let start = when(v.get("start"))?;
    let end = when(v.get("end")).unwrap_or(start + 3600).max(start);
    let body = v.get("body").cloned().unwrap_or(Value::Null);
    let content = s(&body, "content");
    let description = if s(&body, "contentType").eq_ignore_ascii_case("html") { mailcraft_mime::html_to_text(&content) } else { content };
    let attendees = v
        .get("attendees")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(500)
                .filter_map(|x| {
                    let mut p = person(x)?;
                    p.partstat = partstat(&x.get("status").map(|st| s(st, "response")).unwrap_or_default()).to_string();
                    p.rsvp = true;
                    Some(p)
                })
                .collect()
        })
        .unwrap_or_default();
    let uid = [s(v, "iCalUId"), s(v, "id")].into_iter().find(|u| !u.is_empty())?;
    Some(CalEvent {
        uid,
        summary: s(v, "subject"),
        description: description.trim().to_string(),
        location: v.get("location").map(|l| s(l, "displayName")).unwrap_or_default(),
        start,
        end,
        all_day,
        rrule: v.get("recurrence").filter(|r| !r.is_null()).and_then(rrule_of),
        organizer: v.get("organizer").and_then(person),
        attendees,
        status: if v.get("isCancelled").and_then(Value::as_bool).unwrap_or(false) { "CANCELLED".into() } else { String::new() },
        ..Default::default()
    })
}

/// An event as Graph's JSON (for creating or changing it).
pub fn event_json(e: &CalEvent) -> Value {
    let mut v = json!({
        "subject": e.summary,
        "body": {"contentType": "text", "content": e.description},
        "location": {"displayName": e.location},
        "isAllDay": e.all_day,
        "start": if e.all_day { json!({"dateTime": format!("{}T00:00:00", ymd(e.start)), "timeZone": "UTC"}) } else { date_time(e.start) },
        "end": if e.all_day { json!({"dateTime": format!("{}T00:00:00", ymd(e.end.max(e.start + 86_400))), "timeZone": "UTC"}) } else { date_time(e.end) },
        "attendees": e.attendees.iter().map(|a| json!({"type": "required", "emailAddress": {"address": a.email, "name": a.name}})).collect::<Vec<_>>(),
    });
    v["recurrence"] = e.rrule.as_deref().and_then(|r| recurrence_of(r, e.start)).unwrap_or(Value::Null);
    v
}

/// A Graph contact as a card.
pub fn card_of(v: &Value) -> Option<Card> {
    let id = s(v, "id");
    if id.is_empty() {
        return None;
    }
    let typed = |value: String, kind: &str| Typed { value, kind: kind.to_string() };
    let mut phones: Vec<Typed> = Vec::new();
    for (k, kind) in [("businessPhones", "work"), ("homePhones", "home")] {
        phones.extend(v.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(|p| typed(p.to_string(), kind)));
    }
    if let Some(m) = v.get("mobilePhone").and_then(Value::as_str).filter(|m| !m.is_empty()) {
        phones.push(typed(m.to_string(), "cell"));
    }
    Some(Card {
        uid: id,
        name: s(v, "displayName"),
        given: s(v, "givenName"),
        family: s(v, "surname"),
        emails: v
            .get("emailAddresses")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|e| s(e, "address"))
            .filter(|a| !a.is_empty())
            .map(|a| typed(a, ""))
            .collect(),
        phones,
        org: s(v, "companyName"),
        title: s(v, "jobTitle"),
        note: s(v, "personalNotes"),
        extra: Vec::new(),
    })
}

/// A card as Graph's JSON.
pub fn card_json(c: &Card) -> Value {
    let phones = |kind: &str| c.phones.iter().filter(|p| p.kind.to_ascii_lowercase().contains(kind)).map(|p| p.value.clone()).collect::<Vec<_>>();
    let mut work = phones("work");
    let other: Vec<String> = c.phones.iter().filter(|p| p.kind.is_empty()).map(|p| p.value.clone()).collect();
    work.extend(other);
    json!({
        "displayName": c.display(),
        "givenName": c.given,
        "surname": c.family,
        "emailAddresses": c.emails.iter().map(|e| json!({"address": e.value, "name": c.display()})).collect::<Vec<_>>(),
        "businessPhones": work,
        "homePhones": phones("home"),
        "mobilePhone": phones("cell").first().cloned().unwrap_or_default(),
        "companyName": c.org,
        "jobTitle": c.title,
        "personalNotes": c.note,
    })
}

/// Where a calendar's events live: `/me`, or `/users/<address>` for someone else's.
fn owner_path(calendar: &str) -> String {
    match mailcraft_model::pim::shared_calendar_owner(calendar) {
        Some(email) => format!("/users/{}", seg(email)),
        None => "/me".into(),
    }
}

/// Sync an account's calendars and contacts with Exchange Online.
pub fn sync(c: &Client, pim: &Pim, now: i64) -> Result<DavSync> {
    let mut out = DavSync { caldav_home: Some("graph".into()), carddav_home: Some("graph".into()), checked: now, ..Default::default() };
    // The calendars.
    let cals = c.all("/me/calendars?$top=100&$select=id,name,canEdit,hexColor", 200)?;
    let calendars: Vec<CalendarInfo> = cals
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            let id = s(v, "id");
            (!id.is_empty()).then(|| CalendarInfo {
                id,
                name: s(v, "name"),
                color: u8::try_from(i % 8).unwrap_or(0),
                read_only: !v.get("canEdit").and_then(Value::as_bool).unwrap_or(true),
                ctag: String::new(),
            })
        })
        .collect();
    let default_cal = calendars.iter().find(|x| !x.read_only).map(|x| x.id.clone());
    // What changed here goes first.
    for t in &pim.deleted {
        let path = if t.href.starts_with("contact:") {
            format!("/me/contacts/{}", seg(t.href.trim_start_matches("contact:")))
        } else {
            format!("{}/events/{}", owner_path(&t.calendar), seg(&t.href))
        };
        match c.send_raw("DELETE", &path, "application/json", Vec::new()) {
            Ok(_) | Err(NetError::No(_)) => out.deleted.push(t.href.clone()),
            Err(e) => out.errors.push(e.to_string()),
        }
    }
    for e in pim.events.iter().filter(|e| e.dirty) {
        let body = event_json(&e.event);
        let shared = mailcraft_model::pim::shared_calendar_owner(&e.calendar).is_some();
        let r = if e.href.is_empty() && shared {
            // Into someone else's calendar (they let this account edit it).
            c.post(&format!("{}/calendar/events", owner_path(&e.calendar)), &body).map(|v| (v, e.calendar.clone()))
        } else if e.href.is_empty() {
            let cal =
                if e.calendar == LOCAL || !calendars.iter().any(|x| x.id == e.calendar) { default_cal.clone() } else { Some(e.calendar.clone()) };
            let Some(cal) = cal else { continue };
            c.post(&format!("/me/calendars/{}/events", seg(&cal)), &body).map(|v| (v, cal))
        } else {
            let b = serde_json::to_vec(&body).map_err(|x| NetError::Protocol(x.to_string()))?;
            c.send_raw("PATCH", &format!("{}/events/{}", owner_path(&e.calendar), seg(&e.href)), "application/json", b)
                .map(|v| (v, e.calendar.clone()))
        };
        match r {
            Ok((v, cal)) => out.pushed_events.push(StoredEvent {
                calendar: cal,
                href: s(&v, "id"),
                etag: s(&v, "changeKey"),
                event: e.event.clone(),
                dirty: false,
            }),
            Err(x) => out.errors.push(format!("{}: {x}", e.event.summary)),
        }
    }
    for k in pim.cards.iter().filter(|k| k.dirty) {
        let body = card_json(&k.card);
        let r = match k.href.strip_prefix("contact:") {
            Some(id) => {
                let b = serde_json::to_vec(&body).map_err(|x| NetError::Protocol(x.to_string()))?;
                c.send_raw("PATCH", &format!("/me/contacts/{}", seg(id)), "application/json", b)
            }
            None => c.post("/me/contacts", &body),
        };
        match r {
            Ok(v) => out.pushed_cards.push(StoredCard {
                book: CONTACTS.into(),
                href: format!("contact:{}", s(&v, "id")),
                etag: s(&v, "changeKey"),
                card: k.card.clone(),
                dirty: false,
            }),
            Err(x) => out.errors.push(format!("{}: {x}", k.card.display())),
        }
    }
    // Then everything, read again.
    for cal in &calendars {
        let list = c.all(&format!("/me/calendars/{}/events?$top=100&$select={EVENT_FIELDS}", seg(&cal.id)), MAX_ITEMS)?;
        for v in &list {
            if let Some(event) = event_of(v) {
                out.events.push(StoredEvent { calendar: cal.id.clone(), href: s(v, "id"), etag: s(v, "changeKey"), event, dirty: false });
            }
        }
        out.synced_calendars.push(cal.id.clone());
    }
    // Other people's calendars opened here, by their address: editable where their owner let this
    // account edit them (Calendars.ReadWrite.Shared), read-only otherwise.
    let mut calendars = calendars;
    for email in &pim.shared {
        let id = mailcraft_model::pim::shared_calendar_id(email);
        let n = calendars.len();
        let can_edit = c.get(&format!("/users/{}/calendar?$select=canEdit", seg(email))).ok().and_then(|v| v.get("canEdit").and_then(Value::as_bool));
        match c.all(&format!("/users/{}/calendar/events?$top=100&$select={EVENT_FIELDS}", seg(email)), MAX_ITEMS) {
            Ok(list) => {
                for v in &list {
                    if let Some(event) = event_of(v) {
                        out.events.push(StoredEvent { calendar: id.clone(), href: s(v, "id"), etag: s(v, "changeKey"), event, dirty: false });
                    }
                }
                out.synced_calendars.push(id.clone());
            }
            Err(e) => out.errors.push(format!("{email}'s calendar: {e}")),
        }
        calendars.push(CalendarInfo {
            id,
            name: email.clone(),
            color: u8::try_from(n % 8).unwrap_or(0),
            read_only: can_edit != Some(true),
            ctag: String::new(),
        });
    }
    out.calendars = Some(calendars);
    let contacts = c.all(&format!("/me/contacts?$top=100&$select={CONTACT_FIELDS}"), MAX_ITEMS)?;
    for v in &contacts {
        if let Some(card) = card_of(v) {
            out.cards.push(StoredCard { book: CONTACTS.into(), href: format!("contact:{}", card.uid), etag: s(v, "changeKey"), card, dirty: false });
        }
    }
    out.books = Some(vec![BookInfo { id: CONTACTS.into(), name: "Contacts".into(), read_only: false, ctag: String::new() }]);
    out.synced_books.push(CONTACTS.into());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recurrences_both_ways() {
        let weekly = json!({"pattern": {"type": "weekly", "interval": 2, "daysOfWeek": ["monday", "wednesday"]}, "range": {"type": "endDate", "startDate": "2026-10-01", "endDate": "2026-12-31"}});
        assert_eq!(rrule_of(&weekly).as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE;INTERVAL=2;UNTIL=20261231T235959Z"));
        let monthly = json!({"pattern": {"type": "relativeMonthly", "interval": 1, "daysOfWeek": ["friday"], "index": "last"}, "range": {"type": "numbered", "numberOfOccurrences": 6}});
        assert_eq!(rrule_of(&monthly).as_deref(), Some("FREQ=MONTHLY;BYDAY=-1FR;COUNT=6"));
        let yearly = json!({"pattern": {"type": "absoluteYearly", "month": 3, "dayOfMonth": 14}, "range": {"type": "noEnd"}});
        assert_eq!(rrule_of(&yearly).as_deref(), Some("FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=14"));
        assert_eq!(rrule_of(&json!({"pattern": {"type": "hourly"}})), None);
        // And back.
        let start = mailcraft_jmap::utc_date("2026-10-05T09:00:00Z").unwrap();
        let r = recurrence_of("FREQ=WEEKLY;BYDAY=MO,WE;INTERVAL=2;UNTIL=20261231T235959Z", start).unwrap();
        assert_eq!(r["pattern"]["daysOfWeek"], json!(["monday", "wednesday"]));
        assert_eq!((r["pattern"]["interval"].as_u64(), r["range"]["endDate"].as_str()), (Some(2), Some("2026-12-31")));
        let r = recurrence_of("FREQ=MONTHLY;BYDAY=-1FR;COUNT=6", start).unwrap();
        assert_eq!(
            (r["pattern"]["type"].as_str(), r["pattern"]["index"].as_str(), r["range"]["numberOfOccurrences"].as_u64()),
            (Some("relativeMonthly"), Some("last"), Some(6))
        );
        let r = recurrence_of("FREQ=WEEKLY", start).unwrap();
        assert_eq!(r["pattern"]["daysOfWeek"], json!(["monday"]), "the start's own day");
        assert!(recurrence_of("FREQ=SECONDLY", start).is_none() && recurrence_of("junk", start).is_none());
    }

    #[test]
    fn events_and_contacts_both_ways() {
        let v = json!({
            "id": "E1", "iCalUId": "uid-1", "subject": "Mix review", "changeKey": "ck1",
            "body": {"contentType": "html", "content": "<p>Bring the <b>stems</b></p>"},
            "start": {"dateTime": "2026-10-16T14:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-10-16T15:00:00.0000000", "timeZone": "UTC"},
            "location": {"displayName": "Lab"},
            "organizer": {"emailAddress": {"name": "Ana", "address": "ana@contoso.example"}},
            "attendees": [{"type": "required", "status": {"response": "tentativelyAccepted"}, "emailAddress": {"name": "Me", "address": "me@contoso.example"}}],
        });
        let e = event_of(&v).unwrap();
        assert_eq!((e.uid.as_str(), e.summary.as_str(), e.location.as_str()), ("uid-1", "Mix review", "Lab"));
        assert_eq!(e.end - e.start, 3600);
        assert!(e.description.contains("stems"));
        assert_eq!(e.attendees[0].partstat, "TENTATIVE");
        assert_eq!(e.organizer.as_ref().unwrap().email, "ana@contoso.example");
        let back = event_json(&e);
        assert_eq!(back["start"]["dateTime"], "2026-10-16T14:00:00");
        assert_eq!(back["subject"], "Mix review");
        let all_day = CalEvent { all_day: true, start: e.start - 14 * 3600, end: e.start - 14 * 3600 + 86_400, ..e.clone() };
        assert_eq!(event_json(&all_day)["end"]["dateTime"], "2026-10-17T00:00:00");
        assert!(event_of(&json!({"id": "x"})).is_none(), "no start, no event");

        let c = json!({"id": "C1", "displayName": "Ana Lima", "givenName": "Ana", "surname": "Lima",
            "emailAddresses": [{"name": "Ana", "address": "ana@contoso.example"}], "businessPhones": ["+1 555 0100"],
            "mobilePhone": "+1 555 0101", "companyName": "Sound Lab", "jobTitle": "Mixer"});
        let card = card_of(&c).unwrap();
        assert_eq!(
            (card.display(), card.emails[0].value.as_str(), card.phones.len(), card.org.as_str()),
            ("Ana Lima".to_string(), "ana@contoso.example", 2, "Sound Lab")
        );
        let back = card_json(&card);
        assert_eq!((back["mobilePhone"].as_str(), back["businessPhones"][0].as_str()), (Some("+1 555 0101"), Some("+1 555 0100")));
    }
}
