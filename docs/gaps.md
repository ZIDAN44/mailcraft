# Gaps

What MailCraft lacks compared with the everyday Outlook mail workflow, ranked by how often people
hit it. v1 scope follows [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177);
the items it deferred (calendar, contacts, Exchange, PST import, a web build) are now in, with the
limits below.

_Status: v0.1, 2026-10-15._

## Open

| # | Gap | Notes |
|---|---|---|
| 1 | **HTML layout** | HTML mail shows formatted, with its colours, highlights and tables, and hidden parts stay hidden; but it's laid out as one column of text. Newsletters built from nested layout tables lose their columns. A sanitised HTML/CSS layout engine would keep them. |
| 2 | **On-premises Exchange** | Exchange Online works through Microsoft Graph, mail, calendars and contacts. On-premises Exchange without IMAP needs EWS (SOAP); ActiveSync is licensed and out of scope. |
| 3 | **Outlook data files: tasks, and files inside attached messages** | `.pst` files open with their mail, contacts and calendar, and accounts export to `.pst` (mail). Tasks and notes are skipped; an attached message comes in without its own attachments (the reader doesn't expose them); export writes mail only (the writer has no contact or appointment items) and leaves out flags. |
| 4 | **Browser version: more than JMAP** | The web build syncs JMAP accounts over `fetch` and needs the server to allow the page (CORS). A browser can't open IMAP or SMTP connections, so other accounts need a relay; push needs EventSource with credentials (polling instead); the mail cache is in memory (IndexedDB would keep it between visits). |
| 5 | **Right-to-left languages** | Eleven interface languages; Arabic, Hebrew and Persian need egui to lay out bidirectional text. |
| 6 | **Drag attachments out on Linux** | Works on Windows and macOS; Linux would need GTK's drag source. |
| 7 | **Video thumbnails without ffmpeg** | Video attachments get a frame when `ffmpeg` is installed; otherwise they show a plain tile. |

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
- **HTML mail:** shown formatted with its colours, highlights and tables (adjusted for contrast);
  pictures from the web only on request, remembered per sender.
- **Rich-text compose:** written as it will look (no marks), from the Message ribbon or with
  Outlook's shortcuts: font size, bold, italic, underline, strikethrough, font and highlight
  colours, bullets and numbering, alignment, indent, clear formatting, links and pictures, with
  undo; sent as HTML with a plain-text twin.
- **Autoconfig:** the domain's autoconfig, the Mozilla ISPDB, MX and SRV records (RFC 6186).
- **OAuth 2.0:** Gmail and Microsoft 365 through the browser (RFC 8252 + PKCE), XOAUTH2 for IMAP
  and SMTP.
- **Attachments:** text-like previews, attached emails, PDF thumbnails (first page) and video
  frames, Open in the Craft app for the file, Save all, dragging out to other apps.
- **Print and export:** print through a PDF, Save as PDF, Save as `.eml`.
- **Languages:** ten besides English (Spanish, French, German, Brazilian Portuguese, Italian,
  Russian, Japanese, Korean, Simplified and Traditional Chinese), following the system or chosen
  in Options.
- **JMAP:** a second transport (RFC 8620/8621) behind the same jobs and events, with push over
  the session's event stream; shared with the browser in `mailcraft-jmap`.
- **Messages in windows:** double-click (or *Open in new window*) opens a message in a window of
  its own, as in Outlook; new mail is written in a window of its own (an option), replies start
  in the reading pane and pop out; closing a message window keeps it in Drafts.
- **Writing like Outlook:** the tall Send button beside From, To and Cc (which open the
  address book, Select Names), underlined fields, the original under a rule with its From / Sent /
  To / Subject and its own formatting, and "Want to save your changes?" on closing a message
  window.
- **The File page:** account information, server settings, automatic replies (Sieve vacation over
  ManageSieve, JMAP's VacationResponse or Graph's mailbox settings), the space used (IMAP QUOTA,
  JMAP Quota), emptying Deleted Items and Junk, rules, Options and About.
- **Calendar and contacts:** day, week and month views, events with recurrences and time zones
  (iCalendar), people (vCard), kept per account and synced with CalDAV / CardDAV (found through
  RFC 6764); meeting invitations by mail (iTIP/iMIP): accept, tentative, decline, and inviting
  guests.
- **Outlook data files:** File › Open & Export opens a `.pst` as a mailbox on this computer, with
  its folders, messages, attachments and flags, its contacts and its calendar (recurrences
  included); and writes any account's mail to a new `.pst`.
- **Exchange Online:** Microsoft Graph as a third transport, for organisations that switch IMAP
  off: folders, sync, changes, drafts, sending, automatic replies, calendars and contacts (both
  ways).
- **Web build:** `apps/mailcraft-web`, the same window in the browser over JMAP and `fetch`, with
  downloads, the browser's file picker and a `?sample` mailbox.
