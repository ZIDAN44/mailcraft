//! Reading one event (`VEVENT`) of a calendar file, line by line: its properties, the components
//! it holds (kept as written, within their caps), and what's settled once it's whole.

use super::*;

/// Which of an event's classification and Show as have come: the first of each is kept past the
/// cap on its kept lines, so they're never dropped (issue #32).
#[derive(Default)]
struct ShowingSeen {
    class: bool,
    transp: bool,
    busy_status: bool,
}

impl ShowingSeen {
    /// Whether a line of this name is the first of one of those.
    fn first(&mut self, name: &str) -> bool {
        let seen = match name {
            "CLASS" => &mut self.class,
            "TRANSP" => &mut self.transp,
            "X-MICROSOFT-CDO-BUSYSTATUS" => &mut self.busy_status,
            _ => return false,
        };
        !std::mem::replace(seen, true)
    }
}

/// An event being read.
pub(super) struct EventReading<'a> {
    event: CalEvent,
    /// Its `DURATION`, for an end it doesn't give.
    duration: Option<i64>,
    /// Its exceptions' times (`EXDATE`, `RDATE`, `RECURRENCE-ID`): they wait for its zone, as
    /// `DTSTART` may come after them.
    pending: Vec<&'a Line>,
    showing: ShowingSeen,
    /// How deep in the components it holds the line is.
    depth: usize,
    /// The depth of the nested component being left out, with all it holds.
    dropped_from: Option<usize>,
    /// The cap of the outermost component being kept.
    component_max: usize,
}

impl<'a> EventReading<'a> {
    pub(super) fn new() -> Self {
        EventReading {
            event: CalEvent::default(),
            duration: None,
            pending: Vec::new(),
            showing: ShowingSeen::default(),
            depth: 0,
            dropped_from: None,
            component_max: EXTRA_MAX,
        }
    }

    /// Take a line of the event. Whether it ends the event.
    pub(super) fn line(&mut self, l: &'a Line, zones: &FileZones) -> bool {
        match l.name.as_str() {
            "BEGIN" => self.begin(l),
            "END" if self.depth > 0 => self.end(l),
            "END" => return l.value.trim().eq_ignore_ascii_case("VEVENT"),
            _ if self.depth > 0 => {
                if self.dropped_from.is_none() && self.event.extra.len() < self.component_max {
                    self.event.extra.push(l.raw.clone());
                }
            }
            _ => self.property(l, zones),
        }
        false
    }

    /// A nested component (`VALARM`) begins: kept as written, within its cap ([`EXTRA_MAX`]), or
    /// left out whole, with what it holds.
    fn begin(&mut self, l: &Line) {
        self.depth = self.depth.saturating_add(1);
        if self.dropped_from.is_some() {
            return;
        }
        if self.depth == 1 {
            self.component_max = if l.value.trim().eq_ignore_ascii_case("VALARM") { EXTRA_ALARMS_MAX } else { EXTRA_MAX };
        }
        if self.event.extra.len() < self.component_max {
            self.event.extra.push(l.raw.clone());
        } else {
            self.dropped_from = Some(self.depth);
        }
    }

    /// A nested component ends.
    fn end(&mut self, l: &Line) {
        match self.dropped_from {
            None => self.event.extra.push(l.raw.clone()),
            Some(d) if d == self.depth => self.dropped_from = None,
            Some(_) => {}
        }
        self.depth = self.depth.saturating_sub(1);
    }

    /// One of the event's own properties.
    fn property(&mut self, l: &'a Line, zones: &FileZones) {
        let e = &mut self.event;
        match l.name.as_str() {
            // As written: an answer echoes it, and its organizer looks for the meeting by it.
            "UID" => e.uid = l.value.trim().to_string(),
            "SUMMARY" => e.summary = unescape(&l.value),
            "DESCRIPTION" => e.description = unescape(&l.value),
            "LOCATION" => e.location = unescape(&l.value),
            "CATEGORIES" => add_categories(&mut e.categories, &l.value),
            "STATUS" => e.status = l.value.trim().to_ascii_uppercase(),
            "SEQUENCE" => e.sequence = l.value.trim().parse().unwrap_or(0),
            "DTSTART" => {
                if let Some((t, all_day, tzid)) = zones.time(l, "") {
                    e.start = t;
                    e.all_day = all_day;
                    e.vtimezone = zones.definition(&tzid);
                    e.tzid = tzid;
                }
            }
            "DTEND" => {
                if let Some((t, _, _)) = zones.time(l, "") {
                    e.end = t;
                }
            }
            "DURATION" => self.duration = duration(&l.value),
            "RRULE" => e.rrule = Some(l.value.trim().to_string()),
            "EXDATE" | "RDATE" | "RECURRENCE-ID" => {
                if self.pending.len() < MAX_DATES {
                    self.pending.push(l);
                }
            }
            "ORGANIZER" => {
                e.organizer = Some(person(l));
                e.client_scheduled = schedule_agent_client(l.param("SCHEDULE-AGENT"));
            }
            "ATTENDEE" => {
                e.server_delivered |= server_delivered(l.param("SCHEDULE-STATUS"));
                if e.attendees.len() < MAX_ATTENDEES {
                    e.attendees.push(person(l));
                }
            }
            "DTSTAMP" => e.dtstamp = zones.time(l, "").map_or(0, |(t, _, _)| t),
            "CREATED" | "LAST-MODIFIED" => {}
            n => {
                let first_showing = self.showing.first(n);
                if e.extra.len() < EXTRA_MAX || first_showing {
                    e.extra.push(l.raw.clone());
                }
            }
        }
    }

    /// The event read whole: its exceptions' times in its zone, a hidden title taken out, and an
    /// end from its length when it gives none.
    pub(super) fn finish(self, zones: &FileZones) -> CalEvent {
        let mut e = self.event;
        for x in self.pending {
            if x.name == "RECURRENCE-ID" {
                e.recurrence_id = zones.time(x, &e.tzid).map(|(t, _, _)| t);
                continue;
            }
            // At most [`MAX_DATES`] of each, from all their lines; a period (`RDATE;VALUE=PERIOD`)
            // by its start.
            let list = if x.name == "RDATE" { &mut e.rdates } else { &mut e.exdates };
            let room = MAX_DATES.saturating_sub(list.len());
            list.extend(zones.times(x, &e.tzid, room));
        }
        // The server's stand-in for a title it hid ("(Private event)") isn't the event's:
        // it shows as a private appointment, in the user's language (issue #32).
        if details_hidden(&e) {
            e.summary.clear();
        }
        if e.end <= e.start {
            e.end = match self.duration {
                Some(d) => e.start.saturating_add(d.max(0)),
                None if e.all_day => e.start.saturating_add(86_400),
                None => e.start,
            };
        }
        e
    }
}
