//! The reading pane (the open message with its conversation and attachments) and the inline
//! compose form that replaces it while a message is being written.

use egui::{Color32, Margin, Rect, RichText, Sense, Ui, vec2};
use mailcraft_engine::mime::{self, AttachmentKind};
use mailcraft_engine::model::contacts::Contact;
use mailcraft_engine::model::{MessageKey, addr, dates};
use mailcraft_engine::{Session, model::EmailAddress};
use serde_json::json;

use crate::editor::{self, Fmt};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, UiState, widgets};

pub fn show(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.compose.is_some() {
        compose(s, st, ui, acts);
        return;
    }
    let t = st.tokens;
    if s.reading.is_none() {
        empty(ui, &t, if s.list.is_empty() { "" } else { tl!("Select a message to read it") });
        return;
    }
    message(s, st, ui, acts, false);
}

/// A message in a window of its own. What it asks for applies to that message: it becomes the
/// open one first.
pub fn window(s: &mut Session, st: &mut UiState, ui: &mut Ui, w: mailcraft_engine::Reading, acts: &mut Vec<Action>) {
    let key = w.key.clone();
    let saved = s.reading.replace(w);
    let mut mine = Vec::new();
    message(s, st, ui, &mut mine, true);
    s.reading = saved;
    if !mine.is_empty() {
        acts.push(Action::Open(key));
        acts.extend(mine);
    }
}

fn message(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, in_window: bool) {
    let t = st.tokens;
    let Some(r) = s.reading.clone() else { return };
    let summary = s.summary(&r.key).cloned();
    egui::ScrollArea::vertical().auto_shrink(false).id_salt(("reading", in_window, r.key.to_id())).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 24, right: 24, top: 18, bottom: 24 }).show(ui, |ui| {
            let Some(p) = &r.parsed else {
                ui.add_space(30.0);
                match &r.error {
                    Some(e) => {
                        ui.horizontal(|ui| {
                            let (ir, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            icons::paint(ui.painter(), ir, Icon::Warning, t.warn);
                            ui.label(RichText::new(tf!("This message couldn't be downloaded: {error}", error = e)).color(t.dim));
                        });
                        if ui.button(tl!("Try again")).clicked() {
                            acts.push(Action::Open(r.key.clone()));
                        }
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(tl!("Downloading the message…")).color(t.dim));
                        });
                    }
                }
                return;
            };
            // Subject.
            ui.horizontal_wrapped(|ui| {
                let subject = if p.subject.is_empty() { tl!("(no subject)") } else { p.subject.as_str() };
                ui.label(RichText::new(subject).font(theme::bold(21.0)).color(t.text));
                if summary.as_ref().is_some_and(|m| m.flags.flagged) {
                    let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), ir, Icon::Flag, t.flag);
                }
            });
            if let Some(m) = &summary {
                let cats = s.categories_of(m);
                if !cats.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for c in cats {
                            let color = theme::category_color(c.color, t.dark);
                            egui::Frame::new().fill(color.gamma_multiply(0.18)).corner_radius(10).inner_margin(Margin::symmetric(8, 2)).show(
                                ui,
                                |ui| {
                                    ui.label(RichText::new(&c.name).color(color).small());
                                },
                            );
                        }
                    });
                }
            }
            ui.add_space(10.0);

            // Conversation.
            if r.thread.len() > 1 && !in_window {
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
                        ui.label(RichText::new(tf!("To: {names}", names = addr::display_list(&p.to))).color(t.dim).small())
                            .on_hover_text(addr::format_list(&p.to));
                    }
                    if !p.cc.is_empty() {
                        ui.label(RichText::new(tf!("Cc: {names}", names = addr::display_list(&p.cc))).color(t.dim).small())
                            .on_hover_text(addr::format_list(&p.cc));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    let when = summary.as_ref().map_or(p.date, |m| if p.date > 0 { p.date } else { m.date });
                    ui.label(RichText::new(crate::i18n::date(&dates::full_time(when, &s.clock))).color(t.dim).small());
                });
            });
            ui.add_space(8.0);
            if let Some((_, item)) = s.outbox_entry(&r.key) {
                egui::Frame::new().fill(t.warn.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                    let why = match item.error.as_deref() {
                        Some(e) if e != "offline" => tf!("Waiting in the Outbox. Last try: {error}", error = widgets::shorten(e, 140)),
                        _ => tl!("Waiting in the Outbox: it will be sent when you're back online.").to_string(),
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
            } else if !in_window {
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
                    ui.add_space(6.0);
                    if widgets::pill_button(ui, Icon::Print, "Print", &t).on_hover_text(tl!("Open as a PDF to print")).clicked() {
                        acts.push(Action::Print);
                    }
                    let more = widgets::pill_button(ui, Icon::File, "Save", &t);
                    egui::Popup::menu(&more).show(|ui| {
                        if ui.button(tl!("Save as PDF…")).clicked() {
                            acts.push(Action::ExportPdf);
                            ui.close();
                        }
                        if ui.button(tl!("Save as email (.eml)…")).clicked() {
                            acts.push(Action::SaveMessage);
                            ui.close();
                        }
                    });
                });
            }
            ui.add_space(12.0);

            // Attachments.
            // Pictures the formatted message shows in place aren't listed again as files.
            let formatted = s.prefs.show_html && !p.rich.is_empty();
            let in_place = |a: &mime::AttachmentInfo| {
                formatted
                    && a.content_id
                        .as_ref()
                        .is_some_and(|c| p.images.iter().any(|i| i.strip_prefix("cid:").is_some_and(|x| x.eq_ignore_ascii_case(c))))
            };
            let files: Vec<_> = p.attachments.iter().filter(|a| (!a.inline || a.kind() == AttachmentKind::Image) && !in_place(a)).cloned().collect();
            if !files.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if files.len() == 1 { tl!("1 attachment").to_string() } else { tf!("{n} attachments", n = files.len()) })
                            .color(t.dim)
                            .small(),
                    );
                    if files.len() > 1 && ui.link(RichText::new(tl!("Save all…")).small()).clicked() {
                        acts.push(Action::SaveAll);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for a in &files {
                        attachment_card(s, st, ui, &r.key, a, acts);
                    }
                });
                ui.add_space(10.0);
            }
            if p.had_html {
                html_bar(s, &t, ui, p, acts);
            }
            if let Some(u) = &p.unsubscribe {
                ui.label(RichText::new(tf!("Mailing list: unsubscribe at {address}", address = widgets::shorten(u, 80))).color(t.faint).small());
                ui.add_space(6.0);
            }
            let sep = ui.available_rect_before_wrap();
            ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
            ui.add_space(12.0);
            if s.prefs.show_html && !p.rich.is_empty() {
                rich_body(s, st, ui, &r.key, p, acts);
            } else {
                let mut text = p.text.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .font(theme::regular(14.5))
                        .text_color(t.text),
                );
            }
        });
    });
}

fn empty(ui: &mut Ui, t: &Tokens, msg: &str) {
    let msg = tl!(msg);
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
            ui.label(RichText::new(tf!("{n} messages in this conversation", n = thread.len())).color(t.dim).small());
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
                    crate::i18n::date(&dates::list_time(m.date, &s.clock)),
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
    tl!(kind_name(k))
}

fn kind_name(k: AttachmentKind) -> &'static str {
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
        decode(ctx, &format!("att-{}-{index}", key.to_id()), &data, 480)
    })();
    st.previews.insert((key.clone(), index), tex.clone());
    tex
}

/// The first page of a PDF attachment (cached like pictures).
fn pdf_preview(s: &Session, st: &mut UiState, ctx: &egui::Context, key: &MessageKey, index: usize) -> Option<egui::TextureHandle> {
    if let Some(t) = st.previews.get(&(key.clone(), index)) {
        return t.clone();
    }
    if st.previews.len() > 64 {
        st.previews.clear();
    }
    let tex = (|| {
        let raw = s.store.as_ref()?.read_body(key)?;
        let data = mime::attachment_data(&raw, index)?;
        let img = crate::thumbs::pdf_first_page(&data, 260, st.fonts.clone())?;
        Some(ctx.load_texture(format!("pdf-{}-{index}", key.to_id()), img, egui::TextureOptions::LINEAR))
    })();
    st.previews.insert((key.clone(), index), tex.clone());
    tex
}

/// A frame of a video attachment, asked of the platform (which may take a moment).
fn video_preview(s: &Session, st: &mut UiState, ctx: &egui::Context, key: &MessageKey, a: &mime::AttachmentInfo) -> Option<egui::TextureHandle> {
    let id = (key.clone(), a.index);
    if let Some(t) = st.previews.get(&id) {
        return t.clone();
    }
    let path = match st.video_files.get(&id) {
        Some(p) => p.clone(),
        None => {
            // The platform's decoder reads files: put the attachment in one.
            let written = (|| {
                let raw = s.store.as_ref()?.read_body(key)?;
                let data = mime::attachment_data(&raw, a.index)?;
                let dir = std::env::temp_dir().join("mailcraft-thumbs");
                std::fs::create_dir_all(&dir).ok()?;
                let name = format!("{:016x}-{}", mailcraft_engine::store::fnv64(&key.to_id()), mime::sanitize_filename(&a.name));
                let path = dir.join(name);
                std::fs::write(&path, &data).ok()?;
                Some(path)
            })();
            let Some(p) = written else {
                st.previews.insert(id, None);
                return None;
            };
            st.video_files.insert(id.clone(), p.clone());
            p
        }
    };
    let frame = st.video_frames.entry(path).or_insert(crate::VideoFrame::Pending).clone();
    match frame {
        crate::VideoFrame::Pending => {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            None
        }
        crate::VideoFrame::Ready(png) => {
            let tex = decode(ctx, &format!("video-{}-{}", key.to_id(), a.index), &png, 480);
            st.previews.insert(id, tex.clone());
            tex
        }
        crate::VideoFrame::Unavailable => {
            st.previews.insert(id, None);
            None
        }
    }
}

/// Decode a picture from mail or the web into a texture, at most `max` pixels on a side.
/// Pictures are hostile input: size and allocation are capped before decoding.
fn decode(ctx: &egui::Context, name: &str, data: &[u8], max: u32) -> Option<egui::TextureHandle> {
    if data.len() > 30 << 20 {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(256 << 20);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?;
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    // Shrink large pictures; never enlarge small ones (the painter scales them crisply).
    let img = if decoded.width() > max || decoded.height() > max { decoded.thumbnail(max, max) } else { decoded }.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    // Pixel art and tiny swatches stay crisp when scaled up.
    let filter = if size[0].max(size[1]) < 96 { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
    Some(ctx.load_texture(name, ci, filter))
}

/// Above an HTML message: whether its web pictures are hidden, and a switch to plain text.
fn html_bar(s: &Session, t: &Tokens, ui: &mut Ui, p: &mime::Parsed, acts: &mut Vec<Action>) {
    let web = s.remote_pictures();
    let formatted = s.prefs.show_html && !p.rich.is_empty();
    if !web.is_empty() && !s.pictures_allowed() && formatted {
        egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(
            ui,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), ir, Icon::Image, t.dim);
                    ui.label(RichText::new(tl!("Pictures from the web are hidden to protect your privacy.")).color(t.dim));
                    if ui.link(tl!("Show pictures")).clicked() {
                        acts.push(Action::Run("message.showPictures", json!({})));
                    }
                    let who = p.from.email.clone();
                    if !who.is_empty() && ui.link(tf!("Always show from {sender}", sender = who)).clicked() {
                        acts.push(Action::Run("message.showPictures", json!({"always": true})));
                    }
                });
            },
        );
        ui.add_space(6.0);
    }
    ui.horizontal(|ui| {
        if formatted {
            if ui.link(RichText::new(tl!("Show as plain text")).small()).clicked() {
                acts.push(Action::Run("view.html", json!({"on": false})));
            }
            if !web.is_empty() && s.pictures_allowed() && s.prefs.trusted_senders.contains(&p.from.email.to_ascii_lowercase()) {
                ui.label(RichText::new("·").color(t.faint).small());
                if ui.link(RichText::new(tl!("Stop showing pictures from this sender")).small()).clicked() {
                    acts.push(Action::Run("message.blockPictures", json!({})));
                }
            }
        } else if !p.rich.is_empty() && ui.link(RichText::new(tl!("Show formatted")).small()).clicked() {
            acts.push(Action::Run("view.html", json!({"on": true})));
        } else if p.rich.is_empty() {
            ui.label(RichText::new(tl!("Shown as text.")).color(t.faint).small());
        }
    });
    ui.add_space(6.0);
}

/// An HTML message, formatted: emphasis, code, links (opened in the browser on click) and
/// pictures: those inside the message always, those on the web only once allowed.
fn rich_body(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let allowed = s.pictures_allowed();
    let mut shown: Vec<&str> = Vec::new();
    let width = ui.available_width();
    for line in &p.rich {
        if line.is_empty() {
            ui.add_space(9.0);
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for sp in line {
                if let Some(src) = &sp.image {
                    shown.push(src.as_str());
                    if !picture(s, st, ui, key, p, src, allowed, width) {
                        let alt = if sp.text.trim().is_empty() { "picture" } else { sp.text.trim() };
                        ui.label(RichText::new(format!("[{alt}]")).color(t.faint));
                    }
                    continue;
                }
                let font = if sp.heading > 0 {
                    theme::bold(match sp.heading {
                        1 => 22.0,
                        2 => 19.0,
                        3 => 17.0,
                        _ => 15.5,
                    })
                } else if sp.code {
                    egui::FontId::monospace(13.5)
                } else if sp.bold {
                    theme::bold(14.5)
                } else {
                    theme::regular(14.5)
                };
                let mut rt = RichText::new(&sp.text).font(font);
                if sp.italic {
                    rt = rt.italics();
                }
                if sp.strike {
                    rt = rt.strikethrough();
                }
                let (fg, bg) = theme::mail_colors(sp.color, sp.background, &t, if sp.link.is_some() { t.accent } else { t.text });
                if let Some(bg) = bg {
                    rt = rt.background_color(bg);
                }
                match &sp.link {
                    Some(link) => {
                        let r = ui.add(egui::Link::new(rt.color(fg))).on_hover_text(link);
                        if r.clicked() {
                            acts.push(Action::OpenUrl(link.clone()));
                        }
                    }
                    None => {
                        ui.label(rt.color(fg));
                    }
                }
            }
        });
    }
    // Pictures without a description have no place in the text: show them after it.
    if allowed {
        let rest: Vec<&String> = p.images.iter().filter(|u| !shown.contains(&u.as_str())).collect();
        if !rest.is_empty() {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for src in rest {
                    picture(s, st, ui, key, p, src, allowed, width);
                }
            });
        }
    }
}

/// Draw one picture of an HTML message; `false` when it can't be shown (yet).
#[allow(clippy::too_many_arguments)]
fn picture(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, src: &str, allowed: bool, width: f32) -> bool {
    let tex = if let Some(cid) = src.strip_prefix("cid:") {
        let Some(index) = p.attachments.iter().find(|a| a.content_id.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(cid))).map(|a| a.index) else {
            return false;
        };
        preview(s, st, ui.ctx(), key, index)
    } else {
        if !allowed {
            return false;
        }
        match s.pictures.get(src) {
            Some(mailcraft_engine::Picture::Ready(bytes)) => {
                if let Some(t) = st.web_pictures.get(src) {
                    t.clone()
                } else {
                    if st.web_pictures.len() > 200 {
                        st.web_pictures.clear();
                    }
                    let t = decode(ui.ctx(), &format!("web-{src}"), bytes, 1200);
                    st.web_pictures.insert(src.to_string(), t.clone());
                    t
                }
            }
            Some(mailcraft_engine::Picture::Loading) => {
                ui.spinner();
                return true;
            }
            _ => None,
        }
    };
    let Some(tex) = tex else { return false };
    let sz = tex.size_vec2();
    // Tracking pixels and spacers: nothing to see.
    if sz.x <= 3.0 && sz.y <= 3.0 {
        return true;
    }
    let scale = (width.min(900.0) / sz.x).min(1.0);
    ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale));
    true
}

fn attachment_card(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, a: &mime::AttachmentInfo, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let kind = a.kind();
    let tex = match kind {
        AttachmentKind::Image => preview(s, st, ui.ctx(), key, a.index),
        AttachmentKind::Pdf => pdf_preview(s, st, ui.ctx(), key, a.index),
        AttachmentKind::Video => video_preview(s, st, ui.ctx(), key, a),
        _ => None,
    };
    let card =
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
                    if kind == AttachmentKind::Video {
                        // A play mark over the frame.
                        let c = img_rect.center();
                        ui.painter().circle_filled(c, 18.0, Color32::from_black_alpha(140));
                        ui.painter().add(egui::Shape::convex_polygon(
                            vec![c + vec2(-6.0, -9.0), c + vec2(10.0, 0.0), c + vec2(-6.0, 9.0)],
                            Color32::WHITE,
                            egui::Stroke::NONE,
                        ));
                    } else if kind == AttachmentKind::Pdf {
                        ui.painter().rect_stroke(img_rect, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
                    }
                    if resp.on_hover_text(tl!("Open")).clicked() {
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
                ui.horizontal_wrapped(|ui| {
                    // The Craft app for the file, when it's installed; else the system's choice.
                    let craft = mime::craft_app_for(&a.mime, &a.name)
                        .and_then(|(cmd, name)| st.craft_apps.iter().any(|(c, _)| *c == cmd).then_some((cmd, name)));
                    match craft {
                        Some((cmd, name)) => {
                            if ui
                                .small_button(tf!("Open in {app}", app = name))
                                .on_hover_text(tl!("Open it in the Craft app for this kind of file"))
                                .clicked()
                            {
                                acts.push(Action::OpenInCraft(a.index, cmd));
                            }
                            if ui.small_button(tl!("Open")).on_hover_text(tl!("Open with the system's app")).clicked() {
                                acts.push(Action::OpenAttachment(a.index));
                            }
                        }
                        None => {
                            if ui.small_button(tl!("Open")).clicked() {
                                acts.push(Action::OpenAttachment(a.index));
                            }
                        }
                    }
                    if mime::text_preview_kind(&a.mime, &a.name).is_some() && ui.small_button(tl!("Preview")).clicked() {
                        acts.push(Action::Run("attachment.preview", json!({"index": a.index})));
                    }
                    if ui.small_button(tl!("Save as…")).clicked() {
                        acts.push(Action::SaveAttachment(a.index));
                    }
                });
            });
        });
    // Drag the card onto the desktop or another app to copy the file there.
    if st.can_drag_out {
        let drag = ui.interact(card.response.rect, ui.id().with(("attachment-drag", key.to_id(), a.index)), Sense::drag());
        if drag.drag_started() {
            acts.push(Action::DragOut(a.index));
        }
        drag.on_hover_text(tl!("Drag it to a folder or another app to copy it"));
    }
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, hint: &str, check_addresses: bool, focus: bool) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(56.0, 26.0), egui::Label::new(RichText::new(tl!(label)).color(t.dim)));
        let r = ui.add(egui::TextEdit::singleline(value).hint_text(tl!(hint)).desired_width(f32::INFINITY));
        if focus {
            r.request_focus();
        }
    });
    address_check(ui, t, value, check_addresses);
}

/// A recipient box (To, Cc, Bcc) that suggests people from mail already seen: ↑/↓ choose,
/// Enter or Tab takes the highlighted one, a click takes any.
#[allow(clippy::too_many_arguments)]
fn recipient_field(
    ui: &mut Ui,
    st: &mut UiState,
    name: &'static str,
    label: &str,
    value: &mut String,
    hint: &str,
    focus: bool,
    suggestions: &[Contact],
) {
    let t = st.tokens;
    let showing = st.suggest_field == Some(name) && !suggestions.is_empty();
    let mut take: Option<usize> = None;
    if showing {
        let n = suggestions.len();
        // Before the text box sees them: arrows move the highlight, Enter/Tab accept.
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                st.suggest_index = (st.suggest_index + 1) % n;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                st.suggest_index = (st.suggest_index + n - 1) % n;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                take = Some(st.suggest_index.min(n - 1));
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                st.suggest_dismissed = Some(value.clone());
            }
        });
    }
    let r = ui
        .horizontal(|ui| {
            ui.add_sized(vec2(56.0, 26.0), egui::Label::new(RichText::new(tl!(label)).color(t.dim)));
            ui.add(egui::TextEdit::singleline(value).hint_text(tl!(hint)).desired_width(f32::INFINITY))
        })
        .inner;
    if focus {
        r.request_focus();
    }
    if r.changed() {
        st.suggest_index = 0;
        st.suggest_dismissed = None;
    }
    if showing && take.is_none() {
        let area = egui::Area::new(ui.id().with(("suggest", name))).order(egui::Order::Foreground).fixed_pos(r.rect.left_bottom() + vec2(0.0, 2.0));
        area.show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(r.rect.width().min(420.0));
                for (i, c) in suggestions.iter().enumerate() {
                    let on = i == st.suggest_index;
                    let text =
                        if c.address.name.is_empty() { c.address.email.clone() } else { format!("{}  ·  {}", c.address.name, c.address.email) };
                    let resp = ui.selectable_label(on, RichText::new(text).color(t.text));
                    // On press: the box loses focus on the same click, before a release would count.
                    if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
                        take = Some(i);
                    }
                }
            });
        });
    }
    if let Some(i) = take
        && let Some(c) = suggestions.get(i)
    {
        *value = mailcraft_engine::model::contacts::complete(value, &c.address);
        st.suggest_index = 0;
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
            let end = egui::text::CCursor::new(value.chars().count());
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), r.id);
        }
    }
    if r.has_focus() || take.is_some() {
        st.suggest_field = Some(name);
    } else if st.suggest_field == Some(name) {
        st.suggest_field = None;
    }
    // Half-typed addresses are fine while typing; check once the box is left.
    address_check(ui, &t, value, !r.has_focus());
}

fn address_check(ui: &mut Ui, t: &Tokens, value: &str, check_addresses: bool) {
    if check_addresses
        && !value.trim().is_empty()
        && let Err(bad) = addr::parse_list(value)
    {
        ui.horizontal(|ui| {
            ui.add_space(64.0);
            ui.label(RichText::new(tf!("Not an email address: {address}", address = bad)).color(t.danger).small());
        });
    }
}

fn compose(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let accounts: Vec<(String, String)> =
        s.accounts.iter().map(|a| (a.config.id.clone(), EmailAddress::new(a.config.name.clone(), a.config.email.clone()).to_string())).collect();
    // Suggestions for the recipient box being typed in (computed before the form borrows it).
    let suggestions: Vec<Contact> = match (st.suggest_field, s.compose.as_ref()) {
        (Some(f), Some(c)) => {
            let text = match f {
                "to" => c.to.clone(),
                "cc" => c.cc.clone(),
                _ => c.bcc.clone(),
            };
            let token = mailcraft_engine::model::contacts::last_token(&text).to_string();
            if token.chars().count() < 1 || st.suggest_dismissed.as_deref() == Some(text.as_str()) {
                Vec::new()
            } else {
                let present = text.to_lowercase();
                s.suggest_contacts(&token, 8).into_iter().filter(|x| !present.contains(&x.address.email.to_lowercase())).take(6).collect()
            }
        }
        _ => Vec::new(),
    };
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
            if widgets::tool_button(ui, Icon::Drafts, "Save draft", &t, !c.sending).on_hover_text(tl!("Save draft (Ctrl+S)")).clicked() {
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
                ui.label(RichText::new(tf!("Not sent: {error}", error = e)).color(t.danger));
            });
        }
        ui.add_space(8.0);
        ui.add_enabled_ui(!c.sending, |ui| {
            if accounts.len() > 1 {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(56.0, 26.0), egui::Label::new(RichText::new(tl!("From")).color(t.dim)));
                    let current = accounts.iter().find(|(id, _)| *id == c.account).map(|(_, l)| l.clone()).unwrap_or_default();
                    egui::ComboBox::from_id_salt("from").selected_text(current).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
                        for (id, label) in &accounts {
                            ui.selectable_value(&mut c.account, id.clone(), label);
                        }
                    });
                });
            }
            recipient_field(ui, st, "to", "To", &mut c.to, "Names <addresses>, separated by ; or ,", focus_to, &suggestions);
            if c.show_cc {
                recipient_field(ui, st, "cc", "Cc", &mut c.cc, "", false, &suggestions);
                recipient_field(ui, st, "bcc", "Bcc", &mut c.bcc, "Hidden from the other recipients", false, &suggestions);
            } else {
                ui.horizontal(|ui| {
                    ui.add_space(62.0);
                    if ui.link(RichText::new(tl!("Cc / Bcc")).small()).clicked() {
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
            let body_id = egui::Id::new(("compose-body", c.id));
            // The format bar: buttons act on the body's selection (from the last frame).
            let mut format: Option<Fmt> = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.spacing_mut().button_padding = vec2(6.0, 2.0);
                ui.add_enabled_ui(!c.plain, |ui| {
                    for f in Fmt::ALL {
                        let text = match f {
                            Fmt::Bold => RichText::new(tl!(f.label())).strong(),
                            Fmt::Italic => RichText::new(tl!(f.label())).italics(),
                            Fmt::Strike => RichText::new(tl!(f.label())).strikethrough(),
                            _ => RichText::new(tl!(f.label())),
                        };
                        if ui.add(egui::Button::new(text.size(13.0)).frame(false).min_size(vec2(26.0, 22.0))).on_hover_text(tl!(f.tip())).clicked() {
                            format = Some(f);
                        }
                    }
                    ui.add_space(6.0);
                    if ui
                        .add(egui::Button::new(RichText::new(tl!("Picture…")).size(13.0)).frame(false))
                        .on_hover_text(tl!("Insert a picture (or paste one)"))
                        .clicked()
                    {
                        acts.push(Action::InsertPicture);
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut plain = c.plain;
                    if ui.checkbox(&mut plain, RichText::new(tl!("Plain text")).small()).on_hover_text(tl!("Send without formatting")).changed() {
                        c.plain = plain;
                    }
                });
            });
            // Keyboard: Ctrl+B, Ctrl+I, Ctrl+K while writing.
            if !c.plain && ui.memory(|m| m.has_focus(body_id)) {
                ui.input_mut(|i| {
                    for (key, f) in [(egui::Key::B, Fmt::Bold), (egui::Key::I, Fmt::Italic), (egui::Key::K, Fmt::Link)] {
                        if i.consume_key(egui::Modifiers::COMMAND, key) {
                            format = Some(f);
                        }
                    }
                });
            }
            if let Some(f) = format {
                let mut state = egui::TextEdit::load_state(ui.ctx(), body_id).unwrap_or_default();
                let n = c.body.chars().count();
                let sel = state.cursor.char_range().map_or(n..n, |r| {
                    let s = r.as_sorted_char_range();
                    s.start.0.min(n)..s.end.0.min(n)
                });
                let new = editor::apply(&mut c.body, sel, f);
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(new.start), egui::text::CCursor::new(new.end))));
                state.store(ui.ctx(), body_id);
                ui.memory_mut(|m| m.request_focus(body_id));
            }
            let sep = ui.available_rect_before_wrap();
            ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
            ui.add_space(6.0);
            let plain = c.plain;
            let tokens = t;
            egui::ScrollArea::vertical().auto_shrink(false).id_salt("compose-body").show(ui, |ui| {
                let h = ui.available_height().max(200.0);
                let mut layouter =
                    |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| ui.painter().layout_job(editor::layout(buf.as_str(), &tokens, wrap));
                let mut edit = egui::TextEdit::multiline(&mut c.body)
                    .id(body_id)
                    .frame(egui::Frame::NONE)
                    .font(theme::regular(14.5))
                    .hint_text(tl!("Write your message…"))
                    .lock_focus(true);
                if !plain {
                    edit = edit.layouter(&mut layouter);
                }
                let r = ui.add_sized(vec2(ui.available_width(), h), edit);
                // Where a picture goes when one is inserted or pasted.
                if let Some(state) = egui::TextEdit::load_state(ui.ctx(), r.id)
                    && let Some(range) = state.cursor.char_range()
                {
                    st.compose_cursor = range.primary.index.0;
                }
                // A picture on the clipboard is pasted as a picture.
                if r.has_focus()
                    && ui.input(|i| {
                        i.events
                            .iter()
                            .any(|e| matches!(e, egui::Event::Key { key: egui::Key::V, pressed: true, modifiers, .. } if modifiers.command))
                    })
                {
                    acts.push(Action::PastePicture);
                }
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
