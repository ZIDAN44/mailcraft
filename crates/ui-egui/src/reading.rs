//! The reading pane (the open message with its conversation and attachments) and the inline
//! compose form that replaces it while a message is being written.

use egui::{Color32, Margin, Rect, RichText, Sense, Ui, vec2};
use mailcraft_engine::mime::{self, AttachmentKind};
use mailcraft_engine::model::{MessageKey, addr, dates};
use mailcraft_engine::{Session, model::EmailAddress};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, UiState, widgets};

pub fn show(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.compose.is_some() {
        compose(s, st, ui, acts);
        return;
    }
    let t = st.tokens;
    let Some(r) = s.reading.clone() else {
        empty(ui, &t, if s.list.is_empty() { "" } else { "Select a message to read it" });
        return;
    };
    let summary = s.summary(&r.key).cloned();
    egui::ScrollArea::vertical().auto_shrink(false).id_salt(("reading", r.key.to_id())).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 24, right: 24, top: 18, bottom: 24 }).show(ui, |ui| {
            let Some(p) = &r.parsed else {
                ui.add_space(30.0);
                match &r.error {
                    Some(e) => {
                        ui.horizontal(|ui| {
                            let (ir, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            icons::paint(ui.painter(), ir, Icon::Warning, t.warn);
                            ui.label(RichText::new(format!("This message couldn't be downloaded: {e}")).color(t.dim));
                        });
                        if ui.button("Try again").clicked() {
                            acts.push(Action::Open(r.key.clone()));
                        }
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Downloading the message…").color(t.dim));
                        });
                    }
                }
                return;
            };
            // Subject.
            ui.horizontal_wrapped(|ui| {
                let subject = if p.subject.is_empty() { "(no subject)" } else { p.subject.as_str() };
                ui.label(RichText::new(subject).font(theme::bold(21.0)).color(t.text));
                if summary.as_ref().is_some_and(|m| m.flags.flagged) {
                    let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), ir, Icon::Flag, t.flag);
                }
            });
            ui.add_space(10.0);

            // Conversation.
            if r.thread.len() > 1 {
                thread_list(s, &t, ui, &r.thread, &r.key, acts);
                ui.add_space(10.0);
            }

            // Sender card.
            ui.horizontal(|ui| {
                let (ar, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                widgets::avatar(ui, ar.center(), 21.0, &p.from.initials(), theme::avatar_color(&p.from.email, t.dark));
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(p.from.display()).font(theme::bold(15.0)).color(t.text));
                        if !p.from.name.is_empty() {
                            ui.label(RichText::new(format!("<{}>", p.from.email)).color(t.dim));
                        }
                    });
                    if !p.to.is_empty() {
                        ui.label(RichText::new(format!("To: {}", addr::display_list(&p.to))).color(t.dim).small())
                            .on_hover_text(addr::format_list(&p.to));
                    }
                    if !p.cc.is_empty() {
                        ui.label(RichText::new(format!("Cc: {}", addr::display_list(&p.cc))).color(t.dim).small())
                            .on_hover_text(addr::format_list(&p.cc));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    let when = summary.as_ref().map_or(p.date, |m| if p.date > 0 { p.date } else { m.date });
                    ui.label(RichText::new(dates::full_time(when, &s.clock)).color(t.dim).small());
                });
            });
            ui.add_space(8.0);
            if let Some((_, item)) = s.outbox_entry(&r.key) {
                egui::Frame::new().fill(t.warn.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                    let why = match item.error.as_deref() {
                        Some(e) if e != "offline" => format!("Waiting in the Outbox. Last try: {}", widgets::shorten(e, 140)),
                        _ => "Waiting in the Outbox: it will be sent when you're back online.".to_string(),
                    };
                    ui.label(RichText::new(why).color(t.text));
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if widgets::pill_button(ui, Icon::Send, "Send now", &t).clicked() {
                        acts.push(Action::Run("outbox.send", json!({})));
                    }
                    if widgets::pill_button(ui, Icon::Drafts, "Edit", &t).clicked() {
                        acts.push(Action::Run("outbox.edit", json!({"key": r.key.to_id()})));
                    }
                    if widgets::pill_button(ui, Icon::Delete, "Delete", &t).clicked() {
                        acts.push(Action::Run("mail.delete", json!({})));
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if widgets::pill_button(ui, Icon::Reply, "Reply", &t).clicked() {
                        acts.push(Action::Run("mail.reply", json!({})));
                    }
                    if widgets::pill_button(ui, Icon::ReplyAll, "Reply all", &t).clicked() {
                        acts.push(Action::Run("mail.replyAll", json!({})));
                    }
                    if widgets::pill_button(ui, Icon::Forward, "Forward", &t).clicked() {
                        acts.push(Action::Run("mail.forward", json!({})));
                    }
                });
            }
            ui.add_space(12.0);

            // Attachments.
            let files: Vec<_> = p.attachments.iter().filter(|a| !a.inline || a.kind() == AttachmentKind::Image).cloned().collect();
            if !files.is_empty() {
                ui.label(RichText::new(format!("{} attachment{}", files.len(), if files.len() == 1 { "" } else { "s" })).color(t.dim).small());
                ui.horizontal_wrapped(|ui| {
                    for a in &files {
                        attachment_card(s, st, ui, &r.key, a, acts);
                    }
                });
                ui.add_space(10.0);
            }
            if p.had_html {
                ui.label(RichText::new("Shown as text: links and pictures from the web are never loaded automatically.").color(t.faint).small());
                ui.add_space(6.0);
            }
            if let Some(u) = &p.unsubscribe {
                ui.label(RichText::new(format!("Mailing list: unsubscribe at {}", widgets::shorten(u, 80))).color(t.faint).small());
                ui.add_space(6.0);
            }
            let sep = ui.available_rect_before_wrap();
            ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
            ui.add_space(12.0);
            let mut text = p.text.as_str();
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .font(theme::regular(14.5))
                    .text_color(t.text),
            );
        });
    });
}

fn empty(ui: &mut Ui, t: &Tokens, msg: &str) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.35);
            let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Read, t.border);
            ui.add_space(8.0);
            ui.label(RichText::new(msg).color(t.faint));
        });
    });
}

fn thread_list(s: &Session, t: &Tokens, ui: &mut Ui, thread: &[MessageKey], open: &MessageKey, acts: &mut Vec<Action>) {
    egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(Margin::symmetric(10, 8)).show(
        ui,
        |ui| {
            ui.label(RichText::new(format!("{} messages in this conversation", thread.len())).color(t.dim).small());
            for k in thread {
                let Some(m) = s.summary(k) else { continue };
                let current = k == open;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if current || resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, if current { t.accent_soft } else { t.hover });
                }
                let font = if !m.flags.seen { theme::bold(13.0) } else { theme::regular(13.0) };
                let p = ui.painter_at(rect);
                p.text(rect.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, m.from.display(), font, t.text);
                p.text(
                    rect.left_center() + vec2(170.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    widgets::shorten(&m.preview, 70),
                    theme::regular(12.5),
                    t.dim,
                );
                p.text(
                    rect.right_center() - vec2(8.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    dates::list_time(m.date, &s.clock),
                    theme::regular(12.0),
                    t.dim,
                );
                if resp.clicked() && !current {
                    acts.push(Action::Open(k.clone()));
                }
            }
        },
    );
}

fn kind_icon(k: AttachmentKind) -> Icon {
    match k {
        AttachmentKind::Image => Icon::Image,
        _ => Icon::File,
    }
}

fn kind_label(k: AttachmentKind) -> &'static str {
    match k {
        AttachmentKind::Image => "Picture",
        AttachmentKind::Video => "Video",
        AttachmentKind::Audio => "Audio",
        AttachmentKind::Pdf => "PDF",
        AttachmentKind::Text => "Text",
        AttachmentKind::Archive => "Archive",
        AttachmentKind::Document => "Document",
        AttachmentKind::Other => "File",
    }
}

/// Decode an image attachment into a thumbnail texture (cached; `None` if it isn't a picture
/// we can read). Pictures come from mail, so size is capped before decoding.
fn preview(s: &Session, st: &mut UiState, ctx: &egui::Context, key: &MessageKey, index: usize) -> Option<egui::TextureHandle> {
    if let Some(t) = st.previews.get(&(key.clone(), index)) {
        return t.clone();
    }
    if st.previews.len() > 64 {
        st.previews.clear();
    }
    let tex = (|| {
        let raw = s.store.as_ref()?.read_body(key)?;
        let data = mime::attachment_data(&raw, index)?;
        if data.len() > 30 << 20 {
            return None;
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(12_000);
        limits.max_image_height = Some(12_000);
        limits.max_alloc = Some(256 << 20);
        let mut reader = image::ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format().ok()?;
        reader.limits(limits);
        let decoded = reader.decode().ok()?;
        // Shrink large pictures; never enlarge small ones (the painter scales them crisply).
        let img = if decoded.width() > 480 || decoded.height() > 480 { decoded.thumbnail(480, 480) } else { decoded }.to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        // Pixel art and tiny swatches stay crisp when scaled up.
        let filter = if size[0].max(size[1]) < 96 { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
        Some(ctx.load_texture(format!("att-{}-{index}", key.to_id()), ci, filter))
    })();
    st.previews.insert((key.clone(), index), tex.clone());
    tex
}

fn attachment_card(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, a: &mime::AttachmentInfo, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let kind = a.kind();
    let tex = if kind == AttachmentKind::Image { preview(s, st, ui.ctx(), key, a.index) } else { None };
    egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(Margin::same(8)).show(ui, |ui| {
        ui.set_width(220.0);
        ui.vertical(|ui| {
            if let Some(tex) = &tex {
                let sz = tex.size_vec2();
                let scale = (204.0 / sz.x).min(130.0 / sz.y).min(8.0);
                let (r, resp) = ui.allocate_exact_size(vec2(204.0, 130.0), Sense::click());
                ui.painter().rect_filled(r, 4.0, t.chrome);
                let img_rect = Rect::from_center_size(r.center(), sz * scale);
                ui.painter().image(tex.id(), img_rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                if resp.on_hover_text("Open").clicked() {
                    acts.push(Action::OpenAttachment(a.index));
                }
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                ui.painter().rect_filled(ir, 5.0, t.accent_soft);
                icons::paint(ui.painter(), icons::centered(ir, 16.0), kind_icon(kind), t.accent);
                ui.vertical(|ui| {
                    ui.label(RichText::new(widgets::shorten(&a.name, 26)).color(t.text).small()).on_hover_text(&a.name);
                    ui.label(RichText::new(format!("{} · {}", kind_label(kind), widgets::human_size(a.size))).color(t.faint).small());
                });
            });
            ui.horizontal(|ui| {
                if ui.small_button("Open").clicked() {
                    acts.push(Action::OpenAttachment(a.index));
                }
                if ui.small_button("Save as…").clicked() {
                    acts.push(Action::SaveAttachment(a.index));
                }
            });
        });
    });
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, hint: &str, check_addresses: bool, focus: bool) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(56.0, 26.0), egui::Label::new(RichText::new(label).color(t.dim)));
        let r = ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY));
        if focus {
            r.request_focus();
        }
    });
    if check_addresses
        && !value.trim().is_empty()
        && let Err(bad) = addr::parse_list(value)
    {
        ui.horizontal(|ui| {
            ui.add_space(64.0);
            ui.label(RichText::new(format!("Not an email address: {bad}")).color(t.danger).small());
        });
    }
}

fn compose(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let accounts: Vec<(String, String)> =
        s.accounts.iter().map(|a| (a.config.id.clone(), EmailAddress::new(a.config.name.clone(), a.config.email.clone()).to_string())).collect();
    let Some(c) = s.compose.as_mut() else { return };
    // A newly opened message: start in To (new mail) or the body (replies).
    let fresh = st.compose_shown != Some(c.id);
    st.compose_shown = Some(c.id);
    let focus_to = fresh && c.to.trim().is_empty();
    let focus_body = fresh && !focus_to;
    egui::Frame::new().inner_margin(Margin { left: 20, right: 20, top: 14, bottom: 14 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            let label = if c.sending { "Sending…" } else { "Send" };
            if widgets::accent_button(ui, Icon::Send, label, &t, !c.sending).on_hover_text("Send (Ctrl+Enter)").clicked() {
                if c.subject.trim().is_empty() {
                    acts.push(Action::Dialog(crate::Dialog::EmptySubject));
                } else {
                    acts.push(Action::Run("compose.send", json!({})));
                }
            }
            ui.add_space(6.0);
            if widgets::tool_button(ui, Icon::Attach, "Attach file", &t, !c.sending).clicked() {
                acts.push(Action::AttachFiles);
            }
            if widgets::tool_button(ui, Icon::Drafts, "Save draft", &t, !c.sending).on_hover_text("Save draft (Ctrl+S)").clicked() {
                acts.push(Action::Run("compose.saveDraft", json!({})));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::tool_button(ui, Icon::Delete, "Discard", &t, true).clicked() {
                    acts.push(Action::Run("compose.discard", json!({})));
                }
            });
        });
        if let Some(e) = &c.error {
            ui.add_space(4.0);
            egui::Frame::new().fill(t.danger.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.label(RichText::new(format!("Not sent: {e}")).color(t.danger));
            });
        }
        ui.add_space(8.0);
        ui.add_enabled_ui(!c.sending, |ui| {
            if accounts.len() > 1 {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(56.0, 26.0), egui::Label::new(RichText::new("From").color(t.dim)));
                    let current = accounts.iter().find(|(id, _)| *id == c.account).map(|(_, l)| l.clone()).unwrap_or_default();
                    egui::ComboBox::from_id_salt("from").selected_text(current).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
                        for (id, label) in &accounts {
                            ui.selectable_value(&mut c.account, id.clone(), label);
                        }
                    });
                });
            }
            field(ui, &t, "To", &mut c.to, "Names <addresses>, separated by ; or ,", true, focus_to);
            if c.show_cc {
                field(ui, &t, "Cc", &mut c.cc, "", true, false);
                field(ui, &t, "Bcc", &mut c.bcc, "Hidden from the other recipients", true, false);
            } else {
                ui.horizontal(|ui| {
                    ui.add_space(62.0);
                    if ui.link(RichText::new("Cc / Bcc").small()).clicked() {
                        c.show_cc = true;
                    }
                });
            }
            field(ui, &t, "Subject", &mut c.subject, "Add a subject", false, false);
            if !c.attachments.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(62.0);
                    let mut remove = None;
                    for (i, a) in c.attachments.iter().enumerate() {
                        egui::Frame::new()
                            .fill(t.chrome)
                            .stroke(egui::Stroke::new(1.0, t.border))
                            .corner_radius(14)
                            .inner_margin(Margin::symmetric(10, 3))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                    icons::paint(ui.painter(), ir, Icon::Attach, t.dim);
                                    ui.label(
                                        RichText::new(format!("{} ({})", widgets::shorten(&a.name, 30), widgets::human_size(a.data.len()))).small(),
                                    );
                                    if widgets::tiny_icon_button(ui, Icon::Close, "Remove", &t).clicked() {
                                        remove = Some(i);
                                    }
                                });
                            });
                    }
                    if let Some(i) = remove {
                        acts.push(Action::Run("compose.removeAttachment", json!({"index": i})));
                    }
                });
            }
            ui.add_space(6.0);
            let sep = ui.available_rect_before_wrap();
            ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().auto_shrink(false).id_salt("compose-body").show(ui, |ui| {
                let h = ui.available_height().max(200.0);
                let r = ui.add_sized(
                    vec2(ui.available_width(), h),
                    egui::TextEdit::multiline(&mut c.body)
                        .frame(egui::Frame::NONE)
                        .font(theme::regular(14.5))
                        .hint_text("Write your message…")
                        .lock_focus(true),
                );
                if focus_body {
                    r.request_focus();
                    // Replies start above the quoted text.
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(0))));
                        state.store(ui.ctx(), r.id);
                    }
                }
            });
        });
    });
}
