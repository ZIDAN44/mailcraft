//! Organising mail: categories (colour-coded keywords), inbox rules, snoozing, and the
//! Focused/Other split of the Inbox. Pure data and matching: the engine applies them.

use serde::{Deserialize, Serialize};

use crate::dates::{Clock, civil_from_days};
use crate::{FolderRole, Summary, keyword_for};

/// A category: a name and colour shown on messages, stored on the server as a keyword so other
/// clients (and other computers) see it too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    pub keyword: String,
    /// An index into the UI's category palette.
    #[serde(default)]
    pub color: u8,
}

impl Category {
    pub fn new(name: &str, color: u8) -> Category {
        Category { name: name.trim().to_string(), keyword: keyword_for(name), color }
    }
}

/// The categories a new profile starts with. Their keywords are the ones Thunderbird uses for its
/// five default tags, so tags made there show here and back.
pub fn default_categories() -> Vec<Category> {
    [("Important", "$label1", 0), ("Work", "$label2", 1), ("Personal", "$label3", 2), ("To do", "$label4", 3), ("Later", "$label5", 4)]
        .into_iter()
        .map(|(n, k, c)| Category { name: n.into(), keyword: k.into(), color: c })
        .collect()
}

/// What a rule looks at. Text tests are case-insensitive "contains".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "if", content = "value", rename_all = "camelCase")]
pub enum Condition {
    /// The sender's name or address.
    From(String),
    /// Any To or Cc recipient.
    To(String),
    Subject(String),
    /// Subject, sender or recipients.
    AnyText(String),
    HasAttachment,
    /// Newsletters, lists, automatic mail.
    Bulk,
}

impl Condition {
    pub fn matches(&self, m: &Summary) -> bool {
        let has = |hay: &str, needle: &str| !needle.trim().is_empty() && hay.to_lowercase().contains(&needle.trim().to_lowercase());
        let addr = |a: &crate::EmailAddress| format!("{} {}", a.name, a.email);
        match self {
            Condition::From(t) => has(&addr(&m.from), t),
            Condition::To(t) => m.to.iter().chain(&m.cc).any(|a| has(&addr(a), t)),
            Condition::Subject(t) => has(&m.subject, t),
            Condition::AnyText(t) => has(&m.subject, t) || has(&addr(&m.from), t) || m.to.iter().chain(&m.cc).any(|a| has(&addr(a), t)),
            Condition::HasAttachment => m.has_attachments,
            Condition::Bulk => m.bulk,
        }
    }
}

/// What a rule does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "do", content = "value", rename_all = "camelCase")]
pub enum RuleAction {
    /// Move to a folder (its path) of the same account.
    MoveTo(String),
    /// Move to the folder with this role (Archive, Junk…).
    MoveToRole(FolderRole),
    MarkRead,
    Flag,
    Pin,
    /// Add a category (its name).
    Categorize(String),
    /// Move to Deleted Items.
    Delete,
}

/// An inbox rule, run on mail as it arrives in an Inbox (the client applies it, like Outlook's
/// client-side rules).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Only this account's mail (`None`: every account).
    #[serde(default)]
    pub account: Option<String>,
    pub conditions: Vec<Condition>,
    /// All conditions must match (`false`: any one).
    #[serde(default = "yes")]
    pub match_all: bool,
    pub actions: Vec<RuleAction>,
    /// Don't run later rules on a message this one matched.
    #[serde(default)]
    pub stop: bool,
}

fn yes() -> bool {
    true
}

impl Rule {
    pub fn matches(&self, m: &Summary) -> bool {
        if !self.enabled || self.conditions.is_empty() || self.actions.is_empty() {
            return false;
        }
        if self.account.as_ref().is_some_and(|a| *a != m.key.account) {
            return false;
        }
        if self.match_all { self.conditions.iter().all(|c| c.matches(m)) } else { self.conditions.iter().any(|c| c.matches(m)) }
    }
}

/// The actions of every enabled rule that matches `m`, in rule order.
pub fn actions_for(rules: &[Rule], m: &Summary) -> Vec<RuleAction> {
    let mut out = Vec::new();
    for r in rules.iter().filter(|r| r.matches(m)) {
        out.extend(r.actions.iter().cloned());
        if r.stop {
            break;
        }
    }
    out
}

/// A snoozed message: it waits in the Snoozed folder and comes back to `back_to` at `until`.
/// Found again by Message-ID, since moving gives it a new UID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snooze {
    pub account: String,
    pub message_id: String,
    /// The Snoozed folder's path.
    pub folder: String,
    pub back_to: String,
    pub until: i64,
    #[serde(default)]
    pub subject: String,
}

/// Snooze choices, as the menu offers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnoozeUntil {
    /// In three hours.
    LaterToday,
    /// 8:00 tomorrow.
    Tomorrow,
    /// 8:00 next Saturday.
    Weekend,
    /// 8:00 next Monday.
    NextWeek,
}

impl SnoozeUntil {
    pub const ALL: [SnoozeUntil; 4] = [SnoozeUntil::LaterToday, SnoozeUntil::Tomorrow, SnoozeUntil::Weekend, SnoozeUntil::NextWeek];

    pub fn code(self) -> &'static str {
        match self {
            SnoozeUntil::LaterToday => "later",
            SnoozeUntil::Tomorrow => "tomorrow",
            SnoozeUntil::Weekend => "weekend",
            SnoozeUntil::NextWeek => "nextWeek",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SnoozeUntil::LaterToday => "Later today",
            SnoozeUntil::Tomorrow => "Tomorrow",
            SnoozeUntil::Weekend => "This weekend",
            SnoozeUntil::NextWeek => "Next week",
        }
    }

    pub fn from_code(s: &str) -> Option<SnoozeUntil> {
        SnoozeUntil::ALL.into_iter().find(|p| p.code().eq_ignore_ascii_case(s))
    }

    /// When, in Unix seconds, for a user at `clock` (local time).
    pub fn at(self, clock: &Clock) -> i64 {
        let day = clock.local(clock.now).div_euclid(86_400);
        let morning = |d: i64| clock.utc(d * 86_400 + 8 * 3600);
        // 1970-01-01 was a Thursday: 0 = Monday … 6 = Sunday.
        let weekday = (day + 3).rem_euclid(7);
        match self {
            SnoozeUntil::LaterToday => {
                let t = clock.now + 3 * 3600;
                t - t.rem_euclid(3600) + 3600 * i64::from(t.rem_euclid(3600) > 0)
            }
            SnoozeUntil::Tomorrow => morning(day + 1),
            SnoozeUntil::Weekend => morning(day + if weekday < 5 { 5 - weekday } else { 7 - weekday + 5 }),
            SnoozeUntil::NextWeek => morning(day + 7 - weekday),
        }
    }
}

/// The calendar date of a day number (for tests and labels).
pub fn date_of(day: i64) -> (i64, u32, u32) {
    civil_from_days(day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EmailAddress, MessageKey};

    fn m(from: &str, subject: &str, bulk: bool) -> Summary {
        Summary {
            key: MessageKey { account: "a1".into(), ..Default::default() },
            from: EmailAddress::new("", from),
            to: vec![EmailAddress::new("Me", "me@x.example")],
            subject: subject.into(),
            bulk,
            ..Default::default()
        }
    }

    #[test]
    fn rules_match_and_stop() {
        let news = Rule {
            name: "News".into(),
            enabled: true,
            account: None,
            conditions: vec![Condition::Bulk, Condition::From("letters".into())],
            match_all: true,
            actions: vec![RuleAction::MoveTo("News".into()), RuleAction::MarkRead],
            stop: true,
        };
        let flag = Rule {
            name: "Boss".into(),
            conditions: vec![Condition::From("boss@".into())],
            actions: vec![RuleAction::Flag],
            stop: false,
            ..news.clone()
        };
        let rules = vec![news.clone(), flag.clone()];
        assert_eq!(actions_for(&rules, &m("letters@shop.example", "Sale", true)), vec![RuleAction::MoveTo("News".into()), RuleAction::MarkRead]);
        assert!(actions_for(&rules, &m("letters@shop.example", "Hi", false)).is_empty(), "all conditions must hold");
        assert_eq!(actions_for(&rules, &m("BOSS@x.example", "Now", false)), vec![RuleAction::Flag]);
        let any = Rule { match_all: false, ..news.clone() };
        assert!(any.matches(&m("x@y.example", "Hi", true)));
        assert!(!Rule { enabled: false, ..news.clone() }.matches(&m("letters@x", "", true)));
        assert!(!Rule { account: Some("a2".into()), ..news }.matches(&m("letters@x", "", true)));
        assert!(Condition::To("me@".into()).matches(&m("a@b", "", false)));
        assert!(!Condition::Subject("  ".into()).matches(&m("a@b", "anything", false)), "an empty test matches nothing");
        // Rules round-trip through JSON (rules.json, the CLI).
        let json = serde_json::to_string(&rules).unwrap();
        assert!(json.contains("\"if\":\"bulk\""), "{json}");
        assert_eq!(serde_json::from_str::<Vec<Rule>>(&json).unwrap(), rules);
    }

    #[test]
    fn snooze_times_are_local() {
        // Wednesday 2026-10-14 15:20 UTC, in UTC+2 (17:20 local).
        let clock = Clock::new(1_791_991_200 + 20 * 60, 7200);
        let local_day = |t: i64| date_of((t + 7200).div_euclid(86_400));
        let local_hour = |t: i64| (t + 7200).rem_euclid(86_400) / 3600;
        let later = SnoozeUntil::LaterToday.at(&clock);
        assert!(later > clock.now + 3 * 3600 - 1 && later % 3600 == 0);
        let t = SnoozeUntil::Tomorrow.at(&clock);
        assert_eq!((local_day(t), local_hour(t)), ((2026, 10, 15), 8));
        let w = SnoozeUntil::Weekend.at(&clock);
        assert_eq!((local_day(w), local_hour(w)), ((2026, 10, 17), 8), "Saturday");
        let n = SnoozeUntil::NextWeek.at(&clock);
        assert_eq!((local_day(n), local_hour(n)), ((2026, 10, 19), 8), "Monday");
        // On a Sunday, "this weekend" is next Saturday and "next week" is tomorrow.
        let sunday = Clock::new(1_792_324_800, 0); // 2026-10-18 12:00 UTC
        assert_eq!(date_of(sunday.now.div_euclid(86_400)), (2026, 10, 18));
        assert_eq!(date_of(SnoozeUntil::NextWeek.at(&sunday).div_euclid(86_400)), (2026, 10, 19));
        assert_eq!(date_of(SnoozeUntil::Weekend.at(&sunday).div_euclid(86_400)), (2026, 10, 24));
        assert_eq!(SnoozeUntil::from_code("NEXTWEEK"), Some(SnoozeUntil::NextWeek));
    }

    #[test]
    fn default_categories_use_shared_keywords() {
        let c = default_categories();
        assert_eq!(c[0].keyword, "$label1");
        assert_eq!(Category::new(" Clients ", 6), Category { name: "Clients".into(), keyword: "clients".into(), color: 6 });
    }
}
