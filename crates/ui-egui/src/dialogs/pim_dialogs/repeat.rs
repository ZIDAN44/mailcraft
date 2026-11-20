//! How an event or a task repeats, as the forms' Repeat list offers it.

use super::*;

/// A form's Repeat choice: one of the list's words, or a rule kept as it came.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    Never,
    Daily,
    Weekly,
    Monthly,
    Yearly,
    /// A rule the list has no word for (`FREQ=WEEKLY;BYDAY=MO,WE`), saved as it was.
    Custom(String),
}

impl Repeat {
    /// The choices the list always offers, in its order.
    const PLAIN: [Repeat; 5] = [Repeat::Never, Repeat::Daily, Repeat::Weekly, Repeat::Monthly, Repeat::Yearly];

    /// The choice for an event's or a task's rule (`None` or empty: it doesn't repeat).
    pub fn from_rrule(rule: Option<&str>) -> Repeat {
        match rule {
            None | Some("") => Repeat::Never,
            Some(r) => match r.to_ascii_uppercase().as_str() {
                "FREQ=DAILY" => Repeat::Daily,
                "FREQ=WEEKLY" => Repeat::Weekly,
                "FREQ=MONTHLY" => Repeat::Monthly,
                "FREQ=YEARLY" => Repeat::Yearly,
                _ => Repeat::Custom(r.to_string()),
            },
        }
    }

    /// What the list calls it, translated.
    pub fn label(&self) -> &'static str {
        match self {
            Repeat::Never => tl!("Never"),
            Repeat::Daily => tl!("Every day"),
            Repeat::Weekly => tl!("Every week"),
            Repeat::Monthly => tl!("Every month"),
            Repeat::Yearly => tl!("Every year"),
            Repeat::Custom(_) => tl!("Custom"),
        }
    }

    /// The `repeat` the `event.*` and `task.*` commands take for it.
    pub fn param(&self) -> &str {
        match self {
            Repeat::Never => "none",
            Repeat::Daily => "daily",
            Repeat::Weekly => "weekly",
            Repeat::Monthly => "monthly",
            Repeat::Yearly => "yearly",
            Repeat::Custom(rule) => rule,
        }
    }
}

/// A form's Repeat row: the list of choices, and Custom while the form's rule is one of its own.
pub(super) fn repeat_row(ui: &mut Ui, t: &Tokens, id: &str, repeat: &mut Repeat) {
    form_row(ui, t, true, Some(Icon::Sync), "Repeat", |ui| {
        ui.allocate_ui_with_layout(vec2(200.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let r = egui::ComboBox::from_id_salt(id)
                .width(194.0)
                .selected_text(repeat.label())
                .show_ui(ui, |ui| {
                    for r in Repeat::PLAIN {
                        if ui.selectable_label(*repeat == r, r.label()).clicked() {
                            *repeat = r;
                        }
                    }
                    if matches!(repeat, Repeat::Custom(_)) {
                        // Already the choice: picking it again changes nothing.
                        let _ = ui.selectable_label(true, tl!("Custom"));
                    }
                })
                .response
                .accessible_name(tl!("Repeat"));
            let mut choices = Repeat::PLAIN.to_vec();
            if matches!(repeat, Repeat::Custom(_)) {
                choices.push(repeat.clone());
            }
            crate::drop_down::keys(ui, &r, repeat, &choices);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::Repeat;

    /// Each rule reads as the choice the forms showed for it, and gives back the `repeat` they
    /// sent: the list's words for the plain rules, in any case; any other rule as it was.
    #[test]
    fn rules_read_as_the_lists_choices() {
        for (rule, choice, param) in [
            (None, Repeat::Never, "none"),
            (Some(""), Repeat::Never, "none"),
            (Some("FREQ=DAILY"), Repeat::Daily, "daily"),
            (Some("freq=weekly"), Repeat::Weekly, "weekly"),
            (Some("FREQ=MONTHLY"), Repeat::Monthly, "monthly"),
            (Some("Freq=Yearly"), Repeat::Yearly, "yearly"),
            (Some("FREQ=WEEKLY;BYDAY=MO,WE"), Repeat::Custom("FREQ=WEEKLY;BYDAY=MO,WE".into()), "FREQ=WEEKLY;BYDAY=MO,WE"),
            (Some("FREQ=DAILY;COUNT=2"), Repeat::Custom("FREQ=DAILY;COUNT=2".into()), "FREQ=DAILY;COUNT=2"),
        ] {
            let r = Repeat::from_rrule(rule);
            assert_eq!(r, choice, "{rule:?}");
            assert_eq!(r.param(), param, "{rule:?}");
        }
    }
}
