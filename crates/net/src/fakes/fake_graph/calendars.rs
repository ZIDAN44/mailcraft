//! The fake's calendars and contacts: the account's own, and others' calendars shared with it.

use super::*;

impl Data {
    /// `/me/calendars…`, `/me/events…` and `/me/contacts…`: calendars and their events,
    /// events changed or deleted (this mailbox's own only, as Graph's `/me/events`), a series'
    /// occurrences, reminders answered, contacts.
    pub(super) fn on_pim(&mut self, method: &str, parts: &[&str], input: &Value, query: &str) -> Answer {
        match (method, parts) {
            // Not an id of Graph's at all: one starting `MALFORMED`.
            ("DELETE", ["me", "events" | "contacts", id]) if id.starts_with("MALFORMED") => error(400, "ErrorInvalidIdMalformed", "Id is malformed."),
            ("GET", ["me", "calendars"]) => {
                let list: Vec<Value> = self.calendars.iter().map(|(id, name)| json!({"id": id, "name": name, "canEdit": true})).collect();
                ok(json!({ "value": list }))
            }
            ("GET", ["me", "calendars", cal, "events"]) => {
                let list: Vec<Value> = self.events.iter().filter(|e| e["calendar"] == *cal).cloned().collect();
                ok(json!({ "value": list }))
            }
            ("POST", ["me", "calendars", cal, "events"]) => self.new_event(input, cal),
            // `/me/events` reaches only this mailbox's own events, as Graph's does.
            ("PATCH", ["me", "events", id]) => {
                match self.events.iter_mut().find(|e| e["id"] == *id && !e["calendar"].as_str().unwrap_or("").starts_with("user:")) {
                    Some(e) => {
                        merge(e, input);
                        e["changeKey"] = json!("ck2");
                        ok(e.clone())
                    }
                    None => not_found(),
                }
            }
            ("DELETE", ["me", "events", id]) => {
                self.events.retain(|e| e["id"] != *id || e["calendar"].as_str().unwrap_or("").starts_with("user:"));
                (204, Vec::new(), "text/plain")
            }
            ("GET", ["me", "contacts"]) => ok(json!({ "value": self.contacts })),
            ("POST", ["me", "contacts"]) => {
                let id = self.id("CT");
                let mut c = input.clone();
                c["id"] = json!(id);
                c["changeKey"] = json!("ck1");
                self.contacts.push(c.clone());
                answer(201, &c)
            }
            ("DELETE", ["me", "contacts", id]) => {
                if !self.contacts.iter().any(|c| c["id"] == *id) {
                    return not_found();
                }
                self.contacts.retain(|c| c["id"] != *id);
                (204, Vec::new(), "text/plain")
            }
            ("GET", ["me", "events", id, "instances"]) if param(query, "startDateTime").is_some() && param(query, "endDateTime").is_some() => {
                let list = self.instances.iter().find(|(series, _)| series == id).map(|(_, l)| l.clone());
                list.map_or_else(not_found, |l| ok(json!({ "value": l })))
            }
            ("POST", ["me", "events", id, action @ ("dismissReminder" | "snoozeReminder")]) => {
                let known = self.events.iter().any(|e| e["id"] == *id) || self.instances.iter().flat_map(|(_, l)| l).any(|o| o["id"] == *id);
                if !known {
                    return not_found();
                }
                self.reminders.push(((*action).to_string(), (*id).to_string(), input.clone()));
                (202, Vec::new(), "text/plain")
            }
            _ => not_found(),
        }
    }

    /// `/users/{address}/…`: someone else's calendar, read when shared, changed when they allow
    /// it.
    pub(super) fn on_others(&mut self, method: &str, who: &str, parts: &[&str], input: &Value) -> Answer {
        let who = who.replace("%40", "@").to_ascii_lowercase();
        let Some(edit) = self.others.iter().find(|(e, _)| *e == who).map(|(_, e)| *e) else {
            return error(404, "ErrorItemNotFound", "");
        };
        let cal = format!("user:{who}");
        match (method, parts.get(2..).unwrap_or_default()) {
            ("GET", ["calendar"]) => ok(json!({"id": cal, "canEdit": edit})),
            ("GET", ["calendar", "events"]) => {
                ok(json!({"value": self.events.iter().filter(|e| e["calendar"] == cal.as_str()).cloned().collect::<Vec<_>>()}))
            }
            (_, _) if !edit => error(403, "ErrorAccessDenied", "Access is denied."),
            ("POST", ["calendar", "events"]) => self.new_event(input, &cal),
            ("PATCH", ["events", id]) => match self.events.iter_mut().find(|e| e["id"] == *id && e["calendar"] == cal.as_str()) {
                Some(e) => {
                    merge(e, input);
                    ok(e.clone())
                }
                None => not_found(),
            },
            ("DELETE", ["events", id]) => {
                self.events.retain(|e| !(e["id"] == *id && e["calendar"] == cal.as_str()));
                (204, Vec::new(), "text/plain")
            }
            _ => not_found(),
        }
    }

    /// `getSchedule` (issue #140): each person's times taken over the window asked, in UTC, or
    /// why there are none. Refused whole past 62 days or with no one asked, as Graph does.
    pub(super) fn schedule(&mut self, input: &Value) -> Answer {
        self.schedule_asks.push(input.clone());
        let at = |k: &str| input.get(k).and_then(|t| t.get("dateTime")).and_then(Value::as_str).unwrap_or("").to_string();
        let (start, end) = (at("startTime"), at("endTime"));
        let utc = ["startTime", "endTime"].iter().all(|k| input[*k]["timeZone"] == "UTC");
        let people: Vec<String> =
            input["schedules"].as_array().map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        let span = mailcraft_jmap::utc_date(&end).zip(mailcraft_jmap::utc_date(&start)).map(|(e, s)| e - s);
        if !utc || people.is_empty() || span.is_none_or(|s| s <= 0 || s > 62 * 86_400) {
            return error(400, "ErrorInvalidParameter", "the fake refuses the window or the schedules");
        }
        let list: Vec<Value> = people
            .iter()
            .map(|who| {
                let is = |a: &str| a.eq_ignore_ascii_case(who);
                let known = self.schedules.iter().any(|(a, _)| is(a)) || self.others.iter().any(|(a, _)| is(a)) || self.directory.iter().any(|(_, a)| is(a));
                if !known {
                    return json!({"scheduleId": who, "error": {"message": "The recipient wasn't found.", "responseCode": "ErrorMailRecipientNotFound"}});
                }
                let items: Vec<Value> = self
                    .schedules
                    .iter()
                    .filter(|(a, it)| is(a) && it["start"]["dateTime"].as_str().unwrap_or("") < end.as_str() && it["end"]["dateTime"].as_str().unwrap_or("") > start.as_str())
                    .map(|(_, it)| it.clone())
                    .collect();
                let mut v = json!({"scheduleId": who, "availabilityView": "", "scheduleItems": items});
                // When they work, as Graph writes it (issue #185).
                if let Some((_, hours)) = self.working_hours.iter().find(|(a, _)| is(a)) {
                    v["workingHours"] = hours.clone();
                }
                v
            })
            .collect();
        ok(json!({ "value": list }))
    }

    /// An event made in calendar `cal` from what was posted, given its id, iCalendar uid and
    /// first change key: 201 with it.
    fn new_event(&mut self, input: &Value, cal: &str) -> Answer {
        let id = self.id("EV");
        let mut e = input.clone();
        e["id"] = json!(id);
        e["iCalUId"] = json!(format!("ical-{id}"));
        e["changeKey"] = json!("ck1");
        e["calendar"] = json!(cal);
        self.events.push(e.clone());
        answer(201, &e)
    }
}

/// A PATCH's fields over what is stored, each replaced whole.
fn merge(stored: &mut Value, patch: &Value) {
    for (k, v) in patch.as_object().cloned().unwrap_or_default() {
        stored[k.as_str()] = v;
    }
}
