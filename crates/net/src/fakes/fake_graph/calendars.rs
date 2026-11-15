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
