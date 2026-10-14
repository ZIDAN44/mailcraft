# Roadmap

**Stage:** v0.1 — first working version (2026-10-15).
Scope: [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177) (lightweight,
local-first, media-aware v1); the fuller request is
[storytold/artcraft-services#80](https://github.com/storytold/artcraft-services/issues/80).

## Headline numbers

| | |
|---|---|
| Crates | 7 libraries + 2 apps + xtask |
| Commands | 82 (`mailcraft-cli commands`) |
| Tests | 141 (unit, in-memory IMAP and JMAP servers, SMTP and HTTP stubs, TCP end-to-end, headless UI) |
| Languages | 7 (English, Spanish, French, German, Brazilian Portuguese, Japanese, Simplified Chinese) |
| Platforms | Windows, macOS, Linux; portable crates build for wasm32 |

## Milestones

1. **v0.1 — Read and send** ✅ accounts, sync, reading, writing, organising, search, sample mailbox, CLI.
2. **v0.2 — Daily driver** ✅ rich-text compose, formatted HTML view with per-sender pictures,
   outbox, folder management, notifications, contacts autocomplete, categories, rules, snooze.
3. **v0.3 — Modern sign-in** ✅ OAuth2 (Gmail, Microsoft 365), autoconfig, QRESYNC.
4. **v0.4 — Media and Craft integration** (mostly ✅): attachment previews, "Open in Craft app",
   print and PDF export, localisation, JMAP. Still open: PDF / video thumbnails, drag to `.craft`
   projects, JMAP push.
5. **Later (per #2177):** CalDAV/CardDAV calendar and contacts, Exchange, PST import, web build.

## Progress log

- **2026-10-15** — v0.1: workspace, model (threading, search, dates, UTF-7), MIME, store, IMAP client
  and sync (CONDSTORE, IDLE, offline replay), SMTP, worker pool, engine with 44 commands, egui UI
  (three panes, ribbon, dialogs, dark mode), desktop app (keychain, file dialogs), CLI, docs, CI.
- **2026-10-15** — Packaging and release workflows (Windows MSI incl. ARM64, macOS dmg, Linux
  AppImage/deb/rpm/Flatpak, FreeBSD) on the shared Craft pipelines; app icon; `--version`.
- **2026-10-15** — The gap list: folder management, Outbox, QRESYNC, contacts autocomplete,
  notifications, categories / pins / snooze / rules / Focused, an HTTP client, formatted HTML with
  pictures on request, rich-text compose with inline pictures, autoconfig (ISPDB, MX, SRV), OAuth
  2.0, attachment previews and Craft apps, print / PDF / `.eml`, seven interface languages, and
  JMAP as a second transport.
