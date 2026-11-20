//! The folder pane: Favorites, then each account's folders as a tree with unread counts, and
//! after it the shared mailboxes it opens, each a mailbox of its own as Outlook shows them.
//! Messages dragged from the list drop onto a folder to move them; a folder dragged onto another
//! moves inside it; an account dragged onto another takes its place. Right-click a folder to
//! create, rename, move, delete or pin it to Favorites. Favorites, accounts and folders with
//! folders in them collapse, and stay so from one start to the next.

use egui::{RichText, Sense, Ui, vec2};
use mailcraft_engine::model::{Folder, FolderRole};
use mailcraft_engine::{Session, View};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::list::DragKeys;
use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

mod folder_rows;
mod mailboxes;
mod rows;
use folder_rows::{dropped_mail, folder_row};
use mailboxes::{move_items, section_drag, shared_section};
use rows::{CORNER, Count, RowIs, corner_inset, corner_row, corner_top, row, section_header};

fn role_icon(role: FolderRole) -> Icon {
    match role {
        FolderRole::Inbox => Icon::Inbox,
        FolderRole::Drafts => Icon::Drafts,
        FolderRole::Sent => Icon::Sent,
        FolderRole::Archive => Icon::Archive,
        FolderRole::Junk => Icon::Junk,
        FolderRole::Trash => Icon::Delete,
        FolderRole::Other => Icon::Folder,
    }
}

/// What a dragged folder carries.
pub struct DragFolder {
    pub account: String,
    pub path: String,
}

/// What a dragged account carries (its id).
pub struct DragAccount(pub String);

/// What of kind `P` was dropped on `resp` this frame. egui takes what's carried before it looks
/// at its kind (`DragAndDrop::take_payload`), so asking for another kind first threw a dropped
/// folder away (issue #181): it's taken only when it's this kind.
fn dropped<P: std::any::Any + Send + Sync>(resp: &egui::Response) -> Option<std::sync::Arc<P>> {
    if egui::DragAndDrop::has_payload_of_type::<P>(&resp.ctx) { resp.dnd_release_payload::<P>() } else { None }
}

/// The key a folder is collapsed under (see `Prefs::collapsed`).
fn folder_key(account: &str, path: &str) -> String {
    format!("folder:{account}/{path}")
}

/// The folders a folder is inside (`A/B/C` → `A`, `A/B`), as the starts of its own path.
fn ancestors(f: &Folder) -> impl Iterator<Item = &str> {
    let d = f.delimiter.unwrap_or('/');
    f.path.char_indices().filter(move |&(_, c)| c == d).filter_map(|(i, _)| f.path.get(..i))
}

/// An account's folder tree as the pane shows it this frame: which folders have folders in them
/// and which of those are collapsed, read from slices of the folders' paths (no string is made
/// for a folder or its ancestors).
struct Tree<'a> {
    /// Every folder some folder is inside.
    parents: std::collections::HashSet<&'a str>,
    /// The account's folders collapsed (`Prefs::collapsed`'s `folder:{account}/{path}`).
    collapsed: std::collections::HashSet<&'a str>,
}

impl<'a> Tree<'a> {
    fn new(s: &'a Session, a: &'a mailcraft_engine::AccountState) -> Self {
        let id = a.config.id.as_str();
        let collapsed = s
            .prefs
            .collapsed
            .iter()
            .filter_map(|k| k.strip_prefix("folder:").and_then(|k| k.strip_prefix(id)).and_then(|k| k.strip_prefix('/')))
            .collect();
        Tree { parents: a.folders.iter().flat_map(ancestors).collect(), collapsed }
    }

    /// Whether a folder is inside a collapsed one.
    fn hidden(&self, f: &Folder) -> bool {
        !self.collapsed.is_empty() && ancestors(f).any(|up| self.collapsed.contains(up))
    }

    /// For a folder with folders in it, whether it's collapsed.
    fn collapse(&self, f: &Folder) -> Option<bool> {
        self.parents.contains(f.path.as_str()).then(|| self.collapsed.contains(f.path.as_str()))
    }
}

/// The folder to open for a file: the one it's in (itself, when it's a folder).
pub(crate) fn folder_of(file: &std::path::Path) -> std::path::PathBuf {
    if file.is_dir() { file.to_path_buf() } else { file.parent().unwrap_or(file).to_path_buf() }
}

/// Whether more than one account has mail: one with calendars and contacts only has none to
/// gather with another's, nor a folder to tell from another's (issue #86).
fn several_mailboxes(s: &Session) -> bool {
    s.accounts.iter().filter(|a| !a.config.is_calendar_only()).count() > 1
}

/// Where the folder pane keeps its current folder's row for the frame (F6 puts the keyboard there).
const CURRENT_ROW: &str = "folder pane's current row";

/// The row of the folder shown, drawn last frame: where F6 puts the keyboard in the folder pane,
/// as in Outlook (issue #175).
pub(crate) fn current_row(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|d| d.get_temp(egui::Id::unique(CURRENT_ROW)))
}

/// The folder pane; `minimize`, with Minimize at the end of its first row.
pub fn show(s: &Session, t: &Tokens, ui: &mut Ui, minimize: bool, acts: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();
    // The keyboard on the folder shown, last frame.
    let was = current_row(&ctx).filter(|id| ctx.memory(|m| m.focused()) == Some(*id));
    ctx.data_mut(|d| d.remove::<egui::Id>(egui::Id::unique(CURRENT_ROW)));
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        rows::set_density(ui, s.prefs.folder_density);
        let top = ui.cursor().min.y;
        ui.data_mut(|d| {
            d.remove::<egui::Rect>(corner_row());
            if minimize {
                d.insert_temp(corner_top(), top);
            } else {
                d.remove::<f32>(corner_top());
            }
        });
        // Hidden altogether (View › Favorites), or collapsed to its heading.
        let favorites_open = s.prefs.show_favorites && !s.prefs.collapsed.contains("favorites");
        if s.prefs.show_favorites && section_header(ui, t, "Favorites", favorites_open).clicked() {
            acts.push(Action::Run("view.collapse", json!({"key": "favorites"})));
        }
        if favorites_open && several_mailboxes(s) {
            let unread: u32 = s.accounts.iter().filter_map(|a| a.folder(FolderRole::Inbox)).map(|f| f.unread).sum();
            let sel = s.view == View::Unified { role: FolderRole::Inbox } && s.search.is_none();
            if row(ui, t, Icon::Inbox, tl!("All Inboxes"), Count::Unread(unread), 0, RowIs::selected(sel)).clicked() {
                acts.push(Action::Run("folder.unified", json!({"role": "inbox"})));
            }
            let sel = s.view == View::Unified { role: FolderRole::Sent } && s.search.is_none();
            if row(ui, t, Icon::Sent, tl!("All Sent"), Count::Unread(0), 0, RowIs::selected(sel)).clicked() {
                acts.push(Action::Run("folder.unified", json!({"role": "sent"})));
            }
        }
        for fav in s.favorites().into_iter().filter(|_| favorites_open) {
            if let Some(f) = s.folder(&fav.account, &fav.folder) {
                folder_row(s, t, ui, acts, f, 0, true, None);
            }
        }
        if !s.outbox.is_empty() {
            let sel = s.view == View::Outbox && s.search.is_none();
            let n = u32::try_from(s.outbox.len()).unwrap_or(u32::MAX);
            if row(ui, t, Icon::Outbox, tl!("Outbox"), Count::Unread(n), 0, RowIs::selected(sel))
                .on_hover_text(tl!("Messages waiting to be sent"))
                .clicked()
            {
                acts.push(Action::Run("folder.outbox", json!({})));
            }
        }
        // Apart from what's above, when there's anything (issue #188: room above the first
        // account too).
        if ui.cursor().min.y > top {
            ui.add_space(8.0);
        }
        // Accounts, and the shared mailboxes they open, each a mailbox of its own in the order
        // the user put them (issue #1).
        // (An account with calendars and contacts only has no mailbox to show, issue #86.)
        let sections = s.mail_sections();
        if sections.is_empty() && !s.accounts.is_empty() {
            // Nothing but such accounts: what they have is a click away.
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let what = ui.add(egui::Label::new(RichText::new(tl!("Calendars and contacts")).color(t.faint).small()).sense(Sense::click()));
                if what.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    acts.push(Action::Run("view.module", json!({"name": "calendar"})));
                }
            });
        }
        for (at, sec) in sections.iter().enumerate() {
            let Some(a) = s.account(&sec.account) else { continue };
            if let Some(top) = sec.mailbox.as_ref().and_then(|p| a.folders.iter().find(|f| &f.path == p)) {
                shared_section(s, t, ui, acts, a, top, &sections, at);
                continue;
            }
            let key = format!("account:{}", a.config.id);
            let open = !s.prefs.collapsed.contains(&key);
            // Dragged onto another mailbox, it takes its place (above or below it).
            let size = rows::sizes(ui);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), size.row), Sense::click_and_drag());
            let inset = corner_inset(ui, rect);
            if resp.hovered() || resp.dragged() {
                ui.painter().rect_filled(rect, 6.0, t.hover_row);
            }
            section_drag(ui, t, &resp, rect, &sections, at, acts);
            let m = crate::bidi::Mirror::new(rect);
            let chev = m.rect(egui::Rect::from_min_size(egui::pos2(rect.min.x + 6.0, rect.center().y - 6.0), vec2(12.0, 12.0)));
            icons::paint(ui.painter(), chev, if open { Icon::ChevronDown } else { m.closed() }, t.dim);
            ui.painter().circle_filled(m.pos(egui::pos2(rect.min.x + 26.0, rect.center().y)), 4.0, theme::account_color(a.config.color));
            let label = if a.is_local() && !a.is_data_file() { tf!("{email} (sample)", email = a.config.email) } else { a.config.email.clone() };
            crate::widgets::named(ui, &resp, &label, true, Some(open));
            let g = crate::widgets::one_line(ui, &label, theme::bold(size.heading), t.text, rect.width() - 44.0 - inset);
            let resp = if g.elided { resp.on_hover_text(&label) } else { resp };
            ui.painter().galley(egui::pos2(m.start(rect.min.x + 36.0, g.size().x), rect.center().y - g.size().y / 2.0), g, t.text);
            if resp.clicked() {
                acts.push(Action::Run("view.collapse", json!({"key": key})));
            }
            let config = &a.config;
            resp.context_menu(|ui| {
                // This account's mail (issue #104: it was every account's).
                if ui.button(tl!("Send / Receive")).clicked() {
                    acts.push(Action::Run("mail.sync", json!({"account": config.id})));
                    ui.close();
                }
                // A Microsoft account with a password (refused there) switches to Microsoft sign-in.
                let switch = mailcraft_engine::model::microsoft_password_account(config) && s.oauth_available.iter().any(|p| p == "microsoft");
                if a.config.has_server() && ui.button(if switch { tl!("Sign in with Microsoft…") } else { tl!("Sign in again…") }).clicked() {
                    acts.push(Action::SignInAgain(config.id.clone()));
                    ui.close();
                }
                // The account's data file, as Outlook's Open File Location; its settings.
                if let Some(file) = s.store.as_ref().map(|store| store.account_path(&config.id)).filter(|p| p.exists())
                    && ui.button(tl!("Open file location")).clicked()
                {
                    acts.push(Action::OpenFolder(folder_of(&file)));
                    ui.close();
                }
                if !a.is_local() && ui.button(tl!("Account settings…")).clicked() {
                    acts.push(Action::Dialog(Dialog::edit_account(config)));
                    ui.close();
                }
                if ui.button(tl!("Signature…")).clicked() {
                    acts.push(Action::EditSignature { account: config.id.clone(), insert: false });
                    ui.close();
                }
                // Exchange: another mailbox opened beside this one, as Outlook's Open these
                // additional mailboxes.
                if config.ews.is_some() && ui.button(tl!("Add shared mailbox…")).clicked() {
                    acts.push(Action::Dialog(Dialog::AddSharedMailbox { account: config.id.clone(), address: String::new(), error: None }));
                    ui.close();
                }
                if ui.button(tl!("New folder…")).clicked() {
                    // In the folder open now (as Outlook's New Folder starts), else at the top.
                    let here = match &s.view {
                        View::Folder { account, folder } if *account == config.id => Some(folder.as_str()),
                        _ => None,
                    };
                    acts.push(Action::Dialog(Dialog::new_folder(&config.id, here)));
                    ui.close();
                }
                // Its folders back in A to Z order (issue #183), as Outlook's Show All Folders A
                // to Z, once they're in an order of the user's own.
                if ui.add_enabled(s.folders_ordered(&config.id), egui::Button::new(tl!("Sort folders A to Z"))).clicked() {
                    acts.push(Action::Run("folder.sortAZ", json!({"account": config.id})));
                    ui.close();
                }
                // Its data file's import: stop it, go on with it, or what it came to.
                crate::app::importing::menu(ui, s, &config.id, acts);
                // Its place among the mailboxes, as Outlook moves a mailbox up or down.
                move_items(ui, &sections, at, acts);
                ui.separator();
                if ui.button(RichText::new(tl!("Remove account…")).color(t.danger)).clicked() {
                    acts.push(Action::Dialog(Dialog::RemoveAccount { account: config.id.clone(), email: config.email.clone() }));
                    ui.close();
                }
            });
            if open {
                // Folders with folders in them collapse; what's inside a collapsed one is hidden.
                let tree = Tree::new(s, a);
                // A data file being imported says how far it is; its folders come as each is done.
                let importing = crate::app::importing::nav_line(s, t, ui, acts, &a.config.id);
                if a.folders.is_empty() && !importing {
                    ui.label(RichText::new(tl!("  Loading folders…")).color(t.faint).small());
                }
                // In the user's order, when they put them in one (issue #183).
                for f in s.pane_folders(a).into_iter().filter(|f| f.mailbox.is_none()) {
                    if tree.hidden(f) {
                        continue;
                    }
                    folder_row(s, t, ui, acts, f, f.depth() + 1, false, tree.collapse(f));
                }
                // The folders its import will bring in, not in yet.
                crate::app::importing::outline(s, t, ui, &a.config.id, &a.folders);
                ui.add_space(6.0);
            }
        }
        search_folders(s, t, ui, acts);
        // Minimize at the end of the first row, drawn last so it takes the click over it; at
        // the top when nothing is there.
        if minimize {
            let first = ui.data_mut(|d| {
                d.remove::<f32>(corner_top());
                let r = d.get_temp::<egui::Rect>(corner_row());
                d.remove::<egui::Rect>(corner_row());
                r
            });
            let row = first.unwrap_or_else(|| egui::Rect::from_min_size(egui::pos2(ui.min_rect().left(), top), vec2(ui.available_width(), 26.0)));
            let m = crate::bidi::Mirror::new(row);
            let at = m.rect(egui::Rect::from_min_size(egui::pos2(row.max.x - CORNER + 2.0, row.center().y - 11.0), vec2(22.0, 22.0)));
            let mut corner = ui.new_child(egui::UiBuilder::new().max_rect(at).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let chevron = if crate::i18n::current().rtl() { Icon::ChevronRight } else { Icon::ChevronLeft };
            if crate::widgets::tiny_icon_button(&mut corner, chevron, "Minimize the folder pane", t).clicked() {
                acts.push(Action::Run("view.folderPane", json!({"minimized": true})));
            }
        }
    });
    // Its row made again under another id (the window laid out anew: Ctrl+F1 hiding the
    // ribbon), the keyboard stays on it; it went nowhere, and a screen reader with it.
    if was.is_some()
        && let Some(now) = current_row(&ctx).filter(|now| Some(*now) != was)
    {
        ctx.memory_mut(|m| m.request_focus(now));
    }
}

/// Search Folders, after the mailboxes: each a saved search, opened with a click; New Search
/// Folder from the heading's menu (or the row when there are none).
fn search_folders(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.accounts.is_empty() {
        return;
    }
    let open = !s.prefs.collapsed.contains("searchFolders");
    ui.add_space(2.0);
    let new = |ui: &mut Ui, acts: &mut Vec<Action>| {
        if ui.button(tl!("New Search Folder…")).clicked() {
            acts.push(Action::Dialog(Dialog::SearchFolder(Box::new(crate::dialogs::SearchFolderForm::new()))));
            ui.close();
        }
    };
    let resp = section_header(ui, t, "Search Folders", open);
    if resp.clicked() {
        acts.push(Action::Run("view.collapse", json!({"key": "searchFolders"})));
    }
    resp.context_menu(|ui| new(ui, acts));
    if !open {
        return;
    }
    if s.prefs.search_folders.is_empty() {
        if row(ui, t, Icon::Plus, tl!("New Search Folder…"), Count::Unread(0), 0, RowIs::default()).clicked() {
            acts.push(Action::Dialog(Dialog::SearchFolder(Box::new(crate::dialogs::SearchFolderForm::new()))));
        }
        return;
    }
    let showing = s.open_search_folder_name();
    for f in &s.prefs.search_folders {
        let selected = showing == Some(f.name.as_str());
        let r = row(ui, t, Icon::Search, &f.name, Count::Unread(0), 0, RowIs::selected(selected)).on_hover_text(&f.query);
        if r.clicked() {
            acts.push(Action::Run("searchFolder.open", json!({"name": f.name})));
        }
        r.context_menu(|ui| {
            if ui.button(tl!("Customize this Search Folder…")).clicked() {
                acts.push(Action::Dialog(Dialog::SearchFolder(Box::new(crate::dialogs::SearchFolderForm::edit(f)))));
                ui.close();
            }
            new(ui, acts);
            ui.separator();
            // Only the search goes; its mail stays where it is, so no question is asked.
            if ui.button(RichText::new(tl!("Delete Search Folder")).color(t.danger)).clicked() {
                acts.push(Action::Run("searchFolder.delete", json!({"name": f.name})));
                ui.close();
            }
        });
    }
}
