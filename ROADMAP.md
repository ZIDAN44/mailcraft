# Roadmap

**Stage:** v0.13 — The open gaps, again (2026-10-17). Each milestone below is a version: v0.1 was
the first working client, and every later one is named for what it added.
Scope: [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177) (lightweight,
local-first, media-aware v1); the fuller request is
[storytold/artcraft-services#80](https://github.com/storytold/artcraft-services/issues/80).

## Headline numbers

| | |
|---|---|
| Crates | 8 libraries + 3 apps + xtask |
| Commands | 130 (`mailcraft-cli commands`) |
| Tests | 218 (unit, in-memory IMAP, JMAP, Graph, EWS (mail, calendars, contacts, Autodiscover), CalDAV/CardDAV and ManageSieve servers, NTLM test vectors, SMTP and HTTP stubs, data files, TCP end-to-end, an X server drag, headless UI) |
| Languages | 14 (English, Spanish, French, German, Dutch, Brazilian Portuguese, Italian, Russian, Japanese, Korean, Simplified and Traditional Chinese, Arabic, Hebrew) |
| Platforms | Windows, macOS, Linux; the browser (JMAP accounts) |

## Milestones

1. **v0.1 — Read and send** ✅ accounts, sync, reading, writing, organising, search, sample
   mailbox, CLI, packaging.
2. **v0.2 — Daily driver** ✅ rich-text compose, formatted HTML view with per-sender pictures,
   outbox, folder management, notifications, contacts autocomplete, categories, rules, snooze.
3. **v0.3 — Modern sign-in** ✅ OAuth2 (Gmail, Microsoft 365), autoconfig, QRESYNC.
4. **v0.4 — Media and Craft integration** ✅ attachment previews and thumbnails (PDF, video),
   "Open in Craft app", dragging attachments out, print and PDF export, eleven languages, JMAP
   with push.
5. **v0.5 — Beyond mail (deferred by #2177)** ✅ calendar and contacts (CalDAV/CardDAV, meeting
   invitations), the File page (automatic replies, mailbox size), messages in their own windows,
   Exchange Online through Microsoft Graph, Outlook data files, the browser version.
6. **v0.6 — Outlook's reach** ✅ Outlook's message editor and windows, Exchange calendars and
   contacts (Graph), on-premises Exchange (EWS, NTLM), `.pst` contacts, calendar and export,
   newsletter layouts, Arabic and Hebrew, browser push and a kept cache, video frames and
   drag-out everywhere.
7. **v0.7 — The open gaps closed** ✅ CSS layouts, EWS calendars, contacts and Autodiscover,
   mirrored right-to-left windows, `.pst` tasks and the calendar and contacts exported beside it.
8. **v0.8 — POP and Outlook's own data files** ✅ POP accounts with folders on this computer,
   starting from an existing `.pst`, and real Outlook `.pst` files checked end to end.
9. **v0.9 — Outlook's everyday commands** ✅ From an audit against Outlook: Arrange by, a Send /
   Receive tab with Work offline, Undo, more filters, Ignore, importance and read receipts, File's
   Save As and Print, POP copies removed after days; drafts no longer lost offline.
10. **v0.10 — Windows, folders and rules as Outlook keeps them** ✅ Remembered window places and
   sizes, subfolders under their parents, rules run at once, attachments in message windows; from
   a second audit: the Search tab, Follow Up dates, Delay delivery and Replies to, Outlook's menu
   items, hover everywhere.
11. **v0.11 — Notes, previews and Exchange beyond the mailbox** ✅ From issue #1: attachments
   previewed in the reading pane (Word, Excel and PowerPoint too), Notes, three People layouts,
   other people's Exchange calendars, public folders, Dutch; Outlook.com and Hotmail say why
   they need Microsoft sign-in.
12. **v0.12 — Microsoft sign-in that works** ✅ Microsoft sign-in tried end to end: the sign-in page
   opens in the browser on Windows, accounts that sign in that way ask to sign in again rather
   than for a password, an account Microsoft refuses says why, sent mail shows in Sent Items at
   once; dialogs and the folder pane keep their sizes.
13. **v0.13 — The open gaps, again** ✅ From an audit of [docs/gaps.md](docs/gaps.md): other
   people's Exchange calendars can be changed where they allow it, HTML mail's background
   pictures and boxes placed over others, text in its cell's colour, Outlook's notes from a data
   file, right-to-left mail copied as written; calendar boxes that fit their lines.
14. **Next:** what's left in [docs/gaps.md](docs/gaps.md): a relay for IMAP accounts in the
   browser, bidirectional editing, Wayland drag-out, Kerberos-only Exchange, what the `.pst`
   libraries can't do yet.

## Progress log

- **2026-10-15** — **v0.1**: workspace, model (threading, search, dates, UTF-7), MIME, store, IMAP client
  and sync (CONDSTORE, IDLE, offline replay), SMTP, worker pool, engine with 44 commands, egui UI
  (three panes, ribbon, dialogs, dark mode), desktop app (keychain, file dialogs), CLI, docs, CI.
- **2026-10-15** — **v0.1**: packaging and release workflows (Windows MSI incl. ARM64, macOS dmg, Linux
  AppImage/deb/rpm/Flatpak, FreeBSD) on the shared Craft pipelines; app icon; `--version`.
- **2026-10-15** — **v0.2–v0.4** (the gap list): folder management, Outbox, QRESYNC, contacts autocomplete,
  notifications, categories / pins / snooze / rules / Focused, an HTTP client, formatted HTML with
  pictures on request, rich-text compose with inline pictures, autoconfig (ISPDB, MX, SRV), OAuth
  2.0, attachment previews and Craft apps, print / PDF / `.eml`, seven interface languages, and
  JMAP as a second transport.
- **2026-10-15** — **v0.4–v0.5**: pop-out message windows and the File page (automatic replies over ManageSieve
  and JMAP, quotas, emptying folders); JMAP push; HTML colours and tables; dragging attachments
  out; PDF and video thumbnails; Italian, Russian, Korean and Traditional Chinese; calendar and
  contacts (iCalendar, vCard, CalDAV/CardDAV, invitations).
- **2026-10-15** — **v0.5**: Outlook data files (`.pst`) open as mailboxes; Exchange Online through
  Microsoft Graph; the store in memory, JMAP as a portable crate, and the browser version
  (`apps/mailcraft-web`).
- **2026-10-15** — **v0.6**: Outlook's message editor and windows; Exchange calendars and contacts; `.pst`
  contacts, calendar and export; video frames from the system; browser push and IndexedDB;
  Arabic and Hebrew with right-to-left text; newsletter layouts; on-premises Exchange through
  EWS with NTLM; drag-out under X11.
- **2026-10-15** — **v0.7**: CSS columns (flexbox, grid, inline-block, floats); EWS calendars,
  contacts and Autodiscover; mirrored windows for Arabic and Hebrew; `.pst` tasks, and the
  calendar and contacts exported beside the `.pst`.
- **2026-10-15** — **v0.8**: POP accounts (downloaded mail in folders on this computer, a copy left
  on the server), "use an existing data file", and real Outlook `.pst` files checked: open files,
  `.ost` files, RSS Feeds and the Outbox handled; each account in a `.mailcraft` data file in
  Documents\MailCraft Files.
- **2026-10-15** — **v0.9**: an audit against Outlook: Arrange by and Reverse sort, a Send / Receive
  tab with Work offline, Undo, Filter (categorized, important, to me), Ignore a conversation, New
  items, a minimizable folder pane, File › Save As / Save Attachments / Print / Exit, importance
  and read receipts, POP server copies removed after days or once deleted; drafts that can't
  reach the server are kept.
- **2026-10-15** — **v0.10**: windows open where they were (maximized too), a folder made in the
  Inbox shows under it, a new rule runs at once on the Inbox, attachments as Outlook's chips that
  open with a double-click, and notifications for mail rules file away.
- **2026-10-16** — **v0.10**: a second audit against Outlook: a Search tab, Follow Up with dates,
  Delay delivery and Replies to, Forward as attachment, Reply with meeting, Find related, Delete
  all and folder Properties, hover on every control; CJK names drawn in any interface language.
- **2026-10-17** — **v0.11**: issue #1: attachments preview in the reading pane (pictures, PDF
  pages, text, Word, Excel, PowerPoint), Notes, three People layouts, other people's Exchange
  calendars, public folders, Dutch; Outlook.com and Hotmail say why they need Microsoft sign-in.
- **2026-10-17** — **v0.12**: Microsoft sign-in tried end to end (Outlook.com over IMAP, a
  personal Microsoft account through Graph): the sign-in page opens on Windows, "Sign in again"
  instead of a password, refusals say why, Sent Items catches up at once; dialog and pane sizes.
- **2026-10-17** — **v0.13**: the gaps audited again: shared Exchange calendars editable where
  allowed (Graph and EWS), background pictures and placed boxes in HTML mail, cell colours, notes
  from `.pst` files, right-to-left text copied from a right-click; Exchange Online's public
  folders wait on Microsoft (EWS is being retired there).
