//! A date field with a month to pick a day from beside it (issue #103): typed, in the order
//! File › Options writes dates in or as `YYYY-MM-DD` (issue #162), or picked.

use super::pattern::form_day;
use super::*;

/// The first day a month's grid shows (`month`: months since year 0): the Monday on or before
/// its first.
fn grid_start(month: i64) -> i64 {
    let first = dates::days_from_civil(month.div_euclid(12), u32::try_from(month.rem_euclid(12) + 1).unwrap_or(1), 1);
    first - i64::from(dates::weekday_index(first))
}

/// A date box and its month: true when the date changed (typed or picked). `id` tells this
/// form's boxes apart. `text` is the form's (`YYYY-MM-DD` when it's a day, else what was typed,
/// which the form refuses as it did); the box shows and reads it in `order` (issue #162: always
/// `YYYY-MM-DD`, whatever Dates was set to). `hint`: what an empty box says, its pattern when
/// `None`.
pub(crate) fn date_field(ui: &mut Ui, t: &Tokens, id: &str, text: &mut String, order: dates::DateOrder, hint: Option<&str>) -> bool {
    // What the box shows, kept with the form's text it was made from: made again when that
    // changes otherwise (picked, or filled in by the form), as it's typed meanwhile.
    let box_id = egui::Id::unique(("date-pick-box", id));
    let (from, mut shown): (String, String) = ui.ctx().data(|d| d.get_temp(box_id)).unwrap_or_default();
    if from != *text || (shown.is_empty() && !text.is_empty()) {
        shown = form_day(text).map_or_else(|| text.clone(), |d| dates::typed_day(d, order));
    }
    let typed = field(ui, &mut shown, hint.unwrap_or(pattern(order)), 120.0).changed();
    if typed {
        *text = dates::read_day(&shown, order).map_or_else(|| shown.clone(), dates::day_string);
    }
    let button = widgets::tiny_icon_button(ui, Icon::Calendar, "Pick a date", t);
    let shown_id = egui::Id::unique(("date-pick-month", id));
    if button.clicked() {
        // Opened on the date in the box (its month), or the one shown last.
        if let Some(day) = form_day(text) {
            let (y, m, _) = dates::civil_from_days(day);
            ui.ctx().data_mut(|d| d.insert_temp(shown_id, y * 12 + i64::from(m) - 1));
        }
    }
    let mut picked = None;
    egui::Popup::menu(&button).id(egui::Id::unique(("date-pick", id))).show(|ui| {
        let chosen = form_day(text);
        let fallback = chosen.map_or(2026 * 12, |d| {
            let (y, m, _) = dates::civil_from_days(d);
            y * 12 + i64::from(m) - 1
        });
        let mut month: i64 = ui.ctx().data(|d| d.get_temp(shown_id)).unwrap_or(fallback);
        ui.horizontal(|ui| {
            if widgets::tiny_icon_button(ui, Icon::ChevronLeft, "Previous month", t).clicked() {
                month -= 1;
            }
            let m = u32::try_from(month.rem_euclid(12) + 1).unwrap_or(1);
            ui.label(
                RichText::new(tf!("{month} {year}", month = crate::calendar::month_name(m), year = month.div_euclid(12))).font(theme::bold(13.0)),
            );
            if widgets::tiny_icon_button(ui, Icon::ChevronRight, "Next month", t).clicked() {
                month += 1;
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(shown_id, month));
        let w = 30.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for d in dates::WEEKDAYS {
                let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
                ui.painter().text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    tl!(d).chars().next().map(String::from).unwrap_or_default(),
                    theme::regular(11.0),
                    t.faint,
                );
            }
        });
        let start = grid_start(month);
        let this_month = u32::try_from(month.rem_euclid(12) + 1).unwrap_or(1);
        for week in 0..6 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for i in 0..7 {
                    let d = start + week * 7 + i;
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
                    let (_, dm, dd) = dates::civil_from_days(d);
                    if Some(d) == chosen {
                        ui.painter().circle_filled(r.center(), 11.0, t.accent);
                    } else if resp.hovered() {
                        ui.painter().circle_filled(r.center(), 11.0, t.hover);
                    }
                    let c = if Some(d) == chosen {
                        t.on_accent
                    } else if dm == this_month {
                        t.text
                    } else {
                        t.faint
                    };
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, dd.to_string(), theme::regular(12.0), c);
                    let resp = resp.on_hover_text(dates::typed_day(d, order));
                    if resp.clicked() {
                        picked = Some(d);
                        ui.close();
                    }
                }
            });
        }
    });
    if let Some(d) = picked {
        *text = dates::day_string(d);
        shown = dates::typed_day(d, order);
    }
    ui.ctx().data_mut(|d| d.insert_temp(box_id, (text.clone(), shown)));
    typed || picked.is_some()
}

/// How a day is written in `order`, for an empty box.
fn pattern(order: dates::DateOrder) -> &'static str {
    match order {
        dates::DateOrder::Iso => "YYYY-MM-DD",
        dates::DateOrder::DayMonthYear => "DD/MM/YYYY",
        dates::DateOrder::MonthDayYear => "MM/DD/YYYY",
        dates::DateOrder::DayMonthYearDots => "DD.MM.YYYY",
        dates::DateOrder::YearMonthDayDots => "YYYY.MM.DD",
    }
}
