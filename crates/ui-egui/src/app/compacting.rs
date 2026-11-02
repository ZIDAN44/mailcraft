//! Data files made smaller, as the window sees it: the idle policy told how long MailCraft has
//! been left alone, an idle compaction stopped as soon as the user does anything, the part that
//! has a file to itself shown over the window while it lasts, and compacting as MailCraft closes
//! when that was chosen (File › Options › Data files). The File page's row is
//! `panes/backstage/data_file.rs`; the dialogs are in `dialogs/closing.rs`.

use std::collections::{HashMap, HashSet};

use mailcraft_engine::Compacting;
use mailcraft_engine::model::job::{CompactMode, CompactPhase, CompactPlan, ITEM_WEIGHT};

use crate::*;

/// The idle policy is told how long MailCraft has been left alone this often (seconds).
pub(crate) const IDLE_TICK_S: f64 = 30.0;
/// While a data file is compacted the window looks at how it goes this often.
const FOLLOW_EVERY: std::time::Duration = std::time::Duration::from_millis(250);
/// The bar of the part that has the file to itself (which says nothing of how far it is) stops
/// here until it ends: it goes by the time it was expected to take.
const EXCLUSIVE_MAX_PERMILLE: u64 = 950;

/// What the window keeps of compacting data files.
#[derive(Debug, Default)]
pub(crate) struct CompactUi {
    /// When the user last did anything (egui's input time); none before the first frame.
    last_input: Option<f64>,
    /// When the idle policy was last told how long it's been.
    last_tick: Option<f64>,
    /// By account: when this window saw its compaction take the file to itself.
    pub(crate) exclusive_since: HashMap<String, f64>,
    /// Idle compactions already asked to stop (asked once each).
    stopped: HashSet<String>,
    /// The user did something in a window of its own (a message, its source) since the last frame:
    /// that's coming back too, though the main window saw nothing.
    pub(crate) acted_elsewhere: bool,
    /// MailCraft closes once the compactions in [`CompactUi::waiting`] are done.
    pub(crate) closing: bool,
    /// The compactions closing waits for: those it began, and any that had its file to itself.
    pub(crate) waiting: HashSet<String>,
    /// A compaction too long for closing, offered by hand once what closing waits for is done.
    pub(crate) offer: Option<(String, u64)>,
    /// What compacting on close comes to was answered for this close: it isn't asked again.
    pub(crate) answered: bool,
    /// Close now: nothing more is waited for, a part that has its file to itself included.
    pub(crate) dont_wait: bool,
}

impl CompactUi {
    /// Input in a window of its own (a message, its source), as its frame sees it.
    pub(crate) fn note_input(&mut self, ui: &Ui) {
        if ui.input(|i| i.events.iter().any(by_the_user)) {
            self.acted_elsewhere = true;
        }
    }
}

/// Something the user did (not the window being moved, focused or drawn).
fn by_the_user(e: &egui::Event) -> bool {
    matches!(
        e,
        egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::PointerButton { .. }
            | egui::Event::PointerMoved(_)
            | egui::Event::MouseWheel { .. }
            | egui::Event::Touch { .. }
            | egui::Event::Zoom(_)
    )
}

/// What the user knows the account's data file by: its file name, else the account's address.
pub(crate) fn file_name(s: &Session, account: &str) -> String {
    let file = s.store.as_ref().and_then(|st| st.data_file(account)).and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()));
    file.or_else(|| s.account(account).map(|a| a.config.email.clone())).unwrap_or_else(|| account.to_string())
}

/// Where a compaction is, in words, and how far, in thousandths (`None`: not known). The part
/// that has the file to itself says nothing of how far it is: its bar goes by the time it was
/// expected to take (`since`: when this window saw it begin; `now`: the input's time), and stops
/// short of the end until it ends.
pub(crate) fn phase_words(c: &Compacting, since: Option<f64>, now: f64) -> (String, Option<u16>) {
    match c.phase {
        CompactPhase::Measuring => (tl!("Measuring the data file…").to_string(), None),
        CompactPhase::Recompress => (tl!("Compressing mail again").to_string(), c.work.permille(ITEM_WEIGHT)),
        CompactPhase::Copy => (tl!("Copying it into a smaller file").to_string(), c.work.permille(ITEM_WEIGHT)),
        CompactPhase::Exclusive => match c.exclusive_s.filter(|s| *s > 0) {
            Some(expected) => {
                let elapsed = since.map_or(0.0, |s| (now - s).max(0.0));
                // `as` saturates: a clock gone wild only fills the bar to its stop.
                let elapsed_ms = (elapsed * 1000.0) as u64;
                let left = expected.saturating_sub(elapsed as u64).max(1);
                let permille = (elapsed_ms / expected).min(EXCLUSIVE_MAX_PERMILLE);
                (tf!("Finishing: about {n} s", n = left), u16::try_from(permille).ok())
            }
            None => (tl!("Finishing…").to_string(), None),
        },
    }
}

/// The account whose compaction the status bar shows: one having its file to itself first.
pub(crate) fn shown(s: &Session) -> Option<&str> {
    s.compaction.running.iter().min_by_key(|(a, r)| (r.phase != CompactPhase::Exclusive, a.as_str())).map(|(a, _)| a.as_str())
}

/// The compaction the status bar shows ([`shown`]), in its words: what it is, how far, and what
/// hovering it says.
pub(crate) fn status(s: &Session, c: &CompactUi, now: f64) -> Option<(String, Option<u16>, String)> {
    let account = shown(s)?;
    let running = s.compaction.running.get(account)?;
    let file = file_name(s, account);
    let (_, permille) = phase_words(running, c.exclusive_since.get(account).copied(), now);
    let text = match permille {
        Some(p) => tf!("Compacting {file}… {percent}%", file = file, percent = progress::percent(p)),
        None => tf!("Compacting {file}…", file = file),
    };
    Some((text, permille, tl!("Making the data file smaller: the room deleted mail took goes back to the drive.").to_string()))
}

impl MailApp {
    /// Every frame: the idle policy told how long it's been since the user did anything (every
    /// [`IDLE_TICK_S`]), an idle compaction stopped when they do something, the part that has a
    /// file to itself kept on screen, and MailCraft closed once what closing waited for is done.
    pub(crate) fn follow_compaction(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let running = &self.session.compaction.running;
        let c = &mut self.ui.compact;
        let acted = ctx.input(|i| i.events.iter().any(by_the_user)) || std::mem::take(&mut c.acted_elsewhere);
        c.exclusive_since.retain(|a, _| running.get(a).is_some_and(|r| r.phase == CompactPhase::Exclusive));
        for (a, _) in running.iter().filter(|(_, r)| r.phase == CompactPhase::Exclusive) {
            c.exclusive_since.entry(a.clone()).or_insert(now);
        }
        c.stopped.retain(|a| running.contains_key(a));
        // Anything the user does stops a compaction begun because MailCraft was left alone,
        // where it can be stopped (what was compressed again is kept, and goes on another time).
        let mut stop = Vec::new();
        if acted {
            c.last_input = Some(now);
            for (a, r) in running {
                if r.mode == CompactMode::Idle && r.phase != CompactPhase::Exclusive && c.stopped.insert(a.clone()) {
                    stop.push(a.clone());
                }
            }
        }
        let since = *c.last_input.get_or_insert(now);
        let tick = self.ui.has_files && c.last_tick.is_none_or(|t| now - t >= IDLE_TICK_S || now < t);
        if tick {
            c.last_tick = Some(now);
        }
        for a in stop {
            self.run("dataFile.cancelCompact", json!({ "account": a }));
        }
        if tick {
            // `as` saturates: a clock gone backwards is no time at all.
            self.run("dataFile.idle", json!({ "seconds": (now - since).max(0.0) as u64 }));
        }
        self.show_compacting();
        if !self.session.compaction.running.is_empty() || self.ui.compact.closing {
            ctx.request_repaint_after(FOLLOW_EVERY);
        }
    }

    /// The dialog over the window: closing put away (the close is off), the part that has a
    /// file to itself always shown (nothing of its account can be done meanwhile), and closing
    /// once what it waited for is done.
    fn show_compacting(&mut self) {
        let c = &mut self.ui.compact;
        let running = &self.session.compaction.running;
        c.waiting.retain(|a| running.contains_key(a));
        let shown = match &mut self.ui.dialog {
            Some(Dialog::Compacting { account, closing, since }) => {
                // Closing waits for more than one: the dialog goes on to the next as each ends
                // (one having its file to itself first), rather than ending with the first.
                if *closing
                    && !running.contains_key(account.as_str())
                    && let Some(next) = c.waiting.iter().min_by_key(|a| (!c.exclusive_since.contains_key(a.as_str()), a.as_str()))
                {
                    *account = next.clone();
                }
                *since = c.exclusive_since.get(account.as_str()).copied();
                *closing
            }
            _ => false,
        };
        if c.closing && !shown && !matches!(self.ui.dialog, Some(Dialog::CompactOnClose { .. })) {
            c.closing = false;
            c.waiting.clear();
        }
        if c.closing && c.waiting.is_empty() {
            // Done: MailCraft closes as it was asked to (what else closing says is still said).
            c.closing = false;
            c.answered = true;
            self.ui.dialog = None;
            self.act(Action::CloseAnyway);
            return;
        }
        if self.ui.dialog.is_none()
            && let Some((a, since)) = self.ui.compact.exclusive_since.iter().min_by(|x, y| x.0.cmp(y.0))
        {
            self.ui.dialog = Some(Dialog::Compacting { account: a.clone(), closing: false, since: Some(*since) });
            self.ui.dialog_for = None;
        }
    }

    /// Closing: data files are compacted first when that was chosen (File › Options), one has
    /// room to give back, and it's short; one that would take longer is offered by hand, with
    /// how long it takes. One that has its file to itself now can't be stopped: closing waits
    /// for it (killed, the file is checked as MailCraft next opens). A long one is offered once
    /// what closing waits for is done. Whether it asked.
    pub(crate) fn ask_compact_on_close(&mut self) -> bool {
        if self.ui.compact.dont_wait {
            return false;
        }
        let exclusive = self.session.compaction.running.iter().filter(|(_, r)| r.phase == CompactPhase::Exclusive).map(|(a, _)| a.clone());
        let mut waits: HashSet<String> = exclusive.collect();
        if !self.ui.compact.answered {
            self.ui.compact.answered = true;
            let plans = self.session.close_compactions();
            for (a, p) in &plans {
                match p {
                    CompactPlan::InPlace { .. } => {
                        self.run("dataFile.compact", json!({ "account": a, "mode": "close" }));
                        if self.session.compaction.running.contains_key(a) {
                            waits.insert(a.clone());
                        }
                    }
                    CompactPlan::TooLong { seconds } if self.ui.compact.offer.is_none() => self.ui.compact.offer = Some((a.clone(), *seconds)),
                    _ => {}
                }
            }
        }
        // One having its file to itself shown first.
        let first = waits
            .iter()
            .min_by_key(|a| (self.session.compaction.running.get(a.as_str()).is_none_or(|r| r.phase != CompactPhase::Exclusive), a.as_str()))
            .cloned();
        self.ui.dialog = if let Some(account) = first {
            self.ui.compact.closing = true;
            self.ui.compact.waiting = waits;
            let since = self.ui.compact.exclusive_since.get(&account).copied();
            Some(Dialog::Compacting { account, closing: true, since })
        } else if let Some((account, seconds)) = self.ui.compact.offer.take() {
            Some(Dialog::CompactOnClose { account, seconds })
        } else {
            // Nothing asked: a later close asks again.
            self.ui.compact.answered = false;
            return false;
        };
        self.ui.dialog_for = None;
        self.ui.close_asked = true;
        true
    }

    /// The answer to compacting as MailCraft closes: compact it by hand and close once that's
    /// done, or close now (a recompress or copy under way stops, what's done kept).
    pub(crate) fn compact_on_close(&mut self, compact: bool) {
        self.ui.compact.answered = true;
        let account = match &self.ui.dialog {
            Some(Dialog::CompactOnClose { account, .. }) => Some(account.clone()),
            _ => None,
        };
        self.ui.dialog = None;
        if let Some(account) = account.filter(|_| compact) {
            self.run("dataFile.compact", json!({ "account": account }));
            if self.session.compaction.running.contains_key(&account) {
                self.ui.compact.closing = true;
                self.ui.compact.waiting.insert(account.clone());
                let since = self.ui.compact.exclusive_since.get(&account).copied();
                self.ui.dialog = Some(Dialog::Compacting { account, closing: true, since });
                self.ui.close_asked = true;
                return;
            }
        } else if !compact {
            // Close now (or without compacting): nothing more is waited for or offered.
            self.ui.compact.dont_wait = true;
        }
        self.ui.compact.closing = false;
        self.ui.compact.waiting.clear();
        self.ui.compact.offer = None;
        self.act(Action::CloseAnyway);
    }
}
