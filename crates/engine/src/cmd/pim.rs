//! Commands: calendar, people and notes.

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        // Accounts
        // Calendar and People
        CommandSpec::new("view.module", "Mail, Calendar, People or Notes", "Navigation", |s, v| {
            s.module = match p::req_str(v, "name")? {
                "mail" => crate::Module::Mail,
                "calendar" => crate::Module::Calendar,
                "people" => crate::Module::People,
                "notes" => crate::Module::Notes,
                other => return Err(CmdError::Params(format!("unknown module `{other}`"))),
            };
            Ok(json!({"module": s.module}))
        })
        .params(r#"{"name": "mail"|"calendar"|"people"|"notes"}"#),
        // Outlook's keys for its modules: Ctrl+1 Mail, Ctrl+2 Calendar, Ctrl+3 People, Ctrl+5 Notes.
        CommandSpec::new("view.mail", "Mail", "Navigation", |s, _| {
            s.module = crate::Module::Mail;
            Ok(json!({"module": s.module}))
        })
        .key("Mod+1"),
        CommandSpec::new("view.calendar", "Calendar", "Navigation", |s, _| {
            s.module = crate::Module::Calendar;
            Ok(json!({"module": s.module}))
        })
        .key("Mod+2"),
        CommandSpec::new("view.people", "People", "Navigation", |s, _| {
            s.module = crate::Module::People;
            Ok(json!({"module": s.module}))
        })
        .key("Mod+3"),
        CommandSpec::new("view.notes", "Notes", "Navigation", |s, _| {
            s.module = crate::Module::Notes;
            Ok(json!({"module": s.module}))
        })
        .key("Mod+5"),
        CommandSpec::new("calendar.view", "Day, week or month", "Calendar › View", |s, v| {
            s.cal_mode = match p::req_str(v, "mode")? {
                "day" => crate::CalMode::Day,
                "workWeek" | "workweek" => crate::CalMode::WorkWeek,
                "week" => crate::CalMode::Week,
                "month" => crate::CalMode::Month,
                other => return Err(CmdError::Params(format!("unknown view `{other}`"))),
            };
            s.cal_goto(None, 0)
        })
        .params(r#"{"mode": "day"|"workWeek"|"week"|"month"}"#),
        CommandSpec::new("calendar.goto", "Go to a date", "Calendar › Go to", |s, v| s.cal_goto(p::str(v, "date"), p::i64(v, "step").unwrap_or(0)))
            .params(r#"{"date"?: "YYYY-MM-DD"|"today", "step"?: number (days, weeks or months, as on screen)}"#),
        CommandSpec::new("calendar.events", "Events", "Calendar", |s, v| {
            let (d0, d1) = s.cal_days();
            let from = p::str(v, "from").and_then(|x| parse_local(x, &s.clock)).map_or(s.local_midnight(d0), |(t, all_day)| if all_day { s.clock.utc(t) } else { t });
            let to = p::str(v, "to").and_then(|x| parse_local(x, &s.clock)).map_or(s.local_midnight(d1), |(t, all_day)| if all_day { s.clock.utc(t + 86_400) } else { t });
            Ok(json!(s.agenda(from, to).iter().map(|i| json!({
                "uid": i.uid, "summary": i.summary, "location": i.location, "start": i.start, "end": i.end, "allDay": i.all_day,
                "occurrence": i.occurrence, "recurring": i.recurring, "account": i.account, "calendar": i.calendar,
                "needsReply": i.needs_reply, "status": i.status,
            })).collect::<Vec<_>>()))
        })
        .params(r#"{"from"?: date or date-time, "to"?: …} (default: what the calendar shows)"#),
        CommandSpec::new("event.create", "New event", "Calendar › New event", |s, v| {
            let mut e = event_from(s, v, mailcraft_model::ical::CalEvent::default())?;
            let account = match p::str(v, "account") {
                Some(a) => a.to_string(),
                None => s.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into()))?,
            };
            // With guests, the user organizes it.
            if !e.attendees.is_empty() && e.organizer.is_none() {
                let me = s.account(&account).map(|a| (a.config.name.clone(), a.config.email.clone())).unwrap_or_default();
                e.organizer = Some(mailcraft_model::ical::Attendee { email: me.1, name: me.0, partstat: "ACCEPTED".into(), rsvp: false });
            }
            let r = s.create_event(Some(&account), p::str(v, "calendar"), e.clone())?;
            if let Some(uid) = r.get("uid").and_then(Value::as_str) {
                e.uid = uid.to_string();
                s.send_invitations(&e, &account, "REQUEST")?;
            }
            Ok(r)
        })
        .params(r#"{"summary": string, "start": "YYYY-MM-DD[THH:MM]", "end"?: …, "allDay"?: bool, "location"?: string, "description"?: string, "repeat"?: "daily"|"weekly"|"monthly"|"yearly"|RRULE, "attendees"?: "a@x, Name <b@y>" (invited by mail), "categories"?: [string] | "Work, Clients" (colour it as those categories), "reminder"?: minutes before | "none", "account"?: string, "calendar"?: string}"#),
        CommandSpec::new("reminders.list", "Reminders", "File › Reminders", |s, _| {
            let now = s.clock.now;
            Ok(json!(s
                .due_reminders(now)
                .iter()
                .map(|r| json!({
                    "key": r.key, "uid": r.uid, "account": r.account, "summary": r.summary, "location": r.location,
                    "start": r.start, "end": r.end, "allDay": r.all_day, "occurrence": r.occurrence, "due": r.due,
                }))
                .collect::<Vec<_>>()))
        })
        .params("(the reminders due now)"),
        CommandSpec::new("reminders.dismiss", "Dismiss", "Reminders", |s, v| {
            let now = s.clock.now;
            let key = p::str(v, "key");
            if let Some(k) = key
                && !s.due_reminders(now).iter().any(|r| r.key == k)
            {
                return Err(CmdError::Params(format!("no reminder `{k}` is due")));
            }
            let n = s.dismiss_reminders(key, now);
            Ok(json!({ "dismissed": n }))
        })
        .params(r#"{"key"?: string (one reminder; none: every one due)}"#),
        CommandSpec::new("reminders.snooze", "Snooze", "Reminders", |s, v| {
            let now = s.clock.now;
            let key = p::req_str(v, "key")?.to_string();
            let Some(r) = s.due_reminders(now).into_iter().find(|r| r.key == key) else {
                return Err(CmdError::Params(format!("no reminder `{key}` is due")));
            };
            // Minutes from now, or `"beforeStart": n` (n minutes before it starts, as Outlook's
            // "0 minutes before start").
            let until = match (p::i64(v, "minutes"), p::i64(v, "beforeStart")) {
                (Some(m), _) if (1..=mailcraft_model::ical::REMINDER_MAX_MINUTES).contains(&m) => now + m * 60,
                (None, Some(b)) if (0..=mailcraft_model::ical::REMINDER_MAX_MINUTES).contains(&b) && r.start - b * 60 > now => r.start - b * 60,
                (None, Some(_)) => return Err(CmdError::Params("that time has passed".into())),
                _ => return Err(CmdError::Params("`minutes` from 1 to four weeks, or `beforeStart`".into())),
            };
            s.snooze_reminder(&key, until);
            Ok(json!({ "key": key, "until": until }))
        })
        .params(r#"{"key": string, "minutes"?: 1…40320 | "beforeStart"?: minutes before the event starts}"#),
        CommandSpec::new("event.update", "Edit event", "Calendar › Event", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            let current = s.find_event(&uid).map(|(_, e)| e.event.clone()).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
            let e = event_from(s, v, current)?;
            let r = s.update_event(&uid, |x| *x = e)?;
            // The user's own meeting: the guests hear of the change.
            if let Some((account, stored)) = s.find_event(&uid).map(|(a, e)| (a.to_string(), e.event.clone()))
                && organized_by_me(s, &stored)
            {
                s.send_invitations(&stored, &account, "REQUEST")?;
            }
            Ok(r)
        })
        .params(r#"{"uid": string, … the fields of event.create to change}"#),
        CommandSpec::new("event.delete", "Delete event", "Calendar › Event", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            let before = s.find_event(&uid).map(|(a, e)| (a.to_string(), e.event.clone()));
            let occurrence = p::i64(v, "occurrence");
            let r = s.delete_event(&uid, occurrence)?;
            if let Some((account, mut e)) = before
                && organized_by_me(s, &e)
            {
                // One occurrence off: the guests are told of that one.
                if let Some(at) = occurrence.filter(|_| e.rrule.is_some()) {
                    let len = e.end - e.start;
                    e.recurrence_id = Some(at);
                    e.rrule = None;
                    e.start = at;
                    e.end = at + len;
                }
                e.sequence = e.sequence.saturating_add(1);
                s.send_invitations(&e, &account, "CANCEL")?;
            }
            Ok(r)
        })
        .params(r#"{"uid": string, "occurrence"?: unix seconds (only that occurrence)}"#),
        CommandSpec::new("invite.respond", "Answer the invitation", "Reading pane › Invitation", |s, v| s.respond_to_invite(p::req_str(v, "answer")?))
            .params(r#"{"answer": "accepted"|"tentative"|"declined"}"#)
            .when(has_reading),
        CommandSpec::new("invite.apply", "Update the calendar", "Reading pane › Invitation", |s, _| s.apply_invite())
            .params("(an answer from a guest, or a cancellation)")
            .when(has_reading),
        CommandSpec::new("event.respond", "Answer an invitation", "Calendar › Event", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            s.respond_to_event(&uid, p::req_str(v, "answer")?)
        })
        .params(r#"{"uid": string, "answer": "accepted"|"tentative"|"declined"}"#),
        CommandSpec::new("contact.list", "People", "People", |s, v| {
            let q = p::str(v, "query").unwrap_or_default();
            Ok(json!(s.people(q).iter().map(|x| json!({"uid": x.card.uid, "name": x.card.display(), "emails": x.card.emails, "phones": x.card.phones, "org": x.card.org, "title": x.card.title, "account": x.account})).collect::<Vec<_>>()))
        })
        .params(r#"{"query"?: string}"#),
        CommandSpec::new("contact.addressBook", "Address book", "Message › Names", |s, v| {
            let source = p::str(v, "source").map_or_else(|| s.prefs.address_book.clone(), str::to_string);
            let list = s.address_book(&source, p::str(v, "query").unwrap_or_default());
            Ok(json!(list.iter().map(|a| json!({"name": a.name, "email": a.email})).collect::<Vec<_>>()))
        })
        .key("Mod+Shift+B")
        .params(r#"{"query"?: string, "source"?: "all" | "people" | "book:<account>:<book id>" (default: the one it opens with)}"#),
        CommandSpec::new("contact.searchDirectory", "Search the directory", "Message › Names › Address Book", |s, v| {
            let q = p::req_str(v, "query")?;
            s.look_up_directory(q, true);
            let people = s.directory_people(q);
            Ok(json!({
                "people": people.iter().map(|a| json!({"name": a.name, "email": a.email})).collect::<Vec<_>>(),
                "pending": s.directory_pending(q),
                "errors": s.directory_errors(q),
            }))
        })
        .when(|s| s.directory_accounts().is_empty().then_some("the organisation directory is searched through an Exchange account"))
        .params(r#"{"query": string}"#),
        CommandSpec::new("contact.allowDirectory", "Allow searching the directory", "Message › Names › Address Book", |s, v| {
            s.allow_directory(p::str(v, "account"))
        })
        .params(r#"{"account"?: string (default: each Exchange Online account waiting for permission)}"#),
        CommandSpec::new("contact.refreshDirectory", "Refresh address books", "Message › Names › Address Book", |s, _| s.refresh_directory())
            .when(|s| {
                (!s.accounts.iter().any(|a| a.config.ews.is_some() || a.config.graph.is_some()))
                    .then_some("address books are refreshed from an Exchange account")
            }),
        CommandSpec::new("contact.defaultBook", "Address book to open with", "Message › Names › Address Book", |s, v| {
            let source = p::req_str(v, "source")?.trim().to_string();
            if source != "all" && source != "people" && !s.address_books().iter().any(|(b, _, _)| *b == source) {
                return Err(CmdError::Params(format!("no address book `{source}`")));
            }
            s.prefs.address_book = source;
            Ok(json!({"source": s.prefs.address_book}))
        })
        .params(r#"{"source": "all" | "people" | "book:<account>:<book id>"}"#),
        CommandSpec::new("contact.create", "New contact", "People › New contact", |s, v| {
            let c = card_from(v, mailcraft_model::vcard::Card::default())?;
            s.create_card(p::str(v, "account"), c)
        })
        .params(
            r#"{"name": string, "emails"?: [string] | "a, b", "phones"?: [string | {"value": string, "kind"?: "work"|"home"|"cell"|"fax"|"pager"}], "addresses"?: [{"kind"?: "work"|"home", "street"?, "city"?, "region"?, "postcode"?, "country"?}], "birthday"?: "YYYY-MM-DD" | "", "org"?: string, "title"?: string, "note"?: string, "account"?: string}"#,
        ),
        CommandSpec::new("contact.update", "Edit contact", "People › Contact", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            let current = s.find_person(&uid).map(|x| x.card).ok_or_else(|| CmdError::Params(format!("no contact `{uid}`")))?;
            let c = card_from(v, current)?;
            s.update_card(&uid, |x| *x = c)
        })
        .params(r#"{"uid": string, … the fields of contact.create to change}"#),
        CommandSpec::new("contact.delete", "Delete contact", "People › Contact", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            s.delete_card(&uid)
        })
        .params(r#"{"uid": string}"#),
        CommandSpec::new("contact.fromMessage", "Add sender to contacts", "Reading pane", |s, _| {
            let from = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.from.clone()).ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
            if let Some(existing) = s.people(&from.email).into_iter().find(|x| x.card.emails.iter().any(|e| e.value.eq_ignore_ascii_case(&from.email))) {
                return Ok(json!({"uid": existing.card.uid, "existing": true}));
            }
            let card = mailcraft_model::vcard::Card {
                name: from.name.clone(),
                emails: vec![mailcraft_model::vcard::Typed { value: from.email.clone(), kind: String::new() }],
                ..Default::default()
            };
            let account = s.reading.as_ref().map(|r| r.key.account.clone());
            s.create_card(account.as_deref(), card)
        })
        .when(has_reading),
        CommandSpec::new("calendar.openShared", "Open shared calendar", "Calendar", |s, v| s.open_shared_calendar(p::str(v, "account"), p::req_str(v, "email")?))
            .params(r#"{"email": string (whose calendar), "account"?: string (an Exchange account; else the first)}"#),
        CommandSpec::new("calendar.closeShared", "Close shared calendar", "Calendar", |s, v| s.close_shared_calendar(p::req_str(v, "email")?))
            .params(r#"{"email": string}"#),
        CommandSpec::new("calendar.subscribe", "Subscribe to a calendar", "Calendar", |s, v| {
            s.subscribe_calendar(p::str(v, "account"), p::req_str(v, "url")?, p::str(v, "name"))
        })
        .params(r#"{"url": string (an iCal or webcal:// link), "name"?: string, "account"?: string (else the one on screen)}"#),
        CommandSpec::new("calendar.unsubscribe", "Remove a subscribed calendar", "Calendar", |s, v| s.unsubscribe_calendar(p::req_str(v, "calendar")?))
            .params(r#"{"calendar": string (its id, or its link)}"#),
        CommandSpec::new("calendar.refreshSubscribed", "Refresh subscribed calendars", "Calendar", |s, v| {
            s.refresh_web_calendars(p::str(v, "calendar"))
        })
        .params(r#"{"calendar"?: string (one, by its id; else all)}"#),
        CommandSpec::new("note.list", "Notes", "Notes", |s, v| {
            let list: Vec<Value> = s
                .notes(p::str(v, "query").unwrap_or(""))
                .into_iter()
                .map(|n| json!({"uid": n.note.uid, "text": n.note.text, "color": n.note.color, "modified": n.note.modified, "account": n.account}))
                .collect();
            Ok(json!({"notes": list}))
        })
        .params(r#"{"query"?: string}"#),
        CommandSpec::new("note.create", "New note", "Notes", |s, v| {
            let color = p::u64(v, "color").and_then(|c| u8::try_from(c).ok());
            s.create_note(p::str(v, "account"), p::str(v, "text").unwrap_or(""), color)
        })
        .params(r#"{"text"?: string, "color"?: 0|1|2|3|4 (blue, green, pink, yellow, white), "account"?: string}"#),
        CommandSpec::new("note.update", "Edit note", "Notes", |s, v| {
            let color = p::u64(v, "color").and_then(|c| u8::try_from(c).ok());
            s.update_note(p::req_str(v, "uid")?, p::str(v, "text"), color)
        })
        .params(r#"{"uid": string, "text"?: string, "color"?: 0|1|2|3|4}"#),
        CommandSpec::new("note.delete", "Delete note", "Notes", |s, v| s.delete_note(p::req_str(v, "uid")?)).params(r#"{"uid": string}"#),
    ]
}

/// An event from command params, starting from `e` (what's not given stays).
pub(super) fn event_from(s: &Session, v: &Value, mut e: mailcraft_model::ical::CalEvent) -> Result<mailcraft_model::ical::CalEvent, CmdError> {
    let when = |k: &str| -> Result<Option<(i64, bool)>, CmdError> {
        match p::str(v, k) {
            Some(x) => {
                parse_local(x, &s.clock).map(Some).ok_or_else(|| CmdError::Params(format!("`{x}` is not a date (YYYY-MM-DD or YYYY-MM-DDTHH:MM)")))
            }
            None => Ok(v.get(k).and_then(Value::as_i64).map(|n| (n, false))),
        }
    };
    let length = e.end - e.start;
    if let Some((t, all_day)) = when("start")? {
        e.start = t;
        e.all_day = p::bool(v, "allDay").unwrap_or(all_day);
        e.end = t + length.max(if e.all_day { 86_400 } else { 3600 });
    } else if let Some(a) = p::bool(v, "allDay") {
        e.all_day = a;
    }
    if let Some((t, all_day)) = when("end")? {
        // An all-day end date is the last day shown: the event ends the day after.
        e.end = if all_day && e.all_day { t + 86_400 } else { t };
    }
    if e.all_day {
        e.start -= e.start.rem_euclid(86_400);
        e.end = (e.end - e.end.rem_euclid(86_400)).max(e.start + 86_400);
    } else if e.tzid.is_empty()
        && let Some(zone) = s.clock.zone.and_then(|z| z.name())
    {
        // In the user's zone: it repeats at the same time of day there, summer or winter.
        e.tzid = zone.to_string();
    }
    for (k, field) in [("summary", &mut e.summary), ("location", &mut e.location), ("description", &mut e.description)] {
        if let Some(x) = p::str(v, k) {
            *field = x.chars().take(10_000).collect();
        }
    }
    if let Some(list) = p::str(v, "attendees") {
        let people = mailcraft_model::addr::parse_list(list).map_err(|bad| CmdError::Params(format!("not an email address: {bad}")))?;
        let before = std::mem::take(&mut e.attendees);
        e.attendees = people
            .into_iter()
            .take(200)
            .map(|a| {
                let known = before.iter().find(|b| b.email.eq_ignore_ascii_case(&a.email));
                mailcraft_model::ical::Attendee {
                    partstat: known.map_or_else(|| "NEEDS-ACTION".to_string(), |k| k.partstat.clone()),
                    rsvp: known.is_none_or(|k| k.rsvp),
                    email: a.email,
                    name: a.name,
                }
            })
            .collect();
    }
    let list: Option<Vec<String>> = match v.get("categories") {
        Some(Value::String(x)) => Some(x.split([',', ';']).map(str::trim).filter(|c| !c.is_empty()).map(str::to_string).collect()),
        Some(Value::Array(a)) => Some(a.iter().filter_map(Value::as_str).map(str::trim).filter(|c| !c.is_empty()).map(str::to_string).collect()),
        _ => None,
    };
    if let Some(mut list) = list {
        list.dedup();
        e.categories = list.into_iter().take(20).map(|c| c.chars().take(100).collect()).collect();
    }
    // Its reminder: minutes before it starts, or none.
    match v.get("reminder") {
        Some(Value::Null) => mailcraft_model::ical::set_reminder(&mut e, None),
        Some(Value::String(x)) if matches!(x.trim(), "" | "none") => mailcraft_model::ical::set_reminder(&mut e, None),
        Some(x) => match x.as_i64().or_else(|| x.as_str().and_then(|t| t.trim().parse().ok())) {
            Some(m) if (0..=mailcraft_model::ical::REMINDER_MAX_MINUTES).contains(&m) => mailcraft_model::ical::set_reminder(&mut e, Some(m)),
            _ => return Err(CmdError::Params("`reminder` is minutes before it starts (0 to four weeks) or none".into())),
        },
        None => {}
    }
    match p::str(v, "repeat").map(str::trim) {
        Some("" | "none" | "never") => e.rrule = None,
        Some(r @ ("daily" | "weekly" | "monthly" | "yearly")) => e.rrule = Some(format!("FREQ={}", r.to_ascii_uppercase())),
        Some(r) if r.to_ascii_uppercase().starts_with("FREQ=") && r.len() < 500 => e.rrule = Some(r.to_string()),
        Some(r) => return Err(CmdError::Params(format!("`{r}` is not a repeat"))),
        None => {}
    }
    Ok(e)
}

/// A card from command params, starting from `c`.
pub(super) fn card_from(v: &Value, mut c: mailcraft_model::vcard::Card) -> Result<mailcraft_model::vcard::Card, CmdError> {
    let list = |k: &str| -> Option<Vec<String>> {
        match v.get(k)? {
            Value::String(s) => Some(s.split([',', ';', '\n']).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect()),
            Value::Array(a) => Some(a.iter().filter_map(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect()),
            _ => None,
        }
    };
    if let Some(n) = p::str(v, "name") {
        c.name = n.trim().chars().take(300).collect();
        c.given.clear();
        c.family.clear();
    }
    use mailcraft_model::vcard::{PostalAddress, Typed};
    // A list of values, each a string or {"value", "kind"} (a phone's kind: work, home, cell,
    // fax, pager, or none).
    let typed = |k: &str| -> Option<Vec<Typed>> {
        match v.get(k)? {
            Value::Array(a) => Some(
                a.iter()
                    .filter_map(|x| match x {
                        Value::String(s) => Some(Typed { value: s.trim().to_string(), kind: String::new() }),
                        Value::Object(_) => Some(Typed {
                            value: p::str(x, "value").or_else(|| p::str(x, "number")).unwrap_or_default().trim().to_string(),
                            kind: p::str(x, "kind").unwrap_or_default().trim().to_ascii_lowercase(),
                        }),
                        _ => None,
                    })
                    .filter(|t| !t.value.is_empty())
                    .map(|mut t| {
                        t.value = t.value.chars().take(300).collect();
                        t.kind = t.kind.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(20).collect();
                        t
                    })
                    .collect(),
            ),
            Value::String(_) => list(k).map(|l| l.into_iter().map(|value| Typed { value, kind: String::new() }).collect()),
            _ => None,
        }
    };
    if let Some(e) = typed("emails") {
        c.emails = e.into_iter().take(20).collect();
    }
    if let Some(ph) = typed("phones") {
        c.phones = ph.into_iter().take(20).collect();
    }
    if let Some(list) = v.get("addresses").and_then(Value::as_array) {
        let field = |a: &Value, k: &str| p::str(a, k).unwrap_or_default().trim().chars().take(500).collect::<String>();
        c.addresses = list
            .iter()
            .filter(|a| a.is_object())
            .map(|a| PostalAddress {
                kind: field(a, "kind").to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect(),
                street: field(a, "street"),
                city: field(a, "city"),
                region: field(a, "region"),
                postcode: field(a, "postcode"),
                country: field(a, "country"),
            })
            .filter(|a| !a.is_empty())
            .take(10)
            .collect();
    }
    if let Some(b) = p::str(v, "birthday") {
        c.birthday = if b.trim().is_empty() {
            String::new()
        } else {
            mailcraft_model::vcard::birthday_of(b).ok_or_else(|| CmdError::Params(format!("`{b}` is not a date (YYYY-MM-DD)")))?
        };
    }
    for (k, field) in [("org", &mut c.org), ("title", &mut c.title), ("note", &mut c.note)] {
        if let Some(x) = p::str(v, k) {
            *field = x.chars().take(10_000).collect();
        }
    }
    Ok(c)
}

/// The user organizes this event and it has guests to tell.
pub(super) fn organized_by_me(s: &Session, e: &mailcraft_model::ical::CalEvent) -> bool {
    !e.attendees.is_empty()
        && e.organizer.as_ref().is_some_and(|o| {
            s.accounts.iter().any(|a| a.config.email.eq_ignore_ascii_case(&o.email) || a.config.username.eq_ignore_ascii_case(&o.email))
        })
}
