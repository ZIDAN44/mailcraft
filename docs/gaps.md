# Known gaps

This page lists what MailCraft still lacks against the everyday Outlook mail workflow, most-hit
first. Each item says who it affects, what to do meanwhile and why it happens.

The scope is [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177). See
[what comes next](../ROADMAP.md) and [what each version added](../CHANGELOG.md); a gap that gets
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

### 2. `.pst` files: what the libraries can't do

MailCraft imports and exports every item kind in `.pst` files (Outlook data files) through the
fork ZIDAN44/outlook-pst-rs, branch `craft/rw-1.2.14`.

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
- **Workaround:** On a standard macOS account, keep MailCraft in `~/Applications`.
- **Cause:** An Ed25519 signature on updates and releases needs a signing key that only the
  owner can create.

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

### 6. Key tips on the ribbon

- **Impact:** Pressing Alt (or F10) shows no letters on the ribbon's tabs and buttons, so its
  commands can't be reached by Alt letter sequences as in Outlook.
- **Workaround:** The keyboard shortcuts (Ctrl+N, Ctrl+R, Ctrl+Shift+V…), and Tab through the
  ribbon.
- **Cause:** Not built yet. GridCraft has the machinery (a table of key tips, its state and
  badges, generated tests); MailCraft's letters would have to be watched from Outlook's window,
  clean room.

### 7. Drafts: a draft twice after closing during its save

- **Impact:** On a server without UIDPLUS, or one that keeps a draft at another size than
  MailCraft sent, a draft saved just as MailCraft closes, or as the connection to the server is
  lost, can be in Drafts twice after the next connection: the copy that save took there, and the
  one kept on this computer.
- **Workaround:** Delete the older copy in Drafts.
- **Cause:** MailCraft replaces a copy of a draft only when it knows the copy as its own: one it
  filed, remembered by its Message-ID and size, so that another program's copy under the same
  Message-ID stays. It removes it only where it can remove exactly that message (UIDPLUS's
  `UID EXPUNGE`): a plain `EXPUNGE` removes whatever else is marked deleted in the folder.

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
