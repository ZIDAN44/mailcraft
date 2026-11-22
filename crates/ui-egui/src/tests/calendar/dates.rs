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

/// An all-day invitation's card says its day as the follow-up dates do: its weekday as the
/// language writes it (whatever that has in it: the card cut a date and time at its second
/// space) and its date in the order chosen, the day the event is on wherever the viewer is. A
/// timed one says its start and end as before.
#[test]
fn an_invitation_says_its_day() {
    const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    use mailcraft_engine::model::dates::{DateOrder, DateStyle, zone::Zone};
    let orders = [DateOrder::Iso, DateOrder::DayMonthYear, DateOrder::MonthDayYear, DateOrder::DayMonthYearDots, DateOrder::YearMonthDayDots];
    let zones =
        [Clock::new(NOW, 0), Clock::new(NOW, -14 * 3600), Clock::new(NOW, 14 * 3600), Clock::new(NOW, 0).in_zone(Zone::named("America/New_York"))];
    let mut checked = 0;
    for lang in crate::i18n::Lang::all() {
        crate::i18n::set_current(lang);
        for zone in &zones {
            for order in orders {
                for hours12 in [false, true] {
                    let mut clock = *zone;
                    clock.style = DateStyle { order, hours12, full_in_list: false };
                    let s = Session::open(None, clock);
                    for day in (-800_000i64..3_000_000).step_by(9_973).chain([-1, 0, 1, 20_740]) {
                        let mut e = mailcraft_engine::model::ical::CalEvent {
                            all_day: true,
                            start: day * 86_400,
                            end: (day + 1) * 86_400,
                            ..Default::default()
                        };
                        let (y, m, d) = civil_from_days(day);
                        let date = match order {
                            DateOrder::Iso => format!("{y:04}-{m:02}-{d:02}"),
                            DateOrder::DayMonthYear => format!("{d:02}/{m:02}/{y:04}"),
                            DateOrder::MonthDayYear => format!("{m:02}/{d:02}/{y:04}"),
                            DateOrder::DayMonthYearDots => format!("{d:02}.{m:02}.{y:04}"),
                            DateOrder::YearMonthDayDots => format!("{y:04}.{m:02}.{d:02}"),
                        };
                        // 1 January 1970 was a Thursday.
                        let weekday = crate::i18n::t(WEEKDAYS[(day + 3).rem_euclid(7) as usize]);
                        assert_eq!(crate::reading::invite_when(&e, &s), format!("{weekday} {date}"), "{} {day}", lang.code());
                        e.all_day = false;
                        e.start = day * 86_400 + 13 * 3600 + 5 * 60;
                        e.end = e.start + 3600;
                        let was = format!("{} – {}", crate::i18n::date(&dates::full_time(e.start, &clock)), crate::calendar::hm(e.end, &s));
                        assert_eq!(crate::reading::invite_when(&e, &s), was, "{} {day}", lang.code());
                        checked += 1;
                    }
                }
            }
        }
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert!(checked > 100_000, "{checked}");
    let s = Session::open(None, Clock::new(NOW, 0));
    let e = mailcraft_engine::model::ical::CalEvent { all_day: true, start: 20_740 * 86_400, end: 20_741 * 86_400, ..Default::default() };
    assert_eq!(crate::reading::invite_when(&e, &s), "Wed 2026-10-14");
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

/// Audit #476: in the smallest window MailCraft goes to, the month view shows all six of its
/// weeks: each day's number inside the calendar's pane (the last weeks were drawn below its edge,
/// out of reach). And in a roomy window its rows aren't squeezed.
#[test]
fn the_month_view_shows_every_week_in_a_small_window() {
    let (mut a, _) = app("month-small");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "calendar"}));
    a.run("calendar.view", json!({"mode": "month"}));
    // The classic ribbon, taller, leaves the least room.
    for (size, classic) in [(egui::vec2(760.0, 480.0), false), (egui::vec2(760.0, 480.0), true), (egui::vec2(1280.0, 800.0), false)] {
        a.run("view.classicRibbon", json!({ "on": classic }));
        for _ in 0..3 {
            let _ = drawn_with(&ctx, &mut a, size, vec![]);
        }
        let texts = drawn_with(&ctx, &mut a, size, vec![]);
        // The grid's heading (its first weekday) and its days' numbers, in the same room.
        let head = texts.iter().filter(|d| d.text == tl!(dates::WEEKDAYS[0])).max_by(|x, y| x.rect.min.y.total_cmp(&y.rect.min.y)).unwrap();
        let days: Vec<&Drawn> = texts.iter().filter(|d| d.clip == head.clip && d.text.parse::<u32>().is_ok_and(|n| (1..=31).contains(&n))).collect();
        assert_eq!(days.len(), 42, "{size:?}: {:?}", days.iter().map(|d| &d.text).collect::<Vec<_>>());
        let hidden: Vec<String> = days.iter().filter(|d| d.rect.max.y > d.clip.max.y).map(|d| format!("{} at {:?}", d.text, d.rect)).collect();
        assert!(hidden.is_empty(), "{size:?}: below the pane's edge ({:?}): {hidden:?}", head.clip);
    }
}

/// Issue #215: a month after its day takes the form a language gives it there, Russian's and
/// Slovak's genitive (`четверг, 17 декабря 2026`), the month alone as it was (`Декабрь 2026`),
/// the others as before; and the message list's date headings start with a capital, a weekday's
/// too (`Понедельник`, under `Сегодня`), as a heading does.
#[test]
fn a_month_in_a_date_and_a_weekday_heading() {
    let dec17 = days_from_civil(2026, 12, 17);
    let lang = |code: &str| crate::i18n::set_current(crate::i18n::Lang::from_code(code).unwrap());
    lang("ru");
    assert_eq!(crate::calendar::long_date(dec17), "четверг, 17 декабря 2026");
    assert_eq!(crate::calendar::month_name(12), "Декабрь", "alone");
    assert_eq!((crate::calendar::date_month_name(5), crate::calendar::month_name(5)), ("мая", "Май"));
    assert_eq!(crate::i18n::heading(tl!("Monday")), "Понедельник");
    lang("sk");
    assert_eq!(crate::calendar::long_date(dec17), "štvrtok 17. decembra 2026");
    lang("tr");
    assert_eq!(crate::i18n::heading("ikindi"), "İkindi");
    for code in ["fr", "es", "nl", "pt-br", "it"] {
        lang(code);
        assert_eq!(crate::calendar::long_date(dec17), long_date_as_it_was(dec17), "{code}");
    }

    lang("ru");
    let (mut a, _) = app("ru-headings");
    let ctx = egui::Context::default();
    // Mail from Monday (NOW is a Wednesday): under a weekday's heading.
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9971 };
    let raw = b"From: Ana <ana@studio.example>\r\nSubject: Monday's cut\r\n\r\nHi";
    inbox.messages.insert(0, mailcraft_engine::mime::summary_from_full(raw, key, Default::default(), 0, NOW - 2 * 86_400).into());
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.ui.language = "ru".into();
    let headings: Vec<&'static str> =
        a.session.rows().iter().filter_map(|r| if let mailcraft_engine::Row::Header(h) = r { Some(*h) } else { None }).collect();
    let mut drawn = Vec::new();
    for _ in 0..3 {
        drawn = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 800.0));
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let weekdays = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    let shown: Vec<&String> = drawn.iter().filter(|d| headings.iter().any(|h| weekdays.contains(h) && d.to_lowercase() == tl_ru(h))).collect();
    assert!(!shown.is_empty(), "a weekday heading in the list: {headings:?}");
    assert!(shown.iter().all(|d| d.chars().next().is_some_and(char::is_uppercase)), "{shown:?}");
}

/// A weekday as Russian writes it inside a sentence.
fn tl_ru(day: &str) -> String {
    let ru = ["понедельник", "вторник", "среда", "четверг", "пятница", "суббота", "воскресенье"];
    let en = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    en.iter().position(|d| *d == day).and_then(|i| ru.get(i)).map(|s| s.to_string()).unwrap_or_default()
}
