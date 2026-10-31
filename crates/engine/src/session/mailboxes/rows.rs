//! The session: how the message list is arranged, and the rows it shows.

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
    let text = |s: String| s.to_lowercase();
    list.sort_by(|a, b| {
        let primary = match arrange {
            Arrange::Date => b.date.cmp(&a.date),
            Arrange::From => text(person(Some(&a.from))).cmp(&text(person(Some(&b.from)))),
            Arrange::To => text(person(a.to.first())).cmp(&text(person(b.to.first()))),
            Arrange::Subject => text(a.base_subject()).cmp(&text(b.base_subject())),
            Arrange::Size => b.size.cmp(&a.size),
            Arrange::Flag => b.flags.flagged.cmp(&a.flags.flagged),
            Arrange::Attachments => b.has_attachments.cmp(&a.has_attachments),
        };
        let primary = if reverse { primary.reverse() } else { primary };
        primary.then_with(|| b.date.cmp(&a.date)).then_with(|| b.key.cmp(&a.key))
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
