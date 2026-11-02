//! File › Info › Mailbox settings: the space an account's mail takes on the server and on this
//! computer, where its data file is and how much of it could be given back, compacting it now
//! and what that last came to, and emptying Deleted Items and Junk Email.

use mailcraft_engine::model::job::{CompactMode, CompactReport};

use super::*;

/// What the size kept on this computer was measured after: the last compaction's report (a new
/// one, and it's measured again).
pub(super) type Stamp = (u64, u64, u64);

fn stamp(s: &Session, account: &str) -> Option<Stamp> {
    s.compaction.last.get(account).map(|r| (r.before, r.after, r.elapsed_ms))
}

/// What the last compaction came to, in a line (`None`: nothing worth saying: one at idle that
/// found nothing to do).
pub(crate) fn report_line(r: &CompactReport) -> Option<String> {
    let size = widgets::human_bytes;
    Some(if let Some(e) = &r.error {
        tf!("Not compacted: {error}", error = e)
    } else if r.reimport {
        tl!("Too big to compact here, and the drive has no room for a copy: importing the file again makes it as small.").to_string()
    } else if r.cancelled {
        tl!("Compacting was stopped; what was done is kept.").to_string()
    } else if let Some(s) = r.too_long_s {
        tf!(
            "Left as it was: compacting it takes about {time}. Compact now does it however long it takes.",
            time = crate::app::importing::duration(s.saturating_mul(1000))
        )
    } else if (r.compacted || r.copied) && r.after < r.before {
        tf!(
            "Compacted: {before} → {after} ({saved} reclaimed) in {time}",
            before = size(r.before),
            after = size(r.after),
            saved = size(r.before - r.after),
            time = crate::app::importing::duration(r.elapsed_ms)
        )
    } else if r.mode == CompactMode::Manual {
        tl!("Nothing to give back.").to_string()
    } else {
        return None;
    })
}

/// The Mailbox settings card for account `a`.
pub(super) fn mailbox(s: &Session, t: &Tokens, has_files: bool, ui: &mut Ui, a: &AccountState, b: &mut Backstage, acts: &mut Vec<Action>) {
    let id = a.config.id.clone();
    // Mailbox: space used on the server and here, where the data file is, emptying.
    let now = stamp(s, &id);
    let cached = match b.cached.get(&id) {
        Some((bytes, at)) if *at == now => *bytes,
        _ => {
            let bytes = s.cache_size(&id);
            b.cached.insert(id.clone(), (bytes, now));
            bytes
        }
    };
    let size = widgets::human_bytes;
    card(ui, t, tl!("Mailbox settings"), Some(tl!("See how much space your mail takes, and empty Deleted Items and Junk Email.")), |ui| {
        let mut first = true;
        if !a.is_local() {
            let (line, frac) = match a.quota {
                Some((used, limit)) if limit > 0 => (
                    tf!("{free} free of {total}", free = size(limit.saturating_sub(used)), total = size(limit)),
                    Some((used as f32 / limit as f32).clamp(0.0, 1.0)),
                ),
                Some((used, _)) => (tf!("{used} used on the server", used = size(used)), None),
                None => (tl!("The server doesn't say how much space is used.").to_string(), None),
            };
            setting_row(ui, t, true, name(t, tl!("On the server")), Some(note(t, &line)), |ui| {
                if let Some(frac) = frac {
                    ui.add(egui::ProgressBar::new(frac).desired_width(200.0).fill(if frac > 0.9 { t.danger } else { t.accent }));
                }
            });
            first = false;
        }
        // Where, as Outlook shows a data file's place; and a copy as one.
        let online = a.config.keeps_online();
        let file = if has_files && !online { s.store.as_ref().map(|store| store.account_path(&id)) } else { None };
        // A data file of its own: measured once a visit, for what compacting it would give back.
        let data_file = file.is_some() && s.store.as_ref().is_some_and(|store| store.data_file(&id).is_some());
        if data_file && b.measured.insert(id.clone()) {
            acts.push(Action::Run("dataFile.health", json!({ "account": id })));
        }
        let health = s.compaction.health.get(&id).filter(|_| data_file);
        let running = s.compaction.running.contains_key(&id);
        let mut lines = vec![if online {
            tl!("Online only: none of this account's mail is kept on this computer.").to_string()
        } else {
            tf!("Kept on this computer: {size}", size = size(cached))
        }];
        if running {
            lines.push(tl!("Being compacted…").to_string());
        } else if let Some(h) = health.filter(|h| h.can_compact()) {
            lines.push(tf!("{size} can be reclaimed", size = size(h.saving())));
        }
        if let Some(line) = s.compaction.last.get(&id).filter(|_| data_file && !running).and_then(report_line) {
            lines.push(line);
        }
        if let Some(f) = &file {
            lines.push(f.display().to_string());
        }
        setting_row(ui, t, first, name(t, tl!("On this computer")), Some(note(t, &lines.join("\n"))), |ui| {
            if let Some(file) = &file {
                if ui.button(tl!("Save as an Outlook data file…")).clicked() {
                    acts.push(Action::ExportDataFile(id.clone()));
                }
                if ui.button(tl!("Open folder")).clicked() {
                    // The folder the data file is in (or the account's own folder).
                    let dir = if file.is_dir() { file.clone() } else { file.parent().map_or_else(|| file.clone(), Path::to_path_buf) };
                    acts.push(Action::OpenFolder(dir));
                }
            }
            if data_file {
                // Not while it's imported or compacted, nor when it has nothing to give back.
                let importing = s.import_state(&id).is_some_and(|st| st.live);
                let can = !running && !importing && health.is_none_or(|h| h.can_compact());
                let clicked = ui
                    .add_enabled(can, egui::Button::new(tl!("Compact now")))
                    .on_hover_text(tl!("Gives the room deleted mail took back to the drive. Your mail stays as it is."))
                    .clicked();
                if clicked {
                    acts.push(Action::Run("dataFile.compact", json!({ "account": id })));
                    acts.push(Action::Dialog(Dialog::Compacting { account: id.clone(), closing: false, since: None }));
                }
            }
        });
        setting_row(ui, t, false, name(t, tl!("Clean up")), None, |ui| {
            for (role, label) in [
                (mailcraft_engine::model::FolderRole::Junk, "Empty Junk Email…"),
                (mailcraft_engine::model::FolderRole::Trash, "Empty Deleted Items…"),
            ] {
                if let Some(f) = a.folder(role)
                    && ui.add_enabled(f.total > 0 || a.is_local(), egui::Button::new(tl!(label))).clicked()
                {
                    acts.push(Action::Dialog(Dialog::ConfirmEmpty {
                        account: id.clone(),
                        folder: f.path.clone(),
                        name: crate::i18n::folder(f),
                        for_good: true,
                    }));
                }
            }
        });
    });
}
