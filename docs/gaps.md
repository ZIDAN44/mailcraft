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

### 8. Running with the window closed, on Linux and macOS

- **Impact:** **Keep MailCraft running when its window is closed** (File › Options › General) is
  offered on Windows only. On Linux and macOS, closing the window ends MailCraft, and mail isn't
  checked or announced until it's opened again.
- **Workaround:** Minimize the window rather than close it: a minimized MailCraft goes on
  checking for mail and announcing it.
- **Cause:** The option puts MailCraft's icon in the system's notification area, to open it from
  again. On Linux that takes GTK's event loop beside the window's own, or a StatusNotifierItem
  that GNOME shows only with an extension; on a Mac, an app whose window is closed stays in the
  Dock, which MailCraft's quit there doesn't do yet. Neither is built or tested.

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
