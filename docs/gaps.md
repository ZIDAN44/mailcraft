# Gaps

What MailCraft lacks compared with the everyday Outlook mail workflow, ranked by how often people
hit it. v1 scope follows [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177);
the items it deferred (calendar, contacts, Exchange, PST import, a web build) are now in, with the
limits below. What each version brought is under [Done](#done); the milestones are in
[ROADMAP.md](../ROADMAP.md).

_Status: v0.16, 2026-10-19._

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

### v0.16 — Contacts as Outlook keeps them
From issue #1's third round, each reproduced first (the Exchange test server answering as the
tester's does, a contact as Exchange keeps it, Outlook's clipboard), then fixed and tried again.
- **Contacts' fields:** every number keeps what it is (business, home, mobile, fax, pager,
  other; two of business and home on Exchange), business, home and other addresses and the
  birthday are fields of their own, read and written with vCard (`ADR`, `BDAY`), Exchange
  (`PhysicalAddresses`, `Birthday`), Graph and Outlook data files. The contact form is Outlook's
  (three emails, a field per number, one address at a time); People shows each with its kind, and
  the List view has columns for fax, the addresses and birthdays.
- **People's address books** list each account's and an organisation's shared ones (public
  folders); a click shows one alone.
- **Signatures keep sizes and typefaces:** pasted HTML's `font-size`, `font-family` and
  `<font>` become the text's size (whole points: 7.5 is 8) and typeface, sent with the message;
  the editor shows the sizes in its own font.
- **Backspace** right after a whole recipient takes all of it, as Outlook does with a resolved
  name.
- **Someone's Exchange calendar** is editable when its events' own rights say so, for servers
  whose answer about the folder doesn't.
- **Sending and automatic replies never wait behind a sync:** each account has a second, quiet
  connection for them (the main one keeps syncing), signed in with the same token; a message
  that can't go yet goes to the main connection's Outbox, and Sent Items is read after.
- **Address suggestions** under To, Cc and Bcc are as wide as the longest one, and a click
  anywhere on a row takes it.
- **A log file:** `mailcraft.log` in the data folder (the previous run's kept as
  `mailcraft.old.log`, at most 4 MB a run): accounts' statuses and errors, how long a sync took,
  what a calendar or public folder said; never a message's content or a password.
- **French:** a value filled in at run time never comes straight after « de », « le », « que » or
  another word French elides before a vowel (« de Anna »); a test checks the catalog.
- **Public folders on Exchange 2013 and later:** listing is routed to the account's hierarchy
  mailbox (Autodiscover's `PublicFolderInformation`); anything not found while there are public
  folders is asked again in the mailboxes holding their content, so their mail syncs too; a server
  with public folders that won't list them says why in the status bar.

- **Big data files** (measured with generated files: 3 GB, 9.4 GB of 800,000 messages, and
  64.5 GB of 800,000 messages with attachments):
  - Importing wrote each message in a transaction of its own, waiting for the disk each time:
    now 2,000 messages or 16 MB a transaction. 386 messages a second became about 6,000.
  - Each message was parsed twice more for its summary and preview: now the rebuilt headers
    alone, the preview from the item's text.
  - Every folder's list was held to the end, and a folder's whole list at once: now a list goes
    out a page at a time, once its messages' sources are written. Memory for the 9.4 GB file
    peaked at 624 MB (1.74 GB before); the 64.5 GB file took 13 minutes, under 850 MB.
  - The data file kept each value in a power-of-two run of pages (6.4 GB for a 3 GB file): big
    values now go in pieces that fill their pages, and everything is compressed when that saves a
    tenth (mail and lists are text). The 9.4 GB file's cache came to 8.6 GB (17.2 GB before).
  - Exporting wrote a hundred messages at a time, and the writer rewrites a folder's table each
    time: now up to 2,000 or 48 MB.
- **Big mailboxes in the app:**
  - A folder's list is kept in pages of 4,096 by number; only pages that changed are written
    (one flag in a folder of 440,000 rewrote all of it), and told which messages changed only
    theirs are made again.
  - Moving or deleting read the whole source folder again for every message, and lookups went
    through whole lists: deleting 8,000 took 65 s, now 0.2 s; marking a folder read is linear.
  - Lists kept in memory are limited (300,000 summaries besides the folders on screen), the
    least recently used let go; search keeps its word index but not every body's text (read
    again only when a query needs it, in a 48 MB cache), each word's documents in an array, and
    plain words are settled by the index: a repeated search of 800,000 messages takes 2 s (73 s
    before), its peak 2.5 GB (3.5 GB).
  - A local account's change recounted every folder (reading all of them): now the folders it
    touched; a big one keeps the counts it saved when it opens.
  - The message list's rows (and their date headings) were made again every frame: now once per
    change of the list.
- **Still to do for big mailboxes:** the first search of everything reads every body to build its
  index (75 s for 800,000 messages; an index kept on disk would save it); the reading library
  holds a big folder's table of contents while importing it (about 400 MB for 440,000 messages);
  a search matching most of a huge mailbox holds every match's summary.

- **Everyday fixes:**
  - Prefetch over Graph and EWS: an error on one message aborted the rest of the folder. It's
    skipped (fetched on open instead); an auth error, or three errors in a row, still abort.
  - Replied/Forwarded flags were set when a reply was queued in the Outbox. They're now owed,
    saved in `pending-marks.json`, and set on delivery; Edit carries them over, removing the
    queued message drops them.
  - Search: whitespace after a bare `field:` split the query into plain words. The parser now
    joins it to the next token.
  - `contact.delete` from the ribbon or the card ran without confirmation; a dialog comes first.
  - vCard 2.1 `QUOTED-PRINTABLE` values (with `CHARSET`, and `=` soft breaks) were shown raw;
    they're decoded (UTF-8, else Windows-1252 for the single-byte Western sets).
  - New mail called RequestUserAttention, which on Windows keeps the taskbar entry highlighted
    until focus returns. The system notification and the title's unread count remain.
  - Gmail: an EXPUNGE in a label folder only archives to All Mail, so permanent deletes there
    (`mail.delete` with `permanent`, a replaced or sent draft's old copy) left the message. On
    servers with `X-GM-EXT-1` they move into Trash and are expunged by the UIDs COPYUID reports.
  - Prefilled To/Cc on replies (and To/Cc/Bcc of a message pulled back from the Outbox) had no
    trailing separator, so typing appended to the last address. They end with `; ` now.
- **The File pages and the reading pane:**
  - Options was a column of radio buttons and checkboxes: now cards of settings, one a row, the
    control on the right (switches for on/off, joined buttons for a few choices). Account
    information and Open & Export use the same rows; Info's Server settings, Password,
    Signature and Remove account each have a button instead of a menu behind a tile.
  - Export: the tile and a row of address buttons both exported (the first account unless one
    was clicked). One list of accounts now, beside one Export… button; the file is named after
    the account's address (it was "Mail.pst" for an account with no display name) and Save as
    offers Outlook data file (*.pst).
  - The Outbox/Export icon had a stray stroke (its tray's edge drawn closed); From's menu showed
    "▾" as an empty box (no such glyph in the system fonts): drawn now.
  - To, Cc and Bcc: a suggestion clicked lost the box's focus on the button's release, so the
    next address needed another click. The box keeps the cursor now.
  - Print wrote its PDF to a temporary folder: it asks where it goes, then opens it.
  - Ctrl+A in a message's text selected the whole message list as well; with text selected
    there, Ctrl+A and Ctrl+C are the text's. In the attachments, Ctrl+A selects them all and
    Ctrl+click picks some or lets one go, and a click elsewhere or Esc lets go of them all; several
    selected save together (`attachment.saveAll` with `indices`).
  - Rules: `rules.export` and `rules.import` (Open & Export, and the Rules dialog), MailCraft's
    own file, each rule's account by its address (one this computer doesn't have comes in off).
    The Rules dialog shows a rule a line, scrolls, has a find box past six rules, and Run all.

