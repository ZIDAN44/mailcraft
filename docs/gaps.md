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

- **Impact:** A message's text is read a block at a time when it gets the keyboard (a plain-text
  message all at once): a screen reader can't go through it by line, word or character, or hear
  its headings, lists and tables as such. What a command does (a message moved, a setting
  turned on) isn't announced.
- **Workaround:** Tab from the message list to the message's text; Up and Down there go on to the
  next block. A drop-down's choice is changed with Up and Down without opening it.
- **Cause:** egui hands a screen reader text it can go through by line and character only in its
  text boxes; a message's text is laid out as labels, and describing it as a document, as
  WordCraft does its page, is work of its own.

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
