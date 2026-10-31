//! Commands: online meetings (issue #25). The user's own rooms (the generic link provider: a
//! BigBlueButton, Jitsi or OpenTalk room's permanent link and access code), the hosts whose links
//! join a meeting, and adding a room's meeting to an event (Outlook's Teams Meeting button).

use mailcraft_model::ical::{CalEvent, HOSTS_MAX, MeetingLabels, MeetingRoom, ROOMS_MAX, host_entry};

use super::*;

impl Session {
    /// The hosts whose links join a meeting beyond the services everyone knows: the user's own
    /// list, and the servers of their rooms.
    pub fn meeting_hosts(&self) -> Vec<String> {
        let mut out = self.prefs.meeting_hosts.clone();
        for h in self.prefs.meeting_rooms.iter().filter_map(MeetingRoom::host_entry) {
            if !out.contains(&h) {
                out.push(h);
            }
        }
        out
    }

    /// The address that joins an event online, counting the user's meeting hosts.
    pub fn join_link(&self, e: &CalEvent) -> Option<String> {
        e.join_link_in(&self.meeting_hosts())
    }

    /// A saved room by its name or link (case aside); `""` or `"default"`: the first.
    fn meeting_room(&self, which: &str) -> Option<&MeetingRoom> {
        let w = which.trim();
        if w.is_empty() || w.eq_ignore_ascii_case("default") {
            return self.prefs.meeting_rooms.first();
        }
        self.prefs.meeting_rooms.iter().find(|r| r.name.eq_ignore_ascii_case(w) || r.url.eq_ignore_ascii_case(w))
    }
}

fn listing(s: &Session) -> Value {
    json!({"rooms": s.prefs.meeting_rooms, "hosts": s.prefs.meeting_hosts, "forAll": s.prefs.meeting_for_all})
}

/// A text of a few words from the params, capped.
fn words(v: &Value, k: &str, default: &str) -> String {
    let w: String = p::str(v, k).unwrap_or(default).chars().filter(|c| !c.is_control()).take(60).collect();
    if w.trim().is_empty() { default.to_string() } else { w.trim().to_string() }
}

/// Apply `onlineMeeting` to an event being made or changed: a saved room (by name or link, or
/// `true` for the first), a room given whole (`{"url", "name", "code", "details", "labels"}`), or
/// `false` to take its meeting out.
pub(super) fn apply(s: &Session, v: &Value, e: &mut CalEvent) -> Result<(), CmdError> {
    let Some(m) = v.get("onlineMeeting") else { return Ok(()) };
    let mut labels = MeetingLabels::default();
    let room = match m {
        Value::Null | Value::Bool(false) => None,
        Value::String(x) if matches!(x.trim(), "" | "none") => None,
        Value::Bool(true) => Some(
            s.meeting_room("").cloned().ok_or_else(|| CmdError::Params("no online meeting rooms yet (File › Options › Online meetings)".into()))?,
        ),
        Value::String(x) => Some(
            s.meeting_room(x)
                .cloned()
                .ok_or_else(|| CmdError::Params(format!("no online meeting room `{}`", x.chars().take(100).collect::<String>())))?,
        ),
        Value::Object(_) => {
            if let Some(l) = m.get("labels") {
                labels = MeetingLabels {
                    heading: words(l, "heading", &labels.heading),
                    join: words(l, "join", &labels.join),
                    code: words(l, "code", &labels.code),
                };
            }
            let get = |k: &str| p::str(m, k).unwrap_or_default();
            Some(
                MeetingRoom::new(get("name"), p::req_str(m, "url")?, get("code"), get("details"))
                    .ok_or_else(|| CmdError::Params("`url` isn't a web link".into()))?,
            )
        }
        _ => return Err(CmdError::Params("`onlineMeeting` is a room's name, true, false, or {\"url\": …}".into())),
    };
    match room {
        Some(r) => e.add_online_meeting(&r, &labels),
        None => {
            e.remove_online_meeting();
        }
    }
    Ok(())
}

/// Outlook's "Add online meeting to all meetings": a new meeting with guests, and no meeting
/// asked for or already in it, gets the first room's.
pub(super) fn for_all(s: &Session, v: &Value, e: &mut CalEvent) {
    if v.get("onlineMeeting").is_none()
        && s.prefs.meeting_for_all
        && !e.attendees.is_empty()
        && s.join_link(e).is_none()
        && let Some(r) = s.prefs.meeting_rooms.first()
    {
        e.add_online_meeting(r, &MeetingLabels::default());
    }
}

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("meetings.list", "Online meetings", "File › Options › Online meetings", |s, _| Ok(listing(s))),
        CommandSpec::new("meetings.addRoom", "Add a meeting room", "File › Options › Online meetings", |s, v| {
            let get = |k: &str| p::str(v, k).unwrap_or_default();
            let room = MeetingRoom::new(get("name"), p::req_str(v, "url")?, get("code"), get("details"))
                .ok_or_else(|| CmdError::Params("`url` isn't a web link (https://…)".into()))?;
            // The same name or link: changed in place.
            let rooms = &mut s.prefs.meeting_rooms;
            match rooms.iter().position(|r| r.name.eq_ignore_ascii_case(&room.name) || r.url == room.url) {
                Some(i) => {
                    if let Some(r) = rooms.get_mut(i) {
                        *r = room;
                    }
                }
                None if rooms.len() >= ROOMS_MAX => return Err(CmdError::Params(format!("at most {ROOMS_MAX} rooms"))),
                None => rooms.push(room),
            }
            Ok(listing(s))
        })
        .params(r#"{"url": string (the room's link), "name"?: string, "code"?: string (its access code), "details"?: string (more lines for invitations: dial-in…)}"#),
        CommandSpec::new("meetings.removeRoom", "Remove a meeting room", "File › Options › Online meetings", |s, v| {
            let which = p::req_str(v, "room")?;
            let before = s.prefs.meeting_rooms.len();
            s.prefs.meeting_rooms.retain(|r| !(r.name.eq_ignore_ascii_case(which.trim()) || r.url.eq_ignore_ascii_case(which.trim())));
            if s.prefs.meeting_rooms.len() == before {
                return Err(CmdError::Params(format!("no room `{}`", which.chars().take(100).collect::<String>())));
            }
            Ok(listing(s))
        })
        .params(r#"{"room": string (its name or link)}"#),
        CommandSpec::new("meetings.defaultRoom", "Room added first", "File › Options › Online meetings", |s, v| {
            let which = p::req_str(v, "room")?;
            let Some(i) = s.prefs.meeting_rooms.iter().position(|r| r.name.eq_ignore_ascii_case(which.trim()) || r.url.eq_ignore_ascii_case(which.trim())) else {
                return Err(CmdError::Params(format!("no room `{}`", which.chars().take(100).collect::<String>())));
            };
            let r = s.prefs.meeting_rooms.remove(i);
            s.prefs.meeting_rooms.insert(0, r);
            Ok(listing(s))
        })
        .params(r#"{"room": string (its name or link)}"#),
        CommandSpec::new("meetings.addHost", "Add a meeting host", "File › Options › Online meetings", |s, v| {
            let given = p::req_str(v, "host")?;
            let h = host_entry(given).ok_or_else(|| CmdError::Params(format!("`{}` isn't a host name", given.chars().take(100).collect::<String>())))?;
            if !s.prefs.meeting_hosts.contains(&h) {
                if s.prefs.meeting_hosts.len() >= HOSTS_MAX {
                    return Err(CmdError::Params(format!("at most {HOSTS_MAX} hosts")));
                }
                s.prefs.meeting_hosts.push(h);
            }
            Ok(listing(s))
        })
        .params(r#"{"host": string (meet.example.org, or with a path: example.edu/rooms/)}"#),
        CommandSpec::new("meetings.removeHost", "Remove a meeting host", "File › Options › Online meetings", |s, v| {
            let given = p::req_str(v, "host")?;
            let h = host_entry(given).unwrap_or_else(|| given.trim().to_string());
            let before = s.prefs.meeting_hosts.len();
            s.prefs.meeting_hosts.retain(|x| *x != h);
            if s.prefs.meeting_hosts.len() == before {
                return Err(CmdError::Params(format!("no host `{}`", given.chars().take(100).collect::<String>())));
            }
            Ok(listing(s))
        })
        .params(r#"{"host": string}"#),
        CommandSpec::new("meetings.forAll", "Add an online meeting to all meetings with guests", "File › Options › Online meetings", |s, v| {
            s.prefs.meeting_for_all = p::bool(v, "on").unwrap_or(!s.prefs.meeting_for_all);
            Ok(json!({"on": s.prefs.meeting_for_all}))
        })
        .params(r#"{"on"?: bool}"#),
    ]
}
