use super::*;

/// Each account's address books, as Outlook lists contact folders (its Contacts, an
/// organisation's shared ones in public folders): a click shows that one alone.
pub(super) fn show(s: &Session, st: &mut UiState, ui: &mut Ui) {
    let t = st.tokens;
    let all: Vec<(String, mailcraft_engine::model::pim::BookInfo, String)> = s
        .accounts
        .iter()
        .filter_map(|a| s.pim.get(&a.config.id).map(|p| (a, p)))
        .flat_map(|(a, p)| p.books.iter().map(move |b| (a.config.id.clone(), b.clone(), a.config.email.clone())))
        .collect();
    if all.len() < 2 {
        if st.people_book.is_some() {
            st.people_book = None;
        }
        return;
    }
    // A book gone (an account removed): everyone again.
    if st.people_book.as_ref().is_some_and(|(a, b)| !all.iter().any(|(x, y, _)| x == a && y.id == *b)) {
        st.people_book = None;
    }
    let accounts = all.iter().map(|(a, _, _)| a).collect::<std::collections::BTreeSet<_>>().len();
    let row = |ui: &mut Ui, on: bool, label: &str, tip: Option<String>, shared: bool| {
        let height = if ui.ctx().input(|input| input.has_touch_screen()) { 44.0 } else { 26.0 };
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
        widgets::named(ui, &resp, label, true, Some(on));
        if on || resp.hovered() {
            ui.painter().rect_filled(r, 5.0, if on { t.accent_soft } else { t.hover_row });
        }
        let mirror = crate::bidi::Mirror::new(r);
        let icon = egui::Rect::from_center_size(mirror.pos(r.left_center() + vec2(14.0, 0.0)), vec2(14.0, 14.0));
        icons::paint(ui.painter(), icon, if shared { Icon::People } else { Icon::Person }, if on { t.accent } else { t.dim });
        ui.painter().with_clip_rect(r.shrink2(vec2(4.0, 0.0))).text(
            mirror.pos(r.left_center() + vec2(28.0, 0.0)),
            mirror.align(egui::Align2::LEFT_CENTER),
            label,
            theme::regular(13.0),
            t.text,
        );
        match tip {
            Some(tip) => resp.on_hover_text(tip).clicked(),
            None => resp.clicked(),
        }
    };
    ui.label(RichText::new(tl!("Address books")).font(theme::bold(12.0)).color(t.dim));
    if row(ui, st.people_book.is_none(), tl!("All contacts"), None, false) {
        st.people_book = None;
    }
    for (account, b, email) in &all {
        let on = st.people_book.as_ref().is_some_and(|(a, x)| a == account && *x == b.id);
        let name = if b.id == mailcraft_engine::model::pim::LOCAL || b.name.is_empty() || b.name == "Contacts" {
            tl!("Contacts").to_string()
        } else {
            b.name.clone()
        };
        let label = if accounts > 1 && !b.read_only { format!("{name} ({email})") } else { name };
        let tip = if b.read_only { tf!("Shared with you through {account}: read-only", account = email) } else { email.clone() };
        if row(ui, on, &label, Some(tip), b.read_only) {
            st.people_book = Some((account.clone(), b.id.clone()));
        }
    }
    ui.add_space(6.0);
    ui.separator();
}
