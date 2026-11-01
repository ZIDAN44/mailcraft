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

/// The fixed headings an arranged list shows (for translations).
pub const ARRANGE_HEADINGS: [&str; 13] = [
    "Under 10 KB",
    "10 to 25 KB",
    "25 to 100 KB",
    "100 to 500 KB",
    "500 KB to 1 MB",
    "1 to 5 MB",
    "Over 5 MB",
    "Flagged",
    "Not flagged",
    "With attachments",
    "Without attachments",
    "(no subject)",
    "(nobody)",
];

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
/// first among equals.
pub fn arrange_list(list: &mut [Summary], arrange: Arrange, reverse: bool) {
    let newest = |a: &Summary, b: &Summary| b.when().cmp(&a.when()).then_with(|| b.key.cmp(&a.key));
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
            list.sort_by_cached_key(|m| std::cmp::Reverse(text(m)));
        } else {
            list.sort_by_cached_key(text);
        }
        return;
    }
    list.sort_by(|a, b| {
        let primary = match arrange {
            Arrange::Size => b.size.cmp(&a.size),
            Arrange::Flag => b.flags.flagged.cmp(&a.flags.flagged),
            Arrange::Attachments => b.has_attachments.cmp(&a.has_attachments),
            Arrange::Date | Arrange::From | Arrange::To | Arrange::Subject => b.when().cmp(&a.when()),
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
        let mut push = |out: &mut Vec<Row>, index: usize, count: usize, unread: usize, date: i64| {
            // The heading this message goes under, as the list is arranged.
            let m = self.list.get(index);
            let heading = match arrange {
                Arrange::Date => Row::Header(DateGroup::of(date, &self.clock).label()),
                Arrange::From => Row::Group(m.map(|m| person(Some(&m.from))).unwrap_or_default()),
                Arrange::To => Row::Group(m.map(|m| person(m.to.first())).unwrap_or_default()),
                Arrange::Subject => Row::Group(m.map(Summary::base_subject).unwrap_or_default()),
                Arrange::Size => Row::Header(size_group(m.map_or(0, |m| m.size))),
                Arrange::Flag => Row::Header(if m.is_some_and(|m| m.flags.flagged) { "Flagged" } else { "Not flagged" }),
                Arrange::Attachments => Row::Header(if m.is_some_and(|m| m.has_attachments) { "With attachments" } else { "Without attachments" }),
            };
            let heading = match heading {
                Row::Group(g) if g.trim().is_empty() => Row::Header(if arrange == Arrange::Subject { "(no subject)" } else { "(nobody)" }),
                h => h,
            };
            if last.as_ref() != Some(&heading) {
                out.push(heading.clone());
                last = Some(heading);
            }
            out.push(Row::Message { index, count, unread });
        };
        // Pinned mail first, under its own heading (searches keep plain date order).
        let pins = self.search.is_none();
        let pinned = |i: usize| pins && self.list.get(i).is_some_and(|m| m.flags.pinned());
        let (mut pinned_rows, mut pinned_of) = (Vec::new(), Vec::new());
        if self.prefs.conversations && !self.threads.is_empty() {
            for (ti, t) in self.threads.iter().enumerate() {
                let Some(&newest) = t.messages.iter().max_by_key(|&&i| self.list.get(i).map_or(0, Summary::when)) else { continue };
                let unread = t.messages.iter().filter(|&&i| self.list.get(i).is_some_and(|m| !m.flags.seen)).count();
                if t.messages.iter().any(|&i| pinned(i)) {
                    pinned_rows.push(Row::Message { index: newest, count: t.messages.len(), unread });
                    pinned_of.push(Stands::Thread(ti));
                } else {
                    push(&mut out, newest, t.messages.len(), unread, t.latest);
                    out_of.push(Stands::Thread(ti));
                }
            }
        } else {
            for (i, m) in self.list.iter().enumerate() {
                if pinned(i) {
                    pinned_rows.push(Row::Message { index: i, count: 1, unread: usize::from(!m.flags.seen) });
                    pinned_of.push(Stands::One(i));
                } else {
                    push(&mut out, i, 1, usize::from(!m.flags.seen), m.when());
                    out_of.push(Stands::One(i));
                }
            }
        }
        let rows = if pinned_rows.is_empty() {
            out
        } else {
            pinned_rows.insert(0, Row::Header("Pinned"));
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
