# Gaps

What MailCraft lacks compared with the everyday Outlook mail workflow, ranked by how often people
hit it. v1 scope follows [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177):
calendar, contacts, Exchange (EWS / Graph / ActiveSync) and PST import are deliberately later.

_Status: v0.1, 2026-10-15._

## Open

| # | Gap | Notes |
|---|---|---|
| 1 | **Calendar and contacts** | Deferred per #2177. CalDAV / CardDAV first (meeting invites in mail, an address book beyond the recipients seen in mail). |
| 2 | **Exchange, PST** | Deferred per #2177: EWS / Graph / ActiveSync accounts, and importing `.pst` archives. Microsoft 365 works today over IMAP with OAuth. |
| 3 | **JMAP push** | JMAP accounts poll the Inbox every two minutes; the session's EventSource (RFC 8620 §7.3) would make new mail instant, as IDLE does for IMAP. |
| 4 | **Faithful HTML** | HTML mail is shown as formatted text (headings, lists, links, quotes, pictures on request). Layout-heavy newsletters lose their columns and colours; a sanitised HTML renderer would keep them. |
| 5 | **Drag attachments out** | Attachments open in their app or Craft app and save anywhere, but can't yet be dragged onto the desktop or a `.craft` project (egui has no outgoing OS drag). |
| 6 | **Video and PDF thumbnails** | Pictures preview inline and text-like files in a preview; PDFs and videos open in PdfCraft / FilmCraft or the system app without a thumbnail. |
| 8 | **Web build** | The engine and UI crates build for wasm32, but the network layer is native; a browser build needs a JMAP-over-`fetch` transport. |

## Done

- **v0.1:** IMAP/SMTP accounts with keychain passwords; unified inbox; local cache with
  incremental sync (CONDSTORE, UIDVALIDITY, IDLE, offline change replay); date-grouped list with
  conversations, filters, multi-select and drag to folder; reading pane right/bottom/off with
  conversation strip and picture previews; reply / reply all / forward with quoting and
  signatures; drafts; delete / archive / junk / move / flag / read; offline search with field
  operators; dark mode; CLI; sample mailbox.
- **Folders:** create, rename, move (drag) and delete, favourites chosen by the user.
- **Outbox:** mail that can't go out is kept and sent on reconnect; refusals stay with the error.
- **QRESYNC:** expunges and flag changes arrive with the `SELECT` (`VANISHED (EARLIER)`).
- **Contacts autocomplete:** recipients suggested from mail already seen.
- **Notifications:** desktop notifications for new mail, unread count in the window title.
- **Organising:** colour categories (IMAP keywords), pinning, snooze, inbox rules, Focused/Other.
- **HTML mail:** shown formatted; pictures from the web only on request, remembered per sender.
- **Rich-text compose:** bold, italic, strikethrough, headings, lists, quotes, links and code with a
  `text/html` alternative; pictures inserted or pasted inline.
- **Autoconfig:** the domain's autoconfig, the Mozilla ISPDB, MX and SRV records (RFC 6186).
- **OAuth 2.0:** Gmail and Microsoft 365 through the browser (RFC 8252 + PKCE), XOAUTH2 for IMAP
  and SMTP.
- **Attachments:** text-like previews, attached emails, Open in the Craft app for the file, Save all.
- **Print and export:** print through a PDF, Save as PDF, Save as `.eml`.
- **Localisation:** six languages besides English, following the system or chosen in Settings.
- **JMAP:** a second transport (RFC 8620/8621) behind the same jobs and events.
