//! The window's state that isn't the mail's (`UiState`): what's open, typed, picked and laid out;
//! and what of it is kept between runs.

mod kept;

use crate::*;

/// Put message `id`'s editor at hand (`st.editor`), keeping the one there for later.
pub(crate) fn use_editor(st: &mut UiState, id: u64) {
    if st.editor.as_ref().is_some_and(|e| e.id == id) {
        return;
    }
    if let Some(e) = st.editor.take() {
        st.editors.insert(e.id, e);
    }
    st.editor = st.editors.remove(&id);
}

/// UI-only state. Its `Default` is a blank one: [`MailApp::new`] starts it with what the
/// platform says and the few values that aren't their type's default.
/// The sizes the panes were dragged to, in points (issue #171: the message list went back to its
/// first width at every start, as did the folder pane). `None`: never dragged.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneSizes {
    /// The folder pane's width.
    #[serde(default)]
    pub folders: Option<f32>,
    /// The message list's width beside the reading pane, and its height above it.
    #[serde(default)]
    pub list: Option<f32>,
    #[serde(default)]
    pub list_above: Option<f32>,
}

impl PaneSizes {
    /// A size as kept: whole points (a size that wavers by a fraction doesn't write the settings
    /// again), and only one that makes sense (a damaged file's isn't taken).
    pub fn kept(size: f32) -> Option<f32> {
        (size.is_finite() && (40.0..=10_000.0).contains(&size)).then(|| size.round())
    }
}

#[derive(Default)]
pub struct UiState {
    /// Where the main window, the last message window and the last compose window were.
    pub places: std::collections::BTreeMap<String, WindowPlace>,
    /// Closing was confirmed though it loses online-only accounts' unsent mail (issue #4); and
    /// the window is to close on the next frame.
    pub closing_anyway: bool,
    /// The window's GPU device is gone: nothing is drawn any more, and MailCraft closes
    /// ([`MailApp::graphics_lost`]).
    pub graphics_lost: bool,
    /// How the frames run under the guard went ([`MailApp::frame_part_ended`]).
    pub(crate) frame_guard: app::frame::FrameGuard,
    pub close_now: bool,
    /// When the close was last decided (the input's time): [`MailApp::logic`] decides it, and
    /// `ui` in the same pass doesn't again.
    pub(crate) close_checked: Option<f64>,
    /// Where the updater is, from [`Services::update_state`].
    pub update: UpdateState,
    /// The ready version whose bar was put away (Later), and the one the system was told of.
    pub(crate) update_later: Option<String>,
    pub(crate) update_told: Option<String>,
    /// MailCraft's earlier Flatpak this start brought the mail over from, still installed: a bar
    /// says how to remove it until it's dismissed.
    pub(crate) old_flatpak: Option<&'static str>,
    /// What to do with the update as MailCraft closes was answered: closing asks no more.
    pub(crate) update_answered: bool,
    /// Restart now was chosen: the new version opens once installed, if the close goes ahead.
    pub(crate) restart_wanted: bool,
    /// A question about closing is (or was) on screen; put away, the close is off.
    pub(crate) close_asked: bool,
    /// Data files compacted: when the user last did anything, and compacting as MailCraft closes.
    pub(crate) compact: app::compacting::CompactUi,
    /// Calendar reminders come due ([`reminders`]).
    pub reminders: reminders::State,
    pub tokens: Tokens,
    /// The platform has files (temporary ones for other apps): not in the browser.
    pub has_files: bool,
    /// Whether MailCraft can keep running with its window closed (the desktop app), and what
    /// becomes of the window then; looked up at every frame.
    pub keeps_running: Option<KeptOnClose>,
    /// The window was closed to the notification area and is hidden.
    pub(crate) in_tray: bool,
    /// MailCraft minimized the window itself (closed to the Dock or the taskbar, or started put
    /// away), and whether it has lost the focus since (issue #169, [`MailApp::logic`]).
    pub(crate) minimized_away: Option<bool>,
    /// Exit was chosen (the File page's, the tray icon's): the window's close ends MailCraft
    /// rather than hiding it.
    pub(crate) quit_wanted: bool,
    /// Where the data is kept and whether it can be moved; where it moves at the next start.
    pub data_folder: Option<(PathBuf, bool)>,
    pub pending_move: Option<PathBuf>,
    /// `None` follows the system.
    pub dark: Option<bool>,
    pub tab: RibbonTab,
    pub search_text: String,
    pub(crate) focus_search: bool,
    /// The ribbon's key tips (Alt): the letters typed so far.
    pub(crate) keytips: keytips::KeyTips,
    /// The search box has the keyboard focus (the Search tab stays while it does).
    pub(crate) search_focused: bool,
    /// Put the search box's cursor at the end when it next takes the focus.
    pub(crate) search_cursor_end: bool,
    /// When to search for what's typed (a moment after the last key, so typing stays quick).
    pub(crate) search_due: Option<f64>,
    /// The search box's recent searches or advanced search, when open.
    pub(crate) search_drop: SearchDrop,
    /// The time the last frame spent on the search's work.
    pub(crate) search_spent: web_time::Duration,
    /// In a test, the time each look at the clock adds while a search is stepped, in place of
    /// the clock: how much of a search a frame runs then doesn't hang on the machine's speed.
    #[cfg(test)]
    pub(crate) search_tick: Option<web_time::Duration>,
    /// The time the search running has spent on each kind of its work, all told.
    pub(crate) search_times: app::SearchTimes,
    /// Why the message being written couldn't be sent or saved, shown over it until dismissed.
    pub compose_alert: Option<reading::ComposeAlert>,
    /// A message started from another app (files sent to MailCraft, issue #96), whose window
    /// comes to the front once it opens: it opened behind the main window, which that launch had
    /// just brought forward.
    pub(crate) compose_to_front: Option<u64>,
    /// The attachment shown in place of its message's text (a click on its chip), as Outlook
    /// previews attachments in the reading pane.
    pub attachment_view: Option<(MessageKey, usize)>,
    /// The attachment previewed, as it was read to show it.
    pub(crate) attachment_shown: Option<reading::ShownAttachment>,
    /// The messages shown formatted or as plain text for themselves, not by the setting.
    pub(crate) shown_as: reading::body::ShownAs,
    /// The preview a plain click on a chip waits to show, and when: once the double-click time
    /// is out, as a double-click only opens the file (a click's preview showed behind it).
    pub attachment_click: Option<(MessageKey, usize, f64)>,
    /// The attachments selected in a message's strip (a click picks one, Ctrl+click adds or
    /// removes one, Ctrl+A takes them all), and whether the strip has the keyboard: it was the
    /// last thing clicked, so Ctrl+A selects attachments rather than mail.
    pub attachment_selected: Option<(MessageKey, Vec<usize>)>,
    pub attachments_active: bool,
    /// The pointer was over an attachment chip in the last frame drawn.
    pub attachment_hovered: bool,
    /// Where the ribbon was last drawn (a click on the Search tab's buttons takes the focus
    /// from the box, and mustn't close the tab).
    pub(crate) ribbon_rect: Option<egui::Rect>,
    /// The hidden ribbon shown for now (`Prefs::ribbon_hidden`): a tab was clicked, and no click
    /// elsewhere has put it away yet.
    pub(crate) ribbon_peek: bool,
    /// Where the top bar was last drawn, its tabs and search box (the pointer on its way from
    /// the box to the Search tab mustn't close the tab either).
    pub(crate) top_bar_rect: Option<egui::Rect>,
    /// The window was too narrow for the search box beside the tabs: it has a row of its own
    /// (measured as the bar is drawn, for the next frame).
    pub(crate) bar_two_rows: bool,
    /// The ribbon's buttons from this one on go without their labels: it didn't fit the window
    /// ([`widgets::RibbonFit`]).
    pub(crate) ribbon_icons_from: usize,
    pub dialog: Option<Dialog>,
    /// The pictures of the messages shown: attachment thumbnails, pictures in HTML, a picture or
    /// a PDF's pages at the pane's size ([`reading::Pictures`]).
    pub(crate) pictures: reading::Pictures,
    /// The open message's body on its way back from the reading lane.
    pub(crate) bodies: app::Bodies,
    /// Decoded web pictures, by address, and the frame each was last shown in (the oldest go
    /// when there are too many).
    pub(crate) web_pictures: reading::FrameLru<String, Option<egui::TextureHandle>, { reading::WEB_PICTURES }>,
    pub toast: Option<(String, bool, f64)>,
    pub(crate) seen_notices: usize,
    pub(crate) scroll_to_focus: bool,
    /// When the list's focus last moved by the keyboard: the row it's on takes the keyboard once
    /// it's drawn (it may be scrolled into view first), so a screen reader reads the message
    /// reached (issue #175).
    pub(crate) focus_row_at: Option<f64>,
    /// The message row that had the keyboard last: the list's arrows are its, while another
    /// control's are that control's.
    pub(crate) row_keyboard: Option<egui::Id>,
    /// Where the main window's panes were drawn last, for F6 (the top bar and the ribbon are
    /// `top_bar_rect` and `ribbon_rect`); and the list's one stop for Tab, its message's row,
    /// where F6 puts the keyboard in the list.
    pub(crate) panes: Vec<(Pane, egui::Rect)>,
    pub(crate) list_stop: Option<egui::Id>,
    /// The reading pane's: the open message's text, while a screen reader reads it as a document.
    pub(crate) reading_stop: Option<egui::Id>,
    /// A text field on the File page had the keyboard at the end of the last frame: Esc leaves it
    /// rather than the page (egui takes the focus away on Esc before the shortcuts see it).
    pub(crate) backstage_typing: bool,
    /// How many message rows the list shows at once: a page, for Page Up and Page Down.
    pub(crate) list_page: u64,
    /// A message source's window to bring to the front (asked for again while open).
    pub(crate) raise_source: Option<MessageKey>,
    /// The message sources shown as their header only (by key).
    pub(crate) source_headers: std::collections::HashSet<MessageKey>,
    /// The named font families the UI uses are registered (`install_fonts`).
    pub(crate) fonts_ready: bool,
    /// The messages being written that have been shown, to focus a new one's first field once.
    pub(crate) compose_shown: std::collections::HashSet<u64>,
    /// Where the message being written beside the list was drawn, and in which pass.
    pub(crate) compose_area: Option<(u64, egui::Rect)>,
    /// The editors of the messages being written other than the one acting now (`editor`).
    pub(crate) editors: HashMap<u64, rich_edit::Editor>,
    /// The message whose window a dialog for messages belongs to (Save changes, Address Book).
    pub(crate) dialog_for: Option<u64>,
    /// Check Names waits on a directory for the names of this message: checked again once the
    /// directory answers are no longer these (its `rev`).
    pub(crate) names_pending: Option<(u64, u64)>,
    /// The message editor's state (selection, undo, what's typed next).
    pub(crate) editor: Option<rich_edit::Editor>,
    /// The editors of the blocks of text of the quoted originals being changed, and the one with
    /// the cursor (its message, its place).
    pub(crate) quote_editors: reading::QuoteEditors,
    pub(crate) quote_focus: Option<(u64, usize)>,
    /// The message whose text and quoted original are all selected (Ctrl+A in either).
    pub(crate) all_selected: Option<u64>,
    /// Signatures written as HTML code, laid out for the messages being written (issue #90).
    pub(crate) signature_cards: reading::SignatureCards,
    /// The recipient box showing suggestions, the highlighted one, and the text for which Escape
    /// hid them.
    pub(crate) suggest_field: Option<dialogs::Recipients>,
    pub(crate) suggest_index: usize,
    pub(crate) suggest_dismissed: Option<String>,
    /// The recipient box a suggestion was just picked into: it keeps the cursor through the rest
    /// of that click (the release lands outside the box, which would otherwise end its focus).
    pub(crate) suggest_refocus: Option<dialogs::Recipients>,
    /// The people suggested for the text in that box, kept while it and what they're made of
    /// stay the same.
    pub(crate) suggested: Option<reading::Suggested>,
    /// Installed Craft apps (command, path), found at start-up.
    pub(crate) craft_apps: Vec<(&'static str, PathBuf)>,
    /// The interface language setting (`auto` or a language code).
    pub language: String,
    /// The fonts installed, kept to add a CJK fallback later (and for PDF thumbnails).
    pub(crate) fonts: thumbs::Fonts,
    /// A fallback font for Arabic and Hebrew (when the main font hasn't them).
    pub(crate) rtl_font: Option<Vec<u8>>,
    /// The language whose CJK font is loaded.
    pub(crate) cjk_loaded: Option<&'static str>,
    /// Shown text has Chinese characters none of the fonts has (`reading::han_missing`).
    pub(crate) han_missing: bool,
    /// Long message texts laid out by what's in view: their parts' heights, and their selection.
    pub(crate) long_text: HashMap<egui::Id, reading::LongText>,
    /// The open message with what the search on found marked (issue #173).
    pub(crate) marked: reading::Marked,
    /// What's selected of the message texts shown, and whether one has the keyboard.
    pub(crate) text_selection: reading::TextSelection,
    /// The app's name in the window title (`MailCraft`, or `MailCraft – Sample mailbox`).
    pub title_base: String,
    /// This page is a message's own window (the browser): only that message is shown.
    pub page_message: Option<MessageKey>,
    pub(crate) title_shown: String,
    /// Raise the main window on the next frame.
    pub(crate) focus_main: bool,
    /// Video attachments written out for the platform to take a frame from, and the frames, with
    /// the frame each was last asked for in: a frame made into its thumbnail isn't kept, and of
    /// those waiting to be (their message left before they came) only a few
    /// (`reading::bound_video_frames`).
    pub(crate) video_files: HashMap<(MessageKey, usize), PathBuf>,
    pub(crate) video_frames: HashMap<PathBuf, (VideoFrame, u64)>,
    /// Video frames made thumbnails, kept apart from the pictures (a few, by attachment), and
    /// the frame each was last shown in.
    pub(crate) video_thumbs: reading::FrameLru<(MessageKey, usize), Option<egui::TextureHandle>, { reading::VIDEO_THUMBS }>,
    /// The work behind them, on the pictures lane: the files written, the frames decoded.
    pub(crate) videos: reading::Videos,
    /// The File page, while it's open.
    pub backstage: Option<backstage::Backstage>,
    /// The month the calendar's small month shows (`year * 12 + month - 1`; `None`: the day on
    /// screen's).
    pub(crate) mini_month: Option<i64>,
    /// The day and week views, and the Schedule view, have been scrolled to the working day once
    /// (each has its own scroll).
    pub(crate) cal_scrolled: [bool; 3],
    pub(crate) people_query: String,
    /// The address book People shows (account, book id); `None`: all of them.
    pub(crate) people_book: Option<(String, String)>,
    /// The message list's rows and where each starts, kept while the list stays the same.
    pub(crate) list_rows: Option<list::ListRows>,
    /// How People lays contacts out (Outlook's Change View).
    pub people_view: people::PeopleView,
    /// The People list's columns and sort.
    pub people_list: people::ListSettings,
    /// The folder pane's and the message list's sizes as last dragged, kept between runs.
    pub pane_sizes: PaneSizes,
    /// The calendar's search (issue #33): what's typed, the switches, what it found.
    pub(crate) cal_search: calendar::CalSearch,
    /// How far the calendar's month grip has been dragged since it last added or took a month.
    pub(crate) months_drag: f32,
    /// Notes: the search, the note being written on (and its text; a new one's is empty), and
    /// the keyboard to give its text box.
    pub(crate) notes_query: String,
    pub(crate) note_edit: Option<(String, String)>,
    pub(crate) note_focus: bool,
    /// Tasks: the search, and the task picked (uid).
    pub(crate) tasks_query: String,
    /// The month the To-Do Bar's date navigator shows (months since year 0), moved by its
    /// arrows; `None`: this month.
    pub(crate) todo_month: Option<i64>,
    /// The day picked in the To-Do Bar's month, its events listed under it (issue #154); `None`:
    /// what's coming up.
    pub(crate) todo_day: Option<i64>,
    /// What's typed in the To-Do Bar's "Type a new task" box.
    pub(crate) todo_new: String,
    /// The line between the To-Do Bar's appointments and tasks while it's dragged: the
    /// appointments' share of the height (issue #196); kept once it's let go.
    pub(crate) todo_split: Option<f32>,
    pub(crate) task: Option<String>,
    /// The person whose card is shown (uid).
    pub(crate) person: Option<String>,
    /// What People, Tasks and Notes list, as last made (see `shown`).
    pub(crate) shown: shown::Shown,
    /// The invitation in the open message, worked out once per message and calendar change, with
    /// how it fits the user's calendar (issue #194).
    pub(crate) invite: Option<(MessageKey, u64, Option<mailcraft_engine::Invite>, Option<mailcraft_engine::InviteDay>)>,
    /// Files to drag out of the window (the app starts the system drag).
    pub(crate) drag_out: Vec<PathBuf>,
    /// The platform can drag files out.
    pub(crate) can_drag_out: bool,
    /// The account whose import's details are shown, beside the status bar.
    pub(crate) import_details: Option<String>,
    /// Send / Receive's progress window.
    pub(crate) send_receive: super::chrome::send_receive::SendReceiveUi,
}
