//! Activity: what MailCraft is doing in the background, what needs you, and what it said lately,
//! where the status bar said it (it's gone: Windows' own apps have none). A button at the top
//! bar's end shows while there's something: a ring round it fills as the work goes, and a dot on
//! it says something needs you. Clicked, its panel lists each piece with how far it has come and
//! what can be done about it (`panel.rs`). What needs you now also shows as a bar over the panes
//! until it's dealt with or put away (`info_bar.rs`); and work shows where it belongs too (a
//! data file's import under it in the folder pane, an account updating beside its name).

use mailcraft_engine::TransferRow;
use mailcraft_engine::model::FailureKind;
use mailcraft_engine::model::job::{TransferKind, TransferState};

use crate::*;

mod info_bar;
mod panel;

/// How long the button stays once the work is done (seconds): long enough to look at what it
/// came to, and so it doesn't blink on and off between one piece of work and the next.
const LINGER: f64 = 4.0;
/// How long work going on by itself (an account checked, an index pass for a few new messages)
/// lasts before it shows (seconds): most is over in a moment, and the button, its ring and the
/// panel's line for it came and went with each, a flicker. Work the user started shows at once.
pub(crate) const SHOW_AFTER: f64 = 2.0;
/// A pause shorter than this (seconds) is the same work going on: one pass ending as the next
/// begins doesn't start the wait again.
const GAP: f64 = 0.5;
/// How long work that showed stays once it's done (seconds), as it was last, fading as it goes:
/// it's there long enough to read, and work coming back meanwhile goes on in its place.
const STAY: f64 = 1.0;

/// When a piece of work began and was there last, and when it first showed: whether it shows
/// (a line in the Activity panel, the button's ring, the ring beside an account updating).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Held {
    since: f64,
    at: f64,
    shown: Option<f64>,
}

impl Held {
    /// This frame's, from the last (`going`: the work is there; `quiet`: going on by itself,
    /// shown once it has lasted [`SHOW_AFTER`]): `None` once it's gone, at once for a moment's
    /// work, after its [`STAY`] for work that showed.
    pub(crate) fn note(last: Option<Held>, now: f64, going: bool, quiet: bool) -> Option<Held> {
        let mut h = match last {
            Some(h) => h,
            None if going => Held { since: now, at: now, shown: None },
            None => return None,
        };
        if going {
            if h.shown.is_none() && now - h.at > GAP {
                h.since = now;
            }
            h.at = now;
            if h.shown.is_none() && (!quiet || now - h.since >= SHOW_AFTER) {
                h.shown = Some(now);
            }
        }
        let left = now - h.at;
        if h.shown.is_some() { left <= STAY } else { left <= GAP }.then_some(h)
    }

    pub(crate) fn shown(&self) -> bool {
        self.shown.is_some()
    }

    /// Whether the work was there this frame (else it's staying a moment, done).
    pub(crate) fn going(&self, now: f64) -> bool {
        self.at >= now
    }

    /// How much of it shows: fading in as it comes, out at the end of its stay.
    pub(crate) fn opacity(&self, ctx: &egui::Context, now: f64) -> f32 {
        let Some(shown) = self.shown else { return 0.0 };
        if !motion::enabled(ctx) {
            return 1.0;
        }
        let out = ((STAY - (now - self.at)) / f64::from(motion::GENTLE)).clamp(0.0, 1.0) as f32;
        motion::come(ctx, now - shown) * out
    }

    /// In how long (seconds) it may change by itself: shown once it has lasted, or fading.
    pub(crate) fn wake(&self, now: f64) -> Option<f64> {
        match self.shown {
            None => Some((self.since + SHOW_AFTER - now).max(0.0)),
            Some(_) if !self.going(now) => Some(0.0),
            Some(_) => None,
        }
    }
}

/// A piece of work as it was last, and how it's held.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    /// When it came, against the others: the panel keeps its lines in that order.
    order: u64,
    pub(crate) held: Held,
    work: Work,
}

/// The Activity panel's state.
#[derive(Clone, Debug, Default)]
pub(crate) struct ActivityUi {
    pub(crate) open: bool,
    /// Each piece of work by its key: what shows of it, and when.
    pub(crate) rows: std::collections::BTreeMap<String, Row>,
    /// The order the next piece of work that comes takes.
    next: u64,
    /// When work was shown last: the button stays [`LINGER`] after.
    pub(crate) shown_at: Option<f64>,
    /// When the button came and when the panel opened: each fades in from then.
    pub(crate) appeared: Option<f64>,
    pub(crate) opened: Option<f64>,
    /// What needs you, put away from the bar over the panes (each as it said it then: said
    /// differently, it shows again).
    pub(crate) dismissed: std::collections::BTreeSet<String>,
    /// The notices before this one are cleared from the panel's Recent.
    pub(crate) cleared: usize,
    /// Where the button is, for the panel to open under.
    pub(crate) button: Option<egui::Rect>,
    /// This frame's: worked out once, for the button, its panel and the bar over the panes.
    pub(crate) current: Activity,
}

/// How much a problem matters: in the bar over the panes, an error and a warning; something
/// on offer (an update ready to install) only in the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Severity {
    Error,
    Warning,
    Info,
}

/// What a row's button does, turned into an [`Action`] when it's clicked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Do {
    SignIn(String),
    GoOnline,
    /// An account's mail fetched again.
    TryAgain(String),
    ImportDetails(String),
    SendReceive,
    Restart,
    Compacting(String),
}

impl Do {
    pub(crate) fn action(&self) -> Action {
        match self {
            Do::SignIn(id) => Action::SignInAgain(id.clone()),
            Do::GoOnline => Action::Run("mail.workOffline", json!({"on": false})),
            Do::TryAgain(id) => Action::Run("mail.sync", json!({"account": id})),
            Do::ImportDetails(id) => Action::ImportDetails(id.clone()),
            Do::SendReceive => Action::SendReceiveWindow,
            Do::Restart => Action::RestartToUpdate,
            Do::Compacting(id) => Action::Dialog(Dialog::Compacting { account: id.clone(), closing: false, since: None }),
        }
    }
}

/// Something that needs you, or is on offer.
#[derive(Clone, Debug)]
pub(crate) struct Problem {
    /// Which, as long as it says the same (put away, it stays away until then).
    pub(crate) key: String,
    pub(crate) icon: Icon,
    pub(crate) title: String,
    pub(crate) detail: String,
    pub(crate) severity: Severity,
    /// Its button's words (English: the button translates them) and what it does.
    pub(crate) action: Option<(&'static str, Do)>,
}

/// Work under way.
#[derive(Clone, Debug)]
pub(crate) struct Work {
    pub(crate) key: String,
    pub(crate) icon: Icon,
    pub(crate) title: String,
    pub(crate) detail: String,
    /// How far, in thousandths; `None` when that isn't known (the bar slides).
    pub(crate) permille: Option<u16>,
    pub(crate) action: Option<(&'static str, Do)>,
    /// Going on by itself, not asked for: shown once it has lasted [`SHOW_AFTER`].
    pub(crate) quiet: bool,
}

/// Everything there is to show, worked out each frame from the session (its `work` what shows
/// of it, once noted: [`ActivityUi::settle`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct Activity {
    pub(crate) needs: Vec<Problem>,
    pub(crate) work: Vec<Work>,
}

impl Activity {
    /// What needs you and what's under way. `update`: the updater's state; `now`: the input's
    /// time, for the part of a compaction that goes by the time it's expected to take.
    pub(crate) fn of(s: &Session, update: &UpdateState, compact: &super::compacting::CompactUi, now: f64) -> Activity {
        let mut a = Activity::default();
        a.problems(s, update);
        a.under_way(s, update, compact, now);
        a.needs.sort_by_key(|p| p.severity);
        a
    }

    fn problems(&mut self, s: &Session, update: &UpdateState) {
        if s.prefs.work_offline {
            self.needs.push(Problem {
                key: "offline".into(),
                icon: Icon::Info,
                title: tl!("Working offline").into(),
                detail: tl!("Mail isn't sent or received until you go online").into(),
                severity: Severity::Warning,
                action: Some(("Go online", Do::GoOnline)),
            });
        }
        for a in &s.accounts {
            let id = &a.config.id;
            if needs_you(a) {
                let action = match &a.status {
                    SyncStatus::NeedsPassword { .. } | SyncStatus::Error { kind: FailureKind::MicrosoftNoPasswords, .. } => "Sign in",
                    _ => "Details",
                };
                self.needs.push(Problem {
                    key: format!("account:{id}"),
                    icon: Icon::Lock,
                    title: a.config.email.clone(),
                    detail: account_words(s, a),
                    severity: Severity::Error,
                    action: Some((action, Do::SignIn(id.clone()))),
                });
            } else if let SyncStatus::Error { error, .. } = &a.status
                && super::importing::account_status(s, &Tokens::light(), id).is_none()
            {
                let detail = if error.is_empty() { tl!("Couldn't connect").to_string() } else { i18n::error(error) };
                self.needs.push(Problem {
                    key: format!("account:{id}"),
                    icon: Icon::Warning,
                    title: a.config.email.clone(),
                    detail,
                    severity: Severity::Warning,
                    action: Some(("Try again", Do::TryAgain(id.clone()))),
                });
            }
        }
        // An import waiting on the user: stopped, paused, or held up (the drive full).
        for (account, st) in s.accounts.iter().filter_map(|a| s.import_state(&a.config.id).map(|st| (a.config.id.as_str(), st))) {
            if !st.live && !st.removing {
                let (title, ..) = super::importing::status_words(s, account, st);
                let detail = st.progress.blocked.as_ref().map(super::importing::blocked_words).unwrap_or_default();
                self.needs.push(Problem {
                    key: format!("import:{account}"),
                    icon: Icon::Warning,
                    title,
                    detail,
                    severity: Severity::Warning,
                    action: Some(("Details", Do::ImportDetails(account.to_string()))),
                });
            }
        }
        if s.send_receive.unseen_error {
            let first = s.send_receive.open().find(|r| r.transfer.state == TransferState::Failed);
            let detail = first.and_then(|r| r.transfer.error.clone()).map(|e| i18n::error(&e)).unwrap_or_default();
            self.needs.push(Problem {
                key: "send-receive".into(),
                icon: Icon::Warning,
                title: tl!("Send / Receive failed").into(),
                detail,
                severity: Severity::Error,
                action: Some(("Details", Do::SendReceive)),
            });
        }
        if let UpdateState::Ready { version, .. } = update {
            self.needs.push(Problem {
                key: "update".into(),
                icon: Icon::Download,
                title: tf!("MailCraft {version} is ready", version = version),
                detail: tl!("It installs when you close MailCraft, or now as it restarts").into(),
                severity: Severity::Info,
                action: Some(("Restart to update", Do::Restart)),
            });
        }
    }

    fn under_way(&mut self, s: &Session, update: &UpdateState, compact: &super::compacting::CompactUi, now: f64) {
        // Send / Receive, each account's sending and receiving.
        let working: Vec<&TransferRow> = s.send_receive.open().filter(|r| r.transfer.state == TransferState::Working).collect();
        for r in &working {
            let icon = match r.transfer.kind {
                TransferKind::Sending => Icon::Send,
                TransferKind::Receiving => Icon::Download,
            };
            self.work.push(Work {
                key: format!("{:?}:{}", r.transfer.kind, r.account),
                icon,
                title: super::send_receive::email(s, &r.account),
                detail: super::send_receive::going(s, r),
                permille: r.transfer.permille(),
                action: Some(("Details", Do::SendReceive)),
                quiet: true,
            });
        }
        // An account updating that Send / Receive or its data file's import doesn't already say.
        let said = |id: &str| working.iter().any(|r| r.account == id) || s.import_state(id).is_some();
        for a in s.accounts.iter().filter(|a| !said(&a.config.id)) {
            if matches!(a.status, SyncStatus::Syncing { .. } | SyncStatus::Connecting) {
                self.work.push(Work {
                    key: format!("sync:{}", a.config.id),
                    icon: Icon::Sync,
                    title: a.config.email.clone(),
                    detail: account_words(s, a),
                    permille: None,
                    action: None,
                    quiet: true,
                });
            }
        }
        if !s.outbox.is_empty() && !s.prefs.work_offline {
            self.work.push(Work {
                key: "outbox".into(),
                icon: Icon::Outbox,
                title: tf!("Outbox: {n}", n = s.outbox.len()),
                detail: tl!("Sending…").into(),
                permille: None,
                action: None,
                quiet: true,
            });
        }
        for a in &s.accounts {
            let id = &a.config.id;
            if let Some(st) = s.import_state(id).filter(|st| st.live) {
                let (title, permille, detail) = super::importing::status_words(s, id, st);
                self.work.push(Work {
                    key: format!("import:{id}"),
                    icon: Icon::File,
                    title,
                    detail,
                    permille,
                    action: Some(("Details", Do::ImportDetails(id.clone()))),
                    quiet: false,
                });
            }
        }
        if let Some((title, permille, detail)) = super::compacting::status(s, compact, now) {
            let action = super::compacting::shown(s).map(|id| ("Details", Do::Compacting(id.to_string())));
            self.work.push(Work { key: "compact".into(), icon: Icon::File, title, detail, permille, action, quiet: true });
        }
        if let Some((account, path, n)) = s.exporting_now() {
            let who = s.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
            self.work.push(Work {
                key: "export".into(),
                icon: Icon::File,
                title: tf!("Exporting {account} to {file}", account = who, file = mailcraft_engine::model::file_name(path)),
                detail: so_far(n),
                permille: None,
                action: None,
                quiet: false,
            });
        }
        if let Some((_, path)) = s.archiving_now() {
            let title = if path.is_empty() {
                tl!("Deleting old mail").to_string()
            } else {
                tf!("Archiving old mail to {file}", file = mailcraft_engine::model::file_name(path))
            };
            let detail = so_far(s.archive_so_far().unwrap_or(0));
            self.work.push(Work { key: "archive".into(), icon: Icon::Archive, title, detail, permille: None, action: None, quiet: false });
        }
        if let UpdateState::Downloading { version, done, total } = update {
            self.work.push(Work {
                key: "update".into(),
                icon: Icon::Download,
                title: tf!("Downloading MailCraft {version}…", version = version),
                detail: if *total > 0 {
                    tf!("{done} of {total}", done = widgets::human_bytes(*done), total = widgets::human_bytes(*total))
                } else {
                    String::new()
                },
                permille: mailcraft_engine::model::job::progress::permille(*done, *total),
                action: None,
                quiet: false,
            });
        }
        if let Some(work) = s.indexing_work() {
            self.work.push(Work {
                key: "index".into(),
                icon: Icon::Search,
                title: tl!("Making your mail searchable").into(),
                detail: tf!("Messages: {done} of {total}", done = work.done, total = work.total),
                permille: work.permille(mailcraft_engine::model::job::ITEM_WEIGHT),
                action: None,
                quiet: true,
            });
        }
    }

    /// How far all the work has come, in thousandths: `None` while some of it can't say (the
    /// ring then turns), or there's none.
    pub(crate) fn overall(&self) -> Option<u16> {
        let each: Option<Vec<u16>> = self.work.iter().map(|w| w.permille).collect();
        let each = each.filter(|e| !e.is_empty())?;
        let sum: u32 = each.iter().map(|p| u32::from(*p)).sum();
        u16::try_from(sum / u32::try_from(each.len()).unwrap_or(1).max(1)).ok()
    }

    /// The most a problem matters, of those there are.
    pub(crate) fn worst(&self) -> Option<Severity> {
        self.needs.iter().map(|p| p.severity).min()
    }
}

/// "120 messages so far", or nothing before the first.
fn so_far(n: usize) -> String {
    if n == 0 { String::new() } else { tf!("{n} messages so far", n = i18n::number(u64::try_from(n).unwrap_or(u64::MAX))) }
}

/// An account that waits for you to do something: sign in (again, or with Microsoft rather than
/// a password), or choose what to do about a mailbox Microsoft won't open over IMAP.
pub(crate) fn needs_you(a: &mailcraft_engine::AccountState) -> bool {
    match &a.status {
        SyncStatus::NeedsPassword { .. } => true,
        // Every kind but `Other` waits for you.
        SyncStatus::Error { kind, .. } => !kind.is_other(),
        _ => false,
    }
}

/// An account's state in words: what it needs, or what it's doing.
pub(crate) fn account_words(s: &Session, a: &mailcraft_engine::AccountState) -> String {
    // A data file being imported, or one waiting on the user.
    if let Some((_, words)) = super::importing::account_status(s, &Tokens::light(), &a.config.id) {
        return words;
    }
    match &a.status {
        SyncStatus::Error { kind: FailureKind::MicrosoftNoPasswords, .. } => tl!("Sign in with Microsoft").to_string(),
        SyncStatus::Error { kind: FailureKind::MicrosoftImapClosed, .. } => tl!("Mailbox won't open").to_string(),
        SyncStatus::Error { kind: FailureKind::UntrustedCertificate(..), .. } => tl!("Certificate not trusted yet").to_string(),
        SyncStatus::NeedsPassword { .. } => tl!("Password needed").to_string(),
        SyncStatus::Syncing { folder } => tf!("Updating {folder}…", folder = s.folder(&a.config.id, folder).map(i18n::folder).unwrap_or_default()),
        SyncStatus::Connecting => tl!("Connecting…").to_string(),
        SyncStatus::Error { error, .. } => i18n::error(error),
        _ => String::new(),
    }
}

impl MailApp {
    /// Open the Activity panel, as its button does (`--cmd ui.activity` at start-up).
    pub fn open_activity(&mut self) {
        self.ui.activity.open = true;
    }

    /// This frame's activity, worked out once: the work that shows of it (once it has lasted,
    /// and a moment after), and the button's linger.
    pub(crate) fn note_activity(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let mut a = Activity::of(&self.session, &self.ui.update, &self.ui.compact, now);
        a.work = self.ui.activity.settle(now, a.work);
        // Drawn again when work comes to show, or as it fades once done.
        if let Some(wake) = self.ui.activity.rows.values().filter_map(|r| r.held.wake(now)).reduce(f64::min) {
            ctx.request_repaint_after(std::time::Duration::try_from_secs_f64(wake).unwrap_or_default().max(progress::TURN_STEP));
        }
        // What was put away and no longer says the same is forgotten: said again, it shows.
        let now_said: std::collections::BTreeSet<String> = a.needs.iter().map(|p| format!("{}\n{}", p.key, p.detail)).collect();
        self.ui.activity.dismissed.retain(|d| now_said.contains(d));
        self.ui.activity.current = a;
    }

    /// Whether the Activity button shows: while something needs you, its panel is open, or work
    /// shows ([`ActivityUi::settle`]), and a moment after.
    pub(crate) fn activity_shown_now(&self, ui: &Ui) -> bool {
        let now = ui.input(|i| i.time);
        let a = &self.ui.activity;
        !a.current.needs.is_empty() || a.open || self.ui.import_details.is_some() || a.shown_at.is_some_and(|at| now - at < LINGER)
    }
}

impl ActivityUi {
    /// Note this frame's work, and return what shows of it, in the order it came: work the user
    /// started at once, work going on by itself once it has lasted [`SHOW_AFTER`] (a moment's,
    /// an account checked, never: in the panel it was a line that came and went, the panel
    /// saying nothing was going on between), and each a moment after it's done ([`STAY`]).
    pub(crate) fn settle(&mut self, now: f64, work: Vec<Work>) -> Vec<Work> {
        let mut going: std::collections::BTreeMap<String, Work> = work.into_iter().map(|w| (w.key.clone(), w)).collect();
        let keys: std::collections::BTreeSet<String> = self.rows.keys().chain(going.keys()).cloned().collect();
        for key in keys {
            let now_there = going.remove(&key);
            let last = self.rows.remove(&key);
            let quiet = now_there.as_ref().or(last.as_ref().map(|r| &r.work)).is_none_or(|w| w.quiet);
            let Some(held) = Held::note(last.as_ref().map(|r| r.held), now, now_there.is_some(), quiet) else { continue };
            let order = last.as_ref().map_or_else(
                || {
                    self.next += 1;
                    self.next
                },
                |r| r.order,
            );
            let Some(work) = now_there.or(last.map(|r| r.work)) else { continue };
            self.rows.insert(key, Row { order, held, work });
        }
        let mut shown: Vec<&Row> = self.rows.values().filter(|r| r.held.shown()).collect();
        shown.sort_by_key(|r| r.order);
        if !shown.is_empty() {
            self.shown_at = Some(now);
        }
        shown.into_iter().map(|r| r.work.clone()).collect()
    }

    /// How much of a line shows, fading in and out (1 for one not held, as in a test).
    pub(crate) fn opacity(&self, ctx: &egui::Context, key: &str, now: f64) -> f32 {
        self.rows.get(key).map_or(1.0, |r| r.held.opacity(ctx, now))
    }
}

/// The Activity button: its ring, how far the work has come (turning while that isn't known),
/// and a dot when something needs you. Clicked, its panel opens or closes. It fades in as it
/// comes, and its ring in and out.
pub(crate) fn button(ui: &mut Ui, t: &Tokens, a: &Activity, st: &mut ActivityUi) -> egui::Response {
    let time = ui.input(|i| i.time);
    let came = *st.appeared.get_or_insert(time);
    let shown = motion::come(ui.ctx(), time - came);
    ui.multiply_opacity(shown);
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    st.button = Some(rect);
    let summary = a.needs.first().map(|p| format!("{}: {}", p.title, p.detail)).or_else(|| a.work.first().map(|w| w.title.clone()));
    let tip = summary.map_or_else(|| tl!("Activity").to_string(), |s| format!("{}\n{s}", tl!("Activity")));
    widgets::named(ui, &resp, tl!("Activity"), true, None);
    if resp.hovered() || st.open {
        ui.painter().rect_filled(rect, f32::from(crate::sizes::CONTROL_RADIUS), widgets::bar_wash(t));
    }
    icons::paint(ui.painter(), icons::centered(rect, 13.0), Icon::Activity, t.on_bar);
    // The ring, faded in and out (work stopping for a frame doesn't blink it), its share eased
    // to each new value.
    let lit = ui.ctx().animate_bool_with_time(egui::Id::unique("activity-ring"), !a.work.is_empty(), motion::GENTLE);
    if lit > 0.0 {
        let share = a.overall().map(|p| ui.ctx().animate_value_with_time(egui::Id::unique("activity-share"), f32::from(p) / 1000.0, motion::GENTLE));
        progress::ring(ui, rect.center(), 11.0, share, t.on_bar.gamma_multiply(lit), time);
    }
    if let Some(worst) = a.worst() {
        let color = match worst {
            Severity::Error => t.danger,
            Severity::Warning => t.warn,
            Severity::Info => t.on_bar,
        };
        let at = rect.right_top() + vec2(-7.0, 7.0);
        ui.painter().circle(at, 4.0, color, egui::Stroke::new(1.5, t.bar));
    }
    let resp = resp.on_hover_text(tip);
    if resp.clicked() {
        st.open = !st.open;
    }
    resp
}
