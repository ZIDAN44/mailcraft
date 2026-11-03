//! The reading pane: attachments, their cards and their previews (pictures, PDF, video, Office files).

use super::*;

pub(super) fn kind_icon(k: AttachmentKind) -> Icon {
    match k {
        AttachmentKind::Image => Icon::Image,
        _ => Icon::File,
    }
}

/// A frame of a video attachment, asked of the platform (which may take a moment).
pub(super) fn video_preview(
    s: &Session,
    st: &mut UiState,
    ctx: &egui::Context,
    key: &MessageKey,
    a: &mime::AttachmentInfo,
) -> Option<egui::TextureHandle> {
    let id = (key.clone(), a.index);
    let now = st.pictures.frame();
    if let Some((t, used)) = st.video_thumbs.get_mut(&id) {
        *used = now;
        return t.clone();
    }
    // As many shown in this frame as are kept: this chip shows its kind's icon (one past them
    // was read, written and decoded again every frame).
    if st.video_thumbs.len() >= VIDEO_THUMBS && st.video_thumbs.values().all(|(_, used)| *used == now) {
        return None;
    }
    // The platform's decoder reads files, and the browser has none.
    if !st.has_files {
        return None;
    }
    let path = match st.video_files.get(&id) {
        Some(p) => p.clone(),
        // The platform's decoder reads files: the attachment is put in one, on the pictures
        // lane (its kind's icon meanwhile).
        None => match st.videos.file(key, a.index, &a.name, s.store.as_ref()) {
            None => return None,
            Some(None) => {
                keep_video_thumb(st, id, None);
                return None;
            }
            Some(Some(p)) => {
                st.video_files.insert(id.clone(), p.clone());
                p
            }
        },
    };
    // The platform's frame, its pixels decoded on the pictures lane too.
    let limit = ctx.input(|i| i.max_texture_side).try_into().unwrap_or(u32::MAX);
    let pixels = match st.video_frames.entry(path).or_insert(crate::VideoFrame::Pending) {
        crate::VideoFrame::Pending => None,
        crate::VideoFrame::Ready(png) => st.videos.frame(key, a.index, png, limit),
        crate::VideoFrame::Unavailable => Some(None),
    };
    let Some(pixels) = pixels else {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        return None;
    };
    let tex = pixels.map(|(image, options)| ctx.load_texture(format!("video-{}-{}", key.to_id(), a.index), image, options));
    keep_video_thumb(st, id, tex.clone());
    tex
}

/// Most video thumbnails kept (those shown least recently go first, never one shown in this
/// frame).
const VIDEO_THUMBS: usize = 64;

fn keep_video_thumb(st: &mut UiState, id: (MessageKey, usize), tex: Option<egui::TextureHandle>) {
    let now = st.pictures.frame();
    while st.video_thumbs.len() >= VIDEO_THUMBS {
        let Some(oldest) = st.video_thumbs.iter().filter(|(_, (_, used))| *used < now).min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone())
        else {
            break;
        };
        st.video_thumbs.remove(&oldest);
    }
    if st.video_thumbs.len() < VIDEO_THUMBS {
        st.video_thumbs.insert(id, (tex, now));
    }
}

/// Decode a picture from mail or the web into a texture, at most `max` pixels on a side.
pub(crate) fn decode(ctx: &egui::Context, name: &str, data: &[u8], max: u32) -> Option<egui::TextureHandle> {
    let limit = ctx.input(|i| i.max_texture_side).try_into().unwrap_or(u32::MAX);
    let (image, filter) = decode_pixels(data, max, limit)?;
    Some(ctx.load_texture(name, image, filter))
}

/// A picture's pixels, at most `max` pixels on a side and never more than `limit` (the
/// graphics card's largest), and how to draw them; pure, so it can run off the window's
/// thread. Pictures are hostile input: size and allocation are capped before decoding.
pub(crate) fn decode_pixels(data: &[u8], max: u32, limit: u32) -> Option<(egui::ColorImage, egui::TextureOptions)> {
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
    // Never bigger than the graphics card takes (a software or browser one may take 2048 only).
    let max = max.min(limit).max(1);
    // Shrink large pictures; never enlarge small ones (the painter scales them crisply).
    let img = if decoded.width() > max || decoded.height() > max { decoded.thumbnail(max, max) } else { decoded }.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    // Pixel art and tiny swatches stay crisp when scaled up.
    let filter = if size[0].max(size[1]) < 96 { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
    Some((ci, filter))
}

/// An attachment as Outlook shows one: a small chip with a thumbnail (or the kind's icon), the
/// name and size, and a ▾ menu. Double-click opens it; right-click gives the menu too; dragging it
/// out (once the pointer moves) copies the file to another app. The chip senses the pointer under
/// its button, so a press on ▾ or the chip never starts a drag by itself.
pub(super) fn attachment_card(
    s: &Session,
    st: &mut UiState,
    ui: &mut Ui,
    key: &MessageKey,
    a: &mime::AttachmentInfo,
    w: f32,
    acts: &mut Vec<Action>,
) {
    let t = st.tokens;
    let kind = a.kind();
    let thumb = match kind {
        AttachmentKind::Image => st.pictures.get(key, a.index, Want::Thumb).ready(),
        AttachmentKind::Pdf => st.pictures.get(key, a.index, Want::PdfFirst).ready(),
        AttachmentKind::Video => video_preview(s, st, ui.ctx(), key, a),
        _ => None,
    };
    // The Craft app for the file, when it's installed.
    let craft = mime::craft_app_for(&a.mime, &a.name).and_then(|(cmd, name)| st.craft_apps.iter().any(|(c, _)| *c == cmd).then_some((cmd, name)));
    let text_preview = mime::text_preview_kind(&a.mime, &a.name).is_some();
    let lower = a.name.to_ascii_lowercase();
    let pgp_key = a.mime.eq_ignore_ascii_case("application/pgp-keys") || (lower.ends_with(".asc") && lower.contains("key"));
    // Several selected, this one among them: its menu can save them together.
    let several: Option<Vec<usize>> =
        st.attachment_selected.as_ref().filter(|(k, v)| k == key && v.len() > 1 && v.contains(&a.index)).map(|(_, v)| v.clone());
    let menu = |ui: &mut Ui, acts: &mut Vec<Action>| {
        if kind == AttachmentKind::Image && ui.button(tl!("Preview")).clicked() {
            acts.push(Action::Dialog(crate::Dialog::PicturePreview { key: key.clone(), index: a.index, name: a.name.clone() }));
            ui.close();
        }
        if text_preview && ui.button(tl!("Preview")).clicked() {
            acts.push(Action::Run("attachment.preview", json!({"index": a.index})));
            ui.close();
        }
        if ui.button(tl!("Open")).clicked() {
            acts.push(Action::OpenAttachment(a.index));
            ui.close();
        }
        if let Some((cmd, name)) = craft
            && ui.button(tf!("Open in {app}", app = name)).clicked()
        {
            acts.push(Action::OpenInCraft(a.index, cmd));
            ui.close();
        }
        // Someone's OpenPGP key: kept, to check their signatures and encrypt to them.
        if pgp_key && ui.button(tl!("Import OpenPGP key")).clicked() {
            acts.push(Action::Run("pgp.importAttached", json!({"index": a.index})));
            ui.close();
        }
        ui.separator();
        if ui.button(tl!("Save As…")).clicked() {
            acts.push(Action::SaveAttachment(a.index));
            ui.close();
        }
        if let Some(v) = &several
            && ui.button(tf!("Save {n} Selected…", n = v.len())).clicked()
        {
            acts.push(Action::SaveAttachments(v.clone()));
            ui.close();
        }
        if ui.button(tl!("Save All Attachments…")).clicked() {
            acts.push(Action::SaveAll);
            ui.close();
        }
    };
    // The chip: the body (double-click opens, drag copies out) and the ▾ at its right, each lit
    // when the pointer is over it, as Outlook's are. A click selects the chip.
    let (rect, resp) = ui.allocate_exact_size(vec2(w, CHIP_H), Sense::click_and_drag());
    let more_rect = Rect::from_min_max(egui::pos2(rect.right() - 30.0, rect.top()), rect.max).shrink(2.0);
    let more = ui.interact(more_rect, resp.id.with("more"), Sense::click()).on_hover_text(tl!("More options"));
    crate::widgets::named(ui, &more, tl!("More options"), true, None);
    // egui counts the chip as hovered under the ▾ too (it senses drags); the ▾ wins there.
    let on_more = more.contains_pointer();
    let body_hovered = resp.hovered() && !on_more;
    if resp.contains_pointer() || more.contains_pointer() {
        st.attachment_hovered = true;
    }
    // The selection: a click picks this one, Ctrl+click adds or removes it; a right-click or
    // the ▾ on one not selected picks it alone. Any of them gives the strip the keyboard.
    let mut picked: Vec<usize> = st.attachment_selected.as_ref().filter(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default();
    let ctrl = ui.input(|i| i.modifiers.command);
    if resp.clicked() && !on_more && ctrl {
        if let Some(at) = picked.iter().position(|i| *i == a.index) {
            picked.remove(at);
        } else {
            picked.push(a.index);
        }
    } else if (resp.clicked() && !on_more) || ((resp.secondary_clicked() || more.clicked()) && !picked.contains(&a.index)) {
        picked = vec![a.index];
    }
    if resp.clicked() || resp.secondary_clicked() || more.clicked() {
        st.attachment_selected = Some((key.clone(), picked.clone()));
        st.attachments_active = true;
        // Another click lets go of a preview still waiting.
        st.attachment_click = None;
    }
    // A plain click shows it in place of the message's text, as Outlook previews attachments,
    // once the double-click time is out ([`settle_attachment_click`]): a double-click only opens
    // the file, and its first click left the preview showing behind the app it opened in.
    let opened = resp.double_clicked() || resp.triple_clicked();
    if resp.clicked() && !on_more && !ctrl && !opened {
        let (now, wait) = (ui.input(|i| i.time), ui.ctx().options(|o| o.input_options.max_double_click_delay));
        st.attachment_click = Some((key.clone(), a.index, now + wait));
    }
    let previewed = st.attachment_view.as_ref().is_some_and(|(k, i)| k == key && *i == a.index);
    let selected = previewed || picked.contains(&a.index);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect(
            rect,
            4.0,
            t.surface,
            egui::Stroke::new(1.0, if selected { t.accent } else { t.faint.gamma_multiply(0.55) }),
            egui::StrokeKind::Inside,
        );
        let body = Rect::from_min_max(rect.min, egui::pos2(more_rect.left() - 2.0, rect.bottom())).shrink(2.0);
        if selected || body_hovered {
            p.rect_filled(body, 3.0, if selected { t.accent_soft } else { t.hover_row });
        }
        let menu_open = egui::Popup::is_id_open(ui.ctx(), more.id.with("menu"));
        if on_more || menu_open {
            p.rect_filled(more_rect, 3.0, t.hover_row);
        }
        icons::paint(p, icons::centered(more_rect, 13.0), Icon::ChevronDown, if on_more { t.text } else { t.dim });
        let r = Rect::from_min_size(egui::pos2(rect.left() + 6.0, rect.center().y - 15.0), vec2(30.0, 30.0));
        match &thumb {
            Some(tex) => {
                let sz = tex.size_vec2();
                let scale = (30.0 / sz.x).min(30.0 / sz.y);
                p.rect_filled(r, 3.0, t.chrome);
                let img = Rect::from_center_size(r.center(), sz * scale);
                p.image(tex.id(), img, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.rect_filled(r, 4.0, t.accent_soft);
                icons::paint(p, icons::centered(r, 17.0), kind_icon(kind), t.accent);
            }
        }
        let text_x = r.right() + 8.0;
        let text_w = more_rect.left() - 4.0 - text_x;
        let name = widgets::one_line(ui, &a.name, theme::regular(12.5), t.text, text_w);
        let size = widgets::one_line(ui, &widgets::human_size(a.size), theme::regular(11.5), t.dim, text_w);
        p.galley(egui::pos2(text_x, rect.center().y - name.size().y - 1.0), name, t.text);
        p.galley(egui::pos2(text_x, rect.center().y + 1.0), size, t.dim);
    }
    egui::Popup::menu(&more).id(more.id.with("menu")).show(|ui| menu(ui, acts));
    if resp.double_clicked() && !on_more {
        acts.push(Action::OpenAttachment(a.index));
    }
    resp.context_menu(|ui| menu(ui, acts));
    // Drag it onto the desktop or another app to copy the file there.
    if st.can_drag_out && resp.drag_started() && !on_more {
        acts.push(Action::DragOut(a.index));
    }
    if !on_more {
        resp.on_hover_text(a.name.clone());
    }
}

/// The preview a click on a chip waits to show, shown once the double-click time is out (as
/// PDFCraft's full screen waits to turn its page); a double-click took it back meanwhile.
pub(super) fn settle_attachment_click(st: &mut UiState, ui: &Ui) {
    let Some((key, index, due)) = st.attachment_click.clone() else { return };
    let now = ui.input(|i| i.time);
    if now >= due {
        st.attachment_click = None;
        st.attachment_view = Some((key, index));
    } else {
        // A message's own window draws in the main one's frame.
        let wait = std::time::Duration::try_from_secs_f64(due - now).unwrap_or_default();
        ui.ctx().request_repaint_after(wait);
        ui.ctx().request_repaint_after_for(wait, egui::ViewportId::ROOT);
    }
}

/// Back to the message from a previewed attachment: the chip clicked to show it isn't left
/// selected either.
pub(super) fn close_attachment_view(st: &mut UiState) {
    st.attachment_view = None;
    st.attachment_click = None;
    st.attachment_selected = None;
    st.attachments_active = false;
}

/// Above a previewed attachment: back to the message, the file's name and size, and Open and
/// Save As (Outlook's Back to message and its Attachments tab).
pub(super) fn preview_header(t: &Tokens, ui: &mut Ui, a: &mime::AttachmentInfo, acts: &mut Vec<Action>, view: &mut Option<(MessageKey, usize)>) {
    ui.horizontal(|ui| {
        let back = ui.add(egui::Button::new(RichText::new(format!("← {}", tl!("Back to message"))).color(t.accent)).frame_when_inactive(false));
        if back.on_hover_text(tl!("Esc")).clicked() {
            *view = None;
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
        icons::paint(ui.painter(), icons::centered(r, 22.0), kind_icon(a.kind()), t.accent);
        ui.vertical(|ui| {
            ui.add(egui::Label::new(RichText::new(&a.name).font(theme::bold(16.0)).color(t.text)).truncate());
            ui.label(RichText::new(widgets::human_size(a.size)).color(t.dim).small());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if widgets::pill_button(ui, Icon::File, "Save As…", t).clicked() {
                acts.push(Action::SaveAttachment(a.index));
            }
            if widgets::pill_button(ui, Icon::PopOut, "Open", t).clicked() {
                acts.push(Action::OpenAttachment(a.index));
            }
        });
    });
    ui.add_space(10.0);
}

/// What an attachment is read as, kept while it's shown.
#[derive(Clone)]
pub(super) enum Shown {
    Text { header: Vec<(String, String)>, text: String },
    Office(mime::office::Preview),
    Pdf,
    Picture,
    Nothing,
}

/// The attachment, as the reading pane previews it: a picture at the pane's width, a PDF's
/// pages, text, an attached email, a Word document, an Excel workbook or a PowerPoint
/// presentation's slides; other files say there's no preview, with Open and Save As.
pub(super) fn attachment_preview(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, a: &mime::AttachmentInfo, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let id = egui::Id::unique(("attachment-shown", key.to_id(), a.index));
    let shown = match ui.ctx().data(|d| d.get_temp::<std::sync::Arc<Shown>>(id)) {
        Some(x) => x,
        None => {
            // A picture or a PDF is read with the message's others (`Pictures`), not here.
            let data = match a.kind() {
                AttachmentKind::Image | AttachmentKind::Pdf => None,
                _ => s.store.as_ref().and_then(|st| st.read_body(key)).and_then(|raw| shown_data(&raw, a)),
            };
            let x = std::sync::Arc::new(read_shown(a, data.as_deref()));
            ui.ctx().data_mut(|d| d.insert_temp(id, x.clone()));
            x
        }
    };
    let width = ui.available_width();
    match shown.as_ref() {
        Shown::Picture => match st.pictures.get(key, a.index, Want::Big) {
            Got::Ready(tex) => {
                let sz = tex.size_vec2();
                let scale = (width / sz.x).min(1.0);
                ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale));
            }
            Got::Pending => {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| ui.spinner());
            }
            Got::Failed => no_preview(&t, ui, a, acts),
        },
        Shown::Pdf => pdf_pages(st, ui, key, a, width, acts),
        Shown::Text { header, text } => {
            if !header.is_empty() {
                egui::Grid::new(("att-header", a.index)).num_columns(2).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                    for (k, v) in header {
                        ui.label(RichText::new(k).color(t.dim));
                        ui.label(RichText::new(v).color(t.text));
                        ui.end_row();
                    }
                });
                ui.add_space(10.0);
            }
            let mut text = text.as_str();
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .font(theme::regular(14.0))
                    .text_color(t.text),
            );
        }
        Shown::Office(p) => office_preview(&t, ui, p, a.index),
        Shown::Nothing => no_preview(&t, ui, a, acts),
    }
}

/// The bytes an attachment is previewed from: a text file read in the charset it names, as UTF-8
/// (its bytes as they came, which Open and Save take, would show a file in Windows-1252 with
/// replacement characters), anything else as it came.
pub(super) fn shown_data(raw: &[u8], a: &mime::AttachmentInfo) -> Option<Vec<u8>> {
    match mime::text_preview_kind(&a.mime, &a.name) {
        Some("text") => mime::attachment_text(raw, a.index).map(String::into_bytes),
        _ => mime::attachment_data(raw, a.index),
    }
}

/// The text the reading pane previews an attachment of the message `raw` with, if it shows one.
#[cfg(test)]
pub(crate) fn previewed_text(raw: &[u8], a: &mime::AttachmentInfo) -> Option<String> {
    match read_shown(a, shown_data(raw, a).as_deref()) {
        Shown::Text { text, .. } => Some(text),
        _ => None,
    }
}

/// Read an attachment once for its preview (a picture or a PDF needs no bytes here).
pub(super) fn read_shown(a: &mime::AttachmentInfo, data: Option<&[u8]>) -> Shown {
    let kind = a.kind();
    if kind == AttachmentKind::Image {
        return Shown::Picture;
    }
    if kind == AttachmentKind::Pdf {
        return Shown::Pdf;
    }
    let Some(data) = data else { return Shown::Nothing };
    if let Some(p) = mime::office::preview(&a.name, &a.mime, data) {
        return Shown::Office(p);
    }
    match mime::text_preview_kind(&a.mime, &a.name) {
        Some("message") => match mime::parse(data) {
            Some(m) => {
                let mut header = vec![(tl!("From").to_string(), m.from.to_string())];
                if !m.to.is_empty() {
                    header.push((tl!("To").to_string(), addr::format_list(&m.to)));
                }
                header.push((tl!("Subject").to_string(), m.subject.clone()));
                Shown::Text { header, text: m.text.chars().take(300_000).collect() }
            }
            None => Shown::Nothing,
        },
        Some(_) => {
            Shown::Text { header: Vec::new(), text: String::from_utf8_lossy(data.get(..data.len().min(300_000)).unwrap_or_default()).into_owned() }
        }
        None => Shown::Nothing,
    }
}

pub(super) fn no_preview(t: &Tokens, ui: &mut Ui, a: &mime::AttachmentInfo, acts: &mut Vec<Action>) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
        icons::paint(ui.painter(), r, kind_icon(a.kind()), t.faint);
        ui.add_space(8.0);
        ui.label(RichText::new(tl!("There's no preview for this file.")).color(t.dim));
        ui.label(RichText::new(tl!("Open it in its app, or save it.")).color(t.faint).small());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let w = 180.0;
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            if widgets::pill_button(ui, Icon::PopOut, "Open", t).clicked() {
                acts.push(Action::OpenAttachment(a.index));
            }
            if widgets::pill_button(ui, Icon::File, "Save As…", t).clicked() {
                acts.push(Action::SaveAttachment(a.index));
            }
        });
    });
}

/// Most pages of a PDF shown in the reading pane (the rest: Open it).
pub(super) const PDF_PAGES: usize = 60;

/// A PDF's pages one under the other, the pane's width. Only the pages on screen are drawn
/// (and kept): one not drawn yet takes the room the one before it had.
pub(super) fn pdf_pages(st: &mut UiState, ui: &mut Ui, key: &MessageKey, a: &mime::AttachmentInfo, width: f32, acts: &mut Vec<Action>) {
    let t = st.tokens;
    // Made at the next 100 pixels up and drawn to the width: a window being resized doesn't
    // make the pages again for each pixel.
    let px = ((width * ui.ctx().pixels_per_point()).clamp(200.0, 2400.0) / 100.0).ceil() as u32 * 100;
    let count = st.pictures.pages(key, a.index);
    // A4's shape until a page is drawn.
    let mut shape = 297.0 / 210.0;
    for n in 0..count.unwrap_or(1).min(PDF_PAGES) {
        let guess = Rect::from_min_size(ui.cursor().min, vec2(width, width * shape));
        let got = if ui.is_rect_visible(guess) { st.pictures.get(key, a.index, Want::PdfPage { n, px }) } else { Got::Pending };
        if n == 0 && matches!(got, Got::Failed) {
            no_preview(&t, ui, a, acts);
            return;
        }
        let r = match got {
            Got::Ready(tex) => {
                let sz = tex.size_vec2();
                if sz.x >= 1.0 {
                    shape = sz.y / sz.x;
                }
                ui.add(egui::Image::new(&tex).fit_to_exact_size(vec2(width, width * shape))).rect
            }
            Got::Pending | Got::Failed => {
                let (r, _) = ui.allocate_exact_size(vec2(width, width * shape), Sense::hover());
                if ui.is_rect_visible(r) {
                    let label = widgets::one_line(ui, &tf!("Page {n}", n = n + 1), theme::regular(11.5), t.dim, width);
                    ui.painter().galley(r.center() + vec2(-label.size().x / 2.0, 14.0), label, t.dim);
                    if matches!(got, Got::Pending) {
                        egui::Spinner::new().paint_at(ui, Rect::from_center_size(r.center(), vec2(18.0, 18.0)));
                    }
                }
                r
            }
        };
        ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
        ui.add_space(12.0);
    }
    if let Some(c) = count.filter(|c| *c > PDF_PAGES) {
        ui.label(
            RichText::new(tf!("The first {shown} of {total} pages: open it to read the rest.", shown = PDF_PAGES, total = c)).color(t.dim).small(),
        );
    }
}

/// A Word document, workbook or presentation, as its preview reads it.
pub(super) fn office_preview(t: &Tokens, ui: &mut Ui, p: &mime::office::Preview, index: usize) {
    use mime::office::{Block, Preview};
    match p {
        Preview::Document(blocks) => {
            if blocks.is_empty() {
                ui.label(RichText::new(tl!("The document is empty.")).color(t.dim));
            }
            for (k, b) in blocks.iter().enumerate() {
                match b {
                    Block::Paragraph { runs, heading, list } => {
                        let size = if *heading { 18.0 } else { 14.5 };
                        let mut job = egui::text::LayoutJob::default();
                        job.wrap.max_width = ui.available_width();
                        if *list {
                            job.append("•  ", 0.0, egui::TextFormat { font_id: theme::regular(size), color: t.text, ..Default::default() });
                        }
                        for r in runs {
                            let font = if r.bold || *heading { theme::bold(size) } else { theme::regular(size) };
                            job.append(&r.text, 0.0, egui::TextFormat { font_id: font, color: t.text, italics: r.italic, ..Default::default() });
                        }
                        if runs.is_empty() {
                            ui.add_space(8.0);
                        } else {
                            ui.label(job);
                            ui.add_space(4.0);
                        }
                    }
                    Block::Table(rows) => {
                        table(t, ui, egui::Id::unique(("doc-table", index, k)), rows, false);
                        ui.add_space(8.0);
                    }
                }
            }
        }
        Preview::Workbook(sheets) => {
            let sel_id = egui::Id::unique(("sheet-tab", index));
            let mut sel = ui.ctx().data(|d| d.get_temp::<usize>(sel_id)).unwrap_or(0).min(sheets.len().saturating_sub(1));
            if sheets.len() > 1 {
                ui.horizontal_wrapped(|ui| {
                    for (i, (name, _)) in sheets.iter().enumerate() {
                        if ui.selectable_label(i == sel, name).clicked() {
                            sel = i;
                        }
                    }
                });
                ui.ctx().data_mut(|d| d.insert_temp(sel_id, sel));
                ui.add_space(6.0);
            }
            match sheets.get(sel) {
                Some((_, rows)) if !rows.is_empty() => {
                    egui::ScrollArea::horizontal()
                        .id_salt(("sheet", index, sel))
                        .show(ui, |ui| table(t, ui, egui::Id::unique(("sheet-grid", index, sel)), rows, true));
                }
                _ => {
                    ui.label(RichText::new(tl!("The sheet is empty.")).color(t.dim));
                }
            }
        }
        Preview::Slides(slides) => {
            let w = ui.available_width().min(760.0);
            for (k, paras) in slides.iter().enumerate() {
                ui.label(RichText::new(tf!("Slide {n}", n = k + 1)).color(t.faint).small());
                egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(Margin::same(18)).show(
                    ui,
                    |ui| {
                        ui.set_width(w - 38.0);
                        ui.set_min_height((w - 38.0) * 9.0 / 16.0 * 0.6);
                        for (i, para) in paras.iter().enumerate() {
                            let font = if i == 0 { theme::bold(18.0) } else { theme::regular(14.5) };
                            ui.label(RichText::new(para).font(font).color(t.text));
                            ui.add_space(if i == 0 { 8.0 } else { 3.0 });
                        }
                    },
                );
                ui.add_space(12.0);
            }
        }
    }
}

/// Rows of cells as a ruled grid; a workbook's gets column letters and row numbers.
pub(super) fn table(t: &Tokens, ui: &mut Ui, id: egui::Id, rows: &[Vec<String>], sheet: bool) {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).show(ui, |ui| {
        egui::Grid::new(id).striped(true).spacing(vec2(14.0, 6.0)).show(ui, |ui| {
            if sheet {
                ui.label("");
                for c in 0..cols {
                    ui.label(RichText::new(column_name(c)).color(t.faint).small());
                }
                ui.end_row();
            }
            for (r, row) in rows.iter().enumerate() {
                if sheet {
                    ui.label(RichText::new((r + 1).to_string()).color(t.faint).small());
                }
                for c in 0..cols {
                    let cell = row.get(c).map_or("", String::as_str);
                    ui.label(RichText::new(widgets::shorten(cell, 120)).color(t.text));
                }
                ui.end_row();
            }
        });
    });
}

/// A spreadsheet's column name: 0 → A, 26 → AA.
pub(super) fn column_name(mut c: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(char::from(b'A' + u8::try_from(c % 26).unwrap_or(0)));
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    out.iter().rev().collect()
}
