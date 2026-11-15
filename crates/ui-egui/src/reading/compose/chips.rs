//! The message being written: its attachments, a chip each, and what's said of photos that
//! tell where they were taken.

use super::*;

/// The attachments under the subject, wrapped into rows and scrolling past two, as Outlook's
/// are; each one's × takes it out.
pub(super) fn attachment_chips(ui: &mut Ui, t: &Tokens, keep_photo_location: bool, c: &mailcraft_engine::Compose, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.add_space(PAST_SEND + LABEL_W);
        let mut remove = None;
        crate::bidi::vertical(ui, |ui| {
            // Wrapped into rows, scrolling past two, as Outlook's are.
            let n = c.attachments.len();
            widgets::chip_grid(ui, egui::Id::unique(("compose-attachments", c.id)), n, 220.0, 320.0, 28.0, 2, |ui, i, w| {
                let Some(a) = c.attachments.get(i) else { return };
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
                let x_rect = Rect::from_center_size(egui::pos2(rect.right() - 15.0, rect.center().y), vec2(22.0, 22.0));
                let x = ui.interact(x_rect, resp.id.with("remove"), Sense::click()).on_hover_text(tl!("Remove"));
                widgets::named(ui, &x, &format!("{}: {}", tl!("Remove"), a.name), true, None);
                let p = ui.painter();
                p.rect(
                    rect,
                    4.0,
                    if resp.hovered() { t.hover_row } else { t.surface },
                    egui::Stroke::new(1.0, t.faint.gamma_multiply(0.55)),
                    egui::StrokeKind::Inside,
                );
                if x.hovered() {
                    p.rect_filled(x_rect, 3.0, t.hover);
                }
                icons::paint(p, icons::centered(x_rect, 11.0), Icon::Close, if x.hovered() { t.text } else { t.dim });
                let ir = Rect::from_center_size(egui::pos2(rect.left() + 15.0, rect.center().y), vec2(14.0, 14.0));
                icons::paint(p, ir, Icon::Attach, t.dim);
                let text = format!("{} ({})", a.name, widgets::human_size(a.data.len()));
                let g = widgets::one_line(ui, &text, theme::regular(12.0), t.text, x_rect.left() - 4.0 - (ir.right() + 6.0));
                p.galley(egui::pos2(ir.right() + 6.0, rect.center().y - g.size().y / 2.0), g, t.text);
                resp.on_hover_text(a.name.clone());
                if x.clicked() {
                    remove = Some(i);
                }
            });
            // A photo that says where it was taken (looked at once per set of files).
            let size: usize = c.attachments.iter().map(|a| a.data.len()).sum();
            let id = egui::Id::unique(("photo-place", c.id, n, size));
            let placed = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or_else(|| {
                let v = c.attachments.iter().any(|a| mime::without_location(&a.data).is_some());
                ui.data_mut(|d| d.insert_temp(id, v));
                v
            });
            if placed {
                let note = if keep_photo_location {
                    tl!("These photos say where they were taken, and go that way (File › Options › Safety)")
                } else {
                    tl!("Where these photos were taken is left out as they're sent")
                };
                ui.label(RichText::new(note).color(t.faint).small());
            }
        });
        if let Some(i) = remove {
            acts.push(Action::Run("compose.removeAttachment", json!({"index": i})));
        }
    });
}
