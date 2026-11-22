//! The session: how the message list is arranged, and the rows it shows.

use super::nav::NO_ROW;
use super::*;

/// What the message list is sorted and grouped by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Arrange {
    /// Newest first, under Today, Yesterday… (conversations, when on, apply here).
    #[default]
    Date,
    /// By sender, A to Z.
    From,
    /// By the first recipient, A to Z.
    To,
    /// By subject (without Re:, Fwd:…), A to Z.
    Subject,
    /// Largest first, under size ranges.
    Size,
    /// Flagged first.
    Flag,
    /// With attachments first.
    Attachments,
}

impl Arrange {
    pub const ALL: [Arrange; 7] = [Arrange::Date, Arrange::From, Arrange::To, Arrange::Subject, Arrange::Size, Arrange::Flag, Arrange::Attachments];

    pub fn code(self) -> &'static str {
        match self {
            Arrange::Date => "date",
            Arrange::From => "from",
            Arrange::To => "to",
            Arrange::Subject => "subject",
            Arrange::Size => "size",
            Arrange::Flag => "flag",
            Arrange::Attachments => "attachments",
        }
    }

    pub fn from_code(s: &str) -> Option<Arrange> {
        Arrange::ALL.into_iter().find(|a| a.code() == s)
    }

    /// The name shown in the View tab and above the list.
    pub fn label(self) -> &'static str {
        match self {
            Arrange::Date => "Date",
            Arrange::From => "From",
            Arrange::To => "To",
            Arrange::Subject => "Subject",
            Arrange::Size => "Size",
            Arrange::Flag => "Flag status",
            Arrange::Attachments => "Attachments",
        }
    }
}

/// Who a message is shown under: a name, else the address, else nothing.
pub(super) fn person(a: Option<&mailcraft_model::EmailAddress>) -> String {
    a.map(|a| if a.name.trim().is_empty() { a.email.trim().to_string() } else { a.name.trim().to_string() }).unwrap_or_default()
}

/// The heading pinned mail is under, above the rest of the list (translated where it's drawn).
pub const PINNED_HEADING: &str = "Pinned";

/// The fixed headings an arranged list shows (for translations).
pub const ARRANGE_HEADINGS: [&str; 14] = [
    "Under 10 KB",
    "10 to 25 KB",
    "25 to 100 KB",
    "100 to 500 KB",
    "500 KB to 1 MB",
    "1 to 5 MB",
    "Over 5 MB",
    "Flagged",
    "Completed",
    "Not flagged",
    "With attachments",
    "Without attachments",
    "(no subject)",
    "(nobody)",
];

/// Where a list arranged by the flag puts a message: flagged first, then those marked complete.
fn flag_rank(m: &Summary) -> u8 {
    if m.flags.flagged { 2 } else { u8::from(m.flags.follow_up_complete()) }
}

/// The size range a message is shown under.
pub(super) fn size_group(size: u32) -> &'static str {
    match size {
        0..10_240 => "Under 10 KB",
        10_240..25_600 => "10 to 25 KB",
        25_600..102_400 => "25 to 100 KB",
        102_400..512_000 => "100 to 500 KB",
        512_000..1_048_576 => "500 KB to 1 MB",
        1_048_576..5_242_880 => "1 to 5 MB",
        _ => "Over 5 MB",
    }
}

/// Sort a list as `arrange` says (each one's natural order; `reverse` the other way), newest
/// first among equals. (The list's own summaries are shared; a list of plain ones sorts alike.)
pub fn arrange_list<S: std::borrow::Borrow<Summary>>(list: &mut [S], arrange: Arrange, reverse: bool) {
    let newest = |a: &S, b: &S| {
        let (a, b): (&Summary, &Summary) = (a.borrow(), b.borrow());
        b.when().cmp(&a.when()).then_with(|| b.key.cmp(&a.key))
    };
    // By a name: each one's name lower-cased once (it was in every comparison, twice), over
    // the list already newest first, which the sort keeps among equal names.
    let text: Option<fn(&Summary) -> String> = match arrange {
        Arrange::From => Some(|m| person(Some(&m.from)).to_lowercase()),
        Arrange::To => Some(|m| person(m.to.first()).to_lowercase()),
        Arrange::Subject => Some(|m| m.base_subject().to_lowercase()),
        _ => None,
    };
    if let Some(text) = text {
        list.sort_by(newest);
        if reverse {
            list.sort_by_cached_key(|m| std::cmp::Reverse(text(m.borrow())));
        } else {
            list.sort_by_cached_key(|m| text(m.borrow()));
        }
        return;
    }
    list.sort_by(|a, b| {
        let (x, y): (&Summary, &Summary) = (a.borrow(), b.borrow());
        let primary = match arrange {
            Arrange::Size => y.size.cmp(&x.size),
            Arrange::Flag => flag_rank(y).cmp(&flag_rank(x)),
            Arrange::Attachments => y.has_attachments.cmp(&x.has_attachments),
            Arrange::Date | Arrange::From | Arrange::To | Arrange::Subject => y.when().cmp(&x.when()),
        };
        let primary = if reverse { primary.reverse() } else { primary };
        primary.then_with(|| newest(a, b))
    });
}

/// A line in the message list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A date heading (`Today`, `Last week`…), or another heading of a fixed text.
    Header(&'static str),
    /// A heading that's a name (a sender, a recipient, a subject), shown as it is.
    Group(String),
    /// A message (index into [`Session::list`]); with conversations on, the newest message of a
    /// conversation and how many messages (and unread ones) it holds.
    Message { index: usize, count: usize, unread: usize },
    /// Under a heading folded away (issue #189): how many messages it hides, and how many of
    /// them are unread. It takes no room; the heading above it says so.
    Folded { count: usize, unread: usize },
}

/// The heading a message goes under as the list is arranged (`date_heading` for a date's).
pub(super) fn heading_of(arrange: Arrange, m: Option<&Summary>, date_heading: impl FnOnce() -> &'static str) -> Row {
    let heading = match arrange {
        Arrange::Date => Row::Header(date_heading()),
        Arrange::From => Row::Group(m.map(|m| person(Some(&m.from))).unwrap_or_default()),
        Arrange::To => Row::Group(m.map(|m| person(m.to.first())).unwrap_or_default()),
        Arrange::Subject => Row::Group(m.map(|m| m.base_subject()).unwrap_or_default()),
        Arrange::Size => Row::Header(size_group(m.map_or(0, |m| m.size))),
        Arrange::Flag => Row::Header(match m.map(flag_rank) {
            Some(2) => "Flagged",
            Some(1) => "Completed",
            _ => "Not flagged",
        }),
        Arrange::Attachments => Row::Header(if m.is_some_and(|m| m.has_attachments) { "With attachments" } else { "Without attachments" }),
    };
    match heading {
        Row::Group(g) if g.trim().is_empty() => Row::Header(if arrange == Arrange::Subject { "(no subject)" } else { "(nobody)" }),
        h => h,
    }
}

/// What a message row stands for: one message of the list, or a conversation (an index into
/// [`Session::threads`]).
enum Stands {
    One(usize),
    Thread(usize),
}

impl Session {
    /// The list as rows with date headings (and conversations, when on); where each message row
    /// is among them; and each message's row, by its place in the list (its place among the
    /// message rows, [`NO_ROW`] for none).
    pub(crate) fn build_rows(&self) -> (Vec<Row>, Vec<u32>, Vec<u32>) {
        let mut out = Vec::with_capacity(self.list.len() + 8);
        let mut out_of = Vec::with_capacity(self.list.len());
        let mut last: Option<Row> = None;
        let arrange = self.prefs.arrange;
        // The date heading of the last day asked: the list's messages come a day after another
        // (working each one's out was most of laying out tens of thousands).
        let today = self.clock.local_day(self.clock.now);
        let mut by_day: Option<(i64, &'static str)> = None;
        let mut date_heading = |date: i64| {
            let day = self.clock.local_day(date);
            match by_day {
                Some((d, label)) if d == day => label,
                _ => {
                    let label = DateGroup::of_day(day, today).label();
                    by_day = Some((day, label));
                    label
                }
            }
        };
        // The headings folded away here (issue #189): their messages are counted, not shown.
        let folded = self.folded_here();
        let is_folded = |h: &Row| folded.is_some_and(|set| self.heading_key(h).is_some_and(|k| set.contains(&k)));
        let mut folded_at: Option<usize> = None;
        // A message's row under its heading; false when its heading is folded away.
        let mut push = |out: &mut Vec<Row>, index: usize, count: usize, unread: usize, date: i64| {
            let heading = heading_of(arrange, self.list.get(index).map(|m| &**m), || date_heading(date));
            if last.as_ref() != Some(&heading) {
                folded_at = is_folded(&heading).then_some(out.len() + 1);
                out.push(heading.clone());
                if folded_at.is_some() {
                    out.push(Row::Folded { count: 0, unread: 0 });
                }
                last = Some(heading);
            }
            if let Some(Row::Folded { count: c, unread: u }) = folded_at.and_then(|at| out.get_mut(at)) {
                *c = c.saturating_add(count);
                *u = u.saturating_add(unread);
                return false;
            }
            out.push(Row::Message { index, count, unread });
            true
        };
        // Pinned mail first, under its own heading (searches keep plain date order).
        let pins = self.search.is_none();
        let pinned = |i: usize| pins && self.list.get(i).is_some_and(|m| m.flags.pinned());
        let (mut pinned_rows, mut pinned_of) = (Vec::new(), Vec::new());
        if self.prefs.conversations && !self.threads.is_empty() {
            for (ti, t) in self.threads.iter().enumerate() {
                let Some(&newest) = t.messages.iter().max_by_key(|&&i| self.list.get(i).map_or(0, |m| m.when())) else { continue };
                let unread = t.messages.iter().filter(|&&i| self.list.get(i).is_some_and(|m| !m.flags.seen)).count();
                if t.messages.iter().any(|&i| pinned(i)) {
                    pinned_rows.push(Row::Message { index: newest, count: t.messages.len(), unread });
                    pinned_of.push(Stands::Thread(ti));
                } else if push(&mut out, newest, t.messages.len(), unread, t.latest) {
                    out_of.push(Stands::Thread(ti));
                }
            }
        } else {
            for (i, m) in self.list.iter().enumerate() {
                if pinned(i) {
                    pinned_rows.push(Row::Message { index: i, count: 1, unread: usize::from(!m.flags.seen) });
                    pinned_of.push(Stands::One(i));
                } else if push(&mut out, i, 1, usize::from(!m.flags.seen), m.when()) {
                    out_of.push(Stands::One(i));
                }
            }
        }
        let rows = if pinned_rows.is_empty() {
            out
        } else if is_folded(&Row::Header(PINNED_HEADING)) {
            // Pinned mail folded away too: its heading and how much it hides.
            let (count, unread) = pinned_rows.iter().fold((0usize, 0usize), |(c, u), r| match r {
                Row::Message { count, unread, .. } => (c.saturating_add(*count), u.saturating_add(*unread)),
                _ => (c, u),
            });
            let mut rows = vec![Row::Header(PINNED_HEADING), Row::Folded { count, unread }];
            rows.extend(out);
            rows
        } else {
            pinned_rows.insert(0, Row::Header(PINNED_HEADING));
            pinned_rows.extend(out);
            pinned_of.extend(out_of);
            out_of = pinned_of;
            pinned_rows
        };
        // The message rows are in the order `out_of` was pushed, pinned ones first.
        let msg_rows: Vec<u32> =
            rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Message { .. })).map(|(r, _)| u32::try_from(r).unwrap_or(NO_ROW)).collect();
        let mut of_list = vec![NO_ROW; self.list.len()];
        let mut put = |i: usize, p: u32| {
            if let Some(slot) = of_list.get_mut(i) {
                *slot = p;
            }
        };
        for (p, stands) in out_of.iter().enumerate() {
            // Past u32: no row, looked for instead.
            let Ok(p) = u32::try_from(p) else { break };
            match stands {
                Stands::One(i) => put(*i, p),
                Stands::Thread(t) => self.threads.get(*t).into_iter().flat_map(|t| &t.messages).for_each(|&i| put(i, p)),
            }
        }
        (rows, msg_rows, of_list)
    }
}
