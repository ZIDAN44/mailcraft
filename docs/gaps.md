# Gaps

What MailCraft lacks compared with the everyday Outlook mail workflow, ranked by how often people
hit it. v1 scope follows [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177);
the items it deferred (calendar, contacts, Exchange, PST import, a web build) are now in, with the
limits below. What each version brought is under [Done](#done); the milestones are in
[ROADMAP.md](../ROADMAP.md).

_Status: v0.10, 2026-10-15._

## Open

| # | Gap | Notes |
|---|---|---|
| 1 | **Browser version: accounts other than JMAP** | The web build syncs JMAP accounts over `fetch`, with push, and keeps its cache in IndexedDB; the server must allow the page (CORS). A browser can't open IMAP or SMTP connections, so IMAP accounts would need a relay server that holds the password and speaks JMAP to the page: a server product of its own, not built. |
| 2 | **Right-to-left: inside the editor, selecting, Hebrew in the browser** | The window mirrors and right-to-left mail and interface text read right to left. In the message editor a line that mixes right-to-left and left-to-right words is shown in the order egui lays it out (reordering it would put the caret in the wrong places); right-to-left mail bodies can't be selected. Release builds draw Arabic, Japanese and Thai in the browser with fonts from [storytold/craft-fonts](https://github.com/storytold/craft-fonts); it has no Hebrew or Korean font yet, and its Simplified Chinese one is too large for the page, so those still need the system (the desktop app) until craft-fonts has them. |
| 3 | **Outlook data files: what the `.pst` libraries can't do** | Mail, contacts, calendar items and tasks with a due date come in; export writes the mail as `.pst` and the calendar and contacts beside it as `.ics` and `.vcf`. The writer (`outlook-pst-rw`) makes mail items only, marked read, so contacts, appointments and flags can't go inside the `.pst`; the reader (`outlook-pst`) doesn't open the attachments of an attached message; notes and tasks without a date are left out. |
| 4 | **Drag attachments out under Wayland** | Works on Windows, macOS, and Linux and the BSDs under X11 (XDND). A native Wayland window would need the compositor's data-device drag from the window's own Wayland connection, which egui's window layer doesn't share (reaching it would take unsafe code). |
| 5 | **On-premises Exchange: Kerberos-only servers** | EWS signs in with NTLMv2 or Basic and is found with Autodiscover; a server that only takes Kerberos can't be signed in to. ActiveSync is licensed and out of scope. |
| 6 | **HTML layout: positioning and background pictures** | Tables, flexbox rows, grids, inline-block and floated boxes keep their columns; absolutely positioned boxes and background pictures still flow as text. |
| 7 | **Microsoft sign-in needs an app registration** | Outlook.com, Hotmail and Live (and many Microsoft 365 tenants) take no passwords from mail apps: only Microsoft's sign-in, which a build offers when it carries a Microsoft Entra app registration's client id ([docs/oauth.md](oauth.md)). The release workflow passes the repository secret `MAILCRAFT_MICROSOFT_CLIENT_ID`; until it's set, such accounts can't be added (the dialog says why). |
| 8 | **Public folders on Exchange Online; writing to shared calendars** | Public folders list and sync on on-premises Exchange through EWS. Exchange Online routes public-folder requests to a mailbox found through Autodiscover's user settings (`X-AnchorMailbox`), not done yet, and Graph has no public folders. Other people's calendars open read-only, even where their owner allowed editing. |

## Done

### v0.1 — Read and send
IMAP/SMTP accounts with keychain passwords; unified inbox; local cache with incremental sync
(CONDSTORE, UIDVALIDITY, IDLE, offline change replay); date-grouped list with conversations,
filters, multi-select and drag to folder; reading pane right/bottom/off with conversation strip
and picture previews; reply / reply all / forward with quoting and signatures; drafts; delete /
archive / junk / move / flag / read; offline search with field operators; dark mode; CLI; sample
mailbox; packaging for every desktop.

### v0.2 — Daily driver
- **Folders:** create, rename, move (drag) and delete, favourites chosen by the user.
- **Outbox:** mail that can't go out is kept and sent on reconnect; refusals stay with the error.
- **Contacts autocomplete:** recipients suggested from mail already seen.
- **Notifications:** desktop notifications for new mail, unread count in the window title.
- **Organising:** colour categories (IMAP keywords), pinning, snooze, inbox rules, Focused/Other.
- **HTML mail:** shown formatted (headings, lists, links, quotes), pictures from the web only on
  request, remembered per sender.
- **Rich-text compose** with inline pictures, sent as HTML with a plain-text twin.

### v0.3 — Modern sign-in
- **QRESYNC:** expunges and flag changes arrive with the `SELECT` (`VANISHED (EARLIER)`).
- **Autoconfig:** the domain's autoconfig, the Mozilla ISPDB, MX and SRV records (RFC 6186).
- **OAuth 2.0:** Gmail and Microsoft 365 through the browser (RFC 8252 + PKCE), XOAUTH2 for IMAP
  and SMTP.

### v0.4 — Media and Craft integration
- **Attachments:** text-like previews, attached emails, PDF thumbnails (first page) and video
  frames from ffmpeg, Open in the Craft app for the file, Save all, dragging out to other apps
  (Windows and macOS).
- **Print and export:** print through a PDF, Save as PDF, Save as `.eml`.
- **Languages:** ten besides English (Spanish, French, German, Brazilian Portuguese, Italian,
  Russian, Japanese, Korean, Simplified and Traditional Chinese), following the system or chosen
  in Options.
- **JMAP:** a second transport (RFC 8620/8621) behind the same jobs and events, with push over
  the session's event stream.
- **HTML colours and tables**, adjusted for contrast.

### v0.5 — Beyond mail
- **Messages in windows:** double-click (or *Open in new window*) opens a message in a window of
  its own.
- **The File page:** account information, server settings, automatic replies (Sieve vacation over
  ManageSieve, JMAP's VacationResponse), the space used (IMAP QUOTA, JMAP Quota), emptying Deleted
  Items and Junk, rules, Options and About.
- **Calendar and contacts:** day, week and month views, events with recurrences and time zones
  (iCalendar), people (vCard), synced with CalDAV / CardDAV (found through RFC 6764); meeting
  invitations by mail (iTIP/iMIP): accept, tentative, decline, and inviting guests.
- **Outlook data files:** File › Open & Export opens a `.pst` as a mailbox on this computer, with
  its folders, messages, attachments and flags.
- **Exchange Online:** Microsoft Graph as a third transport: folders, sync, changes, drafts,
  sending and automatic replies.
- **Web build:** `apps/mailcraft-web`, the same window in the browser over JMAP and `fetch`
  (`mailcraft-jmap` shared with the desktop), with downloads, the browser's file picker and a
  `?sample` mailbox.

### v0.6 — Outlook's reach
- **Writing like Outlook:** new mail in a window of its own (an option), replies that pop out,
  the tall Send button beside From, To and Cc (which open the address book, Select Names),
  formatting written as it will look from the Message ribbon or Outlook's shortcuts (font size,
  bold, italic, underline, strikethrough, colours, bullets and numbering, alignment, indent,
  links, pictures, undo), the original under a rule with its own formatting, and "Want to save
  your changes?" on closing.
- **Exchange Online calendars and contacts** through Graph, both ways.
- **On-premises Exchange:** Exchange Web Services as a fourth transport, signed in with NTLMv2
  (or Basic): folders, sync, changes, drafts, sending and automatic replies.
- **Outlook data files:** contacts and calendar items (recurrences included) come in; any
  account's mail exports to a new `.pst`.
- **Newsletter layouts:** HTML mail laid out with tables keeps its columns (side by side when
  there's room, one under the other when not), widths, coloured cells and centring.
- **Arabic and Hebrew**, and right-to-left text in mail and the interface put in reading order
  line by line (the Unicode Bidirectional Algorithm), joined, and set against the right edge; a
  font with those scripts is found when the main one lacks them.
- **The browser:** push (the event stream read through `fetch`) and the cache kept in IndexedDB
  between visits.
- **Video frames without ffmpeg** from the system's own thumbnailer (the Windows Shell, Quick
  Look, a Linux video thumbnailer).
- **Dragging attachments out on Linux and the BSDs** under X11, through XDND.

### v0.7 — The open gaps closed
- **CSS layouts:** flexbox rows, grids, inline-block and floated boxes keep their columns too.
- **On-premises Exchange calendars and contacts** through EWS, both ways (recurrences included,
  no meeting mail sent), and **Autodiscover** finding the server from the email address.
- **Right-to-left windows:** in Arabic and Hebrew the window mirrors (folders on the right, the
  reading pane on the left), and right-to-left paragraphs line up on the right while writing.
- **Outlook data files:** tasks with a due date come into the calendar; export writes the
  calendar and contacts beside the `.pst` as `.ics` and `.vcf`.

### v0.8 — POP and Outlook's own data files
- **POP accounts** (RFC 1939, SSL/TLS or STARTTLS): new Inbox mail is downloaded once and kept in
  folders on this computer, with a copy left on the server unless asked otherwise, as Outlook's
  POP accounts do. Drafts, Sent Items and moves stay on this computer.
- **"Use an existing data file":** a POP account can start from an old `.pst`, whose mail comes
  in before the account first connects.
- **Real Outlook data files:** checked against `.pst` files from a current Outlook; a file Outlook
  has open says so, an `.ost` (Outlook's offline copy of a server account) is explained, and the
  RSS Feeds folder and an empty Outbox aren't brought in.
- **Data files, as Outlook's:** each account is one `.mailcraft` file in Documents\MailCraft
  Files (an opened `.pst` gets one too), as Outlook keeps `.pst` files in Outlook Files; mail
  kept before comes into them by itself. File › Options › Data files shows and moves the folder
  (at the next start, every file checked before the old ones go); each account's Mailbox
  settings show its file and save a copy as a `.pst`.

### v0.9 — Outlook's everyday commands
- **POP server copies, as Outlook's settings:** kept on the server, removed after a number of
  days (14 for new accounts) and/or once deleted from Deleted Items.
- **From an audit against Outlook:** View › Arrange by (date, sender, recipient, subject, size,
  flag, attachments) and Reverse sort, with their headings; a Send / Receive tab with Send all
  and Work offline (kept across starts; drafts, sent mail and changes wait on this computer);
  Undo (Ctrl+Z) for moves, deletes, flags and categories; Filter (categorized, important, to or
  Cc me); Delete › Ignore the conversation; New items (event, contact); a minimizable folder
  pane; File › Save As, Save Attachments, Print and Exit; High / Low importance and read
  receipts when writing, and the importance of incoming mail in the list.
- **Bugs fixed along the way:** a draft whose save failed (offline, server down) was lost; it's
  kept and saved when the account connects. Grouped list headings were spaced as messages; the
  ribbon cut off its last buttons in a narrow window.

### v0.10 — Windows, folders and rules as Outlook keeps them
- **Windows remembered:** the main window opens where it was and at its size, maximized if it
  was (Restore returns to its normal size); message and compose windows open where the last one
  was.
- **Subfolders where they belong:** a folder made in the Inbox shows under the Inbox (it showed
  after Deleted Items); the account menu's New folder starts in the folder open now.
- **Run a rule now:** a new rule can run at once on the mail already in the Inbox, as Outlook's
  rule wizard offers; Run now in the rules list does the same.
- **Attachments open, as Outlook's:** small chips with a thumbnail, name and size; double-click
  opens one, ▾ (or right-click) gives Preview, Open, Save As and Save All Attachments. Open never
  worked: a press on it started a drag-out, and on Windows the file was handed over in a way
  apps such as Acrobat ignored. A message window's attachments act on its own message. Chips
  share the width in columns and wrap onto more rows (in the reading pane and in compose), with
  a scroll bar past three rows (two in compose); the body and the ▾ each light up under the
  pointer, which stays an arrow, and a click selects a chip.
- **New mail filed by rules is announced**, as Outlook does (not what a rule deletes or marks
  read).
- **From a second audit against Outlook:**
  - **Search tab** while searching: where to look, From / Sent To / Subject, Has Attachments,
    Unread, Categorized, Flagged and Important (`is:important`, `is:categorized`), Close Search.
  - **Follow Up:** flag for today, tomorrow, this week, next week or no date, Mark complete and
    Clear flag (the ribbon's Flag ▾ and the right-click menu); the reading pane says when it's
    due (red once overdue) and when mail came with high or low importance. The day is kept on
    this computer, as Outlook keeps it for IMAP accounts.
  - **Delay delivery** (held in the Outbox until its time, dated when it goes; MailCraft has to
    be running) and **Replies to** (a Reply-To) while writing.
  - **Menus as Outlook's:** Quick print, Ignore and Find related on a message; Reply with
    meeting and Forward as attachment (the original as an `.eml`) under the reading pane's ⋯;
    Delete all… (to Deleted Items) and Properties… on a folder; Open file location and Account
    settings… on an account.
  - **Hover everywhere:** rows light up in a tint of the app colour (the grey was all but
    invisible on the folder pane), buttons in a stronger grey; the list's filter tabs, By Date,
    the File and ribbon tabs, calendar events, days and hours, and colour swatches gained one.
  - **Fixed:** the language menu drew Japanese, Korean and Chinese as boxes unless the interface
    was in one of them (and so did mail in them); a chip's tooltip said "Double-click to open
    it"; "Pin", "Unpin" and "Always move to Focused/Other" weren't translated.
- **From issue #1 (an Exchange user's list):**
  - **Attachment previews in the reading pane:** a click on an attachment shows it in place of
    the message (Back to message, Open, Save As): pictures, a PDF's pages, text, attached mail,
    and Word documents, Excel workbooks and PowerPoint slides read by MailCraft itself.
  - **Notes**, as Outlook's: coloured sticky notes (Ctrl+5), kept in an Exchange mailbox's
    Notes folder (EWS) or on this computer.
  - **People in three layouts** (Outlook's Change View): the list with an a–z index, business
    cards, and a phone list.
  - **Other people's Exchange calendars**, opened by their address (read-only), through Graph or
    EWS; and **public folders** on on-premises Exchange, under a folder of their own.
  - **Dutch** interface.
  - **Outlook.com and Hotmail:** the Add account dialog and the account's status say they need
    Microsoft sign-in rather than asking for a password again and again; release builds carry
    the sign-in once the repository has its secret.
  - **Fixed:** the list's filter tabs ran into "By Date" in long-worded languages; the status
    bar counted mail in People and Calendar.
