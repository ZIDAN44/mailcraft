//! What People, Tasks and Notes list, kept between frames, and the rows of it in view.
//!
//! Each list is searched and sorted when it's first asked for after something it shows changed,
//! and read from its cache on every other frame (the side pane, the main pane and the status bar
//! each searched and sorted every contact, task or note, every frame). Only the rows in view are
//! laid out ([`rows`]). A list is made again on the next frame it's asked for after any of:
//! - a contact, task or note added, changed or deleted, here or by a sync, or an account's
//!   address books, tasks and notes going or coming in (`Session::pim_revision` moves on);
//! - the search typed; in People, the address book picked;
//! - in People's List view, the column sorted by, its direction, or the language (a column's
//!   text, a birthday's month, is what it sorts by);
//! - in Tasks, Arrange by, or All tasks / Active tasks; in Notes, their order.

use std::rc::Rc;

use egui::Ui;
use mailcraft_engine::{ListedTask, NoteRef, NotesOrder, Person, Session, TasksOrder};

use super::people::{Column, PeopleList};

/// Where an item is kept: its account (a key of `Session::pim`) and its place among that
/// account's cards, tasks or notes. Read back with `get`, so one gone since is skipped.
pub(crate) type At = (String, usize);

/// What People's list was made for: the revision, the search, the address book picked.
pub(crate) type PeopleKey = (u64, String, Option<(String, String)>, bool);

/// The lists as last made, each with what it was made for.
#[derive(Default)]
pub(crate) struct Shown {
    /// People in name order (the side list, Cards, the count).
    pub(crate) people: Option<(PeopleKey, Rc<PeopleList>)>,
    /// The List view's order: the column sorted by, Z to A, and the language's code.
    pub(crate) phone: Option<((PeopleKey, Option<Column>, bool, &'static str), Rc<[At]>)>,
    /// Tasks: the revision, the search, Arrange by, whether done ones are hidden, the task list
    /// shown alone.
    pub(crate) tasks: Option<(TasksKey, Rc<[At]>)>,
    /// Notes: the revision, the search, their order.
    pub(crate) notes: Option<((u64, String, NotesOrder), Rc<[At]>)>,
}

/// What the task list was made for ([`Shown::tasks`]).
pub(crate) type TasksKey = (u64, String, TasksOrder, bool, Option<String>);

/// Tasks as the list shows them, made again when what they were made for changed.
pub(crate) fn tasks(s: &Session, kept: &mut Shown, query: &str) -> Rc<[At]> {
    let shown_list = s.shown_task_list();
    let fresh = |k: &TasksKey| {
        k.0 == s.pim_revision && k.1 == query && k.2 == s.prefs.tasks_order && k.3 == s.prefs.tasks_hide_done && k.4.as_deref() == shown_list
    };
    match &kept.tasks {
        Some((k, list)) if fresh(k) => list.clone(),
        _ => {
            let list: Rc<[At]> = s.tasks(query).iter().map(|t| (t.account.to_string(), t.at)).collect();
            let key = (s.pim_revision, query.to_string(), s.prefs.tasks_order, s.prefs.tasks_hide_done, shown_list.map(str::to_string));
            kept.tasks = Some((key, list.clone()));
            list
        }
    }
}

/// Notes as the pane shows them, made again when what they were made for changed.
pub(crate) fn notes(s: &Session, kept: &mut Shown, query: &str) -> Rc<[At]> {
    let fresh = |k: &(u64, String, NotesOrder)| k.0 == s.pim_revision && k.1 == query && k.2 == s.prefs.notes_order;
    match &kept.notes {
        Some((k, list)) if fresh(k) => list.clone(),
        _ => {
            let list: Rc<[At]> = s.notes(query).iter().map(|n| (n.account.to_string(), n.at)).collect();
            kept.notes = Some(((s.pim_revision, query.to_string(), s.prefs.notes_order), list.clone()));
            list
        }
    }
}

/// The person kept at `at`.
pub(crate) fn person<'a>(s: &'a Session, at: &At) -> Option<Person<'a>> {
    let (account, p) = s.pim.get_key_value(&at.0)?;
    let c = p.cards.get(at.1)?;
    Some(Person { account, book: &c.book, card: &c.card, at: at.1 })
}

/// The task kept at `at`.
pub(crate) fn task<'a>(s: &'a Session, at: &At) -> Option<ListedTask<'a>> {
    let (account, p) = s.pim.get_key_value(&at.0)?;
    let t = p.tasks.get(at.1)?;
    Some(ListedTask { account, list: &t.list, task: &t.task, at: at.1 })
}

/// The note kept at `at`.
pub(crate) fn note<'a>(s: &'a Session, at: &At) -> Option<NoteRef<'a>> {
    let (account, p) = s.pim.get_key_value(&at.0)?;
    Some(NoteRef { account, note: &p.notes.get(at.1)?.note, at: at.1 })
}

/// The first of `0..n` for which `past` is false, `past` being true up to some point and false
/// after it.
fn first_not(n: usize, past: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if past(mid) { lo = mid + 1 } else { hi = mid }
    }
    lo
}

/// Lay out `n` rows down a scrolled list, drawing only those in `viewport` (the list's
/// coordinates), `keep` (a row that must be there, as the note being written on) and the last:
/// the rows before and between are space, so every row is where it would be with all of them
/// drawn, and the list as long. `top(k)` is where row `k` starts (with the spacing after the
/// rows before it), as the layout puts it; `draw` lays out row `k` (and the spacing after it).
/// The last row is drawn so the list ends as it does with every row (its trailing spacing
/// isn't part of it).
pub(crate) fn rows(
    ui: &mut Ui,
    viewport: egui::Rect,
    n: usize,
    top: impl Fn(usize) -> f32,
    keep: Option<usize>,
    mut draw: impl FnMut(&mut Ui, usize),
) {
    if n == 0 {
        return;
    }
    // From the first row that ends below the view's top to the last that starts above its bottom.
    let first = first_not(n, |k| top(k + 1) < viewport.min.y);
    let end = first_not(n, |k| top(k) <= viewport.max.y).max(first);
    let mut wanted: Vec<usize> = (first..end).collect();
    wanted.extend(keep.filter(|&k| k < n));
    wanted.push(n - 1);
    wanted.sort_unstable();
    wanted.dedup();
    let mut next = 0;
    for k in wanted {
        if k > next {
            ui.add_space(top(k) - top(next));
        }
        draw(ui, k);
        next = k + 1;
    }
}
