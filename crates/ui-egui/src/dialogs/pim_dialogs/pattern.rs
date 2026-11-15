//! The event form's recurrence (issue #103): every so many days, weeks (on which weekdays),
//! months or years (on a day, or the nth weekday), and when the series ends. A rule the form
//! can't show stays "Custom", as it was.

use mailcraft_engine::model::ical::{PATTERN_MAX, Pattern, PatternEnd, PatternFreq, last_of_month, nth_of};

use super::*;

/// A `YYYY-MM-DD` date as typed, as days since 1970 (`None`: not one yet).
pub(crate) fn form_day(text: &str) -> Option<i64> {
    let s = text.trim();
    let num = |a: usize, b: usize| s.get(a..b).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse::<u32>().ok());
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let well = s.len() == 10 && s.get(4..5) == Some("-") && s.get(7..8) == Some("-") && (1..=12).contains(&m) && (1..=31).contains(&d);
    let day = dates::days_from_civil(i64::from(y), m, d);
    // Not 31 February: the day read back is the one typed.
    (well && dates::civil_from_days(day) == (i64::from(y), m, d)).then_some(day)
}

/// The frequency a plain Repeat choice stands for.
fn freq_of(r: &Repeat) -> Option<PatternFreq> {
    match r {
        Repeat::Daily => Some(PatternFreq::Daily),
        Repeat::Weekly => Some(PatternFreq::Weekly),
        Repeat::Monthly => Some(PatternFreq::Monthly),
        Repeat::Yearly => Some(PatternFreq::Yearly),
        Repeat::Never | Repeat::Custom(_) => None,
    }
}

/// The Repeat choice that shows a pattern's frequency.
pub(super) fn repeat_of(p: &Pattern) -> Repeat {
    match p.freq {
        PatternFreq::Daily => Repeat::Daily,
        PatternFreq::Weekly => Repeat::Weekly,
        PatternFreq::Monthly => Repeat::Monthly,
        PatternFreq::Yearly => Repeat::Yearly,
    }
}

/// After the Repeat list: the pattern follows the choice (a new one for another frequency, none
/// for Never or a rule kept as it is).
pub(super) fn follow_choice(f: &mut EventForm) {
    match freq_of(&f.repeat) {
        Some(freq) if f.pattern.as_ref().is_none_or(|p| p.freq != freq) => {
            let end = f.pattern.as_ref().map(|p| p.end).unwrap_or_default();
            f.pattern = Some(Pattern { end, ..Pattern::new(freq) });
        }
        Some(_) => {}
        None => f.pattern = None,
    }
}

/// The pattern's rows, under Repeat.
pub(super) fn rows(ui: &mut Ui, t: &Tokens, f: &mut EventForm) {
    let Some(day) = form_day(&f.date) else { return };
    let until_text = &mut f.until_text;
    let Some(p) = f.pattern.as_mut() else { return };
    let unit = match p.freq {
        PatternFreq::Daily => "days",
        PatternFreq::Weekly => "weeks",
        PatternFreq::Monthly => "months",
        PatternFreq::Yearly => "years",
    };
    form_row(ui, t, true, None, "Repeats every", |ui| {
        ui.allocate_ui_with_layout(vec2(ui.available_width(), FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::DragValue::new(&mut p.interval).range(1..=PATTERN_MAX)).accessible_name(tl!("Repeats every"));
            ui.label(crate::i18n::tc("interval", unit));
        });
    });
    match p.freq {
        PatternFreq::Daily => {}
        PatternFreq::Weekly => form_row(ui, t, true, None, "On these days", |ui| {
            ui.allocate_ui_with_layout(vec2(ui.available_width(), FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let start = dates::weekday_index(day);
                for (i, name) in (0u32..).zip(dates::WEEKDAYS) {
                    // None picked: the start's.
                    let on = p.days.contains(&i) || (p.days.is_empty() && i == start);
                    if ui.selectable_label(on, tl!(name)).clicked() {
                        if p.days.is_empty() {
                            p.days.push(start);
                        }
                        if on {
                            p.days.retain(|d| *d != i);
                        } else {
                            p.days.push(i);
                            p.days.sort_unstable();
                        }
                    }
                }
            });
        }),
        PatternFreq::Monthly | PatternFreq::Yearly => {
            let (_, month, mday) = dates::civil_from_days(day);
            let weekday = tl!(dates::WEEKDAYS_LONG.get(usize::try_from(dates::weekday_index(day)).unwrap_or(0)).copied().unwrap_or("Monday"));
            let month = crate::calendar::month_name(month);
            let yearly = p.freq == PatternFreq::Yearly;
            let nth_word = |n: i32| {
                crate::i18n::tc("nth", ["first", "second", "third", "fourth"].get(usize::try_from(n - 1).unwrap_or(9)).copied().unwrap_or("last"))
            };
            let label = |nth: Option<i32>| match (nth, yearly) {
                (None, false) => tf!("On day {day}", day = mday),
                (None, true) => tf!("On {month} {day}", month = month, day = mday),
                (Some(n), false) => tf!("On the {nth} {weekday}", nth = nth_word(n), weekday = weekday),
                (Some(n), true) => tf!("On the {nth} {weekday} of {month}", nth = nth_word(n), weekday = weekday, month = month),
            };
            // What the start's date allows: its day, which of its weekday it is, and the last one.
            let mut choices = vec![None];
            if nth_of(day) <= 4 {
                choices.push(Some(nth_of(day)));
            }
            if last_of_month(day) {
                choices.push(Some(-1));
            }
            // The start moved to a day the choice no longer fits: its day of the month.
            if !choices.contains(&p.nth) {
                p.nth = None;
            }
            form_row(ui, t, true, None, "Which day", |ui| {
                ui.allocate_ui_with_layout(vec2(300.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("event-repeat-day")
                        .width(294.0)
                        .selected_text(label(p.nth))
                        .show_ui(ui, |ui| {
                            for c in choices {
                                ui.selectable_value(&mut p.nth, c, label(c));
                            }
                        })
                        .response
                        .accessible_name(tl!("Which day"));
                });
            });
        }
    }
    // When it ends: a list, and the count or the last day beside it, on one line (the form stays
    // short enough to keep its buttons in view).
    form_row(ui, t, true, None, "Series ends", |ui| {
        ui.allocate_ui_with_layout(vec2(ui.available_width(), FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            // A month on, as a first guess.
            if until_text.trim().is_empty() {
                *until_text = dates::day_string(day.saturating_add(30));
            }
            let count = match p.end {
                PatternEnd::Count(n) => n,
                _ => 10,
            };
            let label = |e: &PatternEnd| match e {
                PatternEnd::Never => tl!("Never"),
                PatternEnd::Count(_) => tl!("After"),
                PatternEnd::Until(_) => tl!("Until"),
            };
            let choices = [PatternEnd::Never, PatternEnd::Count(count), PatternEnd::Until(form_day(until_text).unwrap_or(day))];
            egui::ComboBox::from_id_salt("event-repeat-end")
                .width(110.0)
                .selected_text(label(&p.end))
                .show_ui(ui, |ui| {
                    for c in choices {
                        let on = std::mem::discriminant(&c) == std::mem::discriminant(&p.end);
                        if ui.selectable_label(on, label(&c)).clicked() {
                            p.end = c;
                        }
                    }
                })
                .response
                .accessible_name(tl!("Series ends"));
            match p.end {
                PatternEnd::Never => {}
                PatternEnd::Count(mut n) => {
                    if ui.add(egui::DragValue::new(&mut n).range(1..=PATTERN_MAX)).accessible_name(tl!("After")).changed() {
                        p.end = PatternEnd::Count(n);
                    }
                    ui.label(crate::i18n::tc("count", "occurrences"));
                }
                PatternEnd::Until(_) => {
                    if date_pick::date_field(ui, t, "event-repeat-until", until_text)
                        && let Some(d) = form_day(until_text)
                    {
                        p.end = PatternEnd::Until(d);
                    }
                }
            }
        });
    });
}

/// The rule to send for the form's pattern, or its Repeat choice as it is.
pub(super) fn repeat_param(f: &EventForm) -> String {
    match (&f.pattern, form_day(&f.date)) {
        (Some(p), Some(day)) => p.rule(day),
        _ => f.repeat.param().to_string(),
    }
}
