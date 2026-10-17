# Gaps

What MailCraft lacks compared with the everyday Outlook mail workflow, ranked by how often people
hit it. v1 scope follows [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177);
the items it deferred (calendar, contacts, Exchange, PST import, a web build) are now in, with the
limits below. What each version brought is under [Done](#done); the milestones are in
[ROADMAP.md](../ROADMAP.md).

_Status: v0.15, 2026-10-18._

## Open

| # | Gap | Notes |
|---|---|---|
| 1 | **Browser version: accounts other than JMAP** | The web build syncs JMAP accounts over `fetch`, with push, and keeps its cache in IndexedDB; the server must allow the page (CORS). A browser can't open IMAP or SMTP connections, so IMAP accounts would need a relay server that holds the password and speaks JMAP to the page: a server product of its own, not built. |
| 2 | **Right-to-left: inside the editor, selecting, Hebrew in the browser** | The window mirrors and right-to-left mail and interface text read right to left. In the message editor a line that mixes right-to-left and left-to-right words is shown in the order egui lays it out (reordering it would put the caret in the wrong places); right-to-left mail can't be selected with the pointer (egui 0.36's text layout counts right-to-left letters wrongly, so a selection would copy nothing), but a right-click copies the line or the whole message as written. Release builds draw Arabic, Japanese and Thai in the browser with fonts from [storytold/craft-fonts](https://github.com/storytold/craft-fonts); it has no Hebrew or Korean font yet, and its Simplified Chinese one is too large for the page, so those still need the system (the desktop app) until craft-fonts has them. |
| 3 | **Outlook data files: what the `.pst` libraries can't do** | Mail, contacts, calendar items, notes and tasks with a due date come in; export writes the mail as `.pst` and the calendar and contacts beside it as `.ics` and `.vcf`. The writer (`outlook-pst-rw`) makes mail items only, marked read, so contacts, appointments and flags can't go inside the `.pst`; the reader (`outlook-pst`) doesn't open the attachments of an attached message; tasks without a due date are left out (MailCraft has no task list). |
| 4 | **Drag attachments out under Wayland** | Works on Windows, macOS, and Linux and the BSDs under X11 (XDND). A native Wayland window would need the compositor's data-device drag from the window's own Wayland connection, which egui's window layer doesn't share (reaching it would take unsafe code). |
| 5 | **On-premises Exchange: Kerberos-only servers** | EWS signs in with NTLMv2 or Basic and is found with Autodiscover; a server that only takes Kerberos can't be signed in to. ActiveSync is licensed and out of scope. |
| 6 | **Public folders on Exchange Online** | Public folders list and sync on on-premises Exchange through EWS. Exchange Online reaches them only through EWS, which Microsoft blocks there from October 2026 and removes in April 2027, and Microsoft Graph has no public folders; its suggested replacement, Microsoft 365 Groups, needs permissions only a tenant's administrator can grant. |

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

### v0.11 — Notes, previews and Exchange beyond the mailbox
From [issue #1](https://github.com/ZIDAN44/mailcraft/issues/1), an Exchange user's list:
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

### v0.12 — Microsoft sign-in that works
Microsoft sign-in, tried end to end once the release builds carried a registration:
- **The sign-in page opens:** on Windows, web addresses went through the way files are opened
  and nothing appeared, so Sign in with Microsoft waited on a page that never opened (and links
  from mail went nowhere). They go to Windows' URL handler.
- **"Sign in again", not "Password needed",** for accounts that sign in in the browser; the link
  opens the sign-in.
- **A refusal says why:** an account the server turns down (a Microsoft account with no mailbox
  Graph can reach) was asked again and again and sat at "Connecting…"; it stops with the reason
  until Send / Receive.
- **Sent Items catches up:** Graph and Exchange file the copy a moment after sending; it showed
  only at the next poll. Sent Items is read again a few seconds later.
- **Tokens name their permissions:** each token request says what it's for (a personal
  account's token could otherwise cover reading but not sending), and an SMTP refusal says
  whether the sign-in covered sending. A Microsoft account made with a Gmail address sends
  only once an Outlook.com address is its primary alias ([docs/oauth.md](oauth.md)).
- **Fixed:** small dialogs opened as tall as the last big one, their buttons halfway down (and
  OK, Connect and Remove wore the send icon); opening People widened the folder pane for every
  module.

### v0.13 — The open gaps, again
- **Microsoft sign-in in release builds:** the test builds carry MailCraft's Microsoft Entra
  registration (the repository secret), so Outlook.com, Hotmail and Live accounts sign in
  ([docs/oauth.md](oauth.md)); the gap is closed.
- **Other people's calendars can be changed** where their owner made this account an editor:
  Graph reads the calendar's `canEdit` (with `Calendars.ReadWrite.Shared`), EWS the folder's
  EffectiveRights; new events go into the owner's calendar, and changes and deletions reach it
  too. Those they didn't allow stay read-only, and only those say "read-only".
- **HTML layout: background pictures and placed boxes:** a picture behind a table, a cell or a
  box (`background`, `background-image: url(…)`, `background: … url(…)`) is drawn behind its
  content, cropped to cover it, and held back like the message's other web pictures until
  they're allowed (its colour shows meanwhile); boxes placed with `position: absolute` in a
  `position: relative` one (a label on a picture) sit over it at their offsets, as wide as their
  text unless they say. Text now takes its cell's or table's `color`, so white on a dark band
  stays white.
- **Outlook's notes from a data file:** importing a `.pst` brings its Notes folders' notes into
  MailCraft's notes, with their colour and time (they were left out).
- **Right-to-left mail can be copied:** a right-click on Arabic or Hebrew text in the reading
  pane copies the line, or the whole message, as written (selecting it with the pointer isn't
  possible: egui's layout miscounts right-to-left letters).
- **Several accounts, as Outlook keeps them:** MailCraft opens on the first account's Inbox (All
  Inboxes stays in Favorites), accounts move up and down from their right-click menu (the order is
  kept), and the status bar says one thing for all of them ("All folders are up to date",
  "Updating…", "2 accounts disconnected"), each account's own status on hover, with only those
  that need a password or a sign-in shown on their own.
- **MailCraft's own data files:** a POP account can start from a `.mailcraft` file as well as a
  `.pst`, and File › Open a data file opens one as a mailbox of its own (read from a copy, so
  the file itself isn't touched).
- **The folder pane remembers:** Favorites, accounts and folders with folders in them collapse,
  and stay as they were left from one start to the next; accounts can also be dragged into a
  new order.

### v0.14 — What testers asked for
From issues #1 and #2, each reproduced first (an IMAP server that answers as Outlook.com does,
an Exchange report as it arrived), then fixed and tried again.
- **Microsoft sign-in, the ways it failed:** a Hotmail account added with a password switches to
  Sign in with Microsoft from the status bar and keeps its mail; adding one offers no password;
  "User is authenticated but not connected" (IMAP off, or another account picked in the browser)
  says what to do instead of asking to sign in again, the address the browser signed in as is
  tried, and **Use Microsoft Graph** moves the account off IMAP; Automatic replies on an account
  that can't connect no longer leave the status on "Connecting…".
- **Generated HTML mail:** pictures without a description keep their place (only trackers go);
  pictures are found by `cid:` (%-escapes too), Content-Location or file name; style sheets
  colour cells and text (tag, class, id and descendant selectors); sized text reads as headings;
  rows' colours, whole-column widths and centred pictures, and light stripes, as Outlook shows
  them.
- **Search:** typing stays quick on a big mailbox (each message's words read once; a moment
  after the last key); the box at the top searches People and Notes where you are.
- **Reading:** mail is marked read when shown (after a wait), once another is chosen, or only
  when opened in its own window.
- **Signatures from the message:** Insert signature and Signatures… on the Message ribbon.
- **Notes** arrange by title, colour or last change.
- **Calendar:** events in their category's colour (categories kept with Exchange, Graph and
  CalDAV, chosen on the event); calendars ticked to show or hide; side by side in day and week;
  up to six months in the side pane.
- **People:** the List view's columns are chosen and sort by the one clicked; an organisation's
  shared contacts and calendars in Exchange public folders come in, read-only.
- **Automatic replies on Exchange:** another reply for outside the organisation, and to whom.
- **Mail kept on this computer:** the newest 1,000, 5,000, 20,000 or all of each folder, 40 to
  1,000 downloaded ahead (EWS now pages past the first 1,000).
- **Open folder** opens it (Explorer on Windows).

### v0.15 — What testers asked for, again
From issue #1's next round, each reproduced first (the Exchange test server answering as
Exchange 2013 and later do, Outlook's clipboard as it copies a signature), then fixed and tried
again.
- **Side by side** is Outlook's: each calendar shown in a pane of its own with its days, under a
  heading in its colour with × to hide it; a click in a pane adds the event to that calendar.
- **Someone else's Exchange calendar** opens editable for an editor: a folder's `Modify` right is
  the owner's alone (it's about the folder), so `CreateContents` says it.
- **Public folder contacts and calendars on Exchange 2013 and later:** reading inside a public
  folder is routed to the public folder mailbox holding it (`PR_REPLICA_LIST`, Autodiscover,
  `X-AnchorMailbox` and `X-PublicFolderMailbox`); one that still can't be read says why.
- **Pasted from Outlook, Word or a web page:** HTML on the clipboard keeps its bold, italics,
  colours and links (Word's paragraphs as lines, its pictures left behind); font sizes don't
  carry over yet.
- **Signatures keep their look:** the signature editor is a message editor of its own; what's
  formatted there or pasted is kept as HTML and goes into new messages, replies and Insert
  signature formatted.
- **Select Names** opens with the address book chosen ("Always open with this address book"):
  everyone, every contact, or one account's contacts or shared ones; long names and addresses are
  cut short with "…".
