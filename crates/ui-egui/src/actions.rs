use std::path::PathBuf;

use mailcraft_engine::model::MessageKey;
use serde_json::Value;

use crate::{Dialog, backstage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RibbonTab {
    Home,
    SendReceive,
    View,
    /// While searching, as Outlook's Search tab: the scope, terms to narrow it, Close Search.
    Search,
}

/// Something a pane asks for; run after drawing so panes only borrow what they read.
pub enum Action {
    Run(&'static str, Value),
    /// Close MailCraft though online-only accounts hold unsent mail or changes (issue #4).
    CloseAnyway,
    /// Close, install the downloaded version, and open it.
    RestartToUpdate,
    /// Closing, with the update asked about: installed as MailCraft closes (true), or kept for
    /// later (false).
    UpdateOnClose(bool),
    /// Closing, with a data file to compact: that account's compacted first, MailCraft closing
    /// once it's done (`Some`), or closed now (`None`). The account rides in the action: the
    /// dialog that asked is gone by the time it runs.
    CompactOnClose(Option<String>),
    Open(MessageKey),
    Dialog(Dialog),
    /// An action from one message's window: done to that message (several can be written at
    /// once, each in a window of its own).
    ForCompose(u64, Box<Action>),
    OpenAttachment(usize),
    SaveAttachment(usize),
    ShareAttachment(usize),
    ShareAttachments(Vec<usize>),
    AttachFiles,
    /// Files staged from the system clipboard; discard each staged copy after attaching it.
    AttachClipboardFiles(Vec<PathBuf>),
    FocusSearch,
    /// Make a note and write on it.
    NewNote,
    /// A note's file (by its place among the note's files): opened in another app, or saved
    /// where asked.
    NoteFile {
        uid: String,
        index: usize,
        open: bool,
    },
    /// Search every folder for this (shown in the box, with the Search tab).
    Search(String),
    SetDark(Option<bool>),
    /// Print the open message (as a PDF opened in the system's viewer).
    Print,
    /// Save the open message as a PDF, or as its `.eml` source.
    ExportPdf,
    SaveMessage,
    ShareMessage,
    /// Open an attachment in a Craft app (its command name).
    OpenInCraft(usize, &'static str),
    /// Save every attachment of the open message into a folder.
    SaveAll,
    /// Save some attachments of the open message (by index) into a folder.
    SaveAttachments(Vec<usize>),
    /// The rules to a file, or rules from one (Outlook's Export and Import Rules).
    ExportRules,
    ImportRules,
    /// The category list to a file, or categories from one.
    ExportCategories,
    ImportCategories,
    /// Pick a `.p12` / `.pfx` file for an account's certificate, then ask its password.
    ImportCertificate(String),
    /// The browser: open the account's certificate it keeps, with its password, for this page.
    UnlockCertificate(String),
    /// Pick an OpenPGP secret key file for an account, then ask its passphrase.
    ImportPgpKey(String),
    /// Pick a file of correspondents' OpenPGP keys for an account, and keep them.
    ImportPublicKey(String),
    /// Choose a certificate on a smart card (or in the system's keychain) for an account.
    UseSmartCard(String),
    /// Choose one of the system's own certificates (Windows' store, the macOS Keychain).
    UseSystemCertificate(String),
    /// Sign an account in again: the browser for OAuth accounts, else the password dialog.
    SignInAgain(String),
    /// Insert a picture at the cursor of the message being written (from a file).
    InsertPicture,
    /// A signature of the account (by name; `None`: the one in it, else its own for this kind
    /// of message) into the message being written: in place of the one there, else at the cursor.
    InsertSignature(Option<String>),
    /// Paste a picture from the clipboard, if there is one.
    PastePicture,
    /// A picture into the signature being edited: from a file, or (`paste`) the clipboard's.
    SignaturePicture {
        paste: bool,
    },
    /// A signature saved as a web page (an Outlook `.htm` signature) as the one being edited's
    /// HTML code, with the pictures beside it.
    SignatureFile,
    /// The text's right-click Paste: files on the clipboard are attached.
    PasteFiles,
    /// Choose the interface language (`auto` or a code).
    SetLanguage(String),
    /// Open the signature editor for an account (`insert`: from a message, putting it in).
    EditSignature {
        account: String,
        insert: bool,
    },
    /// A link in a message was clicked (web pages open in the browser; `mailto:` starts a message).
    OpenUrl(String),
    /// Open the system dialer with a contact's number, without placing a call.
    DialPhone(String),
    /// Open a contact's postal address in the system map app.
    OpenMap(String),
    /// Hand a contact, serialized as vCard, to another app through the platform share sheet.
    ShareContact {
        title: String,
        vcard: String,
    },
    /// Save a contact, serialized as vCard, through the platform's save service.
    ExportContact {
        name: String,
        vcard: String,
    },
    /// Ask the system contacts app to create a copy of this contact.
    AddToSystemContacts {
        contact: mailcraft_engine::model::vcard::Card,
    },
    /// Hand a calendar event, serialized as iCalendar, to another app through the platform share sheet.
    ShareCalendarEvent {
        title: String,
        ical: String,
    },
    /// Save a calendar event, serialized as iCalendar, through the platform's download service.
    ExportCalendarEvent {
        name: String,
        ical: String,
    },
    /// Ask the system calendar to create a new event with the saved event's details.
    AddToSystemCalendar {
        title: String,
        location: String,
        description: String,
        start: i64,
        end: i64,
        all_day: bool,
        timezone: Option<String>,
        rrule: Option<String>,
        rdates: Vec<i64>,
        exdates: Vec<i64>,
        attendees: Vec<String>,
    },
    /// Run a command for the open dialog: it closes on success and shows the error otherwise.
    Submit(&'static str, Value),
    /// Bring the main window to the front (after acting from a message window).
    FocusMain,
    /// Open the File page at a page.
    Backstage(backstage::Page),
    /// An attachment is being dragged out of the window: hand it to the system as a file.
    DragOut(usize),
    /// Choose an Outlook/MailCraft data file to inspect, or a mail/calendar/contact file to open.
    OpenDataFile,
    /// Import the data file at `path` (`again`: though it's open as another account already;
    /// `changes`: what becomes of a file changed since MailCraft's copy of it was read, "read"
    /// or "keep"; `take_over`: its copy is an account in another MailCraft data folder, and
    /// becomes this one's).
    ImportDataFile {
        path: String,
        again: bool,
        changes: Option<&'static str>,
        take_over: bool,
    },
    /// Show how an account's import is going, beside the status bar.
    ImportDetails(String),
    /// Choose an existing data file for the account being added (its mail comes in first).
    PickAccountDataFile,
    /// An action from a message's own window, on that message (its attachments, Save As, Print).
    InWindow(MessageKey, Box<Action>),
    /// Choose where to write an account's mail as an Outlook data file.
    ExportDataFile(String),
    /// Show a folder in the system's file manager.
    OpenFolder(PathBuf),
    /// Choose a folder to move MailCraft's data into (at the next start).
    MoveDataFolder,
    /// Show one message formatted (true) or as plain text, whatever the setting for all mail
    /// says (that stays as it is).
    ShowFormatted(MessageKey, bool),
    /// Forget a move asked for.
    CancelDataMove,
}
