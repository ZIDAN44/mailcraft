//! The reading pane: attachments, their cards and their previews (pictures, PDF, video, Office files).

use super::*;

pub(super) fn kind_icon(k: AttachmentKind) -> Icon {
    match k {
        AttachmentKind::Image => Icon::Image,
        _ => Icon::File,
    }
}

/// Decode an image attachment into a thumbnail texture (cached; `None` if it isn't a picture
/// we can read). Pictures come from mail, so size is capped before decoding.
pub(super) fn preview(s: &Session, st: &mut UiState, ctx: &egui::Context, key: &MessageKey, index: usize) -> Option<egui::TextureHandle> {
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

/// A picture attachment at a size to look at (decoded once and kept by egui while it's shown).
pub(crate) fn big_picture(s: &Session, ctx: &egui::Context, key: &MessageKey, index: usize) -> Option<egui::TextureHandle> {
    let id = egui::Id::unique(("big-picture", key.to_id(), index));
    if let Some(t) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)) {
        return t;
    }
    let tex = (|| {
        let raw = s.store.as_ref()?.read_body(key)?;
        let data = mime::attachment_data(&raw, index)?;
        decode(ctx, &format!("big-{}-{index}", key.to_id()), &data, 2048)
    })();
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// The first page of a PDF attachment (cached like pictures).
pub(super) fn pdf_preview(s: &Session, st: &mut UiState, ctx: &egui::Context, key: &MessageKey, index: usize) -> Option<egui::TextureHandle> {
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
pub(super) fn video_preview(
    s: &Session,
    st: &mut UiState,
    ctx: &egui::Context,
    key: &MessageKey,
    a: &mime::AttachmentInfo,
) -> Option<egui::TextureHandle> {
    let id = (key.clone(), a.index);
    if let Some(t) = st.previews.get(&id) {
        return t.clone();
    }
    // The platform's decoder reads files, and the browser has none.
    if !st.has_files {
        return None;
    }
    let path = match st.video_files.get(&id) {
        Some(p) => p.clone(),
        None => {
            // The platform's decoder reads files: put the attachment in one.
            let written = (|| {
                let raw = s.store.as_ref()?.read_body(key)?;
                let data = mime::attachment_data(&raw, a.index)?;
                let dir = mailcraft_engine::store::scratch_dir("thumbs")?;
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
pub(crate) fn decode(ctx: &egui::Context, name: &str, data: &[u8], max: u32) -> Option<egui::TextureHandle> {
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
        AttachmentKind::Image => preview(s, st, ui.ctx(), key, a.index),
        AttachmentKind::Pdf => pdf_preview(s, st, ui.ctx(), key, a.index),
        AttachmentKind::Video => video_preview(s, st, ui.ctx(), key, a),
        _ => None,
    };
    // The Craft app for the file, when it's installed.
    let craft = mime::craft_app_for(&a.mime, &a.name).and_then(|(cmd, name)| st.craft_apps.iter().any(|(c, _)| *c == cmd).then_some((cmd, name)));
    let text_preview = mime::text_preview_kind(&a.mime, &a.name).is_some();
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
    }
    // A plain click shows it in place of the message's text, as Outlook previews attachments.
    if resp.clicked() && !on_more && !ctrl {
        st.attachment_view = Some((key.clone(), a.index));
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

/// Back to the message from a previewed attachment: the chip clicked to show it isn't left
/// selected either.
pub(super) fn close_attachment_view(st: &mut UiState) {
    st.attachment_view = None;
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
            let data = s.store.as_ref().and_then(|st| st.read_body(key)).and_then(|raw| mime::attachment_data(&raw, a.index));
            let x = std::sync::Arc::new(read_shown(a, data.as_deref()));
            ui.ctx().data_mut(|d| d.insert_temp(id, x.clone()));
            x
        }
    };
    let width = ui.available_width();
    match shown.as_ref() {
        Shown::Picture => match big_picture(s, ui.ctx(), key, a.index) {
            Some(tex) => {
                let sz = tex.size_vec2();
                let scale = (width / sz.x).min(1.0);
                ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale));
            }
            None => no_preview(&t, ui, a, acts),
        },
        Shown::Pdf => pdf_pages(s, st, ui, key, a, width, acts),
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

/// Read an attachment once for its preview.
pub(super) fn read_shown(a: &mime::AttachmentInfo, data: Option<&[u8]>) -> Shown {
    let Some(data) = data else { return Shown::Nothing };
    let kind = a.kind();
    if kind == AttachmentKind::Image {
        return Shown::Picture;
    }
    if kind == AttachmentKind::Pdf {
        return Shown::Pdf;
    }
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

/// A PDF's pages one under the other, the pane's width, each drawn as it's first needed (one
/// a frame, so the pane stays responsive).
pub(super) fn pdf_pages(s: &Session, st: &UiState, ui: &mut Ui, key: &MessageKey, a: &mime::AttachmentInfo, width: f32, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let px = (width * ui.ctx().pixels_per_point()).clamp(200.0, 2400.0) as u32;
    let count_id = egui::Id::unique(("pdf-count", key.to_id(), a.index));
    let mut rendered_one = false;
    let mut n = 0usize;
    loop {
        let count = ui.ctx().data(|d| d.get_temp::<usize>(count_id));
        if count.is_some_and(|c| n >= c) || n >= PDF_PAGES {
            break;
        }
        let pid = egui::Id::unique(("pdf-page", key.to_id(), a.index, n, px / 100));
        let tex = match ui.ctx().data(|d| d.get_temp::<Option<egui::TextureHandle>>(pid)) {
            Some(t) => t,
            None if rendered_one => {
                ui.ctx().request_repaint();
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tf!("Page {n}", n = n + 1)).color(t.dim).small());
                });
                break;
            }
            None => {
                rendered_one = true;
                let data = s.store.as_ref().and_then(|st| st.read_body(key)).and_then(|raw| mime::attachment_data(&raw, a.index));
                let page = data.and_then(|d| crate::thumbs::pdf_page(&d, n, crate::thumbs::Fit::Width(px), st.fonts.clone()));
                let tex = page.map(|(img, total)| {
                    ui.ctx().data_mut(|d| d.insert_temp(count_id, total));
                    ui.ctx().load_texture(format!("pdf-{}-{}-{n}", key.to_id(), a.index), img, egui::TextureOptions::LINEAR)
                });
                ui.ctx().data_mut(|d| d.insert_temp(pid, tex.clone()));
                tex
            }
        };
        let Some(tex) = tex else {
            if n == 0 {
                no_preview(&t, ui, a, acts);
            }
            break;
        };
        let sz = tex.size_vec2();
        let scale = width / sz.x;
        let r = ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale)).rect;
        ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
        ui.add_space(12.0);
        n += 1;
    }
    if let Some(c) = ui.ctx().data(|d| d.get_temp::<usize>(count_id)).filter(|c| *c > PDF_PAGES) {
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
