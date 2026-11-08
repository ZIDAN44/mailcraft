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

### 6. Drafts: an older copy marked deleted on a server without UIDPLUS

- **Impact:** On an IMAP server without UIDPLUS, the copy of a draft that a newer save replaces
  is marked deleted rather than removed. MailCraft doesn't show it, but another program may show
  it, crossed out, until the Drafts folder is expunged.
- **Workaround:** Purge or compact Drafts in a program that does, or leave it to the server.
- **Cause:** Without UIDPLUS's `UID EXPUNGE`, the only way to remove one message is a plain
  `EXPUNGE`, which removes whatever else is marked deleted in the folder, other programs' mail
  too; moving each copy to Deleted Items instead would fill it with every version saved.

### 7. Typing a time on a 12-hour clock

- **Impact:** With File › Options set to a 12-hour clock, the boxes you type a time into (an
  event's start and end, Delay delivery) still take it as `HH:MM` on a 24-hour clock; every time
  MailCraft shows is on the 12-hour clock.
- **Workaround:** Type the time on a 24-hour clock (`13:30` for 1:30 PM).
- **Cause:** The boxes read and write `HH:MM` only; reading `1:30 PM` (and each language's way of
  writing it) isn't done yet.

### 8. Newsletters: what the layout doesn't draw yet

- **Impact:** A button, a link with padding and round corners or Outlook's own copy of one in
  VML, shows as its words on its colour: without its round corners, and not at the size its VML
  gives it.
  Columns that set no width share their row equally, where a browser sizes them by what they
  hold.
  A newsletter whose copy for Outlook has more than 200,000 elements, nests them more than 256
  deep, or nests more than a dozen tables one in another, shows the copy it gives other programs,
  as before MailCraft read Outlook's.
- **Workaround:** Most newsletters have a link at the top to see them in a browser.
- **Cause:** The layout keeps text as formatted lines and boxes as table cells, and draws no round
  corners; a link with its own room and size, and a column's width taken from its content, need a
  measure of the content the layout doesn't take yet. Outlook's copy is read into a tree of
  MailCraft's own, which stops at those sizes to stay quick; a copy cut there would run its
  paragraphs together, and tables nested deeper are slow to lay out as tables.

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
