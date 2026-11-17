//! UI state kept between app runs.

use super::*;
use serde_json::{Value, json};

fn persisted_uid(value: &Value) -> Option<Option<&str>> {
    match value.get("uid") {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(uid)) if !uid.is_empty() && uid.len() <= 1024 && !uid.chars().any(char::is_control) => Some(Some(uid)),
        _ => None,
    }
}

fn capped_text(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    &text[..end]
}

impl MailApp {
    /// Preferences, theme, the last module, open message or PIM item, mail view, and calendar view.
    pub fn saved_state(&self) -> Value {
        let calendar_event = if self.session.module == mailcraft_engine::Module::Calendar {
            self.ui.dialog.as_ref().and_then(|dialog| match dialog {
                Dialog::Event(form) => form.uid.as_ref().zip(form.occurrence).map(|(uid, occurrence)| json!({"uid": uid, "occurrence": occurrence})),
                _ => None,
            })
        } else {
            None
        };
        json!({
            "prefs": self.session.prefs,
            "dark": self.ui.dark,
            "language": self.ui.language,
            "windows": self.ui.places,
            "peopleView": self.ui.people_view,
            "peopleList": self.ui.people_list,
            "person": self.ui.person.as_ref().filter(|_| self.session.module == mailcraft_engine::Module::People),
            "task": self.ui.task.as_ref().filter(|_| self.session.module == mailcraft_engine::Module::Tasks),
            "mailView": self.session.view,
            "mailFilter": self.session.filter,
            "focusOther": self.session.focus_other,
            "moduleSearch": self.module_search(),
            "taskFormDraft": self.task_form_draft(),
            "eventFormDraft": self.event_form_draft(),
            "contactFormDraft": self.contact_form_draft(),
            "search": self
                .session
                .search
                .as_ref()
                .filter(|(query, _)| query.len() <= 4096)
                .map(|(query, scope)| json!({ "query": query, "scope": scope })),
            "openMessage": self.session.reading.as_ref().map(|reading| reading.key.to_id()),
            "module": self.session.module,
            "calendarMode": self.session.cal_mode,
            "calendarDay": self.session.cal_day,
            "calendarEvent": calendar_event
        })
    }

    pub fn restore_state(&mut self, v: &Value) {
        self.ui.person = None;
        self.ui.task = None;
        self.ui.phone_reader_open = false;
        if let Some(p) = v.get("prefs").and_then(|p| serde_json::from_value::<mailcraft_engine::Prefs>(p.clone()).ok()) {
            let offline = p.work_offline;
            let keep = (p.keep_messages, p.keep_offline);
            self.session.prefs = p;
            self.session.refresh_list();
            // How much mail to keep, before any account connects (when it isn't the default).
            let d = mailcraft_engine::Prefs::default();
            if keep != (d.keep_messages, d.keep_offline) {
                self.session.apply_sync_prefs();
            }
            // Still working offline from last time: before any account connects.
            if offline {
                self.session.set_work_offline(true);
            }
        }
        self.ui.dark = v.get("dark").and_then(Value::as_bool);
        if let Some(pv) = v.get("peopleView").and_then(|w| serde_json::from_value(w.clone()).ok()) {
            self.ui.people_view = pv;
        }
        if let Some(pl) = v.get("peopleList").and_then(|w| serde_json::from_value(w.clone()).ok()) {
            self.ui.people_list = pl;
        }
        if let Some(w) = v.get("windows").and_then(|w| serde_json::from_value(w.clone()).ok()) {
            self.ui.places = w;
        }
        if let Some(l) = v.get("language").and_then(Value::as_str).filter(|l| *l == i18n::AUTO || i18n::Lang::from_code(l).is_some()) {
            self.ui.language = l.to_string();
        }
        if let Some(module) = v.get("module").and_then(|module| serde_json::from_value::<mailcraft_engine::Module>(module.clone()).ok()) {
            self.session.module = module;
        }
        if let Some(filter) = v.get("mailFilter").and_then(|filter| serde_json::from_value::<mailcraft_engine::ListFilter>(filter.clone()).ok()) {
            self.session.filter = filter;
        }
        if let Some(focus_other) = v.get("focusOther").and_then(Value::as_bool) {
            self.session.focus_other = focus_other;
        }
        if let Some(query) = v
            .get("moduleSearch")
            .and_then(Value::as_str)
            .filter(|query| query.len() <= 4096 && !query.chars().any(char::is_control) && !query.trim().is_empty())
        {
            match self.session.module {
                mailcraft_engine::Module::Calendar => self.ui.cal_search.query = query.to_owned(),
                mailcraft_engine::Module::People => self.ui.people_query = query.to_owned(),
                mailcraft_engine::Module::Tasks => self.ui.tasks_query = query.to_owned(),
                mailcraft_engine::Module::Notes => self.ui.notes_query = query.to_owned(),
                mailcraft_engine::Module::Mail => {}
            }
        }
        if let Some(mode) = v.get("calendarMode").and_then(|mode| serde_json::from_value::<mailcraft_engine::CalMode>(mode.clone()).ok()) {
            self.session.cal_mode = mode;
        }
        // Calendar arithmetic converts this back to a civil date and computes adjacent days.
        // Ignore malformed or out-of-range persisted values rather than letting them overflow.
        if let Some(day) = v.get("calendarDay").and_then(Value::as_i64).filter(|day| (-719_162..=2_932_896).contains(day)) {
            self.session.cal_day = Some(day);
        }
        if self.session.module == mailcraft_engine::Module::Calendar
            && let Some((uid, occurrence)) = v.get("calendarEvent").and_then(|event| {
                let uid = event.get("uid")?.as_str()?;
                let occurrence = event.get("occurrence")?.as_i64()?;
                let date_limits = (-719_162_i64 * 86_400)..=(2_932_896_i64 * 86_400);
                (uid.len() <= 1024 && !uid.chars().any(char::is_control) && date_limits.contains(&occurrence)).then_some((uid, occurrence))
            })
            && let Some(dialog) = Dialog::open_event(&self.session, uid, occurrence)
        {
            self.ui.dialog = Some(dialog);
        }
        if let Some(view) = v.get("mailView").and_then(|view| serde_json::from_value::<mailcraft_engine::View>(view.clone()).ok()) {
            let safe = match &view {
                mailcraft_engine::View::Folder { account, folder } => {
                    account.len() <= 256
                        && !account.chars().any(char::is_control)
                        && folder.len() <= 4096
                        && !folder.chars().any(char::is_control)
                        && self.session.account(account).is_some()
                }
                mailcraft_engine::View::Unified { .. } | mailcraft_engine::View::Outbox => true,
                mailcraft_engine::View::Empty => false,
            };
            if safe {
                self.session.open_view(view);
            }
        }
        if self.session.module == mailcraft_engine::Module::Mail
            && let Some((query, scope)) = v.get("search").and_then(|search| {
                let query = search.get("query")?.as_str()?;
                let scope = search.get("scope").and_then(|scope| serde_json::from_value::<mailcraft_engine::SearchScope>(scope.clone()).ok())?;
                (query.len() <= 4096 && !query.chars().any(char::is_control) && !query.trim().is_empty()).then(|| (query.to_owned(), scope))
            })
        {
            self.ui.search_text = query.clone();
            self.session.prefs.search_scope = scope;
            // Search the local cache in the background, so reopening a large mailbox never
            // holds the Android window while it restores the previous results.
            self.session.run_search_with(&query, scope, true);
        }
        if self.session.module == mailcraft_engine::Module::Mail
            && let Some(key) = v.get("openMessage").and_then(Value::as_str).and_then(mailcraft_engine::model::MessageKey::from_id)
            && key.account.len() <= 256
            && !key.account.chars().any(char::is_control)
            && key.folder.len() <= 4096
            && !key.folder.chars().any(char::is_control)
            && self.session.account(&key.account).is_some()
            && (self.session.summary(&key).is_some()
                || (!key.folder.starts_with('\0')
                    && self
                        .session
                        .store
                        .as_ref()
                        .and_then(|store| store.load_folder(&key.account, &key.folder).ok())
                        .is_some_and(|folder| folder.messages.iter().any(|summary| summary.key == key))))
        {
            self.session.open_message(&key);
            self.ui.phone_reader_open = true;
            self.ui.scroll_to_focus = true;
        }
        if self.session.module == mailcraft_engine::Module::People
            && let Some(uid) = v.get("person").and_then(Value::as_str)
            && uid.len() <= 1024
            && !uid.chars().any(char::is_control)
            && self.session.find_person(uid).is_some()
        {
            self.ui.person = Some(uid.to_owned());
        }
        if self.session.module == mailcraft_engine::Module::Tasks
            && let Some(uid) = v.get("task").and_then(Value::as_str)
            && uid.len() <= 1024
            && !uid.chars().any(char::is_control)
            && self.session.find_task(uid).is_some()
        {
            self.ui.task = Some(uid.to_owned());
        }
        if let Some(draft) = v.get("taskFormDraft")
            && let Some(form) = self.restore_task_form(draft)
        {
            self.ui.dialog = Some(Dialog::Task(Box::new(form)));
        }
        if let Some(draft) = v.get("eventFormDraft")
            && let Some(form) = self.restore_event_form(draft)
        {
            self.ui.dialog = Some(Dialog::Event(Box::new(form)));
        }
        if let Some(draft) = v.get("contactFormDraft")
            && let Some(form) = self.restore_contact_form(draft)
        {
            self.ui.dialog = Some(Dialog::Contact(Box::new(form)));
        }
    }

    fn event_form_draft(&self) -> Option<Value> {
        let Some(Dialog::Event(form)) = self.pending_form() else { return None };
        let calendar = form.calendars.get(form.calendar).map(|(account, id, _)| (account, id));
        let meeting_change = match &form.meeting.change {
            None => None,
            Some(None) => Some(json!({"remove": true})),
            Some(Some(room)) => Some(json!({"room": room})),
        };
        Some(json!({
            "uid": &form.uid,
            "occurrence": form.occurrence,
            "calendar": calendar,
            "summary": capped_text(&form.summary, 4096),
            "location": capped_text(&form.location, 4096),
            "description": capped_text(&form.description, 65_536),
            "date": capped_text(&form.date, 32),
            "start": capped_text(&form.start, 32),
            "endDate": capped_text(&form.end_date, 32),
            "end": capped_text(&form.end, 32),
            "allDay": form.all_day,
            "repeat": capped_text(&form.repeat, 4096),
            "reminder": form.reminder,
            "privacy": form.privacy,
            "showAs": form.show_as,
            "guests": capped_text(&form.guests, 16_384),
            "categories": form.categories.iter().take(100).map(|name| capped_text(name, 256)).collect::<Vec<_>>(),
            "meetingCurrent": form.meeting.current.as_deref().map(|url| capped_text(url, mailcraft_engine::model::ical::URL_MAX)),
            "meetingChange": meeting_change,
        }))
    }

    fn restore_event_form(&self, value: &Value) -> Option<crate::pim_dialogs::EventForm> {
        let mut form = match persisted_uid(value)? {
            Some(uid) => {
                let occurrence = value.get("occurrence").and_then(Value::as_i64)?;
                let valid_date = (-719_162_i64 * 86_400)..=(2_932_896_i64 * 86_400);
                if !valid_date.contains(&occurrence) {
                    return None;
                }
                crate::pim_dialogs::EventForm::open(&self.session, uid, occurrence)?
            }
            None => match Dialog::new_event(&self.session, None) {
                Dialog::Event(form) => *form,
                _ => return None,
            },
        };
        let text = |key: &str, max: usize| -> Option<String> {
            value.get(key).and_then(Value::as_str).filter(|text| text.len() <= max && !text.chars().any(char::is_control)).map(str::to_owned)
        };
        let body_text = |key: &str, max: usize| -> Option<String> {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| text.len() <= max && !text.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')))
                .map(str::to_owned)
        };
        if let Some(calendar) = value.get("calendar").and_then(Value::as_array).filter(|parts| parts.len() == 2)
            && let (Some(account), Some(id)) = (calendar[0].as_str(), calendar[1].as_str())
            && let Some(index) =
                form.calendars.iter().position(|(candidate_account, candidate_id, _)| candidate_account == account && candidate_id == id)
        {
            form.calendar = index;
        }
        if let Some(summary) = text("summary", 4096) {
            form.summary = summary;
        }
        if let Some(location) = text("location", 4096) {
            form.location = location;
        }
        if let Some(description) = body_text("description", 65_536) {
            form.description = description;
        }
        if let Some(date) = text("date", 32) {
            form.date = date;
        }
        if let Some(start) = text("start", 32) {
            form.start = start;
        }
        if let Some(date) = text("endDate", 32) {
            form.end_date = date;
        }
        if let Some(end) = text("end", 32) {
            form.end = end;
        }
        if let Some(all_day) = value.get("allDay").and_then(Value::as_bool) {
            form.all_day = all_day;
        }
        if let Some(repeat) = text("repeat", 4096) {
            form.repeat = repeat;
        }
        if let Some(reminder) = value.get("reminder").and_then(Value::as_i64).filter(|minutes| (0..=20_160).contains(minutes)) {
            form.reminder = Some(reminder);
        } else if value.get("reminder").is_some_and(Value::is_null) {
            form.reminder = None;
        }
        if let Some(privacy) =
            value.get("privacy").and_then(|privacy| serde_json::from_value::<mailcraft_engine::model::ical::Privacy>(privacy.clone()).ok())
        {
            form.privacy = privacy;
        }
        if let Some(show_as) =
            value.get("showAs").and_then(|show_as| serde_json::from_value::<mailcraft_engine::model::ical::ShowAs>(show_as.clone()).ok())
        {
            form.show_as = show_as;
        }
        if let Some(guests) = text("guests", 16_384) {
            form.guests = guests;
        }
        if let Some(categories) = value.get("categories").and_then(Value::as_array) {
            form.categories = categories
                .iter()
                .filter_map(Value::as_str)
                .filter(|name| name.len() <= 256 && !name.chars().any(char::is_control))
                .filter_map(|name| form.all_categories.iter().find(|(available, _)| available.eq_ignore_ascii_case(name)))
                .take(100)
                .map(|(name, _)| name.clone())
                .collect();
        }
        if let Some(current) =
            value.get("meetingCurrent").and_then(Value::as_str).filter(|url| mailcraft_engine::model::ical::meeting_url(url).as_deref() == Some(*url))
        {
            form.meeting.current = Some(current.to_owned());
        }
        if let Some(change) = value.get("meetingChange") {
            if change.get("remove").and_then(Value::as_bool) == Some(true) {
                form.meeting.change = Some(None);
                form.meeting.current = None;
            } else if let Some(room) = change
                .get("room")
                .and_then(|room| serde_json::from_value::<mailcraft_engine::model::ical::MeetingRoom>(room.clone()).ok())
                .and_then(|room| mailcraft_engine::model::ical::MeetingRoom::new(&room.name, &room.url, &room.code, &room.details))
            {
                form.meeting.current = Some(room.url.clone());
                form.meeting.change = Some(Some(room));
            }
        }
        Some(form)
    }

    fn contact_form_draft(&self) -> Option<Value> {
        let Some(Dialog::Contact(form)) = self.pending_form() else { return None };
        Some(json!({
            "uid": &form.uid,
            "name": capped_text(&form.name, 4096),
            "emails": form.emails.iter().map(|value| capped_text(value, 4096)).collect::<Vec<_>>(),
            "moreEmails": form.more_emails.iter().take(100).map(|value| capped_text(value, 4096)).collect::<Vec<_>>(),
            "phones": form.phones.iter().map(|value| capped_text(value, 4096)).collect::<Vec<_>>(),
            "morePhones": form.more_phones.iter().take(100).map(|phone| json!({
                "value": capped_text(&phone.value, 4096),
                "kind": capped_text(&phone.kind, 128),
            })).collect::<Vec<_>>(),
            "addresses": form.addresses.iter().map(|address| json!({
                "kind": capped_text(&address.kind, 128),
                "street": capped_text(&address.street, 4096),
                "city": capped_text(&address.city, 1024),
                "region": capped_text(&address.region, 1024),
                "postcode": capped_text(&address.postcode, 128),
                "country": capped_text(&address.country, 1024),
            })).collect::<Vec<_>>(),
            "addressShown": form.address_shown,
            "birthday": capped_text(&form.birthday, 32),
            "org": capped_text(&form.org, 4096),
            "title": capped_text(&form.title, 4096),
            "note": capped_text(&form.note, 65_536),
        }))
    }

    fn restore_contact_form(&self, value: &Value) -> Option<crate::pim_dialogs::ContactForm> {
        let mut form = match persisted_uid(value)? {
            Some(uid) => crate::pim_dialogs::ContactForm::edit(self.session.find_person(uid)?.card),
            None => crate::pim_dialogs::ContactForm::default(),
        };
        let line = |value: &Value, max: usize| -> Option<String> {
            value.as_str().filter(|text| text.len() <= max && !text.chars().any(char::is_control)).map(str::to_owned)
        };
        let body = |value: &Value, max: usize| -> Option<String> {
            value
                .as_str()
                .filter(|text| text.len() <= max && !text.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')))
                .map(str::to_owned)
        };
        if let Some(name) = value.get("name").and_then(|v| line(v, 4096)) {
            form.name = name;
        }
        if let Some(emails) = value.get("emails").and_then(Value::as_array) {
            for (slot, email) in form.emails.iter_mut().zip(emails.iter()) {
                if let Some(email) = line(email, 4096) {
                    *slot = email;
                }
            }
        }
        if let Some(emails) = value.get("moreEmails").and_then(Value::as_array) {
            form.more_emails = emails.iter().filter_map(|email| line(email, 4096)).take(100).collect();
        }
        if let Some(phones) = value.get("phones").and_then(Value::as_array) {
            for (slot, phone) in form.phones.iter_mut().zip(phones.iter()) {
                if let Some(phone) = line(phone, 4096) {
                    *slot = phone;
                }
            }
        }
        if let Some(phones) = value.get("morePhones").and_then(Value::as_array) {
            form.more_phones = phones
                .iter()
                .filter_map(|phone| serde_json::from_value::<mailcraft_engine::model::vcard::Typed>(phone.clone()).ok())
                .filter_map(|phone| {
                    Some(mailcraft_engine::model::vcard::Typed {
                        value: line(&Value::String(phone.value), 4096)?,
                        kind: line(&Value::String(phone.kind), 128)?,
                    })
                })
                .take(100)
                .collect();
        }
        if let Some(addresses) = value.get("addresses").and_then(Value::as_array) {
            for (slot, address) in form.addresses.iter_mut().zip(addresses.iter()) {
                if let Ok(address) = serde_json::from_value::<mailcraft_engine::model::vcard::PostalAddress>(address.clone()) {
                    slot.kind = line(&Value::String(address.kind), 128).unwrap_or_default();
                    slot.street = body(&Value::String(address.street), 4096).unwrap_or_default();
                    slot.city = line(&Value::String(address.city), 1024).unwrap_or_default();
                    slot.region = line(&Value::String(address.region), 1024).unwrap_or_default();
                    slot.postcode = line(&Value::String(address.postcode), 128).unwrap_or_default();
                    slot.country = line(&Value::String(address.country), 1024).unwrap_or_default();
                }
            }
        }
        if let Some(index) = value.get("addressShown").and_then(Value::as_u64).filter(|index| *index < form.addresses.len() as u64)
            && let Ok(index) = usize::try_from(index)
        {
            form.address_shown = index;
        }
        if let Some(birthday) = value.get("birthday").and_then(|v| line(v, 32)) {
            form.birthday = birthday;
        }
        if let Some(org) = value.get("org").and_then(|v| line(v, 4096)) {
            form.org = org;
        }
        if let Some(title) = value.get("title").and_then(|v| line(v, 4096)) {
            form.title = title;
        }
        if let Some(note) = value.get("note").and_then(|v| body(v, 65_536)) {
            form.note = note;
        }
        Some(form)
    }

    fn task_form_draft(&self) -> Option<Value> {
        let Some(Dialog::Task(form)) = self.pending_form() else { return None };
        let account = form.accounts.get(form.account).map(|(id, _)| capped_text(id, 256));
        Some(json!({
            "uid": &form.uid,
            "account": account,
            "title": capped_text(&form.title, 4096),
            "notes": capped_text(&form.notes, 65_536),
            "start": capped_text(&form.start, 32),
            "due": capped_text(&form.due, 32),
            "status": form.status,
            "percent": form.percent,
            "priority": form.priority,
            "remind": form.remind,
            "reminderDate": capped_text(&form.reminder_date, 32),
            "reminderTime": capped_text(&form.reminder_time, 32),
            "repeat": capped_text(&form.repeat, 4096),
            "categories": form.categories.iter().take(100).map(|name| capped_text(name, 256)).collect::<Vec<_>>(),
        }))
    }

    fn restore_task_form(&self, value: &Value) -> Option<crate::pim_dialogs::TaskForm> {
        let mut form = match persisted_uid(value)? {
            Some(uid) => crate::pim_dialogs::TaskForm::open(&self.session, uid)?,
            None => match Dialog::new_task(&self.session, None) {
                Dialog::Task(form) => *form,
                _ => return None,
            },
        };
        let text = |key: &str, max: usize| -> Option<String> {
            value.get(key).and_then(Value::as_str).filter(|text| text.len() <= max && !text.chars().any(char::is_control)).map(str::to_owned)
        };
        let body_text = |key: &str, max: usize| -> Option<String> {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| text.len() <= max && !text.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')))
                .map(str::to_owned)
        };
        if let Some(account) = text("account", 256)
            && let Some(index) = form.accounts.iter().position(|(id, _)| id == &account)
        {
            form.account = index;
        }
        if let Some(title) = text("title", 4096) {
            form.title = title;
        }
        if let Some(notes) = body_text("notes", 65_536) {
            form.notes = notes;
        }
        if let Some(start) = text("start", 32) {
            form.start = start;
        }
        if let Some(due) = text("due", 32) {
            form.due = due;
        }
        if let Some(status) =
            value.get("status").and_then(|status| serde_json::from_value::<mailcraft_engine::model::pim::TaskStatus>(status.clone()).ok())
        {
            form.status = status;
        }
        if let Some(percent) = value.get("percent").and_then(Value::as_u64).filter(|percent| *percent <= 100)
            && let Ok(percent) = u8::try_from(percent)
        {
            form.percent = percent;
        }
        if let Some(priority) =
            value.get("priority").and_then(|priority| serde_json::from_value::<mailcraft_engine::model::pim::TaskPriority>(priority.clone()).ok())
        {
            form.priority = priority;
        }
        if let Some(remind) = value.get("remind").and_then(Value::as_bool) {
            form.remind = remind;
        }
        if let Some(date) = text("reminderDate", 32) {
            form.reminder_date = date;
        }
        if let Some(time) = text("reminderTime", 32) {
            form.reminder_time = time;
        }
        if let Some(repeat) = text("repeat", 4096) {
            form.repeat = repeat;
        }
        if let Some(categories) = value.get("categories").and_then(Value::as_array) {
            form.categories = categories
                .iter()
                .filter_map(Value::as_str)
                .filter(|name| name.len() <= 256 && !name.chars().any(char::is_control))
                .filter(|name| form.all_categories.iter().any(|(available, _)| available == name))
                .take(100)
                .map(str::to_owned)
                .collect();
        }
        form.done = form.status == mailcraft_engine::model::pim::TaskStatus::Completed;
        Some(form)
    }

    fn module_search(&self) -> Option<&str> {
        match self.session.module {
            mailcraft_engine::Module::Calendar => Some(&self.ui.cal_search.query),
            mailcraft_engine::Module::People => Some(&self.ui.people_query),
            mailcraft_engine::Module::Tasks => Some(&self.ui.tasks_query),
            mailcraft_engine::Module::Notes => Some(&self.ui.notes_query),
            mailcraft_engine::Module::Mail => None,
        }
    }

    fn pending_form(&self) -> Option<&Dialog> {
        match self.ui.dialog.as_ref()? {
            Dialog::DiscardForm(form) => Some(form),
            form => Some(form),
        }
    }
}
