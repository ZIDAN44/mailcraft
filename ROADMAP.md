# Roadmap

**Stage:** v0.7 — the open gaps closed (2026-10-15). Each milestone below is a version: v0.1 was
the first working client, and every later one is named for what it added.
Scope: [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177) (lightweight,
local-first, media-aware v1); the fuller request is
[storytold/artcraft-services#80](https://github.com/storytold/artcraft-services/issues/80).

## Headline numbers

| | |
|---|---|
| Crates | 8 libraries + 3 apps + xtask |
| Commands | 113 (`mailcraft-cli commands`) |
| Tests | 218 (unit, in-memory IMAP, JMAP, Graph, EWS (mail, calendars, contacts, Autodiscover), CalDAV/CardDAV and ManageSieve servers, NTLM test vectors, SMTP and HTTP stubs, data files, TCP end-to-end, an X server drag, headless UI) |
| Languages | 13 (English, Spanish, French, German, Brazilian Portuguese, Italian, Russian, Japanese, Korean, Simplified and Traditional Chinese, Arabic, Hebrew) |
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
8. **Next:** what's left in [docs/gaps.md](docs/gaps.md): a relay for IMAP accounts in the
   browser, bidirectional editing, Wayland drag-out, what the `.pst` libraries can't do yet.

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
