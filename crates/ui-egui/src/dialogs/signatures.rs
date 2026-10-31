//! Dialogs: an account's signatures (issue #11), as Outlook's Signatures and Stationery: named
//! signatures to pick, make, rename and delete, each formatted here or HTML code made elsewhere,
//! and which goes into new messages and which into replies and forwards ("(none)" for none).

use mailcraft_engine::model::signature::FIRST_SIGNATURE;
use serde_json::Value;

use super::*;

/// The signature editor: the account's signatures, the one being edited in a message editor of
/// its own (so it can be formatted, or pasted from Outlook with its look), and its defaults.
#[derive(Clone, Debug)]
pub struct SignatureForm {
    pub account: String,
    /// The account's signatures by name, as saved.
    pub names: Vec<String>,
    /// The one being edited, by its name as saved (`None`: a new one).
    pub editing: Option<String>,
    /// Its name as typed.
    pub name: String,
    pub compose: Compose,
    /// Opened from a message being written: shown over its window, and put into it.
    pub insert: bool,
    pub(crate) editor: SignatureEditor,
    /// The signature as HTML code (`None`: formatted in the editor), its pictures as
    /// `picture:N` (the pictures in [`SignatureForm::pictures`]).
    pub code: Option<String>,
    pub pictures: Vec<String>,
    /// Which goes into new messages, and which into replies and forwards (`None`: none).
    pub for_new: Option<String>,
    pub for_reply: Option<String>,
    /// Whether those were chosen here: until they are, an account's first signature is for
    /// both (as the pickers show it), after, they're as chosen ("(none)" too).
    pub picked: bool,
    /// What saving it as it was opened would send: changed from that, it's saved on switching.
    loaded: Value,
}

impl SignatureForm {
    /// The account's signature for new messages (else its first, else a new one) to edit.
    pub fn new(s: &Session, account: &str, insert: bool) -> SignatureForm {
        let config = s.account(account).map(|a| a.config.clone()).unwrap_or_default();
        // From a message: the signature in it, as Outlook's Signature menu opens on it.
        let in_message = s.compose.as_ref().filter(|c| insert && c.account == account).map(|c| c.signature_name.clone());
        let first = in_message
            .and_then(|n| config.signature_named(&n).map(|g| g.name.clone()))
            .or_else(|| config.default_signature(false).map(|g| g.name.clone()))
            .or_else(|| config.signatures.first().map(|g| g.name.clone()));
        let mut f = SignatureForm {
            account: account.to_string(),
            names: config.signatures.iter().map(|g| g.name.clone()).collect(),
            editing: None,
            name: String::new(),
            compose: Compose::default(),
            insert,
            editor: SignatureEditor(crate::rich_edit::Editor::new(crate::rich_edit::SIGNATURE_ID)),
            code: None,
            pictures: Vec::new(),
            for_new: config.signature_new.clone(),
            for_reply: config.signature_reply.clone(),
            picked: false,
            loaded: Value::Null,
        };
        f.load(s, first.as_deref());
        f
    }

    /// Edit signature `name` (`None`: a new one, given a name not taken).
    pub fn load(&mut self, s: &Session, name: Option<&str>) {
        let sig = name.and_then(|n| s.signature_of(&self.account, n));
        let (body, styles, inline) = sig.as_ref().map(mailcraft_engine::signature_parts).unwrap_or_default();
        let (code, pictures) = match sig.as_ref().map(|g| g.code.clone()).filter(|c| !c.trim().is_empty()) {
            Some(c) => {
                let (c, p) = mailcraft_engine::mime::code_without_pictures(&c);
                (Some(c), p)
            }
            None => (None, Vec::new()),
        };
        self.editing = sig.as_ref().map(|g| g.name.clone());
        self.name = match &sig {
            Some(g) => g.name.clone(),
            None => self.free_name(),
        };
        self.compose = Compose { id: crate::rich_edit::SIGNATURE_ID, account: self.account.clone(), body, styles, inline, ..Default::default() };
        self.editor = SignatureEditor(crate::rich_edit::Editor::new(crate::rich_edit::SIGNATURE_ID));
        self.code = code;
        self.pictures = pictures;
        self.loaded = self.params();
    }

    /// A name no signature has: "Signature", else "Signature 2"…
    fn free_name(&self) -> String {
        let taken = |n: &str| self.names.iter().any(|x| x.eq_ignore_ascii_case(n));
        let base = tl!(FIRST_SIGNATURE).to_string();
        if !taken(&base) {
            return base;
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or(base)
    }

    /// The signature as `account.setSignature` takes it.
    pub fn params(&self) -> Value {
        let mut v = json!({"account": self.account, "name": self.name.trim(), "was": self.editing});
        match &self.code {
            Some(code) => {
                v["signature"] = json!("");
                v["code"] = json!(mailcraft_engine::mime::code_with_pictures(code, &self.pictures));
            }
            None => {
                let text = self.compose.body.trim_end().to_string();
                let mut styles = self.compose.styles.clone();
                styles.fit(&text);
                // Its pictures written into it, so it keeps them.
                if !styles.is_plain() {
                    v["html"] =
                        json!(mailcraft_engine::mime::embed_pictures(&mailcraft_engine::mime::rich::to_html(&text, &styles), &self.compose.inline));
                }
                v["signature"] = json!(text);
            }
        }
        v
    }

    /// What keeps the one being edited from being saved: no name, or another's (saved under it,
    /// it would take that one's place).
    pub fn problem(&self) -> Option<String> {
        if mailcraft_engine::model::signature::signature_name(&self.name).is_none() {
            return Some(tl!("A signature needs a name, on one line.").to_string());
        }
        let mine = |n: &str| self.editing.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(n));
        let taken = self.names.iter().any(|n| n.eq_ignore_ascii_case(self.name.trim()) && !mine(n));
        taken.then(|| tl!("Another signature has that name.").to_string())
    }

    /// Whether there's nothing in the one being edited.
    fn is_empty(&self) -> bool {
        match &self.code {
            Some(c) => c.trim().is_empty(),
            None => self.compose.body.trim().is_empty(),
        }
    }

    /// Save the one being edited, if it changed: the list then names it as saved.
    fn save(&mut self, acts: &mut Vec<Action>) {
        let params = self.params();
        if params == self.loaded {
            return;
        }
        acts.push(Action::Run("account.setSignature", params));
        let name = self.name.trim().to_string();
        let was = self.editing.take();
        let empty = self.is_empty();
        let old = was.clone().unwrap_or_else(|| name.clone());
        for d in [&mut self.for_new, &mut self.for_reply] {
            if d.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&old)) {
                *d = if empty { None } else { Some(name.clone()) };
            }
        }
        self.names.retain(|n| !n.eq_ignore_ascii_case(&old) && !n.eq_ignore_ascii_case(&name));
        if !empty {
            if self.names.is_empty() && was.is_none() && !self.picked {
                // The account's first: for new messages and replies alike, as the engine has it.
                self.for_new = Some(name.clone());
                self.for_reply = Some(name.clone());
            }
            self.names.push(name.clone());
            self.editing = Some(name);
        }
        self.loaded = self.params();
    }

    /// To HTML code (the formatted signature's, to start from) or back to the editor (the
    /// code's text and look).
    pub fn switch(&mut self, to_code: bool) {
        use mailcraft_engine::mime;
        if to_code {
            let mut styles = self.compose.styles.clone();
            styles.fit(&self.compose.body);
            let text = self.compose.body.trim_end();
            let html = if text.is_empty() { String::new() } else { mime::embed_pictures(&mime::rich::to_html(text, &styles), &self.compose.inline) };
            let (code, pictures) = mime::code_without_pictures(&html);
            self.code = Some(code);
            self.pictures = pictures;
        } else {
            let code = mime::code_with_pictures(self.code.as_deref().unwrap_or(""), &self.pictures);
            let (body, styles, inline) = mime::signature_html(&mime::signature_code(&code));
            self.compose.body = body;
            self.compose.styles = styles;
            self.compose.inline = inline;
            self.code = None;
            self.pictures.clear();
        }
    }

    /// The code in a file (as [`mime::signature_file`] reads it), with its pictures written in.
    pub fn take_code(&mut self, html: &str) {
        let (code, pictures) = mailcraft_engine::mime::code_without_pictures(&mailcraft_engine::mime::signature_code(html));
        self.code = Some(code);
        self.pictures = pictures;
    }
}

/// The editor's state (undo, the selection); a copy starts afresh.
pub(crate) struct SignatureEditor(pub crate::rich_edit::Editor);

impl Clone for SignatureEditor {
    fn clone(&self) -> SignatureEditor {
        SignatureEditor(crate::rich_edit::Editor::new(self.0.id))
    }
}

impl std::fmt::Debug for SignatureEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SignatureEditor")
    }
}

/// A picker of signature names, "(none)" first. Whether one was picked.
fn default_picker(ui: &mut Ui, id: &str, label: String, names: &[String], value: &mut Option<String>) -> bool {
    let before = value.clone();
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 22.0], egui::Label::new(label));
        let none = tl!("(none)").to_string();
        egui::ComboBox::from_id_salt(id).width(240.0).selected_text(value.clone().unwrap_or_else(|| none.clone())).show_ui(ui, |ui| {
            ui.selectable_value(value, None, none.as_str());
            for n in names {
                ui.selectable_value(value, Some(n.clone()), n.as_str());
            }
        });
    });
    *value != before
}

pub(super) fn draw(ui: &mut Ui, f: &mut SignatureForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(520.0);
    title(ui, t, "Signatures");
    // Which one: picked from the account's (the one being edited saved first), a new one, or
    // this one deleted.
    let mut pick: Option<Option<String>> = None;
    ui.horizontal(|ui| {
        let shown = if f.editing.is_some() { f.name.clone() } else { tl!("New signature").to_string() };
        egui::ComboBox::from_id_salt("signature-pick").width(220.0).selected_text(shown).show_ui(ui, |ui| {
            for n in &f.names {
                let on = f.editing.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(n));
                if ui.selectable_label(on, n.as_str()).clicked() && !on {
                    pick = Some(Some(n.clone()));
                }
            }
        });
        if ui.button(tl!("New")).clicked() {
            pick = Some(None);
        }
        if ui.add_enabled(f.editing.is_some(), egui::Button::new(tl!("Delete"))).clicked()
            && let Some(name) = f.editing.take()
        {
            acts.push(Action::Run("account.removeSignature", json!({"account": f.account, "name": name})));
            f.names.retain(|n| !n.eq_ignore_ascii_case(&name));
            for d in [&mut f.for_new, &mut f.for_reply] {
                if d.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&name)) {
                    *d = None;
                }
            }
            let next = f.names.first().cloned();
            f.load(s, next.as_deref());
        }
    });
    // Saved first; one that can't be (its name) stays, to be put right.
    let problem = f.problem().filter(|_| !f.is_empty());
    if let Some(next) = pick.filter(|_| problem.is_none()) {
        if !f.is_empty() {
            f.save(acts);
        }
        f.load(s, next.as_deref());
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Name"));
        ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(240.0).char_limit(mailcraft_engine::model::signature::MAX_SIGNATURE_NAME));
    });
    ui.add_space(4.0);
    // Formatted here, or HTML code made elsewhere (a business card in columns).
    let was_code = f.code.is_some();
    let mut code = was_code;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut code, false, tl!("Formatted"));
        ui.selectable_value(&mut code, true, tl!("HTML code"));
    });
    if code != was_code {
        f.switch(code);
    }
    if f.code.is_some() {
        signature_code(ui, f, t, acts);
    } else {
        signature_text(ui, f, t, acts);
    }
    // Outlook's "Choose default signature", for this account.
    ui.add_space(8.0);
    let email = s.account(&f.account).map(|a| a.config.email.clone()).unwrap_or_default();
    ui.label(RichText::new(tf!("Default signatures for {email}", email = email)).strong().small());
    let mut names = f.names.clone();
    if f.editing.is_none() && !f.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(f.name.trim())) {
        names.push(f.name.trim().to_string());
    }
    // The account's first signature, not chosen for anything yet: shown for both, as it's saved.
    if !f.picked && f.names.is_empty() && f.editing.is_none() {
        let first = (!f.is_empty()).then(|| f.name.trim().to_string());
        f.for_new.clone_from(&first);
        f.for_reply = first;
    }
    f.picked |= default_picker(ui, "signature-new", tl!("New messages:").to_string(), &names, &mut f.for_new);
    f.picked |= default_picker(ui, "signature-reply", tl!("Replies/forwards:").to_string(), &names, &mut f.for_reply);
    let problem = f.problem().filter(|_| !f.is_empty());
    if let Some(p) = &problem {
        ui.label(RichText::new(p.as_str()).color(t.danger).small());
    }
    let (ok, cancel) = buttons(ui, t, if f.insert { "Save and insert" } else { "Save" }, problem.is_none());
    if ok {
        let empty = f.is_empty();
        f.save(acts);
        // A default naming one that isn't kept (emptied) is none.
        let kept = |d: &Option<String>| d.clone().filter(|n| f.names.iter().any(|x| x.eq_ignore_ascii_case(n)));
        acts.push(Action::Run("account.signatureDefaults", json!({"account": f.account, "new": kept(&f.for_new), "reply": kept(&f.for_reply)})));
        if f.insert && !empty {
            acts.push(Action::InsertSignature(Some(f.name.trim().to_string())));
        }
        return false;
    }
    !cancel
}

/// The signature formatted in its own editor, with its bar.
fn signature_text(ui: &mut Ui, f: &mut SignatureForm, t: &Tokens, acts: &mut Vec<Action>) {
    ui.label(RichText::new(tl!("Paste one from Outlook or Word and it keeps its look; Ctrl+B, Ctrl+I and Ctrl+U format it.")).color(t.dim).small());
    let SignatureForm { compose, editor, .. } = f;
    crate::rich_edit::signature_bar(ui, t, &mut editor.0, compose, acts);
    // A picture on the clipboard is pasted as a picture (as into a message).
    if ui.memory(|m| m.has_focus(crate::rich_edit::body_id(compose)))
        && ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::PasteImage(_))
                    || matches!(e, egui::Event::Key { key: egui::Key::V, pressed: true, modifiers, .. } if modifiers.command)
            })
        })
    {
        acts.push(Action::SignaturePicture { paste: true });
    }
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("signature-text").max_height(200.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_min_height(200.0);
            crate::rich_edit::body(ui, t, &mut editor.0, compose, None, acts);
            crate::rich_edit::link_dialog(ui, t, &mut editor.0, compose);
        });
    });
}

/// The signature as HTML code: the code, from a file too, saved as written.
fn signature_code(ui: &mut Ui, f: &mut SignatureForm, t: &Tokens, acts: &mut Vec<Action>) {
    ui.label(
        RichText::new(tl!("Paste the HTML of a signature made elsewhere, or open one saved as a web page (Outlook's .htm). It's sent as written; while you write, a message shows its text."))
            .color(t.dim)
            .small(),
    );
    if ui.button(tl!("From a file…")).clicked() {
        acts.push(Action::SignatureFile);
    }
    let code = f.code.get_or_insert_with(String::new);
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("signature-code").max_height(200.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_min_height(200.0);
            ui.add(egui::TextEdit::multiline(code).code_editor().desired_width(f32::INFINITY).frame(egui::Frame::NONE));
        });
    });
}
