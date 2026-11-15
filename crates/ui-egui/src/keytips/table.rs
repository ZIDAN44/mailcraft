//! Which letters reach what: the ribbon's tabs, the search box, and each tab's buttons, as
//! Outlook (classic) shows them when Alt is pressed (watched from its window, clean room). A
//! button is known by its label in English, as the ribbon names it.

use crate::RibbonTab;

/// Where the first letters go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Top {
    /// The File page.
    File,
    /// A ribbon tab: its buttons' letters follow.
    Tab(RibbonTab),
    /// The search box.
    Search,
}

/// The letters on the tabs and the search box, after Alt.
pub const TOPS: &[(&str, Top)] = &[
    ("F", Top::File),
    ("H", Top::Tab(RibbonTab::Home)),
    ("JS", Top::Tab(RibbonTab::SendReceive)),
    ("V", Top::Tab(RibbonTab::View)),
    ("Q", Top::Search),
];

/// Each tab's buttons: their letters after the tab's, and their labels. Two buttons that take
/// each other's place (Mark read, Mark unread) share their letters.
pub const BUTTONS: &[(RibbonTab, &str, &str)] = &[
    (RibbonTab::Home, "N", "New mail"),
    (RibbonTab::Home, "D", "Delete"),
    (RibbonTab::Home, "O", "Archive"),
    (RibbonTab::Home, "R1", "Report junk"),
    (RibbonTab::Home, "R1", "Not junk"),
    (RibbonTab::Home, "MV", "Move to"),
    (RibbonTab::Home, "QS", "Quick steps"),
    (RibbonTab::Home, "RP", "Reply"),
    (RibbonTab::Home, "RA", "Reply all"),
    (RibbonTab::Home, "FW", "Forward"),
    (RibbonTab::Home, "W", "Mark read"),
    (RibbonTab::Home, "W", "Mark unread"),
    (RibbonTab::Home, "G", "Categorize"),
    (RibbonTab::Home, "U", "Flag"),
    (RibbonTab::Home, "L", "Filter"),
    (RibbonTab::SendReceive, "S", "Send / Receive all folders"),
    (RibbonTab::SendReceive, "A", "Send all"),
    (RibbonTab::SendReceive, "D", "Update folder"),
    (RibbonTab::SendReceive, "W", "Work offline"),
    (RibbonTab::View, "W", "Focused Inbox"),
    (RibbonTab::View, "AB", "Arrange by"),
    (RibbonTab::View, "RS", "Reverse sort"),
    (RibbonTab::View, "U", "Compact"),
    (RibbonTab::View, "Y2", "Reading pane"),
];

/// The letters that reach `top`.
pub fn letters(top: Top) -> Option<&'static str> {
    TOPS.iter().find_map(|(letters, t)| (*t == top).then_some(*letters))
}

/// The letters that open `tab`.
pub fn tab_letters(tab: RibbonTab) -> Option<&'static str> {
    letters(Top::Tab(tab))
}

/// The whole sequence of a button on `tab` labelled `label`, in its two parts: its tab's letters,
/// then its own.
pub fn button_letters(tab: RibbonTab, label: &str) -> Option<(&'static str, &'static str)> {
    let own = BUTTONS.iter().find_map(|(t, letters, l)| (*t == tab && *l == label).then_some(*letters))?;
    Some((tab_letters(tab)?, own))
}

/// What a whole sequence reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Top(Top),
    /// A button: the sequence that names it.
    Button(String),
}

/// Every whole sequence, in two parts (a button's: its tab's letters, then its own), and the
/// top it reaches (none: a button).
fn all() -> impl Iterator<Item = (&'static str, &'static str, Option<Top>)> {
    let tops = TOPS.iter().map(|(letters, top)| (*letters, "", Some(*top)));
    let buttons = BUTTONS.iter().filter_map(|(tab, letters, _)| Some((tab_letters(*tab)?, *letters, None)));
    tops.chain(buttons)
}

/// What `sequence` reaches, if it's whole.
pub fn lookup(sequence: &str) -> Option<Entry> {
    let (_, _, top) = all().find(|(head, tail, _)| sequence.strip_prefix(head) == Some(tail))?;
    Some(top.map_or_else(|| Entry::Button(sequence.to_string()), Entry::Top))
}

/// Whether a longer sequence starts with `prefix` (more letters are waited for).
pub fn has_deeper(prefix: &str) -> bool {
    all().any(|(head, tail, _)| head.len() + tail.len() > prefix.len() && starts_with(head, tail, prefix))
}

/// Whether `head` then `tail` starts with `prefix`.
fn starts_with(head: &str, tail: &str, prefix: &str) -> bool {
    match prefix.strip_prefix(head) {
        Some(rest) => tail.starts_with(rest),
        None => head.starts_with(prefix),
    }
}
