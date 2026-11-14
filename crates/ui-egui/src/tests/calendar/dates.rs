//! The calendar's dates: month names, days written out, and the actions that show a day, as
//! each place wrote them by hand before they shared one helper.

use mailcraft_engine::model::dates::{self, civil_from_days, days_from_civil};

use super::*;

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// `long_date` as it was written: its own weekday names and `(day + 3).rem_euclid(7)`.
fn long_date_as_it_was(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    let month = crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")).to_string();
    let wd = (day + 3).rem_euclid(7) as usize;
    let day_name = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].get(wd).copied().unwrap_or("");
    tf!("{weekday}, {day} {month} {year}", weekday = tl!(day_name), day = d, month = month, year = y)
}

/// The actions' ids and parameters, to compare.
fn runs(acts: &[crate::Action]) -> Vec<(&'static str, serde_json::Value)> {
    acts.iter().filter_map(|a| if let crate::Action::Run(id, v) = a { Some((*id, v.clone())) } else { None }).collect()
}

#[test]
fn month_names_and_days_written_out_as_before() {
    for lang in ["en", "de", "ar", "ja"] {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        for m in 1..=12u32 {
            assert_eq!(crate::calendar::month_name(m), crate::i18n::tc("month", MONTHS[m as usize - 1]), "{lang} {m}");
        }
        let (first, last) = (days_from_civil(1, 1, 1), days_from_civil(9999, 12, 31));
        for day in (first..=last).step_by(997).chain([-1, 0, 1, first - 1, last + 1, -1_000_000_000, 1_000_000_000]) {
            assert_eq!(crate::calendar::long_date(day), long_date_as_it_was(day), "{lang} {day}");
        }
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(crate::calendar::long_date(days_from_civil(2026, 10, 22)), "Thursday, 22 October 2026");
    assert_eq!((crate::calendar::month_name(0), crate::calendar::month_name(13)), ("", ""));
}

/// Showing a day (from an invitation, the To-Do Bar's month) and going to one (the calendar's
/// "more", a search result) ask for what they asked before, the date as `civil` wrote it.
#[test]
fn a_day_is_shown_as_before() {
    for clock in [Clock::new(NOW, 0), Clock::new(NOW, -14 * 3600), Clock::new(NOW, 14 * 3600)] {
        for t in (-62_000_000_000i64..253_000_000_000).step_by(99_999_989) {
            let c = dates::civil(t, &clock);
            let date = format!("{:04}-{:02}-{:02}", c.year, c.month, c.day);
            let mut acts = Vec::new();
            crate::calendar::show_day(clock.local_day(t), &mut acts);
            assert_eq!(
                runs(&acts),
                vec![
                    ("view.module", json!({"name": "calendar"})),
                    ("calendar.view", json!({"mode": "day"})),
                    ("calendar.goto", json!({"date": date})),
                ],
                "{t}"
            );
            assert_eq!(runs(&[crate::calendar::goto(clock.local_day(t))]), vec![("calendar.goto", json!({"date": date}))]);
        }
    }
}
