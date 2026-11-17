# Changelog

Every version of MailCraft that testers can install, newest first. What's still missing is in
[known gaps](docs/gaps.md); what comes next is in the [roadmap](ROADMAP.md).

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and MailCraft uses
[Semantic Versioning](https://semver.org/). Before 1.0, any version may change anything. Each
version is a milestone, named in italics under its heading. Numbers like (#11) are tester issues
on GitHub.

## Unreleased

### Added
- Android opens and saves files through its Activity's Storage Access Framework, so document providers receive the live Activity context they require.
- Android reads and decodes pasted clipboard pictures in the background, keeping large images from freezing a message or signature editor.
- Android reads files chosen in its document picker on a worker, so slow providers and large imports do not hold up the app window.
- Android copies files pasted from the system clipboard in the background, keeping large attachments and slow document providers from freezing the compose window.
- The phone contact list opens a touch-sized A–Z picker for jumping to a name.
- Android fills the recipient, copy and blind-copy fields when another app shares a composed email with MailCraft.
- Android resizes the app window for the on-screen keyboard without reserving the keyboard height twice.
- Android 13 and later route system Back through MailCraft's navigation, including save prompts for edited messages.
- Android's status and navigation bars match MailCraft's surfaces and keep their icons readable in both themes.
- Android excludes Keystore-encrypted credentials from cloud backups and device transfers, so restoring a backup cannot restore ciphertext without its key.
- Phone touch controls across navigation, mail lists, settings, formatting, contacts and calendar use a 44-point minimum target where they were previously 42 points.
- Phone compose uses 48-point touch targets for recipient suggestions, address and subject fields, and the full-width Send button.
- Screen readers announce each task's title, due date, status and priority in both phone cards and the full task list.
- On touch screens, long-press an attachment to select several; tap attachments to add or remove them, then share or save the selection.
- Touch screens use 48-point attachment rows with a 44-point remove button while composing.
- The phone contact list keeps its columns readable, scrolls sideways when they do not fit the screen, and gives each row a 48-point touch target.
- Screen readers announce calendar dates and appointments by their visible date, title and time.
- Screen readers announce contacts, address books and the available alphabetical shortcuts.
- Dropdown menus across the app use 44-point touch targets on touch screens.
- The phone navigation bar opens Mail, Calendar, People, Tasks and Notes directly.
- Android opens attachments in compatible apps through a temporary, read-only content URI for MailCraft's private attachment cache.
- Android schedules full mail and calendar/contact sync passes through WorkManager after the app backgrounds and when Android grants background time, while honoring **Work offline**.
- Android background sync and reminder checks use the same cache directory as the foreground app, including when storage migration falls back.
- Android posts a private, count-only notification when a background sync finds new mail; tapping it opens a message directly, or the Inbox if no message key is available.
- On phones, the top-bar Back button closes a message editor with a save prompt and saves an edited note before returning to its list.
- The phone top bar shows the subject of the open draft, or **New mail** before it has one.
- On phones, the mail top bar shows the current folder or search results, and an open message's subject.
- On phones, contact and task details show the selected person's name or task title in the top bar.
- On phones, the top bar identifies the note being edited, or shows **New note** before it has a title.
- On phones, a folder's account address appears beside its name so multiple mailboxes are easy to tell apart.
- Android checks cached calendar and task reminders without a network connection and opens the matching event or task when a reminder is tapped.
- Android follows the Activity's configured language and updates it when the device locale changes while MailCraft is open.
- Android follows the system text-size setting across the interface, including touch controls and scrollable layouts.
- Android reopens the last cached message when MailCraft was closed from the mail view.
- Android reopens the active draft after an interruption, retrying after sync and finding a server-saved copy by its message ID when needed.
- Android restores the last mail search from its offline cache after a restart.
- Android restores the last mail filter and Focused/Other tab with the mail view.
- Android reopens the last calendar event detail when the event is still in the offline cache.
- On phones, Back returns from task details and saves an edited note.
- On phones, Back asks before discarding an edited event, task or contact form.
- Android Back clears calendar, contact, task or note search before leaving that module.
- Android restores the calendar, contacts, tasks and notes search when reopening that module.
- Android restores an unfinished task form after an interruption, checking its account and categories against current data.
- Android restores a calendar event form after an interruption when its event and writable calendar are still available.
- Android restores a contact form after an interruption when an edited contact is still in the offline cache.
- Android checkpoints an edited note after a short pause, when it goes to the background, and before it closes.
- Android returns to the last open contact when MailCraft was closed from its address book.
- Android returns to the last selected task when MailCraft was closed from Tasks.
- Android refreshes phone/tablet layout and text-size settings when display configuration changes without an app restart.
- Phone offline, reconnect and Outbox controls have full-height touch targets.
- All choices in the phone's **More actions** menu have full-height touch targets in every module.
- The phone's **More options** menu has touch-sized entries for Tasks and Notes.
- The phone's **More actions** menu can switch **Work offline** without opening the full ribbon.
- The phone mail bar offers a direct, touch-sized **Send / Receive** button beside **New mail** when a server account is available and **Work offline** is off; it shows progress while accounts connect or sync.
- On phones, pulling the mail list past its top edge shows a **Send / Receive** cue; releasing starts sync when online syncing is available.
- Phone users can select every message in the current list from **More actions**.
- Android notification settings separate new mail, calendar reminders and task reminders; disabling one category leaves the others active.
- The Android notice for disabled notifications opens MailCraft's system notification settings directly.
- Android creates all notification categories at startup and updates their names as soon as the app language changes.
- Android requests notification permission on the UI thread and keeps notifications queued until the prompt is answered.
- Phone-sized File settings now use full-width pages with touch-sized navigation and message actions instead of a fixed desktop sidebar.
- Android uses the device's smallest-screen classification to keep phone layout in landscape without switching tablets when the keyboard resizes their window.
- The phone folder drawer opens in landscape too and closes after you choose a folder.
- The phone folder drawer closes immediately when you switch to another module from its navigation.
- Android preserves the open MailCraft window while a notification, shared document or mail link is handed to it.
- Phone-sized windows now use compact message margins, full-width compose attachments, and a header and quick-action bar sized for touch controls, one mail screen at a time, visible message action menus, and bottom navigation with an unread count; the full ribbon remains available on demand and wider layouts keep their split panes.
- The first-run Outlook data-file option fills the card width and has a touch-sized target on phones.
- Contact detail actions for sharing, saving and adding to the system address book have touch-sized targets.
- Phone contact details show a dedicated Copy button beside each email address, phone number and postal address.
- Phone-sized windows keep new messages, replies and drafts in the main screen, provide touch-sized switches between open messages, and use Back to close the editor with a save prompt when needed.
- Phone message composition uses a full-width Send action above the address fields, leaving the entire phone width for recipient entry.
- Phone mail lists support long-press multi-selection and tap-to-toggle; tapping a draft opens its editor directly, its left-swipe deletes it, and its action menu groups **Edit**, Move, Delete and source actions. Other mail archives on a left-swipe; a right-swipe changes read status. In conversation view, both gestures apply to every message in the conversation and use its aggregate unread state. Focused/Other and filter tabs have full touch targets, and the action bar shows the count, Archive and Delete, with a direct read-status button when it fits and Flag, read status, Move and clear controls in **More actions**.
- Phone message action menus and their nested choices have full-height touch targets.
- Phone mail lists give the sort field, reverse-order and search-scope controls full touch targets, with the filter tabs laid out around them.
- Touch screens use 44-pixel targets for message reactions in both the picker and the message.
- Category rows and color choices in category and appearance settings use touch-sized controls on phones.
- Folder-navigation section headings, including Search Folders, have full touch targets on phones.
- Conversation message lists use touch-sized, two-line rows on phones, keeping sender, preview and date readable.
- Address-book search results show names and email addresses in touch-sized rows on phones.
- Screen readers announce a recipient suggestion's name and email address while composing.
- The To, Cc and Bcc buttons in the contact picker have full touch-sized targets.
- Automatic-reply dates and subject fields use touch-sized inputs and fit narrow phone screens.
- On touch screens, swipe a message left to archive it or right to mark it read or unread; desktop drag-to-move remains unchanged.
- Android declares printed messages as `application/pdf` when handing them to the system PDF app.
- Dropdown menus, nested submenus, selection lists and message action menus now align their contents to the reading direction, including organize actions.
- Phone-sized calendars use a compact view selector, contacts open from a full-screen list into a contact card, tasks use touch-sized cards, and notes expand into a single-column layout.
- Phone calendars give the Today and view-selector controls touch-sized targets; week views show an agenda grouped by day, and tapping a date opens its hourly timeline.
- On short phone screens, the month calendar fits all six weeks with touch-sized days and compact event-count markers.
- Calendar navigation follows the reading direction, including previous and next arrows in Arabic and Hebrew.
- Phone screens show when MailCraft is working offline, disconnected or needs a new sign-in, and when messages are waiting in the Outbox, with direct controls to reconnect, open those messages or send them when online.
- Android's document picker now names the operation it was opened for, including attachment, import and signature selections.
- Android attaches files picked or shared into a waiting action directly from private staging, without an extra Base64 copy.
- MailCraft restores the last open module, mail view, and calendar date and mode after it is reopened.
- Android lets you add several attachments in one pass: the file picker reopens after each selection; Back or Cancel returns to the message with the files chosen so far.
- Android can share up to 50 selected attachments, 25 MiB combined, with other apps through the system share sheet.
- Android can share the open message as an `.eml` file up to 64 MiB with another app.
- Android accepts clipboard text up to 4 MiB when pasting, matching its copy limit.
- Android's native library uses 16 KB ELF segment alignment for devices using 16 KB memory pages.
- Android copies selected message text and contact email addresses to the system clipboard and pastes text from other apps into MailCraft.
- Contact email addresses, phone numbers and physical addresses can be copied from contact details, business cards and the phone list.
- Mail links opened from a message now fill the new message's recipients, subject and body.
- The link editor can now set the Cc, Bcc and message body of a `mailto:` link.
- Android uses its platform trust store to validate mail server connections and S/MIME signatures.
- Android contact phone numbers open the system dialer without placing a call.
- Android telephone links in messages open the system dialer without placing a call.
- Android opens telephone links that include a numeric `;ext=` extension in the system dialer.
- Android contact addresses open the system map app without requesting location access.
- Android calendar event locations open the system map app without requesting location access.
- Calendar event locations can be copied from their details.
- Calendar event organizers and attendees can now be contacted from the event details.
- Contact email addresses open a new message to that person.
- Android can share a contact as a vCard with another app.
- Android can save a contact as a `.vcf` file through the system save picker.
- Android can add a contact's name, nickname, all email addresses, phone numbers, postal addresses, websites, full-date birthday, anniversary and notes to the system contacts app without contacts permissions.
- Android shows translated explanations when links, file handoffs or document picking cannot continue.
- Android can share a calendar event as iCalendar text with another app.
- Android can save a calendar event as an `.ics` file through the system save picker.
- Android can add calendar series with recurrence rules, added dates and excluded dates without calendar permissions.
- Calendar events repeated with added dates are now recognized as recurring in the agenda, search and occurrence-deletion flows.
- Editing an event that repeats only on added dates now preserves those dates unless the repeat setting is cleared.
- Clearing an event's repeat setting now removes both added and excluded recurrence dates.
- Contact email, phone and address links have larger touch targets on phones and tablets.
- Calendar organizer and attendee links have larger touch targets on phones and tablets.
- Calendar event availability and privacy controls use touch-sized rows on phones.
- Calendar invitation response choices wrap into view on narrow screens.
- Online meeting links and controls wrap to fit narrow event forms.
- Calendar organizer and attendee email addresses can be copied from their context menus.
- Android preloads installed PKCS #11 modules in the S/MIME smart-card picker.
- Narrow windows show the mail list and reading pane vertically and use a folder drawer that overlays the content.
- Android's system Back key closes an open menu first, then the folder drawer or current dialog; it returns from an attachment preview to its message and uses the regular close flow at the app's root.
- Touch screens use larger toolbar buttons, switches and message actions, account setup and message fields, recipient suggestions, list filters, attachment and invitation actions, message/contact/task rows and forms, folder rows, calendar days and calendar toggles for easier tapping.
- Touch message rows expose the full message action menu in compact groups, including replies, forwarding, printing, follow-up, junk controls, related-message search and source viewing.
- In the month calendar, tapping a day opens its day view; double-clicking with a mouse starts a new event.
- A native Android app with the shared MailCraft UI, private offline cache, production sync worker, and Keystore-backed credentials.
- Android writes diagnostic logs to its private data directory and records panic locations without panic messages.
- Android installs with MailCraft's own launcher icon.
- Android notifications use the MailCraft icon, with a system icon fallback if the resource is unavailable.
- Android explains when notification permission is off and where to turn it on.
- Android can ask for notification permission again after a previous request was denied.
- Tapping a MailCraft notification now opens a new message or the calendar item that caused it.
- Tapping a MailCraft notification while the app is already open now routes its target to the existing window.
- Android keeps mail sync available in memory if the device Keystore cannot start, and warns that credentials will need entering again.
- Android now stops background sync and seals its offline cache when the app closes.
- Android checkpoints changed drafts when the app loses focus, keeping a local copy and queueing server sync.
- Android checkpoints changed drafts after two seconds without input, no more than once every five seconds while the app stays open.
- Android flushes the latest UI preferences to private storage before the app is suspended.
- Android gives queued offline mail changes time to reach its cache when the app loses focus.
- Android queues a sync for each server account when the app returns to the foreground.
- Android keeps every file in a multi-file share attached to the pending action.
- Android applies the message's total attachment size limit to files shared into a waiting attach action.
- Android refreshes the system time zone and light or dark appearance when the app returns to the foreground.
- Android queues a sync for each server account when the app first opens.
- Android keeps resume and network-recovery syncs pending until the 30-second throttle allows them.
- Android remembers MailCraft preferences, language, theme, and People view across launches.
- Android prepares messages and decodes pictures on worker threads so the window can keep responding.
- Android previews video attachments with the device's media decoder.
- Android reads the device's named time zone for calendar dates and recurring events.
- Android keeps the UI clear of the current status bar, navigation bar and display cutouts.
- Android keeps focused text fields clear of the software keyboard while typing.
- Android follows the device's light or dark appearance when MailCraft uses the system theme.
- Android uses installed system fonts for Chinese, Japanese and Korean text and right-to-left scripts.
- Android can paste a copied clipboard image into a message or signature.
- Android can attach empty files copied to the clipboard.
- Android keeps a shared subject when creating a message with no shared body text.
- Android can attach copied clipboard files when the source app exposes them as document URIs.
- Android removes temporary shared-attachment copies left behind if the process is interrupted.
- Android saves downloads and exports to `Downloads/MailCraft` through MediaStore and opens saved attachments from there.
- Android can use a file shared from Files as the fallback result when its system picker cannot open.
- Android keeps downloaded filenames within device filesystem byte limits while preserving common extensions.
- Android ignores a shared file if the picker action it was meant to complete has been waiting too long.
- Android can choose an image from the system picker to add to an HTML signature.
- Android recognizes picked image types from their file contents when a document provider omits or mislabels the extension.
- Android reads clipboard attachment names from the document provider, even when its URI contains only an opaque id.
- If Android's private cache cannot be opened, MailCraft now keeps network sync available with a temporary in-memory cache and warns that it won't persist.
- Android offers Google and Microsoft browser sign-in when the corresponding OAuth client registration is configured for the build.
- Android can notify about new mail and calendar reminders, with lock-screen content hidden; Android 13 and later ask before sending the first notification.
- Web and calendar links open in Android's default app; `mailto:` links start a draft in MailCraft.
- Android's system document picker can add an attachment or picture; import and export rules and categories; import certificates, OpenPGP keys, signatures or data files; save or open attachments in other apps; and open printed messages as PDFs in a document app. Large data files are staged on disk for the importer.
- Android opens the system save picker for individual attachments, messages, PDFs, notes, rules and categories. **Save all** keeps its bounded queue, and account PST exports stream to Downloads.
- Android limits queued Downloads to 64 files and 64 MiB of buffered data, with one larger file allowed when no save is active.
- Android transfers owned message, PDF, attachment and export buffers straight to its save queue, avoiding another full-size copy while a document is saved or opened.
- Android removes staged data archives when their account or import dialog is cancelled, and keeps submitted sources for imports that may resume later.
- Android opens saved attachments using their filename's MIME type when the document provider reports only a generic type.
- Android preserves an attachment's declared MIME type when saving or opening it, even when its file name suggests a different type.
- Android opens shared `.eml`, `.msg`, `.ics` and `.vcf` files (up to 50 at a time), `mailto:` links, and shared plain text as a new message.
- Android can open supported mail, calendar, contact and data files with **Open with** when a provider reports a generic MIME type but retains the filename in its URI.
- Android opens a shared `.pst` or `.mailcraft` archive in the data-file import flow, up to 4 GiB per archive.
- Android routes shared files, text and mail links from a second launch into the already running MailCraft window.
- Android can receive shared files from other apps as new message attachments, up to 25 MiB total.
- Android includes accompanying plain text from the share intent or its `ClipData` when creating a message for shared attachments.
- Android converts HTML-only shares to plain text before adding them to a message.
- Android caps shared mail and PIM batches at 128 MiB total and streams picked files into private storage with format-specific limits.
- The Open & Export page can open `.eml` and `.msg` messages, `.ics` calendars and `.vcf` contacts as well as `.pst` and `.mailcraft` data files.

### Changed
- The Android app now requires Android 12 (API 31) or later.
- Binary attachments and imported files now cross the UI command boundary as compact base64, avoiding the much larger JSON byte arrays.
- Android keeps its local cache, preferences, logs and staged imports in the system's no-backup private directory, migrating existing data on first launch.

### Fixed
- Android applies system bar safe areas to dialogs as well as the main content, keeping tall forms clear of status and navigation bars.
- Android catches panics in background synchronization at its JNI boundary, so a failed pass can retry without terminating the app process.
- Android avoids copying clipboard text again when the system has already supplied it.
- Android displays the translated save error if a background download cannot start.
- A failed Android file handoff now releases the waiting picker so another selection can start.
- Android's headless background sync honors saved cache-retention and **Work offline** preferences before connecting.
- Android's foreground and headless calendar handling follows named and fixed-offset device time zones.
- Android's background notifications follow the app's system language when **Same as the system** is selected.
- A background new-mail notification opens the newest message found across the sync, rather than an older message from its batch.
- Android updates the background sync gate in the same UI frame as a change to **Work offline**.
- Android restores **Work offline** before an automatic sync can be queued at launch.
- Android now exposes **Work offline**, uses local video previews and reports native file support independently from its Downloads workflow.
- Android no longer shows file-location buttons for private app folders that its system file browser cannot open.
- Android keeps resume and network-recovery syncs pending while **Work offline** is enabled, then relies on the account workers' reconnect sync when it is turned off.
- Android preserves the active file-picker action if a second picker request arrives before it completes.
- The shared file-attachment action now asks a platform's asynchronous picker only once.
- Android removes temporary clipboard-file copies after attaching them to a message.
- Android now ends a rejected multi-file handoff immediately when it contains a data archive, instead of waiting for file results that will never arrive.
- Arabic and Hebrew dialogs now align their contents and action buttons with the reading direction.
- Calendar, People and Notes now mirror their rows, calendar grids and hand-painted text for Arabic and Hebrew; note text follows the direction of its content.
- Android rejects calendar events whose end time is before their start instead of silently changing them to zero duration.
- Android's file association patterns now match document extensions as Android simple globs expect.
- Android notification taps now reach the existing window instead of being lost when NativeActivity is already running.
- Permanent-delete confirmation now applies to quick actions and context menus as well as shortcuts and the ribbon.
- Android's generic-MIME file associations now apply their extension patterns to content URIs.
- Android retries saving UI preferences after a temporary storage failure, including at the next flush and on close.
- Opening a stale Android mail notification now returns to the inbox instead of trying to open a message that is no longer cached.
- Android removes control characters, line breaks and directional formatting marks from notification text.
- Editing an existing `mailto:` link now reads and preserves its subject, CC, BCC and body fields with the RFC 6068 parser.
- Android detects the software keyboard on older supported versions so it no longer covers focused text fields.
- Android warns when notifications or their MailCraft channel are disabled in system settings.
- Android recognizes copied document URIs labeled `text/uri-list` and ignores web links in that list.
- Android keeps a duplicate share activity open until it finishes staging files, preserving temporary access to shared document URIs.
- Android keeps saves and exports on its MediaStore path when platform version detection fails instead of invoking the incompatible picker fallback.
- Android keeps a file queued for **Save as** when another system picker is open, then retries when it can.
- Android checkpoints changed drafts and flushes queued offline writes before normal shutdown.
- Android retries sync when a local-only network returns, even if Android does not mark it as internet-capable.
- Android keeps text accompanying shared files even when Android labels the files with a generic MIME type.
- The month calendar recognizes a mouse double-click on touch-enabled devices after touch input has been used.
- Android opens shared mail and calendar files with the extension that matches their reported type.
- Android skips video previews when frame dimensions are missing and bounds full-frame decoding on older Android versions.
- Android removes only its own unfinished Downloads entries if the system stopped MailCraft during a save.
- Android keeps saved preferences within the same size limit used when restoring them.
- Android routes **Attach Files** through its asynchronous picker path instead of returning no files.
- Android clears a failed or unreadable file-picker request, so a later shared file cannot complete an old action.
- Android waits for captive portal sign-in before treating the network as restored, then retries synchronization.
- A failed background draft checkpoint now reports its error and leaves the message marked unsaved.
- Android removes a staged import file if the app closes before its file picker result reaches the UI.
- Android bounds shared text, URIs, clipboard MIME types and document names before copying platform strings into Rust.
- Android ends a shared-file batch cleanly when a document fails, instead of waiting forever for files it will not process.
- The first-run welcome card now fits narrow windows without clipping its explanation.
- Account, address-book, contact, calendar, task and message dialogs now fit narrow screens; their form fields and action rows wrap or stack when needed.
- Android now starts with the device's local clock and UTC offset, so dates are not initialized as UTC.
- Android avoids queueing a second full sync as soon as startup's account sync begins.
- Android stops a shared-file import if its document provider stops returning bytes, instead of hanging.
- Android removes unreferenced picker staging files older than a day after an interrupted process, while preserving data files used by imports.
- Android opens supported files handed off with a `file://` URI as well as `content://`.
- Android drops queued notifications after permission is denied instead of retrying old alerts.
- Android removes pending temporary PST exports if the app closes before the save picker handles them.
- Android hides local file-location actions for data it keeps in private app storage.
- Android stops oversized picked files while streaming them, before they can fill private storage.

## 0.34.0

*One coloured title bar on Windows, dates written your way, and drafts never lost on their way to the server.*

### Added
- On Windows, MailCraft's top bar is the window's title bar, in its colour, as Outlook's is: its mark, the tabs and the search box share the top of the window with the window's own buttons, the bar's free space moves the window (a double-click maximizes it, and it snaps to the screen's edges), and the window resizes from its edges. **File › Options › Use the system title bar** brings the system's back, from the next start (#22).
- Drafts shows how many drafts it holds, in brackets (`Drafts [2]`), and a message whose reply or forward was begun and left in Drafts is marked with a red **[Draft]** before its sender in the list, as in Outlook (#52).
- **File › Options › General** chooses how dates are written (2026-10-28, 28/10/2026, 10/28/2026 or 28.10.2026), a 24-hour or 12-hour clock (`1:53 PM`), and whether the message list gives every message its whole date and time instead of the time today and the day this week (#46).
- Key tips on the ribbon, as Outlook's: Alt (or F10) shows a letter on each tab and on the search box, a tab's letters show its buttons', and a button's press it (Alt, H, R, P replies; Alt, J, S opens Send / Receive; Alt, Q goes to the search box). Escape goes back a letter, and a character typed with an Alt code shows none.
- The boxes you type a time into (an event's start and end, a task's reminder, Delay delivery) show it on the 12-hour clock when File › Options sets one, and take `1:30 PM`, `1pm`, `12 AM` or each language's own way of writing it, as well as `13:30`.
- A newsletter's buttons are drawn as buttons, with their round corners, and Outlook's at the size its copy gives them; columns that set no width are as wide as what they hold, as in a browser.

### Changed
- Outlook data files (`.pst`) are read and written by MailCraft's own code, written from Microsoft's published specification, in place of a third-party library. Files import as before, and those MailCraft makes or adds to open in Outlook.

### Fixed
- On Windows, maximizing and restoring MailCraft are smooth: they animate as any window's do, the taskbar no longer blinks and the window no longer flashes a blurred picture of itself.
- On Windows, all text in MailCraft is sharp again: the window's own title bar had the picture squeezed by one row of pixels, which softened every line from top to bottom. A stray black line along the top of the window is gone too.
- Removing an account while its `.pst` import was still writing could leave a new account added with the same id writing into the removed account's kept data file. The two now never share a file.
- Sending greets the server with this computer's own address (`EHLO [192.168.1.20]`, as Thunderbird does) instead of `[127.0.0.1]`, which some providers' spam filters took as a reason to drop the message after accepting it (#50).
- A reply or forward being written in the reading pane makes way for the message you select next, kept in Drafts, as Outlook does; the pane went on showing it while the other message was only selected in the list. One with nothing written in it just closes, and one in a window of its own stays open (#51). A reply or forward left as MailCraft made it, or a draft opened again and left unchanged, counts as nothing written: it leaves nothing new in Drafts, and closing it asks nothing, as in Outlook.
- A message closed with its changes saved (**Yes** to "Want to save your changes?"), or made way for in the reading pane, is held until its draft is safely in Drafts on the server or kept on this computer; if neither can keep it, the message opens again in a window with the reason, instead of being lost (#53).
- Messages sent from two MailCraft windows or computers started in the same second no longer share a Message-ID, which could make a mail service drop one of them as a copy of the other.
- A folder's unread count no longer jumps back up after a draft is saved: the folder list the server account refreshed then brought back the counts from before the mail you had just read reached the server.
- A draft or a sent copy filed in a Drafts or Sent folder deleted on the server (by another program) makes the folder again and goes in it, and one renamed there is found again under its new name. It was refused, and a draft kept on this computer was refused again at every connection.
- The ribbon's labels of commands you can't use yet (Delete, Archive… with no message selected), and the classic ribbon's group captions, read clearly: their grey was too faint to read comfortably on the ribbon (3.2:1), and is now as dark as text must be (4.5:1), still lighter than the commands you can use (#45).
- Newsletters lay out much as desktop Outlook shows them (#47): MailCraft reads the copy a newsletter gives Outlook (`<!--[if mso]>`, with its buttons and banner pictures), so columns built for other programs come out as Outlook's table; boxes side by side in a cell sit in a row when they fit; a row's cells are as tall as its tallest, their colours filling it, with what's in them at the top, middle or bottom as they say, and a height they ask for kept as the least; cells that each take a share of a table fill it as a browser shares it out; and the page's own colour no longer shows as a grey highlight behind every word.
- A folder of an Outlook data file being imported shows in the folder pane as soon as it's in, even when it's finished just after another: one finished within half a second of the last one shown (the Inbox, just after Deleted Items) waited for the next folder, or the import's end.
- Renaming or deleting a folder while MailCraft is still getting the mail of the folders before it no longer says "the server refused: no such mailbox".
- A folder renamed twice while offline keeps the mail already on this computer once the renames reach the server; it was downloaded again, and the name in between said "no such mailbox".
- A table's footer row (`<tfoot>`, as an invoice's total often is) shows at the end of its table, as a browser shows it: it was left out of the message and its text, and came first in a table laid out in columns.
- A draft saved just as MailCraft closed, or as the connection was lost, is in Drafts once after the next connection also on a server that adds a header to what it keeps. On a server without UIDPLUS, the copy a save replaces is marked deleted instead of the folder being expunged, so mail another program marked deleted in Drafts no longer goes with it.
- A forward's draft from a folder whose name has accented or non-Latin letters (`Entwürfe`) remembers the message it forwards when it's opened again.
- A message nesting tables by the thousand, as broken or hostile mail can, opens at once instead of after minutes: past what any mail nests, it's read as its words and links.

## 0.33.0 - 2026-11-05

*Replies as they came, and fixes shared with the other Craft apps.*

### Added
- Open a draft from Drafts to carry on writing it (**Edit**, or a double-click), as it was saved: its recipients and Bcc, formatting, signature, pictures, files and the original under a reply. Sending it marks the message it answers.
- Releases for FreeBSD on 64-bit ARM (aarch64).
- MailCraft updates itself however it was installed on Linux and FreeBSD: a `.deb` or `.rpm` through the system's package tool (after your password), a `.tar.gz` in place, and a Flatpak through your software installer, as the `.msi`, portable `.zip`, `.dmg` and `.AppImage` already did.
- MailCraft's releases (`v0.33.0`), not only its test builds, can be published, and they update themselves to the next release. A test build updates to the next test build or release, whichever is newer.

### Changed
- **File › Info › Mailbox settings** gives the data file's location, its Outlook copy and the clean-up each a row of their own.
- The **Open Outlook data file** dialog shows the file on a card with its size and contents, and a bar of the room its copy takes.
- **File › Options › Language** lists English first, then the other languages by their own names, accents set aside.
- The ribbon's tabs share the top bar with the search box, so the window gives one row less to its chrome; the app's name, which the title bar already shows, has gone from it. Send / Receive, Undo, dark mode and settings sit at the bar's end. In a narrow window the search box has a row of its own (#22).

### Fixed
- A server whose certificate comes from your organisation's own CA, as an on-premises Exchange often does, connects once that CA is in the system's certificate store (Windows, the macOS Keychain, the Linux bundle), for receiving and sending, instead of failing with "UnknownIssuer". Certificates Windows lists as untrusted stay refused, whether the server or a CA above it, and the password never reaches such a server (#37).
- Blocks of a newsletter stacked one on another touch, as in Outlook: each table keeps its own spacing and each block its padding, and a pale tint on a pale page still shows (#41).
- A block whose background is a gradient shows the colour Outlook shows for it (#41).
- The plain text of mail laid out in tables has no box-drawing lines around each table, and its preview keeps one block's words apart from the next's (#41).
- Reply, Reply all and Forward keep the original as it came, as Outlook does: mail laid out in columns shows under your text as the reading pane shows it, you change its words where they stand (typing, the ribbon and the shortcuts work in it), and it goes out with its own formatting and pictures.
- The message list's filters stay on one line in a narrow pane. Those that don't fit go into a **⋯** menu beside **By Date**.
- On the File page, a row's buttons and pickers never cover its text, and the page fits a narrow window instead of being cut off.
- The **Export to Outlook data file** card says calendars go inside the `.pst` file too.
- The View tab folds its buttons to icons in a narrow window, as Home does, instead of scrolling sideways.
- A message taken back out of the Outbox to change keeps its formatting, its pictures in place and, for a reply, the original under it as it was.
- Saving a draft again replaces its copy in Drafts on the server instead of adding another, and a server without a Drafts folder gets one.
- Sent from Drafts, a draft no longer leaves the list saying there's one more on the server.
- When the graphics driver resets (after sleep, say), MailCraft keeps every message being written in Drafts and closes, instead of freezing or crashing.
- Esc in a field on the File page leaves the field; the next Esc leaves the File page.
- A text file attached inline and named only by its file name, as Apple Mail sends one, is listed with the message's files, where it can be opened and saved, instead of being run into the message's text.
- A file whose base64 has a stray character in it opens as the file instead of as its encoded text, and a message whose own text has one shows that text instead of turning into a file.
- A file's name written in several pieces (RFC 2231) in a character set other than UTF-8 reads right instead of with `�`, and a message whose text has its boundary in the middle of a line is no longer cut short there.
- On Windows, MailCraft started from Task Scheduler or a remote shell opens in the window already running, not a second one.
- A file a program hands over unquoted, its path split at the spaces, still opens.
- The windows of messages being written, opened or shown as source carry MailCraft's icon on Wayland.
- On Linux, starting MailCraft no longer leaves a busy cursor, and the light or dark setting is followed even when the desktop changes it during start-up or doesn't answer.
- Screen readers can't close MailCraft by losing track of the focused control.
- Test builds install an update only once MailCraft has ended, and never one whose checksum file lists it twice with different sums.
- On a Mac, files MailCraft saves in a folder on a network share no longer fail to save.
- A picture opened again from a message no longer makes Photos ask "Is this file from a trusted source?": the "from the internet" mark an earlier version left on its copy is removed.
- Searching for a phrase whose words repeat ("the the") finds it after a word joined by a dot ("you.the the the"), where the text was checked.

### Security
- An Outlook data file's link to a device (`NUL`, `COM1`) or to a hidden file stream is kept as text, never read.

## 0.32.0 - 2026-11-04

*Pictures open without asking.*

### Added
- A local task list (**Tasks**, Ctrl+4) with due days, status, priority, reminders and repeats. A task with a due day shows in the calendar on that day.
- Tasks round-trip through `.pst` files and go into calendar files (`.ics`) as to-dos.
- Select and copy a message's text across links, tables and quotes. Ctrl+A selects the whole message, Shift with the arrow keys crosses lines, and a drag at the edge scrolls.
- `.pst` export keeps every item kind Outlook stores: flags, categories, pins, signed and encrypted mail, reports, attached messages, contacts, notes, tasks and calendars.
- Export adds to `.pst` files Outlook made, in its Unicode and older ANSI formats, and replaces items changed since the last export.
- `.pst` import brings in Rich Text bodies, mail in any code page, attached messages, linked files, RSS feeds, recalls, sharing invitations, meetings and the files on notes.
- **Mark complete** keeps a follow-up done, with a tick and the day it was done, through servers and `.pst` files.
- Search finds a word whatever its accents, and takes dates in `received:`, `sent:`, `date:`, `after:` and `before:`, including ranges such as `2024-01-01..2024-03-31`.
- Before an import, MailCraft shows what the `.pst` file holds and the room its copy needs. During it, a progress bar shows the time left.
- Pause, stop or continue an import. An import cut off by a crash or by closing MailCraft goes on where it stopped.
- **Compact now** in **File › Info › Mailbox settings**. MailCraft also compacts data files when left idle, and on close if you turn that on in **File › Options › Data files**.
- Turkish as an interface language (#34).
- Drop Outlook message files (`.msg`) or `.eml` files on MailCraft to put them in the folder open, as read mail, with their files and attached messages. A `.msg` opened from the desktop shows like an `.eml` (#38).
- React to a message with an emoji (**React**, beside **Forward**). The reaction goes to everyone the message went to (RFC 9078), and the reactions others send show under the message (#39).
- OpenPGP beside S/MIME: import your secret key file in **File › Info › Security** (its passphrase isn't kept; the key stays in the system keychain). Older keys stay to decrypt old mail (#36).
- PGP/MIME and inline PGP mail is decrypted for showing, and the reading pane says who signed it and whether the signature holds (#36).
- People's OpenPGP keys come from their mail's `Autocrypt:` header and attached keys, or from a key file you import. Your own key goes out with your mail, so answers can be encrypted (#36).
- Sign and encrypt with OpenPGP as PGP/MIME, to yourself too so Sent Items stays readable. With both a certificate and a key, the message uses the one every recipient can receive, or the one you pick (#36).
- The message you're writing says whether every recipient can receive it encrypted, and names those who can't (#6).
- **Encrypt when every recipient can receive it** in **File › Info › Security** turns encryption on by itself while that's so. It's off unless you turn it on (#6).

### Changed
- Opened or saved pictures and plain text files no longer carry Windows' "from the internet" mark, so Photos doesn't ask whether to trust them. Office files, PDFs, web pages, archives and programs keep the mark.
- Double-clicking an attachment only opens it. The reading pane keeps showing the message.
- Plain text moves from the link above a message to **File › Options › Reading** and to each message's **⋯** menu.
- Search runs in the background over an index that keeps up as mail changes. Typing never waits.
- Imports read on every core.
- An opened `.pst` file's copy (`.mailcraft`) goes beside it, instead of in Documents.
- Mail from a `.pst` file is listed by when it arrived, not when it was sent.
- Mail from a `.pst` file joins conversations by Outlook's conversation index when its ids don't place it.
- MailCraft reads and writes `.pst` files with one crate, from the fork [ZIDAN44/outlook-pst-rs](https://github.com/ZIDAN44/outlook-pst-rs).

### Fixed
- Big folders scroll, and long messages with many pictures open, without stutter.
- The window opens at once, even while a data file is checked. Data files reopen at once after a crash.
- Search results appear as soon as typing pauses.
- Pictures shown in a message's body keep their file names when sent or imported.
- Text attachments are saved and forwarded exactly as they came, in their own character set.
- The paperclip shows only on mail with real files, not on mail whose only pictures are in the body.
- Bounces and read receipts from a `.pst` file stay reports.
- A drive that fills during an import pauses it, instead of failing every message after.
- A sign-in that fails to save leaves no stray entries in Windows Credential Manager.
- Two MailCraft windows, or the CLI beside the app, can't overwrite each other's mail folders.
- Opening a message no longer waits for a folder, or the calendars, to finish syncing: the account's second connection brings it (#24).
- On IMAP accounts, a big message with attachments shows its text at once, its files listed; they download after (#24).
- Centred columns sized in points or ems, as Outlook and email tools write them, keep their width instead of filling the window (#41).
- A title centred in such a column stays centred and keeps its size, as Outlook shows it (#41).
- A reminder dismissed or snoozed in MailCraft is dismissed or snoozed on the server too, so Outlook doesn't show it again (Exchange, Microsoft 365, and CalDAV calendars) (#40).
- A reminder dismissed in another calendar program that says so in the event (RFC 9074) isn't shown again in MailCraft (#40).

### Security
- MailCraft never writes into an opened `.pst` file. Changes go into its own copy beside it, which belongs to one data folder.
- Mail goes only from an account that can send. With only a data file open, the message form offers **Add an account**.
- A `.pst` attachment that links to a file is read only from this computer's drives, never from the network or the web.
- A server's answer with lists nested a thousand deep can no longer crash MailCraft.

## 0.31.0 - 2026-10-31

*Signatures and meetings.*

### Added
- Several named signatures. Each account picks one for new messages and one for replies and forwards (#11).
- The **Signature** menu in a message lists the signatures and swaps the one in use (#11).
- Select several messages with Shift and the arrow, Home, End, Page Up and Page Down keys (#26).
- The right-click menu acts on every selected message, and the reading pane says how many are selected (#26).
- **View › Message source** shows a message as it came, headers and all. `mailcraft-cli source` does the same (#27).
- Keywords that other mail programs set show as categories. **Categorize › All categories…** lists them (#28).
- The category list exports to a file and imports back (#28).
- The message list can show a sender's address after the name: always, for strangers only, or never (#29).
- An optional External tag for senders outside your organisation's domains (#29).
- Each account's default calendar can be chosen (#30).
- **File › Options › Mail › After answering a meeting invitation**: leave it, move it to Deleted Items, or move it to a folder (#31).
- **Private**, **Show time only** and **Show as** in the event form (#32).
- Search in the Calendar finds events across every calendar shown, offline, with the mail search's syntax (#33).
- Keep your own BigBlueButton, Jitsi or OpenTalk rooms in **File › Options › Online meetings**. **Add online meeting** puts a room's link into an event (#25).
- **Join meeting** opens the meeting link of an event you received (#25).

### Changed
- Changing **From** while writing swaps an unchanged signature for the new account's (#11).
- A new category gets the same IMAP keyword Thunderbird gives a tag of that name, so both programs share it (#28).
- An invitation that arrives before the account's server calendars are found waits, then goes to the server (#30).
- An accepted invitation shows as busy, not tentative (#32).
- On servers that send invitations themselves (SOGo, Stalwart), guests get one invitation, not two.
- Signature settings saved by 0.31 aren't read by 0.30, which then shows no signature.

### Fixed
- In Calendar, People and Notes, the message list's keys don't act on hidden messages (#26).
- Shift+click selects the rows as they appear, pinned messages included (#26).
- Searching the server for a category finds it (#28).
- A folder that can't keep keywords says so, instead of dropping the category silently (#28).
- A new event goes to the account's default calendar, and two syncs in a row send it once (#30).
- One event that SOGo refuses doesn't stop the whole calendar sync.

### Security
- A sender whose name matches someone you know, from an address that isn't theirs, is warned of in the reading pane and marked in the list (#29).

## 0.30.0 - 2026-10-30

*Sent folder and centred mail.*

### Added
- Paste files copied in any file manager (#10).
- Signatures written as HTML code (#11).
- Alt+numpad character codes on Windows (#23).

### Fixed
- Sent mail goes to the folder the server marks as Sent (`\Sent`, RFC 6154) (#19).
- Fixed-width HTML mail keeps its column width (#10).
- Exchange messages show their text before the attachments finish loading (#24).
- A signature's table keeps its columns, so a photo stays beside the name (#11).
- The Flatpak attaches files dropped or pasted from anywhere in the home folder.
- A saved web-page signature finds its pictures in a folder with an accent in its name.

## 0.29.0 - 2026-10-29

*Pictures and pasted files.*

Thanks to Remo Laubacher for #12–#16.

### Added
- Pictures in a message being written show at the size they are sent. Resize them by the corner handle or the right-click menu (#10).
- Drop files on the main window to start a message with them attached (#10).
- Paste files copied in a file manager to attach them. On Windows, use the right-click **Paste** (#10).
- Signatures with pictures, from a file or pasted: up to 10 pictures of up to 1 MB each (#11).
- A formatting bar in the signature editor (#11).
- A signature copied from Word or Outlook brings its logo when pasted (#11).
- Replies and forwards can open in a window of their own (**File › Options › Mail**) (#11).
- **View › Favorites** hides Favorites from the folder pane (#11).
- `winmail.dat` attachments (Outlook's Rich Text mail) open: their files, pictures and formatted text show like any other message's (#18).
- Meeting invitations go on the calendar as tentative when they arrive, here and on the CalDAV server. **File › Options** turns it off (#20).

### Changed
- Exporting to an existing `.pst` file adds to it and skips messages it already has (#18).
- With the Unread filter on, an opened message stays in the list until you pick another (#13).
- Replies and forwards carry the original message's pictures (#10).

### Fixed
- The search box's text sits centred (#12).
- Pictures in HTML mail show at the sizes the message sets (#10).
- Attachments with no name or extension get the extension their type has, so the system can open them (#11).
- Servers without a Sent folder, or with one named in another language, get a copy of sent mail (#19).
- Closing MailCraft while a message is still sending asks first.
- `mailcraft-cli send` waits for its own message and for its copy in Sent.
- `mailcraft-cli` says when the app has the same data open, instead of showing empty folders.
- Mail in Shift_JIS, GBK, Big5 and EUC-KR shows its text.
- A message without a date shows the time the server received it, not 1 January 1970.
- Attachments open and drag out where `$XDG_RUNTIME_DIR` is missing, as on WSL (#14).
- A signature's pictures get a new Content-ID in every message (#16).

## 0.28.0 - 2026-10-28

*Web calendars.*

Some fixes are ported from the sibling Craft apps, with their authors credited.

### Added
- **Subscribe to a calendar…** takes an iCal or `webcal://` link. The calendar shows read-only and refreshes by itself (#7).
- Right-click a subscribed calendar to refresh or remove it (#7).

### Changed
- People found in Exchange's directory show their full name, not their alias (#9).
- A contact saved without a name takes the name the directory gives (#9).

### Fixed
- Events in read-only calendars can't be deleted.
- A subscribed calendar refreshes after the clock is set back.
- On Windows, a pinned MailCraft keeps its icon after an update. A pin that already lost it needs pinning again once.
- On macOS, the open and save panels follow the interface language.
- On Linux, the deb and rpm packages recommend `libxkbcommon-x11`, which the window needs under X11 and XWayland.
- Release builds always include the fallback fonts for Arabic, Hebrew, Chinese, Japanese, Korean and Thai.

## 0.27.0 - 2026-10-27

*Mirrored right-to-left.*

### Added
- In Arabic and Hebrew the panes are mirrored: icons on the right, counts and times on the left, scroll bars on the left.
- A Hebrew or Arabic paragraph being written starts at the right edge.
- Arabic and Hebrew mail can be selected and copied with the pointer.
- Screen readers name every button, toggle, folder and message row.
- **File › Options › Accent colour** offers eight colours and a picker for any other (#8).
- A **Plain top bar** option draws the top bar in the panes' colours (#8).
- A User-Agent of your own for each Exchange account, for servers that let in only mail programs they know (#9).
- **Select Names** searches the Global Address List, and To and Cc suggest names from it (#9).
- Exchange Online accounts search their organisation's directory, after a one-time sign-in that allows it (#9).
- Exchange servers that accept only Kerberos sign in.
- Simplified Chinese in the browser. The page fetches the font only when it needs it.
- **File › Info › Security › Keep unlocked in this browser** keeps the certificates' password sealed in the browser.

### Changed
- Long names are cut to one line with "…" instead of wrapping.
- The ribbon's buttons sit on a card with an edge. Menus and dialogs have a firmer edge and a soft shadow (#8).
- In dark mode the top bar is a deep shade with white text (#8).

### Fixed
- Closing from the taskbar or Dock while minimized asks before a message being written is lost. On macOS, ⌘Q asks too.
- Latin words inside Hebrew or Arabic text read in the right order, and the caret and word deletion work there.
- The message editor no longer crashes on a cursor past the end of the text.
- A data file that OneDrive or an antivirus holds for a moment is written again instead of failing.
- The System theme follows a Linux or BSD desktop's dark or light setting.
- Ctrl+V pastes a picture.
- A picture in a message sits where it is in the text.
- Each ribbon tab keeps its own scroll place, and file dialogs open where the last one was left.
- A calendar the Exchange server refuses doesn't keep the address books from updating.
- The RPM installs on openSUSE.
- Hebrew and Korean draw in the browser.
- A JMAP account the server made with no sending identity can send.
- A user name with a line break or tab in it is refused, instead of never signing in.
- A sample mailbox from an older build no longer stays on "Loading folders…".

## 0.26.0 - 2026-10-25

*Classic ribbon.*

### Added
- **View › Classic ribbon** shows big icons with their names underneath, in named groups (#8).
- **File › Info › Account settings › Calendar server…** takes CalDAV and CardDAV addresses with their own user name and password (#7).

### Changed
- Importing a renewed certificate keeps the old one to read mail sent to it. The newer one signs and encrypts (#6).
- **File › Info › Security** lists older certificates with **Use for sending** and **Remove** (#6).
- Calendar discovery follows RFC 6764 fully, so calendars such as Posteo's are found (#7).
- Dark mode sets panes, borders and fields further apart, and light mode's faintest text is easier to read (#8).

### Fixed
- Encrypted mail for a certificate MailCraft doesn't have says so, instead of trying every key.
- Signed mail stays valid through servers that strip `X-Mailer`, such as Postfix.

### Security
- Removing an account removes its certificate keys and calendar password from the system keychain.
- Calendar servers over plain http are allowed only on this computer.

## 0.25.0 - 2026-10-25

*Updates that finish.*

### Added
- A bar says when a test build's update is ready: **Restart now** or **Later**. A system notification says so when MailCraft isn't in front.
- The status bar shows the update download's progress.
- Closing MailCraft asks whether to install a downloaded update.
- An interrupted update download resumes, in this run or the next.

### Changed
- On Windows, the bar and the close question say that Windows asks for an administrator.
- An AppImage in a folder you can't write to is left alone, and macOS finds Craft apps in `~/Applications` too. See [how test builds update](docs/updates.md).

## 0.24.0 - 2026-10-24

*S/MIME.*

### Security
- Import a `.p12` or `.pfx` certificate in **File › Info › Security**. Its private key stays in the system keychain (#6).
- Sign and encrypt outgoing mail, by default or per message (#6).
- Signed mail says who signed it and whether to trust it. If a check fails, it says which.
- Encrypted mail is decrypted in memory only, and shows its attachments, previews and invitations.
- Replies to encrypted mail are encrypted.
- The message list marks signed mail with a seal and encrypted mail with a padlock.
- EC certificates sign, encrypt and decrypt.
- Use keys on smart cards (PKCS #11), in the macOS Keychain or in the Windows certificate store, including keys that can't be exported.
- S/MIME works in the browser, with the `.p12` file unlocked by its password.
- The subject, sender and recipients are signed and encrypted too (RFC 9788). An encrypted message's subject reads `[...]` in transit.
- Decrypted mail loads nothing from the web, and only a message protected as a whole counts as signed or encrypted.
- An optional revocation check of the signer's certificate, in **File › Options › Safety**, off by default.

## 0.23.0 - 2026-10-24

*Mail that can't fool you.*

### Added
- `mailto:` links, and `.eml`, `.ics` and `.vcf` files, open in MailCraft. The installers offer it as the mail app.
- `is:awaiting` finds mail you sent 2 to 30 days ago that nobody answered. A Search Folder and a Search tab toggle offer it.
- Repeat rules such as "the last weekday" and "Friday the 13th".

### Changed
- An event keeps its time zone and repeats at the same local time across daylight saving.

### Fixed
- Answering an update to one occurrence changes that occurrence, not the whole series.
- An expired sign-in token renews itself before MailCraft asks you to sign in again.
- Mail that arrives just after a sync shows at once, not up to ten minutes later.
- A message the SMTP server refuses with a mention of "authenticated" can be sent again.
- Ctrl+Q marks a message as read without closing MailCraft.
- Files dropped from the desktop onto a message are attached.
- Search results from several folders say which folder each is in.
- The 32-bit Windows build opens its window.
- `--help` prints the options.

### Security
- Photo location is taken out of JPEG and PNG attachments when you send. **File › Options › Safety** can keep it.
- A red bar warns of a sender whose domain imitates one you deal with, or whose name shows a different address.
- **Check before sending** holds mail to a look-alike domain, or mail that shows eight or more unrelated people each other's addresses.
- Tracking pictures never load, and a message says how many trackers it carries.
- Links open without redirect wrappers and tracking tags. **File › Options › Safety** can open them as they are.
- HTML mail that hides 25 words or more says so and shows the hidden text.
- Invitation updates and cancellations apply only when they come from the organizer.
- A hardening review: MailCraft doesn't open attachments that run code, marks saved attachments as downloaded on Windows, cleans unsafe file names and keeps message pictures off your own network.

## 0.22.0 - 2026-10-22

*Everyday tools.*

### Added
- Event reminders, from None to 2 weeks, with a Reminders window (Dismiss, Snooze, Open Item) and a desktop notification.
- **Block sender**, **Never block sender** and **Never block sender's domain**, with both lists in **File › Options**.
- **Clean up conversation** and **Clean up folder** move messages that a later reply quotes in full to Deleted Items. Undo brings them back.
- Search Folders: saved searches in the folder pane.
- `larger:` and `smaller:` in search, and several addresses in `from:` and `to:`.
- Quick Steps, from the ribbon or Ctrl+Shift+1 to 9.
- The Work Week view.
- **Insert date & time** and **Word count** while writing.
- The search box's scope menu (Current Folder, Subfolders, Current Mailbox, All Mailboxes), recent searches and Advanced search.
- Write several messages at once, each in its own window.
- Hovers, selections and panes move smoothly. **Play animations** in **File › Options** turns this off.
- Test builds update themselves: they download the next test build, check it and install it on close. Signed releases don't.

### Changed
- Searching the server goes newest first, shows results as they come and stops when you clear the box (#4).
- The ribbon is one height on every tab, and drops labels from its end when the window is narrow.

### Fixed
- **Show more** works on servers with QRESYNC, such as Dovecot (#4).
- Settings are saved every few seconds, so a crash doesn't lose them.
- Closing MailCraft with messages being written asks to save them.
- Changes made with `mailcraft-cli run` reach the server at the next `sync`.
- The CLI doesn't crash when its output is closed.
- Ctrl+Shift+digit shortcuts work with Shift and AZERTY keyboards.
- Notifications from a portable copy show MailCraft's name when an installed copy is on the computer.

## 0.21.0 - 2026-10-21

*One MailCraft at a time.*

### Added
- Starting MailCraft again brings the running copy to the front instead of opening a second window on the same data. A different data folder runs its own copy.
- **Show more** at the end of a folder's list brings in older mail beyond what is kept on this computer (#4).
- **Search the server** at the end of a search finds mail that isn't downloaded, for IMAP, Exchange, Microsoft 365 and JMAP accounts, and in the browser (#4).

### Changed
- Shared mailboxes are listed as mailboxes of their own, in the folder pane and the calendar. Drag them, or use **Move up** and **Move down**, as with accounts (#1).
- Exchange invitations show their notes as the invitation does, with each link under its words and long addresses shortened. **Edit notes** opens the text box (#1).
- Every dialog fits inside the window and scrolls when it has more (#1).
- **Online only** explains what is sent straight away and what stays on the server (#4).
- Closing with unsent mail or unsaved changes on an online-only account asks first (#4).
- A portable copy asks for each account's password once, when MailCraft starts (#4).
- **File › Options** says where the account list and settings are kept (#4).
- In a narrow reading pane, the date moves under the sender and the header's buttons wrap.
- An account shows as online once its mail is synced. Calendars and contacts follow.
- Connecting tries IPv6 and IPv4 in turn, so a missing route doesn't hold up the account.

### Fixed
- JMAP folders keep as much mail as you chose, not just one request's worth.
- Flag changes, moves and deletes made during a sync show at once and survive quitting.
- Exchange Online mail fetched in batches isn't downloaded again on the next sync, and server search results stay.
- Moving one message window no longer pulls the others to the same place.
- Exchange appointments that share an ID are no longer merged into one.
- An Exchange birthday saved without a year shows as a day and month.
- Hidden Exchange folders stay hidden.

### Security
- What you type into **Search the server** can't change the query sent to the server.

## 0.20.0 - 2026-10-20

*A window on every Windows PC.*

### Added
- **Online only**, per account: none of its mail is written to this computer, and a message downloads when you open it (#4).
- Portable mode: a `portable.txt` or `MailCraft.portable` file beside the app keeps all settings and data in a `MailCraftData` folder there, and nothing in the registry (#4).
- With several accounts, the calendar lists each account's calendars under its address, in its colour and with the name the server gives them (#1).
- A shared mailbox's calendar shows with the mailbox (#1).
- **Join meeting** opens a meeting's Teams, Zoom, Meet or Webex link, and the links in its notes are listed to open (#1).
- **Check for updates** in **File › About** finds the newest build and offers its installer. Nothing installs by itself.

### Fixed
- The window opens on Windows PCs without a graphics card. MailCraft uses DirectX 12, picks a real GPU first and falls back to Windows' software renderer (WARP).
- A window that can't open shows a message with the reason and where the log is.
- A Microsoft 365 sign-in is kept in Windows Credential Manager, however long its token (#5).
- A meeting shown in two calendars opens (#1).
- A window saved maximized opens once, maximized, without flickering.
- Windows return to the screen when their monitor is gone, and shrink to fit a smaller screen.
- Moving a window to a monitor with another scale, and typing with an input method (Korean, for example), work as expected.
- On Linux, MailCraft draws on the desktop's GPU, opens through XWayland when it can so attachments drag in and out, and never freezes on a hung clipboard.
- Intel Macs and graphics cards with lower limits start normally.
- Odd command-line arguments, a broken error stream and quitting on macOS no longer crash the app.
- MailCraft is sharp on high-DPI Windows monitors.
- The Windows installer says MailCraft needs Windows 10 or later.

### Security
- Files handed to other apps (an opened attachment, video frames) go in a folder of your own, not a shared temporary folder.

## 0.19.0 - 2026-10-20

*Shared mailboxes of their own, and SMTP's own sign-in.*

### Added
- The outgoing server can have its own user name and password, kept in the system keychain (#3).
- A shared mailbox's menu has **Send / Receive**, **New folder**, **Remove** and **Add shared mailbox** (#1).

### Changed
- A shared mailbox appears after the account's folders as a mailbox of its own, with its Inbox, Drafts and Sent Items in their usual places (#1).
- Exchange's special folders keep the names the server gives them, such as "Postvak IN" (#1).
- The conversation above a message shows only while **View › Conversations** is on (#1).

## 0.18.0 - 2026-10-19

*Shared mailboxes and the contact card.*

### Added
- Exchange shared mailboxes. Those Autodiscover names open by themselves, and you can add others in **File › Info › Account settings › Shared mailboxes** (#1).
- A contact card: click a name in a message's header to send mail, add or edit the contact, or copy the address (#1).

### Changed
- Removing an account keeps its data file on disk. The **Remove** dialog names the file, and adding the address again starts a new one.

### Fixed
- Enter submits the password, shared calendar and shared mailbox dialogs.

## 0.17.0 - 2026-10-19

*Copy and paste, and public folder contacts.*

### Fixed
- Copy and paste work in every box, including formatted signatures pasted from Word (#1).
- An organisation's shared address books and calendars in Exchange public folders show (#1).

## 0.16.0 - 2026-10-19

*Contacts as Outlook keeps them.*

### Added
- Contacts keep each phone number's kind, separate business, home and other addresses, and a birthday, across vCard, Exchange, Microsoft 365 and `.pst` files (#1).
- The contact form has three email fields, a field per number and one address at a time (#1).
- People lists each account's address books and the shared ones. Click one to show it alone (#1).
- Signatures keep the font sizes and typefaces of pasted text (#1).
- A log file, `mailcraft.log`, in the data folder. It never holds message text or passwords.
- Export and import rules from **Open & Export** and the **Rules** dialog.
- The **Rules** dialog shows one rule per line, with a find box and **Run all**.
- Select several attachments with Ctrl+A or Ctrl+click and save them together.

### Changed
- Sending and automatic replies use a second connection, so they never wait behind a sync (#1).
- Backspace after a recipient removes the whole address.
- Address suggestions are as wide as the longest one, and a click anywhere on a row picks it.
- **File › Options** is laid out as cards of settings with switches.
- **Export** has one list of accounts and one **Export…** button. The file is named after the account's address.
- **Print** asks where to save the PDF, then opens it.
- New mail no longer keeps the Windows taskbar button highlighted.
- Large `.pst` files import and export much faster, use less memory and take less disk space.
- Big folders stay quick to move, delete and mark read in, and repeated searches of a large mailbox are fast.

### Fixed
- Another person's Exchange calendar is editable when its events allow it (#1).
- Public folders on Exchange 2013 and later list and sync their mail. A server that won't list them says why (#1).
- French text never reads "de Anna" (#1).
- One bad message no longer stops the rest of a folder downloading on Exchange and Microsoft 365.
- Replied and Forwarded marks are set when the reply is delivered, not when it is queued.
- A search like `from: anna` (with a space) works.
- Deleting a contact asks first.
- vCard 2.1 values in quoted-printable show decoded.
- On Gmail, permanently deleting a message in a label folder removes it.
- The To and Cc of a reply end with `; `, so typing adds a new address.
- Clicking an address suggestion keeps the cursor in the box.
- Ctrl+A in a message's text selects only that text.

## 0.15.0 - 2026-10-18

*What testers asked for, again.*

### Added
- Text pasted from Outlook, Word or a web page keeps its bold, italics, colours and links (#1).
- The signature editor is a full message editor. Signatures keep their formatting in new messages, replies and **Insert signature** (#1).

### Changed
- Side by side shows each calendar in its own pane, under a heading in its colour with × to hide it. Clicking in a pane adds the event to that calendar (#1).
- **Select Names** opens with the address book you chose under "Always open with this address book" (#1).

### Fixed
- Another person's Exchange calendar opens editable when you are its editor (#1).
- Public folder contacts and calendars on Exchange 2013 and later can be read. One that still can't says why (#1).
- Long names and addresses in **Select Names** end with "…" instead of overflowing (#1).

## 0.14.0 - 2026-10-17

*What testers asked for.*

### Added
- Choose how much mail each folder keeps: the newest 1,000, 5,000, 20,000 or all, with 40 to 1,000 downloaded ahead (#1).
- Choose when mail is marked read: when shown, when you move to another message, or only when opened in its own window (#1).
- **Insert signature** and **Signatures…** on the **Message** ribbon (#1).
- The search box searches People and Notes when you are there (#1).
- Calendar events show in their category's colour. Categories sync with Exchange, Microsoft 365 and CalDAV (#1).
- Tick calendars to show or hide them, view them side by side in Day and Week, and see up to six months in the side pane (#1).
- People's List view lets you choose columns and sort by any of them (#1).
- An organisation's shared contacts and calendars in Exchange public folders, read-only (#1).
- Exchange automatic replies can send a different reply outside the organisation (#1).
- Notes arrange by title, colour or last change (#1).
- **Open folder** opens it in the file manager (#1).

### Fixed
- A Hotmail account added with a password switches to **Sign in with Microsoft** from the status bar and keeps its mail (#1).
- "User is authenticated but not connected" says what to do. **Use Microsoft Graph** moves the account off IMAP (#1).
- Automatic replies on an account that can't connect no longer leave it at "Connecting…" (#1).
- Generated HTML mail keeps its pictures, style sheet colours, headings, column widths and stripes (#2).
- Typing in search stays quick on a big mailbox (#1).
- Exchange folders sync past the first 1,000 messages (#1).

## 0.13.0 - 2026-10-17

*The open gaps, again.*

### Added
- Outlook.com, Hotmail and Live accounts sign in with Microsoft in test builds.
- Edit another person's calendar when its owner made you an editor. Calendars you can't edit say "read-only".
- HTML mail shows background pictures behind tables, cells and boxes, held back like other web pictures.
- HTML boxes placed with `position: absolute` sit over the box that holds them.
- Importing a `.pst` file brings in its notes, with their colours and times.
- Right-click Arabic or Hebrew text to copy the line, or the whole message.
- Move accounts up and down from their right-click menu, or drag them.
- A POP account can start from a `.mailcraft` file.
- **File › Open a data file** opens a `.mailcraft` file as a mailbox of its own, without changing the file.

### Changed
- MailCraft opens on the first account's Inbox. All Inboxes stays in Favorites.
- The status bar sums up all accounts ("All folders are up to date", "2 accounts disconnected"). Hover to see each account.
- The folder pane remembers which of Favorites, accounts and folders you collapsed.

### Fixed
- Text in HTML mail takes its cell's or table's colour, so white text on a dark band stays white.

## 0.12.0 - 2026-10-17

*Microsoft sign-in that works.*

### Changed
- Accounts that sign in through the browser say **Sign in again**, not **Password needed**, and the link opens the sign-in.
- An account the server refuses stops with the reason until the next **Send / Receive**.
- Each Microsoft sign-in asks for the permissions it needs. An SMTP refusal says whether the sign-in covered sending.

### Fixed
- On Windows, **Sign in with Microsoft** opens the sign-in page, and links in mail open.
- Sent Items shows the sent copy a few seconds after sending on Exchange and Microsoft 365.
- Small dialogs open at their own size, with their buttons in place.
- Opening People no longer widens the folder pane everywhere.

## 0.11.0 - 2026-10-17

*Notes, previews and Exchange beyond the mailbox.*

### Added
- Click an attachment to preview it in the reading pane: pictures, PDF pages, text, attached mail, and Word, Excel and PowerPoint files (#1).
- Notes (Ctrl+5): coloured sticky notes, kept in an Exchange mailbox or on this computer (#1).
- People in three layouts: a list with an a–z index, business cards and a phone list (#1).
- Open another person's Exchange calendar by address, read-only (#1).
- Public folders on on-premises Exchange (#1).
- A Dutch interface.

### Changed
- Adding an Outlook.com or Hotmail account says it needs Microsoft sign-in instead of asking for a password again and again (#1).

### Fixed
- The list's filter tabs no longer run into **By Date** in long-worded languages (#1).
- The status bar no longer counts mail while People or Calendar is open (#1).

## 0.10.0 - 2026-10-16

*Windows, folders and rules as Outlook keeps them.*

### Added
- The main window reopens where it was and at its size, maximized if it was. Message and compose windows open where the last one was.
- Run a new rule at once on the mail already in the Inbox, or use **Run now** in the rules list.
- Attachments as small chips with a thumbnail, name and size. Double-click opens one; ▾ or right-click offers **Preview**, **Open**, **Save As** and **Save All Attachments**.
- Notifications for new mail that rules filed into folders.
- A **Search** tab while searching: where to look, **From**, **Sent To**, **Subject**, **Has Attachments**, **Unread**, **Categorized**, **Flagged**, **Important** and **Close Search**.
- **Follow Up**: flag for today, tomorrow, this week, next week or no date, **Mark complete** and **Clear flag**. The reading pane shows when it's due, in red once overdue.
- The reading pane shows when mail came with high or low importance.
- **Delay delivery** holds a message in the Outbox until its time, while MailCraft is running.
- **Replies to** sets a Reply-To address while writing.
- **Quick print**, **Ignore** and **Find related** on a message; **Reply with meeting** and **Forward as attachment** under the reading pane's **⋯**.
- **Delete all…** and **Properties…** on a folder; **Open file location** and **Account settings…** on an account.
- Rows, buttons, tabs, calendar events and colour swatches light up under the pointer.

### Changed
- **New folder** in the account menu starts in the folder that is open.

### Fixed
- A folder made in the Inbox shows under the Inbox.
- **Open** on an attachment opens it, and on Windows apps such as Acrobat accept the file.
- A message window's attachments act on its own message.
- Japanese, Korean and Chinese show in the language menu and in mail whatever the interface language.
- "Pin", "Unpin" and "Always move to Focused/Other" are translated.

## 0.9.0 - 2026-10-15

*Outlook's everyday commands.*

### Added
- Choose how long POP mail stays on the server: kept, removed after a number of days (14 for new accounts), or removed once deleted from Deleted Items.
- **View › Arrange by** (date, sender, recipient, subject, size, flag, attachments) and **Reverse sort**, with headings.
- A **Send / Receive** tab with **Send all** and **Work offline**, which is kept across starts.
- Undo (Ctrl+Z) for moves, deletes, flags and categories.
- **Filter** by categorized, important, or to or Cc me.
- **Ignore** a conversation, under **Delete**.
- **New items** for an event or a contact.
- A folder pane that minimizes.
- **File › Save As**, **Save Attachments**, **Print** and **Exit**.
- High and Low importance and read receipts when writing. The list shows incoming mail's importance.

### Fixed
- A draft that can't be saved while offline is kept and saved when the account connects.
- Grouped list headings are spaced as headings, not messages.
- The ribbon keeps its last buttons in a narrow window.

## 0.8.0 - 2026-10-15

*POP and Outlook's own data files.*

### Added
- POP accounts (RFC 1939, over SSL/TLS or STARTTLS). New mail downloads once into folders on this computer and leaves a copy on the server unless you choose otherwise.
- **Use an existing data file**: a new POP account can start from an old `.pst` file.
- Each account keeps its mail in one data file (`.mailcraft`) in **Documents\MailCraft Files**. An opened `.pst` file gets one too.
- **File › Options › Data files** shows the data files' folder and moves it.
- Each account's mailbox settings show its data file and save a copy as a `.pst` file.

### Changed
- Mail kept by earlier versions moves into the data files by itself.

### Fixed
- A `.pst` file that Outlook has open says so.
- Opening an `.ost` file (Outlook's offline copy of a server account) explains what it is.
- Importing a `.pst` file skips the RSS Feeds folder and an empty Outbox.

## 0.7.0 - 2026-10-15

*The open gaps closed.*

### Added
- HTML mail laid out with CSS (flexbox, grids, inline-block, floats) keeps its columns.
- On-premises Exchange calendars and contacts, both ways, through Exchange Web Services (EWS).
- Autodiscover finds the Exchange server from the email address.
- In Arabic and Hebrew the window mirrors, with folders on the right and the reading pane on the left.
- Right-to-left paragraphs line up on the right while you write.
- Importing a `.pst` file brings tasks with a due date into the calendar.
- Exporting to `.pst` also writes the calendar and contacts beside it as `.ics` and `.vcf` files.

## 0.6.0 - 2026-10-15

*Outlook's reach.*

### Added
- New mail can open in a window of its own, and replies pop out.
- A tall **Send** button beside **From**, **To** and **Cc**. **To** and **Cc** open the address book (**Select Names**).
- Formatting from the **Message** ribbon or keyboard shortcuts: font size, bold, italic, underline, strikethrough, colours, lists, alignment, indent, links, pictures and undo.
- Replies keep the original message under a rule, with its own formatting.
- "Want to save your changes?" when closing an unsaved message.
- Exchange Online calendars and contacts, both ways, through Microsoft Graph.
- On-premises Exchange accounts through Exchange Web Services (EWS), signed in with NTLMv2 (Windows sign-in) or Basic: folders, sync, drafts, sending and automatic replies.
- Importing a `.pst` file brings in contacts and calendar items, recurrences included.
- Export any account's mail to a new `.pst` file.
- Newsletters laid out with tables keep their columns, widths, coloured cells and centring.
- Arabic and Hebrew: right-to-left text in mail and the interface reads in order, joined and aligned right, with a fallback font when needed.
- In the browser: push, and a cache kept between visits.
- Video previews without ffmpeg, from the system's own thumbnailer.
- Drag attachments out on Linux and the BSDs under X11.

## 0.5.0 - 2026-10-15

*Beyond mail.*

### Added
- Open a message in its own window with a double-click or **Open in new window**.
- The **File** page: account information, server settings, automatic replies, space used, emptying Deleted Items and Junk, rules, **Options** and **About**.
- A calendar with day, week and month views, recurring events and time zones, synced with CalDAV.
- Contacts, synced with CardDAV.
- Meeting invitations by mail: accept, tentative, decline, and invite guests.
- **File › Open & Export** opens a `.pst` file as a mailbox on this computer.
- Exchange Online accounts through Microsoft Graph: folders, sync, drafts, sending and automatic replies.
- A web build: the same window in the browser for JMAP accounts, with a `?sample` mailbox.

## 0.4.0 - 2026-10-15

*Media and Craft integration.*

### Added
- Attachment previews: text, attached mail, a PDF's first page and video frames (through ffmpeg).
- Open an attachment in the Craft app for its file type, or **Save all**.
- Drag attachments out to other apps on Windows and macOS.
- **Print** (through a PDF), **Save as PDF** and **Save as `.eml`**.
- Ten interface languages besides English: Spanish, French, German, Brazilian Portuguese, Italian, Russian, Japanese, Korean, and Simplified and Traditional Chinese. MailCraft follows the system or your choice in **Options**.
- JMAP accounts (RFC 8620/8621), with push.
- HTML mail keeps its colours and tables, adjusted for contrast.

## 0.3.0 - 2026-10-15

*Modern sign-in.*

### Added
- Sign in to Gmail and Microsoft 365 through the browser (OAuth 2.0 with PKCE), for IMAP and SMTP.
- Account settings found from the email domain (autoconfig, the Mozilla ISPDB, MX and SRV records).
- Faster folder opening on servers with QRESYNC, which sends deletions and flag changes as the folder opens.

## 0.2.0 - 2026-10-15

*Daily driver.*

### Added
- Create, rename, move and delete folders, and choose your favourites.
- An Outbox: mail that can't go out waits and sends on reconnect. A server's refusal stays with its error.
- Recipient suggestions from mail you have seen.
- Desktop notifications for new mail, and the unread count in the window title.
- Colour categories, pinning, snooze, inbox rules and Focused/Other.
- HTML mail shown formatted. Pictures from the web load only when you ask, remembered per sender.
- Rich-text writing with inline pictures, sent as HTML with a plain-text copy.

## 0.1.0 - 2026-10-15

*Read and send.*

### Added
- IMAP and SMTP accounts, with passwords in the system keychain.
- A unified inbox.
- A local cache that fetches only what changed, gets new mail as it arrives, and sends offline changes when you reconnect.
- A message list grouped by date, with conversations, filters, multi-select and drag to a folder.
- A reading pane on the right, at the bottom or off, with the conversation and picture previews.
- Reply, Reply all and Forward, with quoting and signatures.
- Drafts.
- Delete, archive, junk, move, flag and mark read.
- Offline search, with field operators such as `from:`.
- Dark mode.
- A command-line client.
- A sample mailbox.
- Packages for Windows, macOS and Linux.
