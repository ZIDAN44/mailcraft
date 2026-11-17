//! What the window needs from where it runs: the network side (`Backend`) and the platform's services.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mailcraft_engine::model::{Event, Job};

use crate::VideoFrame;

/// The network side: the desktop app plugs in `mailcraft-net`'s worker pool.
pub trait Backend {
    fn submit(&mut self, job: Job);
    fn poll(&mut self) -> Vec<Event>;
    /// MailCraft is closing: work that writes to the data files (an import) is paused, and
    /// waited for a few seconds at most.
    fn shutdown(&mut self) {}
    /// Give recent offline changes a chance to reach the cache before the app is suspended.
    fn flush_changes(&mut self, _timeout: std::time::Duration) -> bool {
        true
    }
}

/// No network (the sample mailbox, tests).
pub struct NullBackend;

impl Backend for NullBackend {
    fn submit(&mut self, _job: Job) {}
    fn poll(&mut self) -> Vec<Event> {
        Vec::new()
    }
}

/// Where a build that updates itself is with the next version (`apps/mailcraft/src/self_update.rs`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateState {
    #[default]
    None,
    /// Being downloaded: the version (`0.25.0`), the bytes so far and the size (0 if unknown).
    Downloading { version: String, done: u64, total: u64 },
    /// Downloaded and checked: installed as MailCraft closes; `asks` says what the system asks
    /// on the way.
    Ready { version: String, asks: InstallAsks },
}

/// Where to go when the user opens an app notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationTarget {
    Inbox,
    Message { key: String },
    Reminder { account: String, uid: String, occurrence: i64, due: i64, task: bool },
    Calendar,
    Tasks,
}

/// What the system asks as an update installs, for the window to say so first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InstallAsks {
    /// Nothing: it replaces this copy by itself.
    #[default]
    Nothing,
    /// Windows asks for an administrator (the MSI installs for everyone).
    Administrator,
    /// The system asks for a password (a `.deb` or `.rpm` installs for everyone).
    Password,
    /// The system's software installer opens with it (a Flatpak installs through it).
    Installer,
}

impl InstallAsks {
    /// What to say of it, if anything.
    pub fn say(self) -> Option<&'static str> {
        match self {
            InstallAsks::Nothing => None,
            InstallAsks::Administrator => Some(tl!("Windows asks for an administrator's permission.")),
            InstallAsks::Password => Some(tl!("The system asks for your password to install it.")),
            InstallAsks::Installer => Some(tl!("Your software installer opens to install it.")),
        }
    }
}

impl UpdateState {
    /// The version waiting to be installed.
    pub fn ready(&self) -> Option<&str> {
        match self {
            UpdateState::Ready { version, .. } => Some(version),
            _ => None,
        }
    }
}

/// Where slow work runs without holding up the window ([`Services::background`]): each lane
/// does its work in order, one at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lane {
    /// Decoding a message's pictures and PDF pages.
    Pictures,
    /// Getting a message ready to read.
    Reading,
    /// Decoding a video attachment's preview frame.
    Videos,
}

/// Work handed to a lane.
pub struct Work {
    pub run: Box<dyn FnOnce() + Send + 'static>,
    /// Set when what it makes is no longer wanted (the work looks at it between its steps).
    /// Work that goes on 30 seconds after (a decoder caught in a loop) is left to finish alone,
    /// and the lane carries on in a new thread.
    pub cancelled: Arc<AtomicBool>,
    /// Goes before the lane's work waiting (a picture opened big, a PDF's pages on screen).
    pub first: bool,
}

/// Platform services the UI needs.
pub trait Services {
    /// Unix seconds and the local UTC offset in seconds.
    fn clock(&self) -> (i64, i32);
    /// The app returned to the foreground; refresh cached platform settings if needed.
    fn on_resume(&mut self) {}
    /// Whether the device is a phone, when the platform can distinguish a device from a resized
    /// window. `None` leaves the UI to infer a compact layout from the window size.
    fn phone_form_factor(&self) -> Option<bool> {
        None
    }
    /// Android's preferred text size, as a scale for the whole interface.
    fn system_font_scale(&self) -> Option<f32> {
        None
    }
    /// The platform's current UI locale, if it differs from the process locale.
    fn system_language(&self) -> Option<&str> {
        None
    }
    /// The UI's active language changed; platforms may refresh localized system resources.
    fn language_changed(&mut self) {}
    /// Whether this platform can open the system settings for MailCraft's notifications.
    fn can_open_notification_settings(&self) -> bool {
        false
    }
    /// Open the system settings for MailCraft's notifications.
    fn open_notification_settings(&mut self) -> Result<(), String> {
        Err("notification settings are not available".into())
    }
    /// Refresh platform display settings which may change without restarting the app.
    fn refresh_display_configuration(&mut self) {}
    /// Whether the window has no title bar of the system's and MailCraft's top bar is its title
    /// bar, with the window's buttons, its drag and its resize edges (Windows; issue #22).
    fn own_title_bar(&self) -> bool {
        false
    }
    /// Once a frame while [`Services::own_title_bar`]: keep the window borderless.
    fn keep_frame(&mut self) {}
    /// MailCraft's own title bar is about to maximize or restore the window, in this frame.
    fn before_zoom(&mut self) {}
    /// The local time zone's IANA name (`Europe/Berlin`), when the system says.
    fn zone(&self) -> Option<String> {
        None
    }
    /// Files to attach (an empty list when cancelled).
    fn pick_files(&mut self) -> Vec<PathBuf>;
    /// Where to save a file (`None` when cancelled).
    fn save_path(&mut self, suggested: &str) -> Option<PathBuf>;
    /// Open a file with the system's default app.
    fn open_path(&mut self, path: &std::path::Path) -> Result<(), String>;
    /// Whether a local path can be handed to the system file browser.
    fn can_open_path(&self) -> bool {
        true
    }
    /// Where a build that updates itself is with the next version. The default: nowhere.
    fn update_state(&mut self) -> UpdateState {
        UpdateState::None
    }
    /// Install the waiting version as MailCraft closes, then open it. False when there's none.
    fn restart_to_update(&mut self) -> bool {
        false
    }
    /// Leave the waiting version for later: not installed as MailCraft closes this time.
    fn keep_update_for_later(&mut self) {}
    /// Show a desktop notification (new mail). The default does nothing.
    fn notify(&mut self, _title: &str, _body: &str) {}
    /// Show a notification that can return the user to the item that caused it.
    fn notify_for(&mut self, title: &str, body: &str, _target: NotificationTarget) {
        self.notify(title, body);
    }
    /// Where an installed Craft app is (`pdfcraft`, `wordcraft`…), if it is.
    fn craft_app(&mut self, _command: &str) -> Option<PathBuf> {
        None
    }
    /// Open a file with a given app.
    fn open_with(&mut self, _app: &std::path::Path, _file: &std::path::Path) -> Result<(), String> {
        Err("not available".into())
    }
    /// A rules file to import (`None` when cancelled, or where there's no file picker).
    fn pick_rules_file(&mut self) -> Option<PathBuf> {
        None
    }
    /// A categories file to import (`None` when cancelled, or where there's no file picker).
    fn pick_categories_file(&mut self) -> Option<PathBuf> {
        None
    }
    /// A certificate file (`.p12`, `.pfx`) to import (`None` when cancelled, or where there's no
    /// file picker).
    fn pick_certificate(&mut self) -> Option<PathBuf> {
        None
    }
    /// An OpenPGP key file (`.asc`, `.gpg`, `.key`) to import (`None` when cancelled, or where
    /// there's no file picker).
    fn pick_pgp_key(&mut self) -> Option<PathBuf> {
        None
    }
    /// A folder to save into (`None` when cancelled).
    fn pick_folder(&mut self) -> Option<PathBuf> {
        None
    }
    /// An installed font for a Chinese, Japanese or Korean interface language (`ja`, `ko`,
    /// `zh-hans`, `zh-hant`): its bytes and the index in a collection.
    fn cjk_font(&mut self, _lang: &str) -> Option<(Vec<u8>, u32)> {
        None
    }
    /// A Chinese, Japanese or Korean font is wanted now (the interface is in that language, or
    /// shown text has characters no font has): where fonts are downloaded (the browser), fetch it.
    fn want_font(&mut self, _lang: &str) {}
    /// A font asked for is still on its way ([`Services::cjk_font`] has it later).
    fn font_pending(&self) -> bool {
        false
    }
    /// An installed font with Arabic and Hebrew, for when the main font hasn't them.
    fn rtl_font(&mut self) -> Option<Vec<u8>> {
        None
    }
    /// Installed fonts for right-to-left scripts, in fallback order. The default is the
    /// platform's single font that covers the scripts it supports.
    fn rtl_fonts(&mut self) -> Vec<Vec<u8>> {
        self.rtl_font().into_iter().collect()
    }
    /// A picture on the clipboard, as PNG bytes (`None` when there is none).
    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        None
    }
    /// Start reading and decoding a clipboard picture without blocking the window. `None` keeps
    /// the synchronous fallback; `Some(false)` means there is no image; `Some(true)` means a
    /// request is running and its result will arrive through `take_clipboard_picture`.
    fn clipboard_picture_later(&mut self) -> Option<bool> {
        None
    }
    /// A decoded clipboard picture from an earlier request.
    fn take_clipboard_picture(&mut self) -> Option<Option<Vec<u8>>> {
        None
    }
    /// The files copied to the clipboard (in a file manager), to attach; empty when there are none.
    fn clipboard_files(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
    /// Start attaching files copied in the system clipboard without blocking the UI. `None`
    /// keeps the synchronous platform fallback; `Some(false)` means the clipboard has no files;
    /// `Some(true)` means a request is running or has completed and awaits `take_clipboard_files`.
    fn clipboard_files_later(&mut self) -> Option<bool> {
        None
    }
    /// Completed clipboard file request: paths to attach, or an error to show.
    fn take_clipboard_files(&mut self) -> Option<Result<Vec<PathBuf>, String>> {
        None
    }
    /// Pictures to insert (an empty list when cancelled). The default offers the file picker.
    fn pick_pictures(&mut self) -> Vec<PathBuf> {
        self.pick_files()
    }
    /// The platform's system Back key, when it has one (for example Android's navigation key).
    fn back_key(&self) -> Option<egui::Key> {
        None
    }
    /// Open a web address in the browser. The default can't.
    fn open_url(&mut self, _url: &str) -> Result<(), String> {
        Err("not available".into())
    }
    /// Whether this device can open a phone number in its system dialer.
    fn can_dial_phone(&self) -> bool {
        false
    }
    /// Open the dialer with a phone number; this must not place a call by itself.
    fn dial_phone(&mut self, _number: &str) -> Result<(), String> {
        Err("not available".into())
    }
    /// Whether this device can open contact addresses in a map app.
    fn can_open_map(&self) -> bool {
        false
    }
    /// Open a postal address in the system map app.
    fn open_map(&mut self, _address: &str) -> Result<(), String> {
        Err("not available".into())
    }
    /// Whether this platform can hand a contact to another app.
    fn can_share_contacts(&self) -> bool {
        false
    }
    /// Whether this platform can hand a file to another app through its system share sheet.
    fn can_share_files(&self) -> bool {
        false
    }
    /// Whether this device can save a contact as a vCard file.
    fn can_export_contacts(&self) -> bool {
        false
    }
    /// Whether this platform can ask the system contacts app to create a contact.
    fn can_add_to_system_contacts(&self) -> bool {
        false
    }
    /// Whether this device can hand a calendar event to another app.
    fn can_share_calendar_events(&self) -> bool {
        false
    }
    /// Whether this device can save a calendar event as an iCalendar file.
    fn can_export_calendar_events(&self) -> bool {
        false
    }
    /// Whether this device can prefill an event in its system calendar.
    fn can_add_to_system_calendar(&self) -> bool {
        false
    }
    /// Share a contact serialized as vCard text.
    fn share_contact(&mut self, _title: &str, _vcard: &str) -> Result<(), String> {
        Err("sharing contacts is not available".into())
    }
    /// Share a file's bytes and media type with another app.
    fn share_file(&mut self, _name: &str, _mime: &str, _bytes: &[u8]) -> Result<(), String> {
        Err("sharing files is not available".into())
    }
    /// Share a complete message source with another app.
    fn share_message(&mut self, name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
        self.share_file(name, mime, bytes)
    }
    /// Share several files in one system share sheet. Platforms may fall back to one file only.
    fn share_files(&mut self, files: &[(&str, &str, &[u8])]) -> Result<(), String> {
        if let [(name, mime, bytes)] = files {
            return self.share_file(name, mime, bytes);
        }
        Err("sharing several files is not available".into())
    }
    /// Open the system contacts app's new-contact form with these details.
    fn add_to_system_contacts(&mut self, _contact: &mailcraft_engine::model::vcard::Card) -> Result<(), String> {
        Err("adding a contact to the system address book is not available".into())
    }
    /// Share an event serialized as iCalendar text.
    fn share_calendar_event(&mut self, _title: &str, _ical: &str) -> Result<(), String> {
        Err("sharing calendar events is not available".into())
    }
    /// Open the system calendar's new-event form with these event details and RFC 5545 recurrence data.
    fn add_to_system_calendar(
        &mut self,
        _title: &str,
        _location: &str,
        _description: &str,
        _start: i64,
        _end: i64,
        _all_day: bool,
        _timezone: Option<&str>,
        _rrule: Option<&str>,
        _rdates: &[i64],
        _exdates: &[i64],
        _attendees: &[String],
    ) -> Result<(), String> {
        Err("adding an event to the system calendar is not available".into())
    }
    /// Open a message (its key) in a browser window of its own, as web mail does; `false` when windows are
    /// windows here.
    fn open_message_window(&mut self, _key: &str) -> bool {
        false
    }
    /// Close this page (a message's own window, when it's done).
    fn close_page(&mut self) {}
    /// An Outlook data file to open (`None` when cancelled).
    fn pick_data_file(&mut self) -> Option<PathBuf> {
        None
    }
    /// The folder MailCraft keeps its data in, and whether it can be moved from here (not when
    /// it was chosen at start, or in the browser).
    fn data_folder(&self) -> Option<(PathBuf, bool)> {
        None
    }
    /// How a move carried out at this start went: the new folder, or why nothing moved.
    fn take_data_moved(&mut self) -> Option<Result<PathBuf, String>> {
        None
    }
    /// Where the data moves when MailCraft next starts, if a move was asked for.
    fn pending_data_move(&self) -> Option<PathBuf> {
        None
    }
    /// Ask for the data to move into `to` when MailCraft next starts (`None` forgets the move);
    /// the folder it will be in.
    fn move_data_folder(&mut self, _to: Option<&std::path::Path>) -> Result<Option<PathBuf>, String> {
        Err("not available".into())
    }
    /// A frame of a video file for its thumbnail. Called each frame until it isn't
    /// [`VideoFrame::Pending`], so slow work belongs on another thread.
    fn video_frame(&mut self, _path: &std::path::Path) -> VideoFrame {
        VideoFrame::Unavailable
    }
    /// Run `work` on `lane`'s thread. `Err` gives it back (no threads here: the browser, the
    /// tests, a thread that couldn't start), and the caller does it a step a frame instead.
    fn background(&mut self, _lane: Lane, work: Work) -> Result<(), Work> {
        Err(work)
    }
    /// Whether [`Services::background`] takes work. Asked each frame that work is out there,
    /// which is when a lane caught on cancelled work is given a new thread.
    fn has_background(&self) -> bool {
        false
    }
    /// Whether files can be dragged out of the window to other apps (see
    /// [`MailApp::take_drag_out`]).
    fn can_drag_out(&self) -> bool {
        false
    }
    /// Whether this platform can read and write private local files. Downloads-based platforms
    /// default to no local files, but a native app may support both local files and downloads.
    fn has_files(&self) -> bool {
        !self.downloads()
    }
    /// Whether files are handed over as downloads (the browser, which has no files) instead of
    /// saved through [`Services::save_path`].
    fn downloads(&self) -> bool {
        false
    }
    /// Whether `open_file` can hand a saved document to another app rather than downloading it.
    fn opens_files_after_saving(&self) -> bool {
        false
    }
    /// Hand `bytes` to the user as a download named `name`.
    fn download(&mut self, _name: &str, _bytes: &[u8]) {}
    /// Hand an owned buffer to the platform without making another copy.
    fn download_owned(&mut self, name: &str, bytes: Vec<u8>) {
        self.download(name, &bytes);
    }
    /// Hand an owned buffer to the platform with its MIME type, when known.
    fn download_owned_with_mime(&mut self, name: &str, _mime: &str, bytes: Vec<u8>) {
        self.download_owned(name, bytes);
    }
    /// Save one file using a user-selected destination when available; otherwise use the usual
    /// download location.
    fn save_as_owned(&mut self, name: &str, mime: &str, bytes: Vec<u8>) {
        self.download_owned_with_mime(name, mime, bytes);
    }
    /// Number of save attempts finished since the last call, including cancelled or failed saves.
    fn take_download_completions(&mut self) -> usize {
        0
    }
    /// Whether this platform can stream an existing file to its download destination.
    fn downloads_files(&self) -> bool {
        false
    }
    /// Register the local output of an asynchronous data-file export for the platform to save.
    fn export_file(&mut self, _account: &str, _name: &str, _path: PathBuf) -> Result<(), String> {
        Err("saving a file from disk is not available".into())
    }
    /// Observe an engine event before the UI applies it.
    fn event(&mut self, _event: &mailcraft_engine::model::Event) {}
    /// Save `bytes` and open the saved document with the system's app for its type.
    fn open_file(&mut self, name: &str, bytes: &[u8]) {
        self.download(name, bytes);
    }
    /// Save and open an owned document buffer without copying it on platforms that can keep it.
    fn open_file_owned(&mut self, name: &str, bytes: Vec<u8>) {
        self.open_file(name, &bytes);
    }
    /// Save and open an owned document buffer with its MIME type, when known.
    fn open_file_owned_with_mime(&mut self, name: &str, _mime: &str, bytes: Vec<u8>) {
        self.open_file_owned(name, bytes);
    }
    /// Start choosing a file without waiting: `purpose` is `attach`, `picture`,
    /// `certificate:<account>`, `account-data-file`, `data-file`, `pgp-secret:<account>`,
    /// `pgp-public:<account>`, `rules`, `categories`, `signature` or `signature-picture`; the
    /// result arrives through [`Services::take_picked`]. `false` (the default) means
    /// [`Services::pick_files`] answers at once.
    fn pick_later(&mut self, _purpose: &str) -> bool {
        false
    }
    /// Files chosen since the last call: (purpose, name, bytes).
    fn take_picked(&mut self) -> Vec<(String, String, Vec<u8>)> {
        Vec::new()
    }
    /// Errors from native pickers since the last call.
    fn take_pick_errors(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Cancel an outstanding native picker when a system handoff interrupts or fails it.
    fn cancel_pending_pick(&mut self) {}
    /// Use a file shared from another app as the pending native picker result, when this
    /// platform has a picker that can fall back to the system share sheet.
    fn receive_shared_picker_file(&mut self, _path: &std::path::Path, _last_in_batch: bool) -> Result<bool, String> {
        Ok(false)
    }
    /// Remove a temporary local copy returned by the native picker after it has been consumed.
    fn discard_picked_file(&mut self, _path: &std::path::Path) {}
    /// Track a temporary copy used by a dialog, removing it if the dialog is closed.
    fn track_picked_file(&mut self, _path: &std::path::Path) {}
    /// Keep staged copies referenced by the open account or data-file dialog.
    fn keep_staged_files(&mut self, _active: &[PathBuf]) {}
    /// A staged file is now owned by an account or a resumable data-file import.
    fn retain_picked_file(&mut self, _path: &std::path::Path) {}
    /// Names of files saved through a platform download/export action.
    fn take_downloaded(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Whether the desktop prefers dark, where the window system doesn't say (Linux and the BSDs
    /// read it from the desktop portal); `None` when not known.
    fn system_dark(&self) -> Option<bool> {
        None
    }
}

/// Services for tests: a fixed clock, no dialogs.
pub struct NullServices(pub i64);

impl Services for NullServices {
    fn clock(&self) -> (i64, i32) {
        (self.0, 0)
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("not available".into())
    }
}
