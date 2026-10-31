//! Online meetings in events (issue #25): which links join one, the RFC 7986 `CONFERENCE`
//! property, and a meeting added from a saved room: its link as the place, a `CONFERENCE` line
//! for other calendars' Join buttons, and a block in the notes between marker lines that can be
//! taken out again (as Outlook adds and removes a Teams meeting's block).

use serde::{Deserialize, Serialize};

use super::{CalEvent, links_in, parse_line};

/// The longest room name, access code and link kept.
const NAME_MAX: usize = 100;
const CODE_MAX: usize = 100;
pub const URL_MAX: usize = 2048;
/// The most extra text (dial-in numbers…) a room adds to the notes, and its most lines: a
/// block is found again only when it's short ([`BLOCK_LINES`]).
const DETAILS_MAX: usize = 2000;
const DETAILS_LINES: usize = 20;
/// The most meeting hosts the user can list, and rooms they can keep.
pub const HOSTS_MAX: usize = 100;
pub const ROOMS_MAX: usize = 50;
/// The most lines between a block's markers.
const BLOCK_LINES: usize = 40;

/// A room of the user's own to meet in online (a BigBlueButton, Jitsi or OpenTalk room…): its
/// permanent link and access code, added to a meeting as it is (the generic link provider).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MeetingRoom {
    pub name: String,
    pub url: String,
    /// The access code asked for at the door (empty: none).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub code: String,
    /// More lines for the invitation (dial-in numbers…).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub details: String,
}

impl MeetingRoom {
    /// A room as given, checked: a web link (`https://` assumed when there's no scheme), the
    /// rest trimmed and capped. `None` when the link isn't one.
    pub fn new(name: &str, url: &str, code: &str, details: &str) -> Option<MeetingRoom> {
        let url = meeting_url(url)?;
        let name = one_line(name, NAME_MAX);
        Some(MeetingRoom {
            name: if name.is_empty() { host_of(&url).unwrap_or_default() } else { name },
            url,
            code: one_line(code, CODE_MAX),
            details: details
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .chars()
                .filter(|c| *c == '\n' || !c.is_control())
                .take(DETAILS_MAX)
                .collect::<String>()
                .lines()
                .take(DETAILS_LINES)
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string(),
        })
    }

    /// The host (and the path up to the room) its links start with: the meeting server's other
    /// rooms join meetings too (`meet.example.edu/rooms/`).
    pub fn host_entry(&self) -> Option<String> {
        let rest = self.url.split_once("://").map(|(_, r)| r)?;
        let rest = rest.split(['?', '#']).next().unwrap_or(rest).to_ascii_lowercase();
        let (host, path) = rest.split_once('/').unwrap_or((&rest, ""));
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        host_entry(&if parent.is_empty() { host.to_string() } else { format!("{host}/{parent}/") })
    }
}

/// The words of the block added to the notes, in the user's language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeetingLabels {
    pub heading: String,
    pub join: String,
    pub code: String,
}

impl Default for MeetingLabels {
    fn default() -> Self {
        MeetingLabels { heading: "Online meeting".into(), join: "Join:".into(), code: "Access code:".into() }
    }
}

/// One line of text, without control characters, at most `max` characters.
fn one_line(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

/// A meeting link as given: `https://` assumed without a scheme; only web links, with a host.
pub fn meeting_url(given: &str) -> Option<String> {
    let g = given.trim();
    let lower = g.to_ascii_lowercase();
    let url = if lower.starts_with("https://") || lower.starts_with("http://") {
        g.to_string()
    } else if g.contains("://") {
        return None;
    } else {
        format!("https://{g}")
    };
    if url.len() > URL_MAX
        || url.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}'))
    {
        return None;
    }
    // A link is found in the notes again as it was given (one ending in `.` or an unopened `)`
    // would be cut short there, and its block never found to take out).
    (host_of(&url).is_some() && links_in(&url) == [url.as_str()]).then_some(url)
}

/// A link's host, lower case, without a port or user: `None` when it has none worth the name.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // A user name in a meeting link (`https://meet.example@evil.example/`) only misleads.
    if authority.contains('@') {
        return None;
    }
    let host = authority.split(':').next().unwrap_or(authority).to_ascii_lowercase();
    let ok = host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|l| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-'))
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
    ok.then_some(host)
}

/// A meeting host as the user types it (`meet.example.edu`, `https://meet.example.edu/rooms/`):
/// lower case, without a scheme; a path, kept, narrows it to links under that path. `None` when
/// it isn't a host.
pub fn host_entry(given: &str) -> Option<String> {
    let g = given.trim().to_ascii_lowercase();
    let g = g.split_once("://").map_or(g.as_str(), |(_, r)| r);
    let g = g.split(['?', '#']).next().unwrap_or(g);
    let (host, path) = g.split_once('/').unwrap_or((g, ""));
    let host = host_of(&format!("https://{host}"))?;
    if path.len() > 200 || path.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    Some(if path.is_empty() { host } else { format!("{host}/{path}") })
}

/// Whether a link is under one of these hosts ([`host_entry`]s): the host or one of its
/// subdomains, and, when the entry has a path, under it.
fn under_host(host: &str, path: &str, entry: &str) -> bool {
    let (h, p) = entry.split_once('/').unwrap_or((entry, ""));
    let on = host == h || host.strip_suffix(h).is_some_and(|sub| sub.ends_with('.'));
    // Under the path a segment at a time: `example.edu/talk` isn't `example.edu/talkshow`.
    let p = p.trim_matches('/');
    on && (p.is_empty() || path.trim_end_matches('/') == p || path.strip_prefix(p).is_some_and(|rest| rest.starts_with('/')))
}

/// A BigBlueButton room's code in its link: Greenlight's and PILOS's `abc-def-ghi` (three or
/// four groups of three).
fn bbb_room_code(seg: &str) -> bool {
    let groups: Vec<&str> = seg.split('-').collect();
    (3..=4).contains(&groups.len()) && groups.iter().all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

/// A UUID (`8-4-4-4-12` hex digits), as OpenTalk's rooms are named.
fn uuid(seg: &str) -> bool {
    let groups: Vec<&str> = seg.split('-').collect();
    groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12]) && groups.iter().all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether a web address joins an online meeting: one of the services [`super::is_meeting_link`]
/// knows, a self-hosted room by the shape of its link (BigBlueButton's `/rooms/abc-def-ghi`
/// through Greenlight or PILOS, OpenTalk's `/room/<uuid>`), or a link under one of `hosts`
/// (the user's own list, and their rooms' servers: [`host_entry`]).
pub fn is_meeting_link_in(url: &str, hosts: &[String]) -> bool {
    if super::is_meeting_link(url) {
        return true;
    }
    let scheme = url.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://")) || url.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("http://"));
    let Some(host) = host_of(url).filter(|_| scheme) else { return false };
    let rest = url.split_once("://").map_or("", |(_, r)| r);
    // The path as written (rooms' codes are lower case), and in lower case for the hosts.
    let path = rest.split_once('/').map_or("", |(_, p)| p);
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let path = path.to_ascii_lowercase();
    let shaped = segs.windows(2).any(|w| match w {
        [kind, code] => (matches!(*kind, "rooms" | "b" | "gl") && bbb_room_code(code)) || (*kind == "room" && uuid(code)),
        _ => false,
    });
    shaped || hosts.iter().take(HOSTS_MAX + ROOMS_MAX).any(|h| under_host(&host, &path, h))
}

/// A `CONFERENCE` property (RFC 7986 §5.11): where to join and what it offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conference {
    pub uri: String,
    /// `FEATURE`: `AUDIO`, `VIDEO`, `PHONE`, `CHAT`, `SCREEN`, `MODERATOR`, `FEED` (upper case).
    pub features: Vec<String>,
    pub label: String,
}

impl Conference {
    /// A web link to join with sound or pictures (a phone number or a chat isn't).
    fn joins(&self) -> bool {
        let web = self.uri.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://"))
            || self.uri.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("http://"));
        let media = self.features.is_empty() || self.features.iter().any(|f| matches!(f.as_str(), "AUDIO" | "VIDEO" | "SCREEN"));
        web && media && host_of(&self.uri).is_some() && self.uri.len() <= URL_MAX
    }
}

/// Whether a kept line is a `CONFERENCE` property.
fn is_conference(raw: &str) -> bool {
    raw.get(..10).is_some_and(|n| n.eq_ignore_ascii_case("CONFERENCE")) && raw.get(10..11).is_some_and(|c| c == ";" || c == ":")
}

/// A `CONFERENCE` line for a room: `VALUE=URI` (required), its features and its name.
pub fn conference_line(url: &str, label: &str) -> String {
    let label: String = label.chars().filter(|c| !c.is_control() && *c != '"').take(NAME_MAX).collect();
    let url: String = url.chars().filter(|c| !c.is_control() && !c.is_whitespace()).collect();
    if label.trim().is_empty() {
        format!("CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO:{url}")
    } else {
        format!("CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=\"{}\":{url}", label.trim())
    }
}

/// Whether a notes line opens a meeting block: `---- words ----`.
fn block_start(line: &str) -> bool {
    let l = line.trim();
    l.len() > 8 && l.starts_with("----") && l.ends_with("----") && l.trim_matches('-').trim().chars().count() > 0
}

/// Whether a notes line closes one: dashes only.
fn block_end(line: &str) -> bool {
    let l = line.trim();
    l.len() >= 4 && l.bytes().all(|b| b == b'-')
}

/// The first meeting block in some notes: its byte range (from its first line's start to the
/// end of its last, line break included) and the links in it. A block is the lines from a
/// `---- … ----` line to the next line of dashes, a few dozen at most, its first line with a
/// link (`Join: https://…`): a quoted `-----Original Message-----` isn't one. With `only`, a
/// block with one of those links.
fn find_block(text: &str, only: &[String]) -> Option<(usize, usize, Vec<String>)> {
    let mut at = 0;
    let lines: Vec<(usize, &str)> = text
        .split_inclusive('\n')
        .map(|l| {
            let start = at;
            at += l.len();
            (start, l)
        })
        .collect();
    for (i, (start, line)) in lines.iter().enumerate() {
        if !block_start(line) {
            continue;
        }
        let Some((j, (end_at, end_line))) = lines.iter().enumerate().skip(i + 1).take(BLOCK_LINES).find(|(_, (_, l))| block_end(l)) else { continue };
        let inner = lines.get(i + 1..j).unwrap_or(&[]);
        let first = inner.iter().map(|(_, l)| *l).find(|l| !l.trim().is_empty()).unwrap_or("");
        let links = links_in(&inner.iter().map(|(_, l)| *l).collect::<String>());
        if !links_in(first).is_empty() && (only.is_empty() || links.iter().any(|l| only.contains(l))) {
            return Some((*start, end_at + end_line.len(), links));
        }
    }
    None
}

/// The block a room adds to an event's notes.
pub fn meeting_block(room: &MeetingRoom, labels: &MeetingLabels) -> String {
    let heading = one_line(&labels.heading, 60);
    let top = format!("---- {} ----", if heading.is_empty() { "Online meeting" } else { heading.as_str() });
    let mut out = format!("{top}\n{} {}\n", one_line(&labels.join, 40), room.url);
    if !room.code.is_empty() {
        out.push_str(&format!("{} {}\n", one_line(&labels.code, 40), room.code));
    }
    for l in room.details.lines().filter(|l| !block_start(l) && !block_end(l)).take(DETAILS_LINES) {
        out.push_str(l.trim_end());
        out.push('\n');
    }
    out.push_str(&"-".repeat(top.chars().count().clamp(8, 60)));
    out.push('\n');
    out
}

/// Notes with a block taken out, and the blank lines that set it apart.
fn cut(text: &str, start: usize, end: usize) -> String {
    let before = text.get(..start).unwrap_or("").trim_end();
    let after = text.get(end..).unwrap_or("").trim_start_matches(['\r', '\n']);
    match (before.is_empty(), after.trim().is_empty()) {
        (_, true) => before.to_string(),
        (true, false) => after.to_string(),
        (false, false) => format!("{before}\n\n{after}"),
    }
}

impl CalEvent {
    /// Its `CONFERENCE` properties (RFC 7986), as kept from the file it came in.
    pub fn conferences(&self) -> Vec<Conference> {
        self.extra
            .iter()
            .filter(|raw| is_conference(raw))
            .filter_map(|raw| parse_line(raw))
            .map(|l| Conference {
                uri: l.value.trim().to_string(),
                features: l
                    .param("FEATURE")
                    .map(|f| f.split(',').map(|x| x.trim().trim_matches('"').to_ascii_uppercase()).collect())
                    .unwrap_or_default(),
                label: l.param("LABEL").map(|x| super::unescape(x.trim_matches('"'))).unwrap_or_default(),
            })
            .take(20)
            .collect()
    }

    /// The address that joins it online: the server's own, a `CONFERENCE` link (RFC 7986: the
    /// organizer's calendar said so), else the first meeting link in its location or notes,
    /// counting links under `hosts` ([`is_meeting_link_in`]).
    pub fn join_link_in(&self, hosts: &[String]) -> Option<String> {
        if is_meeting_link_in(&self.join_url, hosts) {
            return Some(self.join_url.clone());
        }
        if let Some(c) = self.conferences().into_iter().find(Conference::joins) {
            return Some(c.uri);
        }
        links_in(&self.location).into_iter().chain(links_in(&self.description)).find(|l| is_meeting_link_in(l, hosts))
    }

    /// The online meeting added to it (by MailCraft or another calendar): its `CONFERENCE`
    /// link, else the link in its meeting block.
    pub fn online_meeting(&self) -> Option<String> {
        self.conferences()
            .into_iter()
            .find(Conference::joins)
            .map(|c| c.uri)
            .or_else(|| find_block(&self.description, &[]).and_then(|(_, _, l)| l.into_iter().next()))
    }

    /// Add a room's meeting, as Outlook adds a Teams meeting: its link as the place (after the
    /// place there is, as `Room 4; https://…`), a `CONFERENCE` property, and its block at the end
    /// of the notes. A meeting already there is replaced; this room's block, when the notes have
    /// it (perhaps edited), stays as it is.
    pub fn add_online_meeting(&mut self, room: &MeetingRoom, labels: &MeetingLabels) {
        if self.online_meeting().is_some_and(|u| u != room.url) {
            self.remove_online_meeting();
        }
        if find_block(&self.description, std::slice::from_ref(&room.url)).is_none() {
            let notes = self.description.trim_end();
            let block = meeting_block(room, labels);
            self.description = if notes.is_empty() { block } else { format!("{notes}\n\n{block}") };
        }
        if !self.conferences().iter().any(|c| c.uri == room.url) && self.extra.len() < 200 {
            self.extra.push(conference_line(&room.url, &room.name));
        }
        let place = self.location.trim();
        if place.is_empty() {
            self.location = room.url.clone();
        } else if !place.contains(&room.url) {
            self.location = format!("{place}; {}", room.url);
        }
    }

    /// Take its online meeting out: the meeting blocks in its notes, its `CONFERENCE`
    /// properties, and their links from its place. Whether there was one.
    pub fn remove_online_meeting(&mut self) -> bool {
        let mut links: Vec<String> = self.conferences().into_iter().map(|c| c.uri).collect();
        let before = self.extra.len();
        self.extra.retain(|raw| !is_conference(raw));
        let mut found = self.extra.len() != before;
        for _ in 0..10 {
            let Some((start, end, l)) = find_block(&self.description, &[]) else { break };
            self.description = cut(&self.description, start, end);
            links.extend(l);
            found = true;
        }
        // Notes Exchange made text from HTML list each link again at the end, as `[1]: https://…`
        // (`mime::html_to_text`): the meeting's go with it.
        let footnote = |l: &str| {
            let l = l.trim();
            l.strip_prefix('[')
                .and_then(|r| r.split_once("]:"))
                .is_some_and(|(n, url)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && links.iter().any(|x| x == url.trim()))
        };
        if found && self.description.lines().any(footnote) {
            let kept: String = self.description.split_inclusive('\n').filter(|l| !footnote(l)).collect();
            self.description = kept.trim_end().to_string();
        }
        // The link as the whole place, or after it or before it (`Room 4; https://…`); the rest
        // stays as it was written.
        let mut place = self.location.trim().to_string();
        for l in links.iter().filter(|l| !l.is_empty()) {
            if place == *l {
                place.clear();
            } else if let Some(rest) = place.strip_suffix(l.as_str()).and_then(|p| p.trim_end().strip_suffix(';')) {
                place = rest.trim_end().to_string();
            } else if let Some(rest) = place.strip_prefix(l.as_str()).and_then(|p| p.trim_start().strip_prefix(';')) {
                place = rest.trim_start().to_string();
            } else {
                place = place.replacen(&format!("; {l};"), ";", 1);
            }
        }
        if place != self.location.trim() {
            self.location = place;
            found = true;
        }
        found
    }
}
