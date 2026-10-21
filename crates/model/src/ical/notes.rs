//! An event's notes as they read: links under their words, addresses as links (Exchange's and Google's invitations).

/// Most web addresses taken from an event's notes.
pub(super) const MAX_LINKS: usize = 20;

/// The web addresses in a text, once each and in order: written out, or as the `[1]: https://…`
/// footnotes an HTML body's links become when it's made text (a Teams invitation's "Join the
/// meeting now"). Only `http` and `https`; a sentence's closing punctuation isn't part of one.
pub fn links_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut at = 0;
    while out.len() < MAX_LINKS {
        let next = [lower.get(at..).and_then(|r| r.find("https://")), lower.get(at..).and_then(|r| r.find("http://"))].into_iter().flatten().min();
        let Some(rel) = next else { break };
        let start = at + rel;
        let rest = text.get(start..).unwrap_or("");
        let len = rest.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '[' | ']' | '{' | '}' | '|')).unwrap_or(rest.len());
        let mut link = rest.get(..len).unwrap_or("");
        // Closing punctuation, and a closing bracket the link didn't open.
        loop {
            let trimmed = link.trim_end_matches(['.', ',', ';', ':', '!', '?']);
            let trimmed = if trimmed.ends_with(')') && trimmed.matches('(').count() < trimmed.matches(')').count() {
                trimmed.get(..trimmed.len() - 1).unwrap_or(trimmed)
            } else {
                trimmed
            };
            if trimmed.len() == link.len() {
                break;
            }
            link = trimmed;
        }
        at = start + len.max(1);
        let host_ok = link.split_once("://").is_some_and(|(_, r)| r.split(['/', '?', '#']).next().is_some_and(|h| h.contains('.') && h.len() < 256));
        if host_ok && link.len() <= 4096 && !out.iter().any(|l| l == link) {
            out.push(link.to_string());
        }
    }
    out
}

/// A piece of an event's notes as they're shown ([`note_lines`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotePart {
    Text(String),
    /// A link to open: its words, or its address when it has none (`label == url`).
    Link {
        label: String,
        url: String,
    },
}

/// The most of an event's notes shown as lines with links (the rest as it is).
pub(super) const NOTE_MAX: usize = 200_000;

/// An event's notes as lines of text and links, the way an invitation reads (issue #1). Made
/// text from HTML by Exchange (`BodyType Text`), each link is written after its words in angle
/// brackets, `Join with Google Meet<https://meet.google.com/…>`: the words become the link and
/// the address goes, as in the HTML. Of the text before, the link's words are those after the
/// last `. `, `: `, `) ` or `– ` on the line (`(US) +1 555-0100<tel:…>` links `+1 555-0100`);
/// an address in brackets with no words before it on its line, or one written out, is a link of
/// its own. Web and `mailto:` links open; a `tel:` one stays its words.
pub fn note_lines(text: &str) -> Vec<Vec<NotePart>> {
    let cut = text.char_indices().nth(NOTE_MAX).map_or(text.len(), |(i, _)| i);
    let mut lines: Vec<Vec<NotePart>> = text.get(..cut).unwrap_or(text).lines().map(note_line).collect();
    // A few words on a line, then (past blank lines) an address alone on one: the words are its
    // link, as a Google invitation's Yes, No, Maybe and More options are.
    let mut i = 0;
    while i < lines.len() {
        let url = match lines.get(i).map(Vec::as_slice) {
            Some([NotePart::Link { label, url }]) if label == url => url.clone(),
            _ => {
                i += 1;
                continue;
            }
        };
        let words = (0..i).rev().find(|j| !lines.get(*j).is_some_and(|l| l.iter().all(|p| matches!(p, NotePart::Text(t) if t.trim().is_empty()))));
        let short = words.and_then(|j| match lines.get(j).map(Vec::as_slice) {
            Some([NotePart::Text(t)]) if (1..=24).contains(&t.trim().chars().count()) && !t.trim_end().ends_with(':') => {
                Some((j, t.trim().to_string()))
            }
            _ => None,
        });
        match short {
            Some((j, label)) => {
                if let Some(l) = lines.get_mut(j) {
                    *l = vec![NotePart::Link { label, url }];
                }
                lines.drain(j + 1..=i.min(lines.len().saturating_sub(1)));
                i = j + 1;
            }
            None => i += 1,
        }
    }
    lines
}

pub(super) fn note_line(line: &str) -> Vec<NotePart> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some((before, url, after)) = bracketed_link(rest) {
        let (text, label) = split_label(before);
        bare_links(text, &mut out);
        let tel = url.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("tel:"));
        match (label.is_empty(), tel) {
            (true, true) => {}
            (false, true) => push_text(&mut out, label),
            (true, false) => out.push(NotePart::Link { label: url.to_string(), url: url.to_string() }),
            (false, false) => out.push(NotePart::Link { label: label.to_string(), url: url.to_string() }),
        }
        rest = after;
    }
    bare_links(rest, &mut out);
    out
}

/// The first `<scheme:…>` in a line: the text before it, the address, the text after.
pub(super) fn bracketed_link(line: &str) -> Option<(&str, &str, &str)> {
    let mut from = 0;
    loop {
        let open = from + line.get(from..)?.find('<')?;
        let inner = line.get(open + 1..)?;
        let close = inner.find('>');
        let url = close.and_then(|c| inner.get(..c)).unwrap_or("");
        let lower = url.to_ascii_lowercase();
        let scheme = ["https://", "http://", "mailto:", "tel:"].iter().any(|s| lower.starts_with(s));
        if let Some(c) = close
            && scheme
            && url.len() > 7
            && !url.contains(char::is_whitespace)
        {
            return Some((line.get(..open)?, url, inner.get(c + 1..)?));
        }
        from = open + 1;
    }
}

/// Text before a link: what stays text, and the link's own words.
pub(super) fn split_label(before: &str) -> (&str, &str) {
    let at = [". ", ": ", ") ", "– ", "\t"].iter().filter_map(|d| before.rfind(d).map(|i| i + d.len())).max().unwrap_or(0);
    let (text, label) = before.split_at(at.min(before.len()));
    let trimmed = label.trim_start();
    (before.get(..text.len() + (label.len() - trimmed.len())).unwrap_or(text), trimmed.trim_end())
}

pub(super) fn push_text(out: &mut Vec<NotePart>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(NotePart::Text(t)) = out.last_mut() {
        t.push_str(s);
    } else {
        out.push(NotePart::Text(s.to_string()));
    }
}

/// Text, its written-out web addresses links of their own.
pub(super) fn bare_links(text: &str, out: &mut Vec<NotePart>) {
    let mut rest = text;
    for link in links_in(text) {
        let Some(at) = rest.find(&link) else { continue };
        push_text(out, rest.get(..at).unwrap_or(""));
        out.push(NotePart::Link { label: link.clone(), url: link.clone() });
        rest = rest.get(at + link.len()..).unwrap_or("");
    }
    push_text(out, rest);
}

/// Whether a web address joins an online meeting: Microsoft Teams, Zoom, Google Meet, Webex,
/// GoTo Meeting, Jitsi, Whereby, Skype.
pub fn is_meeting_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")) else { return false };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.split(':').next().unwrap_or(host);
    let on = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    (on("teams.microsoft.com") && (path.starts_with("l/meetup-join") || path.starts_with("meet/")))
        || (on("teams.live.com") && path.starts_with("meet"))
        || (on("zoom.us") && (path.starts_with("j/") || path.starts_with("w/") || path.starts_with("my/")))
        || on("meet.google.com")
        || (on("webex.com") && (path.contains("j.php") || path.starts_with("meet") || path.contains("/meet")))
        || (on("gotomeeting.com") && path.starts_with("join"))
        || on("meet.goto.com")
        || on("meet.jit.si")
        || on("whereby.com")
        || on("join.skype.com")
}
