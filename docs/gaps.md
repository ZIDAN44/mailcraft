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
  `MAILCRAFT_WAYLAND=1` is set, nor a message from the list onto a message being written in a
  window of its own (#270). Dragging works on Windows, macOS and X11.
- **Workaround:** Run under X11 or XWayland (the default), or use **Save**.
- **Cause:** winit 0.30 can't start a drag on Wayland. winit 0.31 can, with
  `ActiveEventLoop::start_drag` (rust-windowing/winit#4571), once egui and the Craft forks move
  to it. Wayland doesn't tell an app where its windows are, so a message let go over another of
  its windows can't be told from one let go anywhere else.

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
  contacts sync both ways, and task lists the bridge offers as a CalDAV server's (not tried
  through the bridge); it doesn't offer notes.
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

- **Impact:** Every guest counts as required, in a meeting's Scheduling Assistant and in the
  calendar's **Next free time**, and rooms aren't looked at. The meeting's band in the assistant
  is moved with a click, not dragged, and shows its day alone. Exchange's own suggested times
  aren't asked for.
- **Workaround:** Leave an optional guest out while finding the time, and add them after; for a
  day other than the meeting's, change its date.
- **Cause:** The search goes over the times MailCraft has (the calendar's, and the guests'
  free/busy times), which works for every kind of account, where the servers' own suggestions
  (Exchange's `SuggestionsViewOptions`, Microsoft Graph's `findMeetingTimes`) come from those
  servers alone; MailCraft's guests have no optional kind yet.

### 14. Message list groups: what isn't done yet

- **Impact:** A heading folded stays folded as it was left: Outlook's **Expand/collapse
  defaults** (all expanded or all collapsed each time a folder opens) isn't there, and Left and
  Right don't fold or show a heading from the keyboard as Outlook's do.
- **Workaround:** **Expand all groups** in the list's **By Date ⏷** menu shows everything again.
- **Cause:** What's folded is kept as the folder was last shown, the one way there is; the
  arrow keys move between messages, never stopping on a heading.

### 15. Moving an event: what isn't done yet

- **Impact:** An event's length isn't changed by dragging its edge, and a meeting someone else
  organized isn't dragged (its time is theirs to change).
- **Workaround:** Open the event and change its end in its form.
- **Cause:** The calendar drags an event's box only, not its edges.

### 16. People's photos: what isn't shown yet

- **Impact:** On Exchange Online (Microsoft 365) and Gmail, colleagues show their initials, not
  the photos their organisation has. Your own photo isn't shown for your account, and a
  contact's photo can't be added or changed in MailCraft.
- **Workaround:** Save the photo in the person's contact elsewhere (Outlook, a phone, the
  address book's website): it shows here once the address book syncs.
- **Cause:** Only on-premises Exchange is asked for photos (`GetUserPhoto`); Microsoft Graph's
  own photo request isn't made yet, and the contact form has no picture in it.

### 17. Meetings: what isn't done yet

- **Impact:** A new time for one occurrence of a series can't be proposed from its invitation,
  nor taken when a guest proposes one. An account with calendars only (no mail) can't delegate a
  meeting. The meeting form doesn't show the guests' free/busy times beside it, as Outlook's
  Scheduling Assistant does. Invitations already in the Inbox aren't offered as an account is
  added: Invitations in the Inbox… in the calendar finds them.
- **Workaround:** Change the occurrence in the calendar: its guests are sent it. Delegate from an
  account with mail, or in the server's web calendar. Open the guests' calendars in the Schedule
  view (Open shared calendar…) to find a time, and make the meeting from there.
- **Cause:** Taking a proposal moves the meeting as a whole; an occurrence's proposal needs that
  occurrence changed on its own. A delegate is told by the account's mail: without mail, the
  server would have to tell them, and Nextcloud and Stalwart don't (Nextcloud doesn't tell the
  organizer whom either). The form is a dialog of its own, apart from the calendar's views.

### 18. Tasks on CalDAV servers: what isn't done yet

- **Impact:** A task's due or start time of day (Nextcloud's Tasks can give one) shows beside
  its day, and stays when the day is changed here, but can't be set or changed here. A server
  with no task list (a new Nextcloud account, before its
  Tasks app makes one) keeps none of MailCraft's tasks: they stay on this computer. MailCraft
  doesn't make, rename or delete task lists, nor sync subtasks as such, nor Microsoft To Do.
- **Workaround:** Make the task list in the server's own web page (Nextcloud's Tasks makes one
  when opened); MailCraft finds it at its next sync. Set due times there.
- **Cause:** Tasks are kept by day, as Outlook's are. A collection is taken for a task list only
  when it says it takes `VTODO`s: one that says nothing (SOGo's calendars' `.ics` and `.xml`
  views) isn't, so tasks don't go where they'd be lost.

### 19. Everything with one person: what isn't done yet

- **Impact:** Its files and links come from the mail with them that's on this computer, the
  newest 300 messages; mail not downloaded yet (an account kept online only) isn't counted, and
  it says how many. Its search finds a word of a message's text in those 300; older mail by its
  subject, names and first lines. The web addresses in their signature are shown, not offered
  for their contact; only the newest of their messages with a signature is read. Someone
  writing from an address their contact doesn't have shows apart.
- **Workaround:** Open a message to download it. Add their other addresses to their contact to
  see them as one person.
- **Cause:** It's made from what's on this computer, nothing asked of a server. The contact form
  has Outlook's places for phone numbers, not yet one for a web page.

### 20. AI agents: what isn't done yet

- **Impact:** With MailCraft closed, agents read its data folder but run no command and see no
  window: MailCraft isn't started for them. What they read is what's on this computer: a message
  not downloaded yet is listed without its text, and a search reads the text of 1,000 messages
  at most, saying how many it left.
- **Workaround:** Open MailCraft for an agent to run commands; open a folder to download its
  mail.
- **Cause:** Only the open window asks you about a command, so nothing runs while it's closed.
- **Impact:** An organisation's profile can turn agents off, let them only read, or have every
  command asked about, but can't let them reach only some accounts (its own domain's): which
  accounts agents read is each person's choice.
- **Workaround:** Each person turns accounts off under **File › Options › AI agents**.
- **Cause:** The accounts kept from agents are the person's own setting, by account; a profile
  names domains, and isn't read where that setting is checked yet.

### 21. An organisation's profile: what isn't done yet

- **Impact:** A profile can't set the interface's font, nor point updates at a mirror of the
  organisation's own; the browser build reads none. A signature's details come from Exchange's
  or Microsoft 365's directory, else from what the person gives in File › Info (no LDAP
  directory is asked). Locks hold in MailCraft on this computer, not for someone with an
  administrator's rights or another mail program.
- **Workaround:** Updates can be turned off for the organisation to deploy each version itself.
  What must hold everywhere (a disclaimer, encryption) is set on the mail server too.
- **Cause:** The interface font waits for fonts to be chosen at all (#166); a mirror for updates
  being signed (#233). Locks are a program's own, which its user's system can override.

### 22. Work and school accounts: Need admin approval

- **Impact:** Where an organisation lets people approve only apps from a verified publisher,
  **Sign in with Microsoft** stops at **Need admin approval**, and the account can't be added.
- **Workaround:** An administrator approves MailCraft once for everyone, or the organisation
  gives its own app in its profile ([how](oauth.md#your-organisation-needs-to-approve-mailcraft)).
- **Cause:** MailCraft's Microsoft app has no verified publisher. That takes a publisher verified
  in Microsoft's partner programme.

### 23. Starting at sign-in: a portable copy, Flatpak and Snap, a Mac's Login Items

- **Impact:** **File › Options › General › Start MailCraft when you sign in** isn't offered in a
  portable copy, nor in the Flatpak or a Snap. On a Mac, MailCraft turned off under **System
  Settings › General › Login Items › Allow in the Background** still shows the option on.
- **Workaround:** A portable copy on Windows: a shortcut to `mailcraft.exe` in the Startup folder
  (type `shell:startup` in Explorer's address bar). The Flatpak: the desktop's own list of apps
  started at sign-in (GNOME Tweaks' Startup Applications, KDE's Autostart). On a Mac, turn it
  on again there.
- **Cause:** A portable copy leaves nothing on the computer, and the path it would name changes
  with the USB stick's drive letter. A sandbox starts an app at sign-in only through its Background
  portal, which MailCraft doesn't ask yet. macOS keeps Login Items' switches where apps can't read
  them.

### 24. Backups on a schedule

- **Impact:** MailCraft on a computer doesn't back itself up (#266); in the browser, **File › Open
  & Export › Back up…** saves everything as one file, when asked.
- **Workaround:** A backup program that copies MailCraft's two folders, with MailCraft closed or
  from a snapshot of the disk ([which folders](../README.md#build-and-run)); **Export to Outlook
  data file** for an account's mail as a `.pst`.
- **Cause:** On a computer, MailCraft's data is files a backup program copies already. A backup
  MailCraft made itself while it runs would have to read each data file as it was at one moment,
  while it goes on writing to it; the browser's backup is made in memory, too small a place for a
  computer's mailboxes.

### 25. Tables in a message being written: what isn't done yet

- **Impact:** A table pasted into a message (#268) goes as it is: its cells can't be edited in
  MailCraft, nor rows or columns added. A cell merged down several rows keeps to its first row,
  and a picture in a cell is left out. A table can't be made from nothing (Outlook's
  **Insert › Table**).
- **Workaround:** Change the table where it was made (Excel, the web page) and paste it again;
  Backspace after a table takes it out.
- **Cause:** The message editor holds text and its formatting; a table is kept beside the text
  as the HTML it goes out as, drawn where its text stands, which the editor doesn't edit.

### 26. A portable copy's notifications in the notification centre

- **Impact:** On Windows, a portable copy's new-mail notification clicked once its banner has
  gone, in the notification centre, does nothing; one clicked while it shows brings MailCraft
  forward at that mail (#273). An installed MailCraft's does from the notification centre too.
- **Workaround:** Click MailCraft on the taskbar or in the notification area; the mail is at the
  top of the Inbox.
- **Cause:** Windows answers a click in the notification centre by starting the program a link
  in the notification is registered for (`mailcraft:`). The installer registers it for everyone;
  a portable copy writes nothing to the registry, and Windows doesn't start a link one program
  registers for its user alone from a notification.

### 27. Moving an event to another calendar: what isn't done yet

- **Impact:** An event moves to another calendar of its account from its form's **Calendar** box
  (#279), not by dragging it onto a calendar in the side pane or from a menu on it. Exchange's
  calendars (Microsoft 365, on-premises Exchange) aren't moved between. A meeting on a CalDAV
  server that won't move items stays in its calendar, and one on the server isn't moved to this
  computer's.
- **Workaround:** Open the event and pick its calendar. On Exchange, move it in Outlook on the
  web.
- **Cause:** The calendar views have no menu on an event, and dragging stops at their own days
  and times. Moving between Exchange's calendars takes Graph's and EWS's own move, not written
  yet. A meeting is moved with a WebDAV MOVE, which tells no one; copied and deleted instead, a
  server that schedules sends the guests the meeting again (Stalwart did), or a cancellation.

### 28. Rules on other headers

- **Impact:** A rule can look at a message's mailing list (its `List-Id`, #288), but not at any
  other header (`X-Mailer`, a header a service adds). A List-Id rule can't be an Exchange
  server's rule.
- **Workaround:** A rule on the sender, the recipients or the subject; for a list, its List-Id.
- **Cause:** MailCraft's own rules run on the message list's entries, which keep a few headers,
  not all; Exchange's server rules have no test of one header, only of words anywhere in them.

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
- **Windows Server 2016, and Windows 10 before version 1709, without a graphics card** (over
  Remote Desktop, a virtual machine without one): the window can't open (#256). MailCraft draws
  with DirectX 12 through wgpu, which needs more than those versions' own software renderer
  has. A graphics card whose driver has DirectX 12 draws it there (over Remote Desktop, with the
  policy that lets its sessions use it), as does Windows 10 version 1709, Windows Server 2019 or
  later.
