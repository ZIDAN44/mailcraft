# Known gaps

This page lists what MailCraft still lacks against the everyday Outlook mail workflow, most-hit
first. Each item says who it affects, what to do meanwhile and why it happens.

See [what comes next](../ROADMAP.md) and [what each version added](../CHANGELOG.md); a gap that gets
fixed moves to the changelog.

## Open

### 1. Right-to-left in dialogs, menus and the editor

- **Impact:** In Arabic and Hebrew, the mail window and its panes mirror, but dialogs, Calendar,
  People and Notes don't. Menu items stay on the left. In the editor, a line that mixes
  right-to-left and left-to-right words shows them in the wrong order.
- **Workaround:** None yet.
- **Cause:** The menus need a one-line change, `top_down_justified(Align::Min)`, in
  storytold/egui's `containers/menu.rs`. The editor shows mixed lines in the order egui lays them
  out.

### 2. `.pst` files: a moved data folder

MailCraft imports and exports every item kind in `.pst` files (Outlook data files).

- **Impact:** If you move a data folder to a new path, the `.mailcraft` copies it keeps beside
  opened `.pst` files look free to any other data folder.
- **Workaround:** None yet.
- **Cause:** Each copy records the path of the data folder that owns it. After a move, nothing is
  at that path any more.

Some things are left out on purpose; they are listed under [Not planned](#not-planned).

### 3. Dragging attachments out on a Wayland-only desktop

- **Impact:** You can't drag an attachment out of MailCraft on a Wayland-only desktop, or when
  `MAILCRAFT_WAYLAND=1` is set. Dragging works on Windows, macOS and X11.
- **Workaround:** Run under X11 or XWayland (the default), or use **Save**.
- **Cause:** winit 0.30 can't start a drag on Wayland. winit 0.31 can, with
  `ActiveEventLoop::start_drag` (rust-windowing/winit#4571), once egui and the Craft forks move
  to it.

### 4. Signed updates and releases

Test builds update themselves ([how test builds update](updates.md)). These parts are missing:

- **Impact:**
  - An update is checked only by its SHA-256 checksum. That proves the download is intact, not
    who made it.
  - On a standard (non-administrator) macOS account, a copy in `/Applications` isn't replaced.
  - Every tester gets a new build within the hour. There is no staged rollout.
  - Each update downloads the whole installer. There are no delta downloads.
  - On macOS, after each update the system asks once whether MailCraft may read its passwords
    from the keychain.
- **Workaround:** On a standard macOS account, keep MailCraft in `~/Applications`. On macOS,
  choose **Always Allow** when asked (it lasts until the next update).
- **Cause:** An Ed25519 signature on updates and releases needs a signing key that only the
  owner can create. The keychain ties an app's passwords to its Apple developer team, and a
  build signed without an Apple certificate has none, so each build is a new app to it; a
  designated requirement by bundle id doesn't change that.

### 5. OpenPGP: what isn't done yet

- **Impact:** Your own OpenPGP key works in the desktop app only: in the browser, OpenPGP mail
  sent to you stays locked and can't be signed or encrypted with OpenPGP. Inline PGP is read only
  in a message that is one plain text (quoted-printable or 8-bit); a PGP block in an HTML message,
  in a base64 text, or beside attachments isn't. MailCraft encrypts with version 4 keys
  (SEIPDv1), so a correspondent with only a version 6 key can't be encrypted to. A key's
  trust comes from where it came from (their own mail, or a file you imported): there's no web of
  trust and no key servers.
- **Workaround:** Read OpenPGP mail in the desktop app; ask for version 4 keys.
- **Cause:** The browser keeps no secret keys yet (its S/MIME certificates' way, a file locked
  with its password, would need an unlock dialog of its own). The rest is left for later.

### 6. Drafts: an older copy marked deleted on a server without UIDPLUS

- **Impact:** On an IMAP server without UIDPLUS, the copy of a draft that a newer save replaces
  is marked deleted rather than removed. MailCraft doesn't show it, but another program may show
  it, crossed out, until the Drafts folder is expunged.
- **Workaround:** Purge or compact Drafts in a program that does, or leave it to the server.
- **Cause:** Without UIDPLUS's `UID EXPUNGE`, the only way to remove one message is a plain
  `EXPUNGE`, which removes whatever else is marked deleted in the folder, other programs' mail
  too; moving each copy to Deleted Items instead would fill it with every version saved.

### 7. Newsletters nested past what's read

- **Impact:** A newsletter whose copy for Outlook has more than 200,000 elements, nests them more
  than 256 deep, or nests more than two dozen tables one in another (real newsletters nest about
  four to eight), shows the copy it gives other programs (as a browser shows it), not Outlook's.
- **Workaround:** None needed for most: that copy is the one browsers show. Most newsletters also
  have a link at the top to see them in a browser.
- **Cause:** Outlook's copy is read into a tree of MailCraft's own, which stops at those sizes so
  that hostile or huge mail stays quick: a copy cut there would run its paragraphs together, and
  past two dozen tables, each one nested deeper makes its text slower to read.

### 8. EteSync

- **Impact:** MailCraft can't sign in to an EteSync account itself. EteSync encrypts contacts,
  calendars, tasks and notes end to end, with its own protocol (Etebase).
- **Workaround:** Run EteSync's own bridge, `etesync-dav`, which offers the account as a calendar
  and contacts server on this computer. In MailCraft, add an account and, under its form, click
  **Add a CalDAV and CardDAV account**: the address `http://localhost:37358/`, the EteSync user
  name, and the password the bridge shows (not the EteSync password). Calendars and
  contacts sync both ways. Lists of tasks don't (MailCraft syncs tasks with Exchange only), and
  the bridge doesn't offer notes.
- **Cause:** It needs an Etebase client of MailCraft's own: the Rust library EteSync publishes
  brings C libraries with it (OpenSSL and libsodium), which MailCraft doesn't build with.

### 9. Spelling: what isn't checked yet

- **Impact:** Spelling is checked as you type in the interface's language only, and in the
  message's text only (not its subject). There's no Spelling window to go through a message's
  mistakes one by one (Outlook's F7), and no check before sending. English uses the Craft apps'
  built-in word list, which takes a few common misspellings (`thier`, `untill`, `occured`) as
  words. In the browser, only English is checked.
- **Workaround:** Install a Hunspell dictionary for your language (Linux: the `hunspell-xx`
  package; macOS: `.aff` and `.dic` files in `~/Library/Spelling`; any system: LibreOffice and its
  language's dictionary). An English Hunspell dictionary installed there is used instead of the
  built-in list.
- **Cause:** The checker is the Craft apps' shared `craft-proof`. Its better English (SCOWL) and
  its other languages' data come from storytold's `craft-i18n` at build time, which MailCraft's
  builds don't include yet; it brings no dictionaries of its own for other languages.

### 10. Screen readers: what isn't read yet

- **Impact:** A window of its own (a new message, which opens in one unless turned off, and a
  message opened with Enter or a double-click) shows a screen reader nothing at all. In the main
  window, a message's text is read as one text, by line, word and letter, but not what it's made
  of: a heading, a list's items, a table's cells and a link are read as lines of text, with
  nothing saying what they are, and a screen reader can't go from heading to heading.
- **Workaround:** Turn off **New messages in their own window** in **File › Options › Mail**
  (replies stay in the main window as they start), and read messages in the reading pane rather
  than opening them. Tab goes to each link in a message, read as a link.
- **Cause:** egui's eframe gives only the main window an adapter for screen readers; a fix for
  the others is in review upstream (emilk/egui#8810). A message's text is drawn in runs laid out
  from it, which don't keep its headings, lists and tables as such; a screen reader needs them
  told, as WordCraft tells its page's.

### 11. Archiving: what isn't done yet

- **Impact:** **Clean up old items** and AutoArchive take mail only: old calendar items and
  completed tasks stay where they are, where Outlook archives them too. A message can't be kept
  out of AutoArchive on its own (Outlook's "Do not AutoArchive" on an item), and AutoArchive
  doesn't delete expired items.
- **Workaround:** Give a folder that holds what must stay **Don't archive this folder** in its
  **Properties**, or move that mail to such a folder.
- **Cause:** The archive writes a folder's mail as an export does, without the calendar, contacts
  and tasks, which an export writes whole. No server keeps a message's own AutoArchive setting
  for MailCraft to read, nor does MailCraft keep settings of its own for single messages, and it
  keeps no expiry for mail.

### 12. Work time: what isn't done yet

- **Impact:** A week always starts on Monday, in the calendar and its months: Outlook's **First
  day of week** isn't there. Your work time is kept on this computer alone: one set in Outlook
  for an Exchange or Microsoft 365 mailbox isn't read, and colleagues see the one Outlook set,
  not MailCraft's. The work hours are the same each day worked.
- **Workaround:** Set the same work time in **File › Options › Calendar** as in Outlook.
- **Cause:** The calendar's views, months and dates are laid out from Monday in each place that
  draws them, not from a setting. MailCraft doesn't read or write a mailbox's own settings
  (Exchange's `GetUserConfiguration`, Microsoft Graph's `mailboxSettings`) yet.

### 13. Finding a time: what isn't done yet

- **Impact:** A meeting's own form has no Scheduling Assistant: its guests' times and AutoPick
  are in the calendar alone (**Next free time**, over the calendars shown). Everyone shown counts
  as required, and rooms aren't looked at. Exchange's own suggested times aren't asked for.
- **Workaround:** Show the guests' calendars (**Open shared calendar**), find the time with
  **Next free time**, and click it for the meeting; leave an optional guest's calendar unticked.
- **Cause:** The event form edits one event and reads no one's times; the search goes over the
  times the calendar has, which works for every kind of account, where the servers' own
  suggestions (Exchange's `SuggestionsViewOptions`, Microsoft Graph's `findMeetingTimes`) come
  from those servers alone.

### 14. Message list groups: what isn't done yet

- **Impact:** A heading folded stays folded as it was left: Outlook's **Expand/collapse
  defaults** (all expanded or all collapsed each time a folder opens) isn't there, and Left and
  Right don't fold or show a heading from the keyboard as Outlook's do.
- **Workaround:** **Expand all groups** in the list's **By Date ⏷** menu shows everything again.
- **Cause:** What's folded is kept as the folder was last shown, the one way there is; the
  arrow keys move between messages, never stopping on a heading.

### 15. Moving an event: what isn't done yet

- **Impact:** An event that repeats isn't dragged to another time (Outlook moves the occurrence
  dragged, on its own), nor is an event's length changed by dragging its edge. A meeting someone
  else organized isn't dragged either.
- **Workaround:** Open the event and change its start and end in its form: a repeating one
  moves as a whole there.
- **Cause:** Nothing changes one occurrence of a series on its own yet: the form and the
  calendar change the whole series, and the series moved for one occurrence went wrong on its
  days (Monday's moved to Tuesday, the rule still said Mondays).

## Not planned

These are outside MailCraft's control, or accepted as they are.

- **IMAP and SMTP accounts in the browser.** A browser can't open those connections. It would
  take a separate relay server.
- **Public folders on Exchange Online.** They can only be reached through EWS (Exchange Web
  Services), which Microsoft is retiring.
- **S/MIME revocation checks, smart cards and the system certificate store in the browser.**
  Browsers don't allow them.
- **Speed on very large `.pst` files.** Files of tens of GB import a little slower than our
  targets and use up to about 1 GB of memory.
- **Outlook's rules in `.pst` files.** They aren't imported or exported.
- **Linked files on another computer or the web.** `.pst` import and export leave them out.
- **Repeat rules Outlook can't express.** Export writes such a series as its occurrences, up to
  2,000 of them.
- **The system's Share menu.** Windows 11's Share window lists only apps installed as signed MSIX
  packages, and the Mac's Share menu only apps with a signed extension; GNOME Files and COSMIC
  Files take no menu entry from a package, and a Flatpak or an AppImage can't add one anywhere.
  Instead: **Send to › MailCraft** in Explorer (added for whoever installs or updates MailCraft;
  another user on the computer can copy it into their own `shell:sendto` folder), **Send with
  MailCraft** in Dolphin, Nemo and PCManFM-Qt, and files dropped on MailCraft's Dock icon or
  window.
