//! A message's source in a window of its own (View message source, issue #27): read only, its
//! lines selectable, the whole of it copied with a button, its header alone at a click. Only
//! the lines in view are laid out, so a long source scrolls as quickly as a short one.

use crate::*;

impl MailApp {
    /// Each message source asked for, in a native window (in the browser, a window in the page).
    pub(crate) fn source_windows(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let (sources, st) = (&self.session.sources, &mut self.ui);
        for v in sources {
            let key = v.key.clone();
            let subject = if v.subject.trim().is_empty() { tl!("(no subject)").to_string() } else { v.subject.trim().to_string() };
            let id = egui::ViewportId::from_hash_of(("source-window", key.to_id()));
            let builder = egui::ViewportBuilder::default()
                .with_title(tf!("{subject} – Message source", subject = subject))
                .with_inner_size([860.0, 680.0])
                .with_min_inner_size([420.0, 300.0]);
            let builder = match st.places.get("source") {
                Some(p) if super::windows::opening(ctx, id) => p.apply(builder),
                _ => builder,
            };
            let close = ctx.show_viewport_immediate(id, builder, |ui, _| {
                let mut place = st.places.get("source").copied();
                super::windows::after_open(ui, place);
                super::windows::WindowPlace::note(&mut place, ui);
                if let Some(p) = place {
                    st.places.insert("source".into(), p);
                }
                if st.raise_source.as_ref() == Some(&key) {
                    st.raise_source = None;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                let mut closing = ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape));
                let header_only = st.source_headers.contains(&key);
                egui::Panel::top(egui::Id::unique(("source-window-bar", key.to_id())))
                    .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(10, 6)).stroke(egui::Stroke::new(1.0, t.border)))
                    .show(ui, |ui| {
                        // From the side the interface's text starts on (the right in Arabic).
                        let bar = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
                        ui.allocate_ui_with_layout(bar, crate::bidi::row_from_start(Align::Center), |ui| {
                            for (only, label) in [(false, tl!("Whole message")), (true, tl!("Header only"))] {
                                if ui.selectable_label(header_only == only, label).clicked() {
                                    if only {
                                        st.source_headers.insert(key.clone());
                                    } else {
                                        st.source_headers.remove(&key);
                                    }
                                }
                            }
                            ui.add_space(8.0);
                            let ready = !v.pending && v.error.is_none();
                            if ui.add_enabled(ready, egui::Button::new(tl!("Copy all"))).clicked() {
                                ui.ctx().copy_text(v.text(header_only));
                            }
                            if ready {
                                ui.label(RichText::new(widgets::human_size(v.size)).color(t.dim).small());
                            }
                            // Drawn inside the page (the browser), the window has no title bar.
                            if ui.ctx().embed_viewports() {
                                ui.with_layout(crate::bidi::row_from_end(Align::Center), |ui| {
                                    if widgets::tool_button(ui, Icon::Close, "Close", &t, true).clicked() {
                                        closing = true;
                                    }
                                });
                            }
                        });
                        if v.truncated && !header_only {
                            ui.label(
                                RichText::new(tf!(
                                    "Only the first {shown} of {size} are shown. Save as email (.eml) keeps all of it.",
                                    shown = widgets::human_size(mailcraft_engine::SOURCE_SHOWN.min(v.size)),
                                    size = widgets::human_size(v.size)
                                ))
                                .color(t.dim)
                                .small(),
                            );
                        }
                    });
                egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface).inner_margin(Margin::same(8))).show(ui, |ui| {
                    source_text(ui, &t, v, header_only);
                });
                closing
            });
            if close {
                super::windows::forget_open(ctx, id);
                st.source_headers.remove(&key);
                acts.push(Action::Run("message.closeSource", json!({"key": key.to_id()})));
            }
        }
    }
}

/// The source's lines (or its header's), only those in view laid out (which, returned).
pub(crate) fn source_text(ui: &mut Ui, t: &Tokens, v: &mailcraft_engine::SourceView, header_only: bool) -> std::ops::Range<usize> {
    if let Some(e) = &v.error {
        ui.label(RichText::new(tf!("The source couldn't be downloaded: {error}", error = e)).color(t.danger));
        return 0..0;
    }
    if v.pending {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(RichText::new(tl!("Downloading the message…")).color(t.dim));
        });
        return 0..0;
    }
    let n = if header_only { v.header_lines.min(v.lines.len()) } else { v.lines.len() };
    let font = egui::FontId::monospace(12.5);
    let row = ui.fonts_mut(|f| f.row_height(&font));
    // The lines touch: set before the rows are counted, which reckons a row with the spacing
    // under it (set inside, the window's lower part stayed empty).
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::both()
        .auto_shrink(false)
        .id_salt(("source", v.key.to_id()))
        .show_rows(ui, row, n, |ui, rows| {
            for line in v.lines.get(rows.clone()).unwrap_or_default() {
                ui.add(egui::Label::new(RichText::new(line).font(font.clone()).color(t.text)).wrap_mode(egui::TextWrapMode::Extend).selectable(true));
            }
            rows.start..rows.end.min(n)
        })
        .inner
}
