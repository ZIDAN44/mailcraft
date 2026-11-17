use super::*;

/// Everyone as business cards, side by side (Outlook's Business Card view): a click picks one,
/// a double-click edits it.
pub(super) fn show(
    s: &Session,
    st: &mut UiState,
    ui: &mut Ui,
    acts: &mut Vec<Action>,
    can_share_contacts: bool,
    can_export_contacts: bool,
    can_add_to_system_contacts: bool,
) {
    let t = st.tokens;
    let people = shown(s, st);
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("people-cards").show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
            if people.is_empty() {
                ui.label(RichText::new(tl!("No contacts yet.")).color(t.faint));
                return;
            }
            let card_w = 280.0;
            let card_h = 132.0;
            let gap = 12.0;
            let cols = (((ui.available_width() + gap) / (card_w + gap)).floor() as usize).max(1);
            for row in people.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for p in row {
                        let c = &p.card;
                        let name = c.display();
                        let selected = st.person.as_deref() == Some(c.uid.as_str());
                        let (r, resp) = ui.allocate_exact_size(vec2(card_w, card_h), Sense::click());
                        let fill = if selected {
                            t.accent_soft
                        } else if resp.hovered() {
                            t.hover_row
                        } else {
                            t.surface
                        };
                        ui.painter().rect(r, 8.0, fill, egui::Stroke::new(1.0, if selected { t.accent } else { t.border }), egui::StrokeKind::Inside);
                        // A band in the person's colour down the left, with their initials.
                        let band = egui::Rect::from_min_size(r.min, vec2(74.0, card_h));
                        let color = theme::avatar_color(&name, t.dark);
                        ui.painter().rect_filled(band.shrink(1.0), egui::CornerRadius { nw: 7, sw: 7, ne: 0, se: 0 }, color.gamma_multiply(0.18));
                        widgets::avatar(ui, band.center(), 24.0, &initials(&name), color);
                        let x = band.right() + 12.0;
                        let w = r.right() - x - 10.0;
                        let mut y = r.top() + 12.0;
                        let mut line = |text: &str, font: egui::FontId, color: egui::Color32| {
                            if text.is_empty() {
                                return;
                            }
                            let g = widgets::one_line(ui, text, font, color, w);
                            let h = g.size().y;
                            ui.painter().galley(egui::pos2(x, y), g, color);
                            y += h + 3.0;
                        };
                        line(&name, theme::bold(14.5), t.text);
                        line(&c.title, theme::regular(12.0), t.dim);
                        line(&c.org, theme::regular(12.0), t.dim);
                        for e in c.emails.iter().take(2) {
                            line(&e.value, theme::regular(12.0), t.accent);
                        }
                        for ph in c.phones.iter().take(1) {
                            line(&ph.value, theme::regular(12.0), t.text);
                        }
                        if resp.clicked() {
                            st.person = Some(c.uid.clone());
                        }
                        if resp.double_clicked() && !s.person_read_only(&c.uid) {
                            acts.push(Action::Dialog(Dialog::edit_contact(c)));
                        }
                        if can_share_contacts || can_export_contacts || can_add_to_system_contacts || !c.emails.is_empty() || !c.phones.is_empty() {
                            resp.context_menu(|ui| {
                                if can_share_contacts && ui.button(tl!("Share contact")).clicked() {
                                    acts.push(Action::ShareContact {
                                        title: tl!("Share contact").to_owned(),
                                        vcard: mailcraft_engine::model::vcard::write(c),
                                    });
                                    ui.close();
                                }
                                if can_export_contacts && ui.button(tl!("Save contact")).clicked() {
                                    acts.push(Action::ExportContact { name: name.clone(), vcard: mailcraft_engine::model::vcard::write(c) });
                                    ui.close();
                                }
                                if can_add_to_system_contacts && ui.button(tl!("Add to Android contacts")).clicked() {
                                    acts.push(add_contact_action(c));
                                    ui.close();
                                }
                                if !c.emails.is_empty() {
                                    crate::bidi::menu_button(ui, tl!("Copy address"), |ui| {
                                        for email in c.emails.iter().take(2) {
                                            if ui.button(contact_menu_value(&email.value)).clicked() {
                                                ui.ctx().copy_text(email.value.clone());
                                                ui.close();
                                            }
                                        }
                                    });
                                }
                                if !c.phones.is_empty() {
                                    crate::bidi::menu_button(ui, tl!("Copy phone number"), |ui| {
                                        for phone in c.phones.iter().take(1) {
                                            if ui.button(contact_menu_value(&phone.value)).clicked() {
                                                ui.ctx().copy_text(phone.value.clone());
                                                ui.close();
                                            }
                                        }
                                    });
                                }
                            });
                        }
                    }
                });
                ui.add_space(gap);
            }
        });
    });
}

fn contact_menu_value(value: &str) -> String {
    let mut shown = value.chars().take(80).collect::<String>();
    if shown.len() < value.len() {
        shown.push('…');
    }
    shown
}
