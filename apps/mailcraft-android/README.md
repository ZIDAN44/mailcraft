# MailCraft for Android

MailCraft supports Android 12 (API 31) and later. Its native library uses 16 KB ELF segment
alignment for Android 15+ devices; Android extracts the library from the APK during installation.

This is the first native Android app shell for the shared MailCraft engine, network worker, and
egui interface. It packages a Rust `cdylib` as an Android `NativeActivity` through `eframe` and
`cargo-apk`, with a small Java activity subclass to relay new intents without replacing the open
Rust window, and MailCraft's generated app icon.

The app stores the MailCraft cache and UI preferences in Android's private no-backup directory,
which Android excludes from automatic cloud backup. On upgrade, MailCraft moves an existing
`mailcraft` data folder there and keeps using the original location if that move fails. It
uses the production network worker for account setup and synchronization, and restores the last
open module, mail view, contact, task or calendar-event detail, and calendar date and mode after it
is reopened.
Unfinished new tasks and edits to existing tasks are restored too; a new task's account and
categories are checked against current data. New calendar events and edits to cached
events return with their details and selected calendar when it is still available. New contacts
and edits to cached contacts restore their name, contact methods, addresses and notes.
Cached mail is available offline, and the last open cached message, mail search, message filter and
Focused/Other tab reappear from the cache with the mail view. Searches in the calendar, contacts,
tasks and notes also return with their module. Mail servers and signed messages are checked against
Android's platform trust store.
Diagnostic logs go to `mailcraft/mailcraft.log` in that private directory; panic entries
record the thread and source location without the panic message. Message preparation and picture decoding run on dedicated workers, leaving the window
available while larger messages are opened. Calendar dates and recurring events use Android's
time zone, including named zones' daylight-saving rules and fixed offsets. With **Same as the system** selected, MailCraft follows the locale configured for
its Android Activity, including locale changes while it is open and in background notifications; it also follows Android's light or
dark appearance when set to the system theme. Video attachments use Android's media decoder for
previews. Changed drafts are checkpointed to the offline cache after two seconds without input,
at least five seconds apart, when the app loses focus, and before it closes, then queued for the
next server sync. When that saved copy is available, an interrupted active draft reopens in its
editor on the next launch. Edited notes are saved after two seconds without input, at least five seconds
apart, when the app loses focus, and before it closes. MailCraft queues a sync for each server
account when it first opens and whenever it returns to the foreground, unless **Work offline** is on.
When connectivity returns while MailCraft is open, it queues a sync unless **Work offline** is on;
resume and recovery syncs stay pending until MailCraft is online again. They wait for the 30-second
throttle when a recent sync has already run. WorkManager also queues a full pass when the NativeActivity
pauses and repeats it periodically when Android grants time and a network is available. If the
NativeActivity is still alive, the pass uses its shared Rust workers; after Android stops them, it
is delegated to WorkManager's isolated `:mailcraft_sync` process, which resolves the same private
cache directory as the foreground app, opens it under its exclusive lock and runs the same network
pool headlessly. Keeping this work in a separate process also isolates the Android Keystore context
from the NativeActivity runtime. This remains true if Android storage migration had to keep the
original directory. When the app opens, it cancels queued cold-start passes and waits briefly for
the background process to release the cache before loading the foreground session. Both paths
honor **Work offline**. Android can defer these passes. A separate periodic WorkManager check
reads cached reminders without a network connection. A headless sync or reminder check posts a
private, count-only notification for new mail and due calendar or task reminders when Android
notification permission and the corresponding channel allow them; tapping a reminder opens its
event or task. Reminder notifications are remembered across runs and won't be repeated unless their
snooze time changes. When MailCraft loses focus, it briefly
waits for queued offline changes to reach the cache and flushes the latest UI
preferences to private storage before Android suspends it. The interface uses available
Android system fonts for CJK and right-to-left text without
bundling font files, follows Android's text-size setting across the interface, and refreshes its
phone or tablet layout as the display configuration changes. It stays within the device's system
bars, display cutouts and software keyboard. Touch screens get larger buttons,
menu spacing, message actions, message, contact, task and folder rows,
contact detail links, calendar invite contacts, calendar days and calendar toggles. Swipe a draft left to delete it, other mail left to archive it, or any message right to mark it read or unread. Message rows have a visible full action menu, grouped to fit smaller screens, with reply, reply all, forward, print, follow-up, junk controls, related-message search, source viewing and the other message actions. Long-press a message to select it, then tap other messages to add or remove them; More actions can select every message in the current list. The action bar shows the selection count, Archive and Delete, with Flag, read status and Move in its menu. Tapping a day in the month view opens its day view. On narrow screens the
mail list and reading pane stack vertically; the phone's bottom bar switches directly between Mail,
Calendar, People, Tasks and Notes. A temporary overlay drawer keeps account folders within
reach. Selecting a folder or another module closes the drawer. Android's system Back button and the phone top bar's Back button close the message editor
with a save prompt, close the drawer and dialogs, ask before discarding an edited event, task or
contact form, return from message, contact and task details, and clear a module search. They also
save an edited note before returning to its list or leaving the app. Sign-in
credentials are stored in encrypted SharedPreferences backed by Android Keystore.
If Android Keystore is temporarily unavailable, MailCraft keeps credentials in memory for the
current run and asks for them again after the app closes.
Only the encrypted keyring preferences are excluded from cloud backups and device transfers because
the Keystore key does not transfer with them; other shared preferences remain eligible for backup.
After restoring a device, sign in to the accounts again.
S/MIME and OpenPGP use MailCraft's shared Rust crypto code. Import a `.p12` or `.pfx` certificate,
or an OpenPGP secret key, from **File › Info › Security**. Their secret key material is kept in
Android's Keystore-backed encrypted credential store; an OpenPGP passphrase is used only for the
import and is not retained.
New mail, calendar and task reminders use separate Android notification channels, so each can
be adjusted independently in Android settings. MailCraft creates all three at startup and keeps
their names in the app's current language. Android 13 and later ask for
permission the first time one is needed, and notification contents are hidden on the lock screen.
Opening a new-mail notification goes to the message; opening a reminder goes to its event or task.
Tapping a notification while MailCraft is already open routes its target to the existing window.
If notification permission is off, MailCraft explains how to enable it and opens its Android notification settings from the notice.
Web and calendar links open through Android's default apps. `mailto:` links start a draft in
MailCraft. Tapping a contact's phone number or a `tel:` link in a message opens the Android dialer
for review and never starts a call; numeric `;ext=` telephone-link extensions are preserved. Tapping a contact address or a calendar event location opens the map app without
requesting location access. Tapping a contact email, calendar organizer or attendee starts a
message to them in MailCraft. Contact details and business cards can be shared as vCards with another Android app, saved as `.vcf` files, or added to the system contacts app without contacts permissions. Event details can be shared as iCalendar text, saved as `.ics` files through Android's document picker, or a single event or recurring series with added and excluded dates can be added to a calendar app without granting MailCraft calendar access.
The Android document picker opens files for attachments, pictures, certificates, keys, signatures
and imports. Attachment picking supports selecting up to 50 files at once, within the message's
25 MiB total attachment limit. Selected attachments are staged privately and attached directly
from that copy. Opening an attachment hands compatible apps a temporary,
read-only content URI for the private copy; MailCraft does not expose its filesystem path.
Files shared from another app can also complete a waiting action;
attachments use their private staging copy directly. Selected and copied clipboard images can be
added to messages and signatures. Clipboard files are attached when their source app exposes document URIs.
Text copied from MailCraft, including selected message text, contact email addresses, phone numbers, physical addresses, calendar event locations and organizer or attendee addresses, is available in Android's system clipboard. Contact detail rows, business cards, the phone list and event details have copy options in their context menus. Text copied from other apps can be pasted into MailCraft's text fields.
Individual saves (attachments, messages, PDFs, notes, rules and categories) open Android's system
save picker. **Print** opens a PDF copy in the system viewer.
Downloads, **Save all** batches and account PST exports go to `Downloads/MailCraft`
through MediaStore. **Save all** queues each non-inline attachment and reads one attachment at a
time to bound memory use. Large account PST exports stream to their destination without buffering
the archive in memory. Android can also hand
`.eml`, `.msg`, `.ics` and `.vcf` files to MailCraft on launch through **Open with** or the system
share sheet (up to 50 files at once), including when a document provider reports a generic MIME
type but retains the filename extension in its URI. A single `.pst` or `.mailcraft` archive can
also be shared into the existing data-file import flow, up to 4 GiB. MailCraft can handle `mailto:`
links as a mail app and turn shared plain text into a new message. If MailCraft is already running, a
second activity stages the incoming files and hands them to the existing window.
Other documents shared from another app open a new message with those files attached, up to 25 MiB
total, matching MailCraft's message attachment limit. Any accompanying plain text is included in
the message. HTML-only shares are converted to plain text before they're added. Selecting multiple
files in one share adds each file to the waiting action, within the message's 25 MiB total attachment
limit.
When a picker action is waiting, the shared file completes it, including imports and adding
pictures; otherwise the file is added to the open message or starts a new one.
Incoming documents are streamed to private storage with format-specific size limits before the
shared importer opens them. Shared-attachment copies are kept in a separate staging folder and
removed on the next launch if Android interrupted the handoff. Shared mail and PIM batches are
limited to 128 MiB; a picked or shared data archive is limited to 4 GiB, and smaller imports have
tighter per-format limits.
Opening an attachment passes its private copy to a compatible app through a temporary read-only
content URI. Up to 50 selected attachments, 25 MiB combined, can also be shared with another app
through Android's share sheet; those shared copies are saved in `Downloads/MailCraft`. The open
message can also be shared as an `.eml` file up to 64 MiB.
The picker also imports certificates, OpenPGP keys, rules, categories and HTML signatures, and saves
rule and category exports.
The **Open a data file** page also opens `.eml` and `.msg` mail, `.ics` calendars and `.vcf`
contacts. These files are staged privately before MailCraft imports or displays them.

If Android stops MailCraft during a file copy, the next launch removes unreferenced picker staging
files older than a day. Data-file sources used by a MailCraft account or a resumable import are
kept.

When a `.pst` or `.mailcraft` file is picked for import or account setup, Android streams it into
MailCraft's private app storage. This gives the existing importer a stable filesystem path, including
for large archives, without buffering the archive in memory. If the account or import dialog is
cancelled, its staged copy is removed; after submission or starting an import, the source stays
available for work that may need to resume later.

## Build and run

Install Android SDK Build Tools 35, the Android NDK, JDK 17, `cargo-apk`, and the Rust targets for
the ABIs you plan to build. The checked-in Gradle wrapper downloads Gradle 8.13 and verifies its
SHA-256 checksum. The Gradle project packages the Rust library with the small WorkManager adapter.
From `apps/mailcraft-android/android`, build arm64 plus the common x86_64 emulator ABI with:

```sh
./gradlew :app:assembleDebug -PmailcraftAbis=arm64-v8a,x86_64
```

On Windows, run `gradlew.bat` with the same arguments.

To include Google or Microsoft sign-in, provide the registration values from
[`docs/oauth.md`](../../docs/oauth.md) as Gradle properties (`mailcraftMicrosoftClientId`,
`mailcraftGoogleClientId` and `mailcraftGoogleClientSecret`) or matching `MAILCRAFT_*` environment
variables. The Gradle properties work with an already-running Gradle daemon. The Android build
tracks these values and rebuilds the Rust library when they change.

Install to a connected device or emulator with:

```sh
./gradlew :app:installDebug -PmailcraftAbis=arm64-v8a,x86_64
```

Run the cold-start background-sync regression test on a connected emulator with:

```sh
./gradlew :app:connectedDebugAndroidTest -PmailcraftAbis=x86_64 -PmailcraftTestProbe=true
```

The test probe uses the separate `ai.storyteller.mailcraft.debug.probe` application id, so Gradle
can remove its test installation without removing data from the regular debug app.

For an arm64-only build, use `-PmailcraftAbis=arm64-v8a` and install the
`aarch64-linux-android` Rust target. For an x86_64-only emulator build, use
`-PmailcraftAbis=x86_64` and install `x86_64-linux-android`.
The Gradle project reads the app version from the workspace `Cargo.toml`; the release application
id remains `ai.storyteller.mailcraft`. Browser sign-in for Google and Microsoft is offered when
their client registrations are available to the build; see [`docs/oauth.md`](../../docs/oauth.md)
for the `MAILCRAFT_*_CLIENT_ID` variables and provider setup.
