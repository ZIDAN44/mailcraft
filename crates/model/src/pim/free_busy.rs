//! Someone's free/busy times (issue #140): when they're taken, and what by when they share that,
//! as Exchange's availability service gives them for a colleague who shares only those.

use serde::{Deserialize, Serialize};

use super::ical::ShowAs;

/// A time someone's calendar has taken.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Busy {
    pub start: i64,
    pub end: i64,
    /// Busy, tentative, out of office or working elsewhere (never free: a free time isn't one).
    pub show_as: ShowAs,
    /// What it is and where, when they share that much (empty otherwise).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub location: String,
    /// Marked private: its time alone is shown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private: bool,
}

/// One person's free/busy times over a window, or why there are none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeBusyAnswer {
    /// Their address, as it was asked for.
    pub email: String,
    pub busy: Vec<Busy>,
    pub error: Option<String>,
    /// When they work, when the server says (issue #185).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<super::work_hours::WorkHours>,
}
