//! The contact form: a person's name, emails, phones by kind, business, home and other
//! address, birthday, organisation and notes, as Outlook's contact window has them.

use super::*;

/// Outlook's phone fields, and the kind each keeps.
const PHONES: [(&str, &str); 5] = [("Business", "work"), ("Home", "home"), ("Mobile", "cell"), ("Fax", "fax"), ("Other", "")];
/// Outlook's addresses, and the kind each keeps.
const ADDRESSES: [(&str, &str); 3] = [("Business", "work"), ("Home", "home"), ("Other", "")];

/// A person being added or edited: Outlook's contact form (its emails, phones by kind, business,
/// home and other address, birthday).
#[derive(Clone, Debug, Default)]
pub struct ContactForm {
    pub uid: Option<String>,
    pub name: String,
    /// Email, Email 2 and Email 3.
    pub emails: [String; 3],
    /// More than three, kept as they were.
    pub more_emails: Vec<String>,
    /// Business, home, mobile, fax and other (as [`PHONES`]).
    pub phones: [String; 5],
    /// Numbers beyond one of each kind, kept as they were.
    pub more_phones: Vec<mailcraft_engine::model::vcard::Typed>,
    /// The kind of the number in the Other place, when it's one the form has no place for
    /// (`pager`): kept as it was.
    pub other_phone_kind: String,
    /// Business, home and other (as [`ADDRESSES`]).
    pub addresses: [mailcraft_engine::model::vcard::PostalAddress; 3],
    /// Addresses beyond one of each kind, kept as they were.
    pub more_addresses: Vec<mailcraft_engine::model::vcard::PostalAddress>,
    /// The kind of the address in the Other place, when it's one the form has no place for
    /// (`postal`): kept as it was.
    pub other_address_kind: String,
    /// The address shown in the form.
    pub address_shown: usize,
    /// `YYYY-MM-DD`.
    pub birthday: String,
    pub org: String,
    pub title: String,
    pub note: String,
    pub error: Option<String>,
}

impl ContactForm {
    /// A new contact with a name and address filled in (Add to contacts on someone in a message).
    pub fn from_address(a: &mailcraft_engine::model::EmailAddress) -> ContactForm {
        let mut f = ContactForm { uid: None, name: a.name.trim().to_string(), ..Default::default() };
        f.emails[0] = a.email.trim().to_string();
        f
    }

    pub fn edit(c: &mailcraft_engine::model::vcard::Card) -> ContactForm {
        let mut f = ContactForm {
            uid: Some(c.uid.clone()),
            name: c.display(),
            org: c.org.clone(),
            title: c.title.clone(),
            note: c.note.clone(),
            birthday: c.birthday.clone(),
            ..Default::default()
        };
        for (i, e) in c.emails.iter().enumerate() {
            match f.emails.get_mut(i) {
                Some(slot) => *slot = e.value.clone(),
                None => f.more_emails.push(e.value.clone()),
            }
        }
        for ph in &c.phones {
            let kind = match ph.kind.as_str() {
                "mobile" => "cell",
                k if PHONES.iter().any(|(_, x)| *x == k) => k,
                _ => "",
            };
            let slot = PHONES.iter().position(|(_, k)| *k == kind).and_then(|i| f.phones.get_mut(i)).filter(|s| s.is_empty());
            match slot {
                Some(slot) => {
                    *slot = ph.value.clone();
                    if kind.is_empty() {
                        f.other_phone_kind.clone_from(&ph.kind);
                    }
                }
                None => f.more_phones.push(ph.clone()),
            }
        }
        for a in &c.addresses {
            let known = ADDRESSES.iter().position(|(_, k)| *k == a.kind);
            let i = known.unwrap_or(2);
            match f.addresses.get_mut(i).filter(|s| s.is_empty()) {
                Some(slot) => {
                    *slot = a.clone();
                    if known.is_none() {
                        f.other_address_kind.clone_from(&a.kind);
                    }
                }
                None => f.more_addresses.push(a.clone()),
            }
        }
        f.address_shown = f.addresses.iter().position(|a| !a.is_empty()).unwrap_or(0);
        f
    }

    pub(crate) fn params(&self) -> Value {
        let emails: Vec<&str> =
            self.emails.iter().map(|e| e.trim()).chain(self.more_emails.iter().map(String::as_str)).filter(|e| !e.is_empty()).collect();
        let mut phones: Vec<Value> = PHONES
            .iter()
            .zip(&self.phones)
            .filter(|(_, v)| !v.trim().is_empty())
            .map(|((_, kind), v)| json!({"value": v.trim(), "kind": if kind.is_empty() { self.other_phone_kind.as_str() } else { kind }}))
            .collect();
        phones.extend(self.more_phones.iter().map(|x| json!({"value": x.value, "kind": x.kind})));
        let address = |kind: &str, a: &mailcraft_engine::model::vcard::PostalAddress| json!({"kind": kind, "street": a.street, "city": a.city, "region": a.region, "postcode": a.postcode, "country": a.country});
        let mut addresses: Vec<Value> = ADDRESSES
            .iter()
            .zip(&self.addresses)
            .filter(|(_, a)| !a.is_empty())
            .map(|((_, kind), a)| address(if kind.is_empty() { self.other_address_kind.as_str() } else { kind }, a))
            .collect();
        addresses.extend(self.more_addresses.iter().map(|a| address(&a.kind, a)));
        let mut p = json!({
            "name": self.name.trim(),
            "emails": emails,
            "phones": phones,
            "addresses": addresses,
            "birthday": self.birthday.trim(),
            "org": self.org.trim(),
            "title": self.title.trim(),
            "note": self.note,
        });
        if let Some(u) = &self.uid {
            p["uid"] = json!(u);
        }
        p
    }
}

pub fn contact(ui: &mut Ui, f: &mut ContactForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(560.0);
    // The person's initials once a name is typed, as the people list shows them.
    let initials: String = f.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
    let badge = if initials.is_empty() { Ok(Icon::Person) } else { Err(initials) };
    form_header(ui, t, badge, if f.uid.is_some() { "Edit contact" } else { "New contact" });
    let tall = ui.ctx().content_rect().height() * 0.62;
    egui::ScrollArea::vertical().max_height(tall.max(260.0)).auto_shrink([false, true]).show(ui, |ui| {
        // Room on the right for the scroll bar, so it doesn't run over the fields.
        ui.set_max_width((ui.available_width() - 14.0).max(120.0));
        form_row(ui, t, false, None, "Name", |ui| {
            field(ui, &mut f.name, tl!("First and last name"), f32::INFINITY);
        });
        form_row(ui, t, false, None, "Company", |ui| {
            field(ui, &mut f.org, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Job title", |ui| {
            field(ui, &mut f.title, "", f32::INFINITY);
        });
        for (i, label) in ["Email", "Email 2", "Email 3"].into_iter().enumerate() {
            if let Some(v) = f.emails.get_mut(i) {
                form_row(ui, t, false, None, label, |ui| {
                    field(ui, v, "", f32::INFINITY);
                });
            }
        }
        for (i, (label, _)) in PHONES.iter().enumerate() {
            if let Some(v) = f.phones.get_mut(i) {
                let label = match *label {
                    "Business" => "Business phone",
                    "Home" => "Home phone",
                    "Mobile" => "Mobile phone",
                    "Fax" => "Business fax",
                    _ => "Other phone",
                };
                form_row(ui, t, false, None, label, |ui| {
                    field(ui, v, "", f32::INFINITY);
                });
            }
        }
        // One address at a time, as Outlook's form: choose which.
        form_row(ui, t, false, None, "Address", |ui| {
            let shown = f.address_shown.min(2);
            let name = |i: usize| match i {
                0 => tl!("Business"),
                1 => tl!("Home"),
                _ => tl!("Other"),
            };
            let r = egui::ComboBox::from_id_salt("contact-address")
                .selected_text(name(shown))
                .show_ui(ui, |ui| {
                    for i in 0..3 {
                        let filled = f.addresses.get(i).is_some_and(|a| !a.is_empty());
                        let label = if filled { format!("{} ✓", name(i)) } else { name(i).to_string() };
                        if ui.selectable_label(shown == i, label).clicked() {
                            f.address_shown = i;
                        }
                    }
                })
                .response
                // The row's name is painted, not a widget: the box carries it.
                .accessible_name(tl!("Address"));
            let mut at = shown;
            crate::drop_down::keys(ui, &r, &mut at, &[0, 1, 2]);
            if at != shown {
                f.address_shown = at;
            }
        });
        let shown = f.address_shown.min(2);
        if let Some(a) = f.addresses.get_mut(shown) {
            form_row(ui, t, false, None, "Street", |ui| {
                area(ui, &mut a.street, "", 2);
            });
            form_row(ui, t, false, None, "Postal code", |ui| {
                field(ui, &mut a.postcode, "", 110.0);
                ui.label(RichText::new(tl!("City")).color(t.dim));
                field(ui, &mut a.city, "", f32::INFINITY);
            });
            form_row(ui, t, false, None, "State/Province", |ui| {
                field(ui, &mut a.region, "", f32::INFINITY);
            });
            form_row(ui, t, false, None, "Country/Region", |ui| {
                field(ui, &mut a.country, "", f32::INFINITY);
            });
        }
        form_row(ui, t, false, None, "Birthday", |ui| {
            field(ui, &mut f.birthday, tl!("YYYY-MM-DD"), 140.0);
        });
        form_row(ui, t, false, None, "Notes", |ui| {
            area(ui, &mut f.note, "", 3);
        });
    });
    if let Some(e) = &f.error {
        ui.label(RichText::new(e).color(t.danger));
    }
    footer_rule(ui, t);
    let mut keep = true;
    let any_email = f.emails.iter().any(|e| !e.trim().is_empty());
    // A row of its own height, as Follow Up's (issue #220).
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::accent_button(ui, crate::icons::Icon::Person, "Save", t, !f.name.trim().is_empty() || any_email).clicked() {
            f.error = None;
            acts.push(Action::Submit(if f.uid.is_some() { "contact.update" } else { "contact.create" }, f.params()));
        }
        if ui.button(tl!("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}
