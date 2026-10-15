# Gaps

What MailCraft lacks compared with the everyday Outlook mail workflow, ranked by how often people
hit it. v1 scope follows [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177);
the items it deferred (calendar, contacts, Exchange, PST import, a web build) are now in, with the
limits below.

_Status: v0.1, 2026-10-15._

## Open

| # | Gap | Notes |
|---|---|---|
| 1 | **HTML layout beyond tables** | Mail laid out with tables keeps its columns, widths, coloured bands and centring; layouts made with CSS alone (floats, flexbox, grid, positioning) and background pictures still flow as one column. |
| 2 | **On-premises Exchange: calendars and finding the server** | Mail and automatic replies work through EWS (NTLM or Basic); its calendars and contacts aren't synced over EWS yet, the server's address is typed in (no Autodiscover), and Kerberos-only servers can't be signed in to. ActiveSync is licensed and out of scope. |
| 3 | **Outlook data files: tasks, and files inside attached messages** | `.pst` files open with their mail, contacts and calendar, and accounts export to `.pst` (mail). Tasks and notes are skipped; an attached message comes in without its own attachments (the reader doesn't expose them); export writes mail only (the writer has no contact or appointment items) and leaves out flags. |
| 4 | **Browser version: more than JMAP** | The web build syncs JMAP accounts over `fetch`, with push, and keeps its cache in IndexedDB; the server must allow the page (CORS). A browser can't open IMAP or SMTP connections, so other accounts would need a relay server, which MailCraft doesn't have. |
| 5 | **Right-to-left: the window itself** | Arabic and Hebrew interfaces, and right-to-left mail, read right to left; but the window isn't mirrored (the folder pane stays on the left), the message editor shows right-to-left lines as egui lays them out, and right-to-left mail bodies can't be selected. egui has no bidirectional layout of its own (MailCraft orders each line itself). |
| 6 | **Drag attachments out under Wayland** | Works on Windows, macOS, and Linux and the BSDs under X11 (XDND); a native Wayland window would need the compositor's data-device drag, which needs the window's own Wayland connection. |

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
  newsletters laid out with tables keep their columns (side by side when there's room, one under
  the other when not), widths, coloured cells and centring; pictures from the web only on
  request, remembered per sender.
- **Rich-text compose:** written as it will look (no marks), from the Message ribbon or with
  Outlook's shortcuts: font size, bold, italic, underline, strikethrough, font and highlight
  colours, bullets and numbering, alignment, indent, clear formatting, links and pictures, with
  undo; sent as HTML with a plain-text twin.
- **Autoconfig:** the domain's autoconfig, the Mozilla ISPDB, MX and SRV records (RFC 6186).
- **OAuth 2.0:** Gmail and Microsoft 365 through the browser (RFC 8252 + PKCE), XOAUTH2 for IMAP
  and SMTP.
- **Attachments:** text-like previews, attached emails, PDF thumbnails (first page) and video
  frames (from ffmpeg when it's installed, otherwise the system's own thumbnailer: the Windows
  Shell, Quick Look, or a Linux video thumbnailer), Open in the Craft app for the file, Save all,
  dragging out to other apps (on Linux and the BSDs under X11, through XDND).
- **Print and export:** print through a PDF, Save as PDF, Save as `.eml`.
- **Languages:** twelve besides English (Spanish, French, German, Brazilian Portuguese, Italian,
  Russian, Japanese, Korean, Simplified and Traditional Chinese, Arabic, Hebrew), following the
  system or chosen in Options.
- **Right-to-left text:** Arabic, Hebrew and Persian in mail and in the interface are put in
  reading order line by line (the Unicode Bidirectional Algorithm), joined, and set against the
  right edge; a font with those scripts is found when the main one lacks them.
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
- **On-premises Exchange:** Exchange Web Services as a fourth transport, signed in with NTLMv2
  (or Basic): folders, sync, changes, drafts, sending and automatic replies.
- **Exchange Online:** Microsoft Graph as a third transport, for organisations that switch IMAP
  off: folders, sync, changes, drafts, sending, automatic replies, calendars and contacts (both
  ways).
- **Web build:** `apps/mailcraft-web`, the same window in the browser over JMAP and `fetch`, with
  push (the event stream read through `fetch`), the cache kept in IndexedDB between visits,
  downloads, the browser's file picker and a `?sample` mailbox.
