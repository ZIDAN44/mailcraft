use super::*;

/// Everyone in a table (Outlook's List view): the name and the columns chosen (right-click the
/// header), sorted by the column clicked.
pub(super) fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, can_dial_phone: bool, can_open_map: bool) {
    let t = st.tokens;
    let mut people = shown(s, st);
    let list = st.people_list.clone();
    if let Some(col) = list.sort {
        people.sort_by_cached_key(|p| (col.text(s, p).to_lowercase(), p.card.display().to_lowercase()));
    }
    if list.descending {
        people.reverse();
    }
    let touch = ui.ctx().input(|input| input.has_touch_screen());
    let viewport_width = ui.available_width();
    egui::ScrollArea::horizontal().auto_shrink([false, false]).id_salt("people-table-horizontal").show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::same(14)).show(ui, |ui| {
            let available_width = (viewport_width - 28.0).max(0.0);
            let row_height = if touch { 48.0 } else { 30.0 };
            // Column widths: the name a share and a half, the others by their weight.
            let cols: Vec<Option<Column>> = std::iter::once(None).chain(list.columns.iter().copied().map(Some)).collect();
            // Keep a useful minimum width per column on phones so email addresses and phone numbers
            // remain readable. The whole table scrolls sideways while its rows still scroll vertically.
            let min_width = if touch { 200.0 + cols.len().saturating_sub(1) as f32 * 120.0 } else { 0.0 };
            let w = available_width.max(min_width);
            ui.set_min_width(w);
            let weight = |c: &Option<Column>| c.map_or(1.5, Column::weight);
            let total: f32 = cols.iter().map(weight).sum::<f32>().max(1.0);
            let widths: Vec<f32> = cols.iter().map(|c| w * weight(c) / total).collect();
            let row = |ui: &mut Ui, cells: &[String], header: bool, fill: Option<egui::Color32>| {
                let (r, resp) = ui.allocate_exact_size(vec2(w, row_height), Sense::click());
                widgets::named(ui, &resp, cells.first().map(String::as_str).unwrap_or_default(), true, None);
                if let Some(f) = fill {
                    ui.painter().rect_filled(r, 4.0, f);
                }
                let mut x = r.left() + 8.0;
                let mut rects = Vec::new();
                let mut actions = Vec::new();
                for (index, ((cw, text), column)) in widths.iter().zip(cells).zip(&cols).enumerate() {
                    let is_phone = matches!(*column, Some(Column::Business | Column::Mobile | Column::Home));
                    let is_address = matches!(*column, Some(Column::BusinessAddress | Column::HomeAddress));
                    let is_email = matches!(*column, Some(Column::Email));
                    let action = if !header && !text.is_empty() && can_dial_phone && is_phone {
                        Some(Action::DialPhone(text.clone()))
                    } else if !header && !text.is_empty() && can_open_map && is_address {
                        Some(Action::OpenMap(text.clone()))
                    } else if !header && is_email {
                        crate::mailto::link(text).map(Action::OpenUrl)
                    } else {
                        None
                    };
                    let copy_label = if !header && !text.is_empty() && is_email {
                        Some(tl!("Copy address"))
                    } else if !header && !text.is_empty() && is_phone {
                        Some(tl!("Copy phone number"))
                    } else if !header && !text.is_empty() && is_address {
                        Some(tl!("Copy address"))
                    } else {
                        None
                    };
                    let clickable = action.is_some();
                    let font = if header { theme::bold(12.5) } else { theme::regular(13.0) };
                    let color = if header {
                        t.dim
                    } else if clickable {
                        t.accent
                    } else {
                        t.text
                    };
                    let g = widgets::one_line(ui, text, font, color, cw - 12.0);
                    let text_pos = egui::pos2(x, r.center().y - g.size().y / 2.0);
                    ui.painter().galley(text_pos, g.clone(), color);
                    if clickable {
                        let underline_y = text_pos.y + g.size().y + 1.0;
                        ui.painter()
                            .line_segment([egui::pos2(x, underline_y), egui::pos2(x + g.size().x, underline_y)], egui::Stroke::new(1.0, t.accent));
                    }
                    let cell = egui::Rect::from_min_size(egui::pos2(x - 8.0, r.top()), vec2(*cw, r.height()));
                    rects.push(cell);
                    let response = if action.is_some() || copy_label.is_some() {
                        let response = ui.interact(cell, ui.scope_id().with(("contact-action", index)), Sense::click());
                        widgets::named(ui, &response, text, true, None);
                        let response = if clickable { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
                        let response = if let Some(copy_label) = copy_label {
                            response.context_menu(|ui| {
                                if ui.button(copy_label).clicked() {
                                    ui.ctx().copy_text(text.clone());
                                    ui.close();
                                }
                            });
                            response
                        } else {
                            response
                        };
                        Some((response, action))
                    } else {
                        None
                    };
                    actions.push(response);
                    x += cw;
                }
                (resp, rects, actions)
            };
            // The header: click a column to sort by it (again: the other way); right-click to choose.
            let arrow = if list.descending { " ↓" } else { " ↑" };
            let heads: Vec<String> = cols
                .iter()
                .map(|c| {
                    let name = c.map_or_else(|| tl!("Full name").to_string(), |c| tl!(c.label()).to_string());
                    if *c == list.sort { format!("{name}{arrow}") } else { name }
                })
                .collect();
            let (head, rects, _) = row(ui, &heads, true, None);
            if head.clicked()
                && let Some(p) = head.interact_pointer_pos()
                && let Some(i) = rects.iter().position(|r| r.contains(p))
                && let Some(c) = cols.get(i)
            {
                if *c == st.people_list.sort {
                    st.people_list.descending = !st.people_list.descending;
                } else {
                    st.people_list.sort = *c;
                    st.people_list.descending = false;
                }
            }
            head.on_hover_text(tl!("Click a column to sort by it; right-click to choose the columns"))
                .context_menu(|ui| column_menu(ui, &mut st.people_list));
            let line = ui.available_rect_before_wrap();
            ui.painter().line_segment([line.left_top(), line.right_top()], egui::Stroke::new(1.0, t.border));
            ui.add_space(2.0);
            if people.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(tl!("No contacts yet.")).color(t.faint));
            }
            egui::ScrollArea::vertical().auto_shrink(false).id_salt("people-phone").show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, p) in people.iter().enumerate() {
                    let c = &p.card;
                    let cells: Vec<String> = cols.iter().map(|col| col.map_or_else(|| c.display(), |col| col.text(s, p))).collect();
                    let selected = st.person.as_deref() == Some(c.uid.as_str());
                    let id = ui.scope_id().with(("phone-row", i));
                    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
                    let fill = if selected {
                        Some(t.accent_soft)
                    } else if hovered {
                        Some(t.hover_row)
                    } else if i % 2 == 1 {
                        Some(t.chrome)
                    } else {
                        None
                    };
                    let (resp, _, cell_actions) = ui.push_id(id, |ui| row(ui, &cells, false, fill)).inner;
                    let action = cell_actions.into_iter().flatten().find_map(|(response, action)| response.clicked().then_some(action).flatten());
                    if let Some(action) = action {
                        st.person = Some(c.uid.clone());
                        acts.push(action);
                    } else if resp.clicked() {
                        st.person = Some(c.uid.clone());
                    }
                    if resp.double_clicked() && !s.person_read_only(&c.uid) {
                        acts.push(Action::Dialog(Dialog::edit_contact(c)));
                    }
                }
            });
        });
    });
}
