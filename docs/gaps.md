# Gaps

What MailCraft lacks compared with the everyday Outlook mail workflow, ranked by how often people
hit it. v1 scope follows [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177);
the items it deferred (calendar, contacts, Exchange, PST import, a web build) are now in, with the
limits below. What each version brought is under [Done](#done); the milestones are in
[ROADMAP.md](../ROADMAP.md).

_Status: v0.29, 2026-10-28._

## Open

| # | Gap | Notes |
|---|---|---|
| 1 | **Browser version: accounts other than JMAP** | The web build syncs JMAP accounts over `fetch`, with push, and keeps its cache in IndexedDB; the server must allow the page (CORS). A browser can't open IMAP or SMTP connections, so IMAP accounts would need a relay server that holds the password and speaks JMAP to the page: a server product of its own, not built. |
| 2 | **Right-to-left: inside the editor, dialogs and menus** | The mail window mirrors, inside its panes too, and right-to-left mail and interface text read right to left and can be selected with the pointer and copied. Not yet: dialogs, Calendar, People and Notes aren't mirrored inside; egui's menu items stay on the left (the Craft apps' egui mirrors where menus open and their arrows, but lays their items out with `top_down_justified(Align::Min)`: a one-line change in the shared fork, storytold/egui's `containers/menu.rs`); in the message editor a line mixing right-to-left and left-to-right words shows in the order egui lays it out. |
| 3 | **Outlook data files: what the `.pst` libraries can't do** | Mail, contacts, calendar items, notes and tasks with a due date come in; export writes the mail as `.pst` and the calendar and contacts beside it as `.ics` and `.vcf`. The writer (`outlook-pst-rw` 1.2.14) makes mail items only (`IPM.Note`), marked read, so contacts, appointments and flags can't go inside the `.pst`. The reader (`outlook-pst` 1.2.0) reads an attached message's own attachment table, but hands the message back as `dyn Message`, and opening those attachments needs `read_embedded`, which is crate-private (in the writer's fork too): a small change in the library. Tasks without a due date are left out (MailCraft has no task list). |
| 4 | **Drag attachments out on a Wayland-only desktop** | Works on Windows, macOS, and Linux and the BSDs under X11 (XDND), and on Wayland desktops through XWayland, which the window opens on by default. With `MAILCRAFT_WAYLAND=1`, or on a desktop without XWayland, the window is native Wayland, where the Craft apps' winit (0.30, storytold/winit) can take drops but not start a drag. Upstream winit 0.31 adds it (`ActiveEventLoop::start_drag`, rust-windowing/winit#4571); it comes to MailCraft when egui and the Craft forks move to 0.31 (still in beta, and egui on 0.30.13). |
| 5 | **Public folders on Exchange Online** | Public folders list and sync on on-premises Exchange through EWS. Exchange Online reaches them only through EWS, which Microsoft blocks there from October 2026 and removes in April 2027, and Microsoft Graph has no public folders; its suggested replacement, Microsoft 365 Groups, needs permissions only a tenant's administrator can grant. |
| 6 | **S/MIME: what isn't done yet** | The browser can't check revocation (authorities' servers don't answer web pages) and has no smart cards or system store (Chrome's Web Smart Card API is for Isolated Web Apps only). Its certificates are locked again after a reload unless *Keep unlocked in this browser* is on. |
| 7 | **Updates: a signature, signed releases, macOS without an administrator** | Test builds update themselves ([docs/updates.md](updates.md)); signed releases don't yet. A download is held to the SHA-256 GitHub lists for it, which proves it arrived whole but not who made it: an Ed25519 signature checked against a key built into MailCraft (as Tauri and Sparkle do) needs a signing key kept by the release workflows, the owner's to create. On macOS a bundle in `/Applications` on a standard account isn't replaced (it would need an authorization prompt). No staged rollout, and no smaller downloads (zsync, deltas). |

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

### v0.17 — Copy and paste, and public folder contacts
- **The clipboard:** nothing could be pasted (a signature copied from Outlook, or text into any
  box) or copied out since egui 0.37, which made the clipboard a feature of eframe's own, left
  out with its default features. It's named now, and a test checks it stays named. Seen in the
  app: text pasted into the search box, a formatted signature copied from Word pasted with its
  sizes, typefaces and colours, text copied out reaching the system clipboard.
- **Public folder contacts and calendars:** Exchange answers FindFolder with each folder in an
  element of its kind (`Folder`, `ContactsFolder`, `CalendarFolder`, `TasksFolder`,
  `SearchFolder`) and only `Folder` was read, so the organisation's shared address books and
  calendars were passed over without a word. Every kind is read, in the answer's order (the
  mailbox's own folder list now pages by the right count too). The fake Exchange answers the
  same way, which its public folder test failed on until then, and the log says how many public
  address books and calendars were found.

### v0.18 — Shared mailboxes and the contact card
- **Shared mailboxes (Exchange):** none showed, and none could be added. The ones Autodiscover
  names (`AlternativeMailbox`, delegate and team mailboxes the account has full access to; an
  archive isn't one, a legacy name is resolved to its address) open under the account by
  themselves; others are added by address in Info › Account settings › Shared mailboxes, and
  removed there. Each is listed with its `Mailbox` and the `X-AnchorMailbox` header, its mail
  folders only, under its name (the address too when two share one); one with no access is
  logged and passed over. The fake Exchange has shared mailboxes, and its test lists and syncs
  them. Not yet: Exchange Online accounts connected through Microsoft Graph.
- **The contact card:** From, To and Cc names in a message's header are links: a click opens a
  card with the name, the address (which starts a message to them), Send email, Add to contacts
  or Edit contact, and Copy address; right-click has the same. Add to contacts opens a new
  contact with the name and address filled in. `mail.new` takes `to`.
- **Enter in one-field dialogs:** the password, shared calendar and shared mailbox dialogs gave
  the field its focus back before looking for the Enter that ended input, so only the button
  submitted them. A headless test presses Enter.
- **A removed account's data file stays:** removing an account deleted its `.mailcraft` file
  with all the mail in it. Outlook leaves a removed account's `.pst` on disk (Microsoft: removing
  one only disconnects it), and so does MailCraft now: the file is closed and no one's, the
  Remove dialog names it, and the notice says where it is. The same address added again gets a
  file of its own (`address (2).mailcraft`), the kept one untouched; Open & Export's picker
  starts in MailCraft Files, and the kept file opens there as a mailbox of its own. A file that
  failed to open, and an account moving to Graph (starting again from the server), still go.

### v0.19 — Shared mailboxes of their own, and SMTP's own sign-in
- **A shared mailbox is a mailbox of its own:** it showed as a folder inside the account's
  mailbox, its Inbox drawn as any folder. As Outlook, it now comes after the account's folders
  under its name, with an arrow that collapses it (its unread mail beside the name then), and its
  Inbox, Drafts, Sent Items… with their icons and in their places; pinned to Favorites, a folder
  says whose it is. Each folder knows the mailbox it's in and the role it has there (asked of
  Exchange in one GetFolder); its own role stays Other, so the account's Inbox, notifications and
  rules aren't taken for the shared one's, and its top and special folders can't be renamed,
  moved or deleted. Its menu has Send / Receive, New folder, Remove (one added by hand) and Add
  shared mailbox, which an Exchange account's menu has too. A tester who couldn't find
  "Shared mailboxes" was on v0.17, before it was added to Info › Account settings.
- **Exchange's folder names:** a mailbox made in Dutch showed "Inbox", "Sent Items"… where Outlook
  shows the names Exchange gave them ("Postvak IN", "Verzonden items"). Special folders from
  Exchange (EWS and Graph) keep the server's name, shown as it is rather than translated; IMAP and
  JMAP folders are as before.
- **The conversation above a message:** the reading pane listed every message of a conversation
  above the one opened even when the list wasn't grouped by conversation, and nothing turned it
  off. As Outlook, it shows only while View › Conversations is on, and turning that on or off
  changes the open message at once.
- **The outgoing server's own sign-in (issue #3):** IMAP or POP and SMTP shared one user name and
  password, so an account whose servers sign in differently couldn't send. Add account and Server
  settings have "The outgoing server has its own user name and password"; its password is kept in
  the keychain apart from the account's and never in MailCraft's files, stays when left empty in
  Server settings, and is forgotten with the account. Sending without it says where to enter it
  rather than signing in as the IMAP user; a test checks the SMTP server sees its own user.

### v0.20 — A window on every Windows PC
- **No GPU, no window:** on some Windows PCs without a GPU the app never showed its window. The
  process stayed in the background, kept later launches out (its data files were open: "open in
  another copy of MailCraft") and crashed when Windows shut it down. wgpu started Vulkan and
  OpenGL as well as DirectX 12; without a GPU those load only leftover or overlay drivers and
  Windows' OpenGL 1.1 software renderer, while DirectX 12 works through WARP. On Windows only
  DirectX 12 starts now, and MailCraft picks the adapter: a real GPU first (discrete, or
  integrated with `WGPU_POWER_PREF=low`), WARP when there's none, all of them and the choice in
  `mailcraft.log`. Shaders are compiled with FXC, part of Windows, rather than any
  `dxcompiler.dll` on the search path. `WGPU_BACKEND` still overrides it, and
  `WGPU_FORCE_FALLBACK_ADAPTER=1` draws with WARP.
- **A window that can't open says why:** an error or a panic while the window opens shows a
  message box with the reason, what may help and where the log is, and the process ends.
- Checked on a PC with an RTX 4070 Ti SUPER, an integrated Radeon and WARP: the GPU draws (185 MB
  of its memory, the others none); forced, WARP draws the whole window (200 MB on the Basic
  Render Driver); with no adapter, the message shows and the process exits on OK.
- **What the other Craft apps learnt:** their fixes to shared ground (graphics start-up,
  Windows, Linux, packaging) were read across sixteen repositories and checked against
  MailCraft; the ones that applied are in:
  - the device asks for the adapter's own limits and whole texture size (a GPU short of egui's
    fixed limits, or a window wider than 8192 pixels, ended the start); after its first frames
    a wgpu error is logged rather than ending the app; Intel Macs leave out wgpu's indirect-call
    validation, which Metal's Intel compiler aborts on; Linux draws on the desktop's GPU;
  - on Windows a window saved maximized no longer appears, vanishes and appears again at
    start-up (measured 6 times in 6; now shown once, maximized, through the patched winit);
  - on Linux the window opens through XWayland when there is one, so attachments drag in and
    out on Wayland desktops (a stale `DISPLAY` opens on Wayland), and clipboard reads give up
    after two seconds instead of freezing the window while another app hangs;
  - the DMG is built with `hdiutil create`, as `makehybrid` left Finder information on the app
    that `codesign --strict` rejects, and the MSI says MailCraft needs Windows 10 or later
    rather than installing an app that can't start on 7 or 8.
- **WordCraft, PdfCraft, DeckCraft and GridCraft, every change to their desktop glue:** the
  four closest siblings' 367 commits to their desktop apps, packaging and build tools were read
  one by one (not by keyword), and these adopted:
  - winit 0.30.13 as PdfCraft carries it (now storytold/winit, the fork the apps share): a
    window dragged to a monitor with another scale on Windows 11, input methods on Windows
    (Korean), X11, Wayland and macOS, and windows created maximized;
  - windows come back on screen when their monitor went, and fit a screen smaller than they were;
  - an argument that isn't Unicode no longer ends the app or the CLI before they start, messages
    on start-up can't panic on a broken standard error, and quitting on macOS no longer crashes;
  - `mailcraft.exe` declares per-monitor DPI awareness;
  - files handed to other apps (an opened attachment, video frames) go in a folder of the user's
    own, not in a shared `/tmp`.
  Looked at and left: OpenGL fallbacks (DirectX 12 with WARP covers a PC without a GPU),
  egui's keyboard zoom (MailCraft binds none of its keys), GridCraft's damaged-font guard
  (egui 0.37 skips such a font; a test keeps it so), links (MailCraft already opens only web and
  mail links), single-instance hand-off and portable mode (features of their own), WordCraft's
  built-in file browser for Linux desktops without a file chooser (a feature of its own), and
  app-specific fixes.
- **Calendars from issue #1, each reproduced first** (a fake Exchange with two accounts, a Dutch
  mailbox, a shared mailbox's calendar and a Teams meeting as Teams writes it):
  - every account's own calendar was "Calendar" in one colour; with several accounts they're
    listed under each one's address, an account's own calendar in the account's colour (its
    events too) and named as the server names it ("Agenda"); side by side, two of one name say
    whose each is;
  - a shared mailbox's calendar never showed; it's read with the mailbox, named after it,
    editable as its rights say (one not shared with the account is passed over quietly);
  - a meeting's links couldn't be opened from its notes; Join meeting opens its Teams, Zoom,
    Meet or Webex link (Graph's `onlineMeeting.joinUrl`, else the first meeting link in its
    body, read whole from the HTML: the text's footnotes cut a 904-character link to 395), and
    its notes' links are listed to open;
  - a meeting in two calendars at once didn't open (a click started a new event behind it).
- **Check for updates (File › About):** the newest release from GitHub (pre-releases included,
  and a later test build of this version told by the build's commit), Download offering this
  system's installer (or the release page on Linux), What's new; nothing installs by itself.
- **A Microsoft 365 sign-in kept on Windows (issue #5):** "the sign-in couldn't be kept in the
  keychain: password encoded as UTF-16 is longer than platform limit of 2560 chars". Credential
  Manager holds 2560 bytes an entry and the keyring crate wrote UTF-16, so nothing over 1280
  characters fit, and a Microsoft refresh token is about 2,000. Reproduced on Credential Manager;
  a secret is now kept as UTF-8 over as many entries as it needs (earlier passwords still read,
  leftover parts removed), tested there growing, shrinking and removed.
- **Online only, and a portable copy that leaves nothing behind (issue #4):** asked for to run
  MailCraft from a USB stick or on a shared PC with no mail left there, and without downloading
  thousands of messages. Reproduced first: one IMAP account, five messages, twenty seconds, and
  its data file (557 KB) held every subject and body; the password was in Credential Manager.
  - **Online only, per account** (Add account, Account settings, File › Options › Mail): the
    store holds that account's folders, lists, message sources, changes waiting for the server,
    Outbox, calendars, contacts and notes in memory (`Store::set_online_only`); turned on, what
    was kept comes into memory and its data file goes; turned off, it's written out again; at
    start it's held in memory from the outset (a data file left by a crash goes). Nothing is
    downloaded in full ahead of reading: a message comes when it's opened. Its snoozed mail is
    remembered without the subject, its notifications name neither sender nor subject (Windows
    keeps them), and attachments opened with other apps go from the temporary folder when
    MailCraft closes. Not for POP, whose mail has no other home, nor the sample mailbox. As
    Thunderbird's "Keep messages for this account on this computer" off, the list still comes
    from the server at each start.
  - **Portable** (the Craft apps' scheme, as PdfCraft and PhotoCraft): `portable.txt` or
    `MailCraft.portable` beside the app (beside the `.app` on macOS) keeps the account list,
    the window's state, the log and the data files in `MailCraftData` there; passwords and
    sign-ins stay in memory (asked again at the next start), nothing goes to the registry, and
    a new account starts with Online only ticked. A folder that can't be written turns it off
    and says why. The Windows portable zip ships `portable.txt`.
  - Checked with the app against a fake IMAP server: online only, the mail listed and opened,
    the app killed, and none of its subjects or bodies anywhere in the data folder; portable,
    `%APPDATA%`, Documents, Credential Manager and the registry the same before and after.
    Turning online only on for an account that already kept mail removes its data file, but
    the disk may still hold what the file held until it's overwritten: for no trace at all, add
    the account with Online only ticked. What another app has open (an attachment in Word) stays
    until that app lets it go, and what Windows keeps of its own (its search index, if the
    data folder is indexed; the GPU driver's shader cache) is outside MailCraft's reach.

### v0.21 — One MailCraft at a time
- **Started again, the running MailCraft comes forward:** a second launch (a second click on the
  taskbar or Start, a shortcut, a `mailto:` handler) opened another window on the same data,
  whose accounts' data files were open in the first ("open in another copy of MailCraft"), and
  its log replaced the running one's. Now the first MailCraft on a data folder listens on a
  socket in the user's own scratch folder, named after the data folder and the Windows session
  or display, and a later launch on that data asks it to show its window and leaves before it
  touches anything. The window comes out of the taskbar and in front (on Wayland it asks for
  attention, as it can't put itself in front), handled while minimized too. Another data folder
  is another MailCraft. Launches started together take turns, a socket left by a crash is
  replaced, and any failure leaves the launch a window of its own, as before. As PdfCraft does
  (`single_instance.rs`); macOS, where Finder already brings a running app forward, gets it for
  launches from a terminal or a script.
- Checked in the app: the first window minimized behind Notepad, a second launch leaves in 181 ms
  and the window is restored and in front; after a clean close or a kill, the next launch opens.
- **Shared mailboxes as mailboxes of their own (issue #1):** a shared mailbox an account opens
  was listed under that account, in the folder pane and the calendar. Each is now a mailbox of
  its own there, after its account at first, and moves as an account does (drag its heading, or
  Move up and Move down in its menu); the order is kept with the settings, and the calendar
  list follows it, each shared mailbox's calendar under its own name.
- **Invitations that read, dialogs that fit (issue #1):** an Exchange invitation's notes came as
  Exchange makes HTML text, each link written after its words in angle brackets, and showed so
  in the notes box, long Google links and all; the dialog grew taller than the window, its title
  and buttons off screen (a 2160-pixel screen at 150%). The notes now read as the invitation
  does: each link under its words (`Join with Google Meet`, `View all guest info`, a guest's
  name, Google's Yes, No and More options), long addresses shortened, phone links as text, with
  Edit notes for the box. Every dialog stays within the window, what doesn't fit scrolling.
- **Mail past what's kept (issue #4):** past the newest 1,000 a folder keeps (File › Options),
  nothing could be seen or searched. The end of a folder's list now says how many more its
  server has, with Show more (as many again, kept for that folder across syncs:
  `windows.json` with the account, for every kind of account); the end of a search has Search
  the server, which asks IMAP servers (`UID SEARCH`, UTF-8 words as literals) in the folders
  searched, and what they find comes into those folders' lists and the results, found by words
  in mail not downloaded too. Bringing in older mail takes the newest of what isn't kept, so a
  message found by searching doesn't leave a gap.
- **Online only made clearer (issue #4):** its explanation now says that sending goes straight
  to the server, that only mail that can't go yet waits, and which calendars, contacts and notes
  stay (those on a server) and which go (those kept only in MailCraft). Closing with unsent mail
  or changes not yet on the server of an online-only account asks first. An account with no
  password kept (a portable copy's) asks for it when MailCraft starts, once. File › Options says
  where the account list and settings are (the app folder, not the data files' folder in
  Documents, where 3dluvr looked).
- Checked in the app against fake IMAP (1,500 messages, online only) and Exchange servers: the
  shared mailbox moved above its account, in the calendar list too; the Google invitation read
  with its links, the dialog scrolled to its Save; Show more brought the 501 others; Message 120,
  not kept, found by Search the server and opened; offline, a flag change made closing ask, and
  Close anyway closed; a portable copy asked for its password at its second start.
- **Searching the server for every kind of account (gap #7):** Search the server asked IMAP
  servers only; an Exchange, Microsoft 365 or JMAP account said it couldn't. Each now asks its
  server its own way: JMAP an `Email/query` filter (`text`, `from`, `subject`, `NOT`, the `$seen`
  and `$flagged` keywords), Microsoft Graph `$search` (KQL, the whole quoted) or, for the read and
  flag state alone, `$filter`, and EWS a `QueryString` (AQS) or a `Restriction`. What a user types
  can't break out of the query (quotes and backslashes dropped, XML escaped). What's found stays
  in its folder past the newest kept, as IMAP keeps what it fetches: each sync checks the server
  still has it there and follows its read and flag state, and drops it when it's gone. The
  browser searches JMAP servers and shows more too (its own `windows.json`).
- **JMAP folders kept no more than one request's worth:** a JMAP folder kept at most the server's
  `maxObjectsInGet` messages (often 500, 100 on some), whatever it was to keep, so Show more
  showed nothing more. The newest are now asked a page at a time.
- **The reading pane's header in a narrow pane:** the date ran over the sender's address; it now
  keeps its room, or goes under the sender when the pane is too narrow, and Reply, Forward,
  Print, Save and More wrap instead of being cut off.
- Checked in the app against a fake Exchange server (1,500 in a folder, 1,000 kept): Message 321
  found by Search the server and opened; a second search and a sync kept it; Show more brought
  the other 499; a query of quotes, a backslash and `]]>` asked without an error. And in the
  browser against a fake JMAP server (601 in a folder): 300 kept (100 before), Search the server
  and Show more (all 601).
- **Edits made mid-sync:** the pool handed a flag change, move or delete to the account
  thread, which only picked it up after a full sync (minutes on a big Exchange mailbox). Until
  then a folder refresh could repaint the old state, and quitting discarded the job. The pool now
  writes the edit to the cache and `ops.json` itself; the thread uploads between folders and
  re-applies queued edits to any folder it refreshes. `write_atomic` gives each call its own
  temporary file, since two threads may now write the same file.
- **Graph `$batch`:** the immutable-id preference was sent only on the outer request, so batched
  message fetches returned ids in the other format; the next sync didn't recognise them and fetched
  again, and server-search hits vanished. Every entry now carries the header. Batches shrink to 4
  entries and 429 entries are retried after `Retry-After`. When more than 20 messages are new, the
  summaries come from the listing instead of one request per message; the downloaded source later
  supplies threading and importance.
- **Message windows:** saved geometry went into the viewport builder every frame, and since all
  message windows share one saved place, moving one pulled the others onto it. Geometry is now
  applied only as a window first appears.
- **Exchange calendars and contacts:** several non-recurring appointments with the same UID were
  merged into one (now `<uid>/<start>`); a birthday saved without a year showed the placeholder
  1604 (now "20 February", and saved back with the placeholder); folders with
  `PidTagAttributeHidden` set, and their subfolders at any depth, are no longer listed.
- **Connecting:** the two address families alternate, each attempt limited to 3 s unless it's the
  last candidate, and IPv4 leads after an IPv6 miss; a connection whose exchange failed is never
  reused; the account reports Online as soon as mail is done, with calendars and contacts after.
- **Test builds update themselves (v0.22):** File › About only linked to a newer build. A build
  from the unsigned test-build workflow now checks for the next one shortly after start and about
  hourly, downloads the installer matching how it was installed (Windows `.msi` or portable
  `.zip`, macOS `.dmg`, Linux `.AppImage`), accepts it only if its size and SHA-256 match the
  release's record, and installs it as MailCraft closes, or at once from **Restart to update**
  (status bar, File › About). `.deb`, `.rpm`, Flatpak and FreeBSD copies keep the Download link.
  `MAILCRAFT_NO_AUTO_UPDATE=1` turns it off; signed releases never update themselves this way.

### v0.22 — Outlook's everyday tools
- **Reminders:** an event's reminder (Outlook's list, None to 2 weeks; 15 minutes for a new
  one) is the VALARM it carries, kept by CalDAV and mapped to Exchange's and Graph's own
  fields. When one comes due a Reminders window opens (each with its time and how soon,
  overdue in red; Dismiss, Dismiss All, Open Item, Snooze from 5 minutes to a day or "0 minutes
  before start") and a desktop notification is shown once. Answers are kept per occurrence.
- **Junk Email lists:** Block sender, Never block sender and Never block sender's domain (the
  ribbon's Junk drop-down and the message's menu), and both lists in File › Options. Blocked
  senders' mail goes to Junk as it arrives; safe senders' mail filed in Junk by the server comes
  back to the Inbox, and their pictures load. Not junk can also trust the sender.
- **Clean Up:** Clean up conversation and Clean up folder (Delete drop-down) move to Deleted
  Items each message a later reply in its chain quotes in full, as whole words; never one
  unread, flagged, pinned, categorized, not downloaded, or with attachments the reply lacks.
  Undo brings them back.
- **Search Folders:** saved searches in the folder pane (unread, flagged, important,
  categorized, with attachments, large, pinned, from specific people, or any search), over every
  folder but Deleted Items and Junk. The search language gains `larger:`/`smaller:` and lists
  in `from:`/`to:`.
- **Quick Steps:** named sets of marks and one move, from the ribbon or Ctrl+Shift+1 to 9.
- **Calendar:** the Work Week view (Monday to Friday).
- **Writing:** Insert date & time (written out, as numbers, with the time, or the time alone)
  and Word count (words, characters with and without spaces, paragraphs, lines; the selection's
  when there is one).
- **Reliability found on the way:** settings are saved every few seconds when they change, not
  only on a clean close; the log's terminal echo and the CLI no longer panic when their output
  is gone; the ribbon drops labels from its end when the window is too narrow for them; Junk's
  high-water mark is set when the folder is known; changes made with `mailcraft-cli run` now
  reach the server with the next `sync` (they were dropped).
- **Several messages at once:** New mail, Reply or Forward while a message is being written
  opens another window, as Outlook does; the first stays open (it was dropped, unsaved). Each
  window keeps its own selection and undo, and its dialogs show in it. Closing MailCraft with
  messages written asks to save them; saved, each is kept on this computer at once and filed in
  Drafts on the server at the next connection.
- **Notifications from a portable copy** say MailCraft when an installed MailCraft is on the
  computer (its Start menu shortcut is what Windows goes by); a portable copy alone still shows
  under Windows PowerShell's name, since naming it would mean writing to the registry or the
  Start menu.
- **Motion:** hovers and selections ease in and out (ribbon buttons, folders, messages; the
  selection bar grows from its middle), the ribbon tab's underline glides to the tab chosen, the
  folder pane slides between its full and minimized widths, a message just opened fades up into
  the reading pane, and notices slide in and fade away. Coming in decelerates, leaving
  accelerates, in 100 to 250 ms, on cubic Bézier curves solved in `motion.rs`. Play animations
  (File › Options) turns it all off, egui's own fades and smooth scrolling included. The
  ribbon is one height on every tab, so switching tabs no longer nudges the window below.
- **Issue #4 again, on Dovecot:** Show more did nothing there. A server with QRESYNC (Dovecot
  has it) syncs a folder from what its `SELECT` says, and that way never took in the larger
  window; both ways now do. Search the server searched each folder whole in one `UID SEARCH`,
  showed nothing until every folder was done, and went on after Esc (Dovecot reads every
  message when it has no text index). A folder is now searched newest first, a range of message
  numbers at a time (`UID SEARCH 2501:3000 TEXT …`, the ranges growing while the server answers
  within a second and shrinking past four), what each range finds shown at once; clearing the
  search box, a new search or leaving the folder stops it between two ranges, and the account
  goes on with its other work. JMAP, Graph and EWS show theirs a folder at a time and stop the
  same way, in the browser too.
- **The search box, as Outlook's:** where to look on its left (Current Folder, Subfolders,
  Current Mailbox, the default, or All Mailboxes; kept for the next searches, and in the
  Search tab and the list's header too); the last ten searches drop down from it (kept on
  Enter, a pick, or leaving the box with a search in it, never every word typed; Clear recent
  searches; in memory only while an account is kept online only, since what's searched for
  tells what the mail says); and Advanced search under the chevron on its right (words, From, To, Subject,
  message text, has attachments, unread, flagged, high importance, and other terms kept as
  written), filled from what's in the box and written back to it.
- Checked in the app against Dovecot 2.3 in Docker (3,000 messages, 25 kept, online only),
  the build before the fix first: Show more sent `SELECT … (QRESYNC …)` and nothing else, the
  list still 25; now it fetches the next 25 (`UID FETCH 2951:2975`), 50 shown. Search the server
  for "needle" asked `UID SEARCH 2501:3000`, `1501:2500`, `1:1500` and showed its 30.
- Not done: Contact groups (distribution lists), Copy Folder, Read Aloud, and spell checking
  (needs a dictionary MailCraft can ship). IMAP METADATA (RFC 5464, suggested on issue #4) is
  for annotations on mailboxes, not for searching; a server's own text index (Dovecot's FTS) is
  used by the same `SEARCH` without MailCraft asking anything else, and a fuzzy search (RFC 6203)
  is left for when a server that offers it can be tested.

### v0.23 — Mail that can't fool you, and a hardening review
- **Photos without where they were taken:** a phone writes the place into every photo; it's
  taken out of JPEG and PNG attachments as a message is sent (the EXIF GPS block zeroed where it
  lies, an XMP position left out, PNG's `eXIf` too), the picture, orientation and date untouched.
  The message being written says so under its attachments; File › Options › Safety can keep it.
- **Senders passing for someone else:** a red bar when a sender's domain imitates one the user
  deals with (their own, and the people they write to or keep as contacts): look-alike letters
  (Cyrillic, Greek, accents, `0` for `o`, `rn` for `m`), punycode, one letter added, dropped,
  changed or swapped, or the real domain worn inside another (`paypal.com.account-check.example`);
  and when the sender's name shows an address that isn't the one it came from. In the reading
  pane, a message's window, and a message opened from a file or attached to another.
- **Check before sending:** a message to a recipient whose domain looks like one the user
  writes to, or showing eight or more people who may not know each other each other's
  addresses (each organisation counted once, each address at a big mail service on its own), is
  held with Check before sending: Send anyway, or Put everyone in Bcc.
- **Trackers:** a message says how many tracking pictures and click-reporting links it carries;
  tracking pictures are never fetched, even after Show pictures; a link opens out of redirect
  wrappers that carry the real address (Safe Links, Google, Proofpoint, Amazon SES and others)
  and without the tags that only identify the reader (`utm_*`, `fbclid`, `mc_eid`…). File ›
  Options › Safety can open links as they are.
- **Hidden text:** HTML mail hiding 25 words or more (zero size, no height, `display:none`,
  `opacity:0`, `hidden`) says so, with why it matters (spam filters, and instructions for the
  tools and assistants that read mail for someone) and the text to read.
- **Waiting for a reply:** `is:awaiting` finds mail you sent two to thirty days ago that nobody
  has answered (no message from anyone else names it as the one it answers or follows); a
  Search Folder preset and a Search tab toggle offer it.
- **A hardening review, area by area (2026-10-23):** sync and protocols, the mail window,
  calendar and contacts, security, and the platform, with the Craft family's practice (WordCraft,
  PdfCraft, GridCraft, DeckCraft, craft-libs, craft-i18n, craft-common-actions) as the yardstick
  for what the apps share. Each problem was reproduced here first and fixed with a test:
  - *Safety:* an attachment that runs code (`.exe`, `.hta`, `.lnk`, `.iso`…) was handed to the
    system to open; it no longer is, and saved attachments carry Windows' internet-zone mark.
    File names lost right-to-left overrides, zero-width marks and device names (`CON`). A
    message's pictures reach the internet only, never this computer or its network. A bare CR in
    a hostile Message-ID no longer reaches a reply's headers. One IMAP command's answer is capped
    (256 MB, a million responses, 15 minutes); a hostile server could grow memory without end.
  - *Data:* answering an update to one occurrence of a series deleted the whole series, here and
    on the server; an invitation's update or cancellation is applied only when it comes from the
    meeting's organizer, and never over a newer copy; long event uids no longer share a CalDAV
    item name. The cache's files reach the disk before they replace the old ones, and are
    private to their owner on Unix.
  - *Sync:* an OAuth token that ran out made the account ask to sign in again; it's renewed once
    first. Mail that arrived between the Inbox's sync and IDLE waited up to ten minutes. An SMTP
    refusal whose text said "authenticated" stranded the message as refused for good.
  - *Everyday use:* Ctrl+Q (Mark as read) also closed MailCraft. Files dropped from the desktop
    onto a message are attached. Results of a search over several folders say which folder.
  - *Platform:* the 32-bit Windows build never opened its window (DirectX 12 can't make its
    pipelines in a 32-bit process; it draws with OpenGL now). `--help` prints the options. A
    manual release with a version override names its files right; riscv64 packages declare the
    glibc they need; notarization stapling is retried.
  - Checked against real servers in Docker as well (`fakes/real_servers.rs`, run by hand): on
    Dovecot, Show more, the server search in ranges, a 41 MB message whole under the new caps,
    and mail from another client noticed in under a second; on GreenMail, sending.
  - *Calendar time zones:* an event keeps its zone (`TZID`: the IANA database's, Windows' names,
    or the file's own `VTIMEZONE`, written back with it) and repeats at its wall-clock time there
    across summer time, on that zone's days; deleted and moved occurrences match in it, a
    date-only `UNTIL` is that zone's day, and times show in the viewer's zone with the offset of
    their own date. Rules: `BYSETPOS` picks among a period's days ("the last weekday"), `BYDAY`
    limits a daily rule, and `BYMONTHDAY` with `BYDAY` means both (Friday the 13th).
- **Fuzzing and dependency checks:** `fuzz/` holds a cargo-fuzz target for every parser of
  server and file input (messages, HTML, iCalendar, vCard, IMAP, JMAP, the search box, `.pst`,
  photos), run weekly; the first runs found three crashes, fixed. cargo-deny checks licences,
  advisories and sources (`deny.toml`).
- **The system's mail app:** `mailto:` links start a message (RFC 6068: To, Cc, Bcc, subject,
  body), and `.eml`, `.ics` and `.vcf` files open (a message shown, events and people added),
  from the command line or Open with, handed to a MailCraft already running. The installers
  offer MailCraft as the mail app and for those files.
- Left from the review, larger: Microsoft Graph and JMAP incremental sync, screen-reader names
  on the custom widgets, HTML mail's box styles (padding, borders), the Linux AppImage's
  libraries, Linux's dark-mode setting, and Cmd+Q on a Mac asking about unsaved messages.

### v0.24 — Signed and encrypted mail (S/MIME)
- **Certificates:** File › Info › Security imports a `.p12` or `.pfx` file (PBES2 with AES, and the
  older SHA-1 3DES and RC2-40 ones Windows and older programs write), shows whose it is, who
  issued it and until when, and replaces or removes it. Its private key is kept in the system's
  keychain only; MailCraft's files hold the certificate. Sign and Encrypt new messages are
  switches there and on each message being written.
- **Reading:** a signed message says who signed it and whether to trust that: the signature over
  what was signed, a chain to a root the system trusts (or the root that came with one's own
  certificate), its dates, that it's for email, and that it's the sender's address; otherwise
  which of those failed, or that the message changed after it was signed. Detached and opaque
  signatures, RSA PKCS #1 v1.5 and PSS, ECDSA. Encrypted mail (AES-CBC, AES-GCM, 3DES; PKCS #1
  v1.5 and OAEP) is decrypted in memory only and shown with what it held: attachments, previews,
  invitations. Following EFAIL, only a message protected as a whole counts, nothing from the web
  loads in decrypted mail, and CBC without a signature says it could have been changed.
- **Sending:** signed (SHA-256 with the attributes RFC 8551 asks for, the chain without its
  root) and/or encrypted to every recipient and to oneself: AES-GCM when they all said they read
  it, else AES-256-CBC, which every program reads. People's certificates come from their signed
  mail; a recipient without one, or one's own certificate out of date, is said before anything
  goes. Replies to encrypted mail are encrypted; drafts of encrypted mail are encrypted to oneself.
- Checked with OpenSSL 3 in Docker both ways (MailCraft reads OpenSSL's signatures and encryption
  of every kind above; OpenSSL verifies and decrypts MailCraft's), and end to end against GreenMail
  (`fakes/real_servers.rs`), and in the app with the system keychain. Found on the way and fixed:
  the GCM tag length must be written out, and a chain that carries its own root is still trusted.
  A fuzz target (`smime`) reads BER, signatures, encrypted mail, certificates and `.p12` files.
- **The message list** marks signed mail with a seal and encrypted mail with a padlock.
- **Header protection (RFC 9788):** the subject, sender and recipients go inside the signature
  and the encryption too; encrypted mail's subject reads `[...]` on the way (File › Info ›
  Security can keep it visible), and a protected message's own header fields are the ones shown.
- **EC certificates encrypt and decrypt** (ECDH, RFC 5753): SHA-384 derivation and AES-256 wrap
  sent, SHA-1 to SHA-512 and every AES wrap read. Checked with OpenSSL both ways.
- **Revocation, opt-in** (File › Options › Safety): a signer's certificate is asked about where
  it says, OCSP first, then its CRL, each answer counting only when its authority signed it, it's
  about that certificate and it's current. Withdrawn says so in red; no answer is said as such,
  never taken for withdrawn. Off at first, since asking tells the authority whose mail is read.
- **Smart cards (PKCS #11) and the macOS Keychain:** File › Info › Security › Use a smart card…
  loads the card's module (OpenSC's reads most cards), finds the certificates with their keys and
  uses one; the key never leaves the card, which signs, decrypts and agrees EC secrets itself. On
  macOS the Keychain's identities are offered too. Tested against SoftHSM 2 (`crates/keys/softhsm.sh`),
  OpenSSL checking the card's signatures.
- **The Windows certificate store:** Use one from Windows… lists Current User › Personal's
  certificates for email that have a key (a `.pfx` double-clicked into Windows, one IT deployed, a
  smart card Windows reads itself), the key staying in CNG, non-exportable ones included. Listing
  and signing go through `rustls-cng`, as PdfCraft signs; RSA decrypting and EC key agreement go
  through `crates/keys/src/windows/cng.rs`, the one isolated `unsafe` file (with `macos/sec.rs`,
  which lets an EC key in the Keychain decrypt too). Tested against CNG itself: OpenSSL's RSA mail
  (CBC, GCM, 3DES, OAEP) and EC mail (ECDH) decrypted in Windows, signatures OpenSSL verifies,
  malformed keys and points refused, and the test identities in the real store with keys that
  can't be exported, then removed.
- **In the browser:** the `.p12` itself is kept (it's encrypted with its password, which isn't),
  and opened with its password for the page; signing, encrypting and decrypting are the desktop's
  code. After a reload, Unlock… gives the password again.

### v0.25 — Updates that finish
- **Said when it's ready:** a test build's downloaded and checked update brings a bar above the
  panes (*Restart now*, *Later*), and the system's notification when MailCraft isn't in front;
  the status bar shows the download's progress. Before, it waited silently for MailCraft to
  close, so someone who never closed it never got it.
- **Closing asks:** *Install MailCraft 0.25.0 as it closes?* (Update and close, Close without
  updating, Cancel); with a download still going, that it pauses and carries on next time. If it
  finishes while asked, the question becomes the one about installing.
- **Downloads that carry on:** the installer goes to a partial file as it comes, and one cut off
  (the connection, or MailCraft closed early on a slow connection) carries on with an HTTP range
  request, in this run or the next, before its SHA-256 and size are checked. Checks go on hourly
  while MailCraft is open, after a failure from 5 minutes, doubling.
- **Windows, macOS and Linux:** the MSI still installs for everyone, and the bar and the close
  question say Windows asks for an administrator. An AppImage in a folder this user can't write is
  left alone; macOS finds Craft apps in `~/Applications` too. What each platform does:
  [docs/updates.md](updates.md).
- **Checked:** unit tests for the streamed download, range resumes, wrong files, back-off, the
  install kinds and the helper scripts; headless UI tests for the bar, the notification and every
  close question (Restart now and Cancel included); a fuzz target, `http`, for HTTP answers read
  whole and streamed, a download's `Content-Range` and what the updater reads from GitHub (2.4
  million runs clean). An independent review of the whole change found six problems, each fixed
  with a test where one fits: a failed download not tried again in the same run, Restart now
  outliving a cancelled close, a macOS swap that could put a partial copy in place, paths with a
  typographic apostrophe, a server answering with a part from the start, and a race at closing. By hand: a test build updated a portable copy to `v0.24.0-test.21` from GitHub
  through the bar, the close question and Restart now, and resumed a 54 MB download from 20 MB.

### v0.26 — Renewed certificates, calendars anywhere, the classic ribbon
- **Renewed certificates (issue #6):** importing a renewed certificate no longer drops the old
  one. The newer (in date, then issued later) signs and encrypts; the older is kept with its key
  to decrypt mail that was sent to it, in either order of import. File › Info › Security lists
  each older one with *Use for sending* and *Remove*. A message is decrypted with the key it
  names, so mail for a certificate MailCraft doesn't have says so instead of trying every key (a
  smart card would ask for its PIN). At most 16 older ones. The browser does the same with
  the `.p12` files it keeps (each still under its own password), as Thunderbird's certificate
  store, the system stores Outlook uses and Proton's older keys all keep old keys for reading;
  Unlock… opens every one its password fits.
- **Keys go with the account:** removing an account now removes its certificates' keys and its
  calendar password from the system keychain; before, the S/MIME key stayed behind.
- **Calendars anywhere (issue #7):** discovery follows RFC 6764 fully: the path in the TXT record
  beside the SRV one, then the well-known address, then the server's root, on the SRV record's
  port. Posteo publishes `path=/` on port 8443, where `/.well-known/caldav` answers 404, so its
  calendar was never found before. For a server that still isn't found, or one that wants an app
  password, File › Info › Account settings › *Calendar server…* takes CalDAV and CardDAV
  addresses (the server's, a home, or one calendar as a provider's help page gives it) and a user
  name and password of their own. Plain http is allowed only to this computer.
- **The classic ribbon (issue #8):** View › Classic ribbon (or File › Options) draws big icons
  with their names underneath, in named groups (New, Delete, Move, Respond, Tags, Find…), and
  scrolls rather than dropping labels in a narrow window.
- **Dark mode told apart (issue #8):** panes a step lighter than the folder pane and ribbon,
  borders 1.8:1 against a pane (they were 1.3:1), clearer fields, a line above the status bar;
  light mode's faintest text now passes 3:1. A test holds both themes to these contrasts.
- **Checked:** unit tests for which certificate a message is for (OpenSSL's and MailCraft's
  mail, RSA and EC), renewals in either order, use-older, forgetting, the cap, imports at once on
  threads; the TXT parser (cut and overlong records); discovery and hand-given addresses against
  the fake DAV server; `account.dav`'s validation; a headless test of the classic ribbon on every
  tab. Against real servers: Radicale (its own password) and GreenMail in Docker through the
  network pool, and Posteo's published DNS records; in the app, File › Info with two certificates
  and a hand-given calendar, and an account removed taking every keychain entry it had. An
  independent review found seven problems, each fixed with a test. After the calendar server
  changed, edits could still go to the old server with the new password. Remove could race
  "Use for sending". A new calendar user name reused the old one's password. A timeout read as
  "no calendars here". A local http address could redirect elsewhere in the clear. Re-importing an
  older certificate gave the wrong notice. The 16-certificate cap blocked swaps.
- **Against a real provider's setup:** docker-mailserver (Postfix and Dovecot) with Radicale. The
  whole renewal ran in the app over it: signed mail each way, Zoe encrypting to Riley's 2025
  certificate, Riley renewing, the old mail still opening, Zoe switching to the new certificate
  once Riley signed with it (checked in the CMS with OpenSSL). The real-server tests ran there
  too: 3000 messages, a 43 MB one, new mail noticed in half a second, the S/MIME round trip, a
  hand-given calendar. This found one bug: Postfix strips `X-Mailer` for privacy from every MIME
  part, so a clear signature over a protected copy of it broke. What sent a message is no longer
  copied inside the signed part. In the browser, both certificates were imported and kept across a
  reload, unlocked with one password and swapped.

### v0.27 — Arabic and Hebrew mirrored, colours of your own, Exchange's directory
- **Arabic and Hebrew mirrored inside the panes:** rows MailCraft paints (folders, accounts,
  messages, buttons) put icons and avatars on the right and counts and times on the left;
  rows egui lays out (the top bar, the ribbon, the reading pane's header, the message being
  written, the status bar) run from the right; scroll bars sit on the left and submenus open to
  the left. A Hebrew or Arabic paragraph being written starts on the right edge. Long names are
  cut to one line with "…" instead of wrapping (in every language). A value filled into a
  translated sentence (an address, a server's error) keeps its own order.
- **The Craft apps' egui and winit:** both now come from storytold's forks, pinned by revision,
  as the other Craft apps take them. The egui fork counts right-to-left letters right (stock
  egui 0.37 lost the caret, and deleting a word could empty the box) and draws right-to-left
  text in order itself, so MailCraft's own reordering, which turned Latin runs inside Hebrew
  backwards, is gone.
- **Nothing lost on closing:** closing MailCraft from the taskbar or Dock while minimized now
  asks before a message being written is lost (eframe ran only the app's logic then, and the
  window closed). On macOS ⌘Q asks too, and quitting goes through AppKit, so a Touch Bar Mac
  can't crash on the way out.
- **Screen readers:** every button, toggle, folder and message row MailCraft draws says what it
  is and what it's called (336 controls were unnamed); a question that must be answered is an
  alert dialog.
- **Crashes found by a random hand:** a seeded soak test plays the whole window (clicks, drags,
  keys, typing in five scripts, both themes, two widths). It found the message editor crashing on
  a cursor past the end of the text, and egui's right-to-left count; both are fixed.
- **Smaller fixes:** the cache rewrites a file OneDrive or an antivirus holds for a moment
  instead of failing; a relative `APPDATA`, `HOME` or `XDG_DATA_HOME` no longer places the mail;
  COM0, LPT0 and the superscript ports are saved as device names; a very tall PDF page or picture
  stays within the graphics card's limit; each ribbon tab keeps its own scroll place; file
  dialogs open where the last one was left; the System theme follows a Linux or BSD desktop's
  dark or light; Ctrl+V pastes a picture again; a picture in a message sits where it is in the
  text; accounts waiting for a password don't cover the status bar's counts; "Sent To", "Has
  Attachments", "To:" and "Cc:" are translated; on macOS an update retries a disk image the
  system is still busy with.
- **Packaging:** the RPM installs on openSUSE and requires the glibc MailCraft needs; the AppImage
  and MailCraft.app ship the licences; downloaded packaging tools are pinned and hash-checked; the
  riscv64 packages declare glibc 2.39; macOS packages are verified after packaging; Hebrew and
  Korean are drawn in the browser.
- **Colours of your own (issue #8):** File › Options › Accent colour offers eight colours
  (MailCraft's teal first) and a picker for any other. Each is made to read: darkened in light
  mode, lightened in dark, until text in it passes 4.5:1 on every pane, with the selection and
  hover tints worked out so text, dim text and the accent still read on them. *Plain top bar*
  draws the top bar in the panes' colours. In dark mode the top bar is now a deep shade with white
  on it: the light teal behind white text read 2.3:1.
- **Edges (issue #8):** the ribbon's buttons sit on a card with an edge, as Outlook's; a button
  under the pointer has an edge as well as its fill; menus, lists and dialogs that open over the
  window have a firmer edge and a soft shadow.
- **A User-Agent of the account's own (issue #9):** a server that lets in only the mail programs
  it knows answered MailCraft with HTTP 403. File › Info › Account settings (and Add account) take
  a User-Agent for an Exchange account, sent with every request to its server, Autodiscover's
  too; left empty, MailCraft names itself. Only visible ASCII, up to 256 characters: nothing
  that could end the header. Changing it connects again at once.
- **Exchange's directory (issue #9):** the Global Address List was never searched, so someone
  who hadn't written to the user, or joined since, wasn't found. Now Select Names has a Global
  Address List, *All addresses* includes it, and To and Cc suggest from it, asked of the server
  (EWS ResolveNames) once three letters are typed, or at once with Enter or →. Answers are kept
  five minutes, "no one" only half a minute, so a new employee turns up soon after the directory
  knows them; Refresh forgets them all and reads the address books again. The calendar and
  contacts are now read apart, so a calendar the server refuses no longer keeps the address books
  as they were.
- **Exchange Online's directory too:** an Exchange Online account (Microsoft Graph) searches its
  organisation's directory the same way: the directory itself (`/users`, everyone, someone who
  joined today included), or where a tenant won't list its users, the people relevant to the user
  with the directory's (`/me/people`). Its permissions (`User.ReadBasic.All`, `People.Read`) are
  ones users grant themselves, asked for in a sign-in of their own the first time: Select Names
  offers *Allow searching the directory…*, and the browser asks. They're never asked for with the
  mail's: a tenant that lets users consent only to some permissions would otherwise block the mail
  sign-in, and accounts signed in before couldn't refresh a token naming permissions they never
  granted. Its lasting sign-in is kept apart in the keychain and goes with the account; the
  permission taken away is asked for again. Personal Microsoft accounts have no directory.
  Checked against the fake Graph server over HTTP (each token good for its own permissions only,
  `$search` refused without `ConsistencyLevel: eventual`, the people only with the directory asked
  for) and the consent sign-in played through the browser's redirect; not yet against a real
  Microsoft 365 tenant.
- **Found on the way:** a user name with a line break or tab in it (a `\r` from JSON) was kept,
  and could never sign in: it's refused now. An answer to Select Names arriving while a person was
  selected could move someone else under the selection: the selection now follows the person. A
  sample mailbox whose folders were gone (an older build's data file) stayed on "Loading folders…"
  for good: it's filled again.
- **Checked:** unit and headless tests for the User-Agent (kept, refused, cleared, sent on every
  request), the directory (several found, one, none, an Exchange-only address, junk, a 100,000
  letter search), how long answers are kept, Refresh, Select Names updating with the selection
  kept, every accent and bar against the contrast tests. For real: HAProxy in Docker answering 403
  to anything but `Evolution/…`, in front of the fake EWS server signing in with NTLM: refused as
  MailCraft, synced as Evolution through the CLI, the network worker and the app; Dana Whitfield,
  added to the directory 45 seconds in, found by the next search. GreenMail and Radicale for the
  rest of the real-server suite; a soak of 30 seeds of 1,500 actions.

- **Gaps 2, 5 and 7 (and a look at 3 and 4):**
  - **Right-to-left mail selects (gap 2):** the Craft apps' egui counts right-to-left letters right, so Arabic and
    Hebrew mail is now selected with the pointer and copied as it reads (it couldn't be selected at all before),
    Latin names and times inside it included; right-click still copies a line or the whole message.
  - **Simplified Chinese in the browser (gap 2):** Noto Sans CJK SC (16 MB, from craft-fonts) is too large to build
    into the page, so the web package puts it beside it and the page fetches it only when it's wanted: the
    interface in Simplified Chinese, or mail with characters no font it brought can draw. Tried in the browser on
    Chinese mail delivered through a real server: one fetch, then the text drawn right.
  - **Kerberos-only Exchange (gap 5):** a server that offers only `Negotiate` (and one that lists NTLM but doesn't do
    it) is signed in to with Kerberos (sspi, MIT / Apache-2.0, without its network client: MailCraft talks to the
    KDC over TCP itself). The KDC is found from DNS (`_kerberos._tcp.<realm>`, as Active Directory publishes it) or
    `MAILCRAFT_KDC`; the realm from the user name (`riley@corp.example`, `CORP\riley`) or the address. No KDC: NTLM
    in Negotiate as before. A wrong password or unknown user says so plainly. Tried for real: Samba's Active
    Directory domain controller and Apache's mod_auth_gssapi (Kerberos only) in Docker before the fake EWS
    server, with the sign-in held per connection and per request, both name forms, a wrong password and an
    unknown user. (MIT's KDC alone isn't enough: sspi wants the PAC an Active Directory KDC gives.)
  - **Kept unlocked in the browser (gap 7):** File › Info › Security › *Keep unlocked in this browser* keeps the
    password that opened the certificates, sealed with an AES-GCM key the browser makes non-extractable
    (WebCrypto, kept in IndexedDB), and opens them again on the next visit; switched off, the sealed password
    goes, and the key once no account needs it. Tried in the browser against Stalwart: the key couldn't be
    exported, the kept file held only ciphertext, and after a reload a message was signed without the password,
    sent through the server and verified by OpenSSL.
  - **Found on the way:** the browser's sealing first used a view of a temporary for the nonce, which was gone when
    WebCrypto read it, so nothing it sealed opened after a reload: caught by the reload test, fixed. A JMAP account
    the server made with no identity to send from couldn't send; one is now made for the address (RFC 8621 §6).
    The certificate dialogs no longer say the password isn't kept when it is.
  - **Gaps 3 and 4 checked:** the `.pst` limits are the libraries' (above); Wayland drag-out comes with winit 0.31.

### v0.28 — Calendars from the web, full names from the directory
- **Calendars subscribed to by their links (issue #7):** *Subscribe to a calendar…* under the
  calendar list takes an iCal or `webcal://` link (`webcal://` read over HTTPS; a link on the
  user's own network works too) and an optional name. The calendar shows read-only beside the
  account's own, named as it names itself (`X-WR-CALNAME`, RFC 7986's `NAME`) unless the user
  named it, and is read again every three hours, or as often as it asks (`REFRESH-INTERVAL`,
  `X-PUBLISHED-TTL`, kept between an hour and a week), asking only for what changed (`ETag`,
  `Last-Modified`); a failure keeps what was read, says why in the list and is tried again in half
  an hour. Right-click refreshes or removes it. Its alarms aren't reminders (a holiday list's
  aren't the user's), and a sync of the account's own calendars leaves it alone. The browser
  reads it too where the calendar's server lets web pages (CORS); `mailcraft-cli run
  calendar.subscribe` reads it before it returns. Tried for real: a calendar published by
  Radicale, a static file from nginx (unchanged on the second read, a 304), Thunderbird's German
  holidays over `webcal://` (58 events), a missing link, and a 5.5 MB file of 25,000 events (read in
  under half a second, up to the parser's cap).
- **Full names from the directory (issue #9, follow-up):** someone found in Exchange's directory
  showed as their alias (`john.doe`) when the server names the mailbox that way and MailCraft
  had no mail from them. The search now asks for the full contact and takes its display name, else
  its given name and surname, before the mailbox's name; and someone known only by an address (a
  contact saved without a name) takes the directory's name when found. Tried against grommunio
  (Docker), whose directory names a user with no display name by its alias: "Dana Whitfield" now,
  `dana.noname` before.
- **From the sibling Craft apps** (their fixes ported, their authors credited):
  - **Windows:** a pinned MailCraft kept a blank icon after an update (the shortcut took its icon
    from the installer's cache, which each upgrade deletes); the shortcut now points at
    `mailcraft.exe` and shows the icon built into it. A pin that already lost its icon needs
    pinning again once.
  - **macOS:** the system's open and save panels (attaching a file, saving an attachment, the
    overwrite question) stayed in English; `Info.plist` now names the 14 interface languages, and
    a test keeps the list matching the catalogs. The signing certificate is decoded in a private
    folder of its own, so a link planted in the temporary folder can't redirect it.
  - **Linux:** the deb and rpm recommend `libxkbcommon-x11`, without which the window couldn't
    open under X11 or XWayland ("Library libxkbcommon-x11.so could not be loaded"); tried on clean
    Debian 12, Fedora 42 and openSUSE Leap 15.6.
  - **Release builds:** one made without craft-fonts (the Arabic, Hebrew, Chinese, Japanese,
    Korean and Thai fallback fonts) fails instead of shipping without them.
  - **CI:** only the job that drafts a release can write to the repository; FreeBSD and Windows
    ARM64 build on release pushes, weekly and when their own files change, not on every push.
- **Found on the way:** an event in a read-only calendar (someone else's, one subscribed to) could
  be deleted: gone until the next read, or a deletion the server then refused. Refused now, as a
  change already was. A subscribed calendar whose last read was "in the future" (the clock set
  back) would have waited for the clock; it's read again.

### v0.29 — Pictures at their size, pasted files, signatures with pictures
Built on Remo Laubacher's pull requests #12–#16; what follows is where v0.29 ended up.
- **Search box (#12):** the placeholder and typed text sat a few pixels above the scope menu; they
  sit centred, in the People and Notes search boxes too.
- **Unread filter (#13):** a message opened with Unread on was marked read and dropped from the
  list, closing the reading pane as it was clicked. It stays in the list while it's read, as
  Outlook's, and goes once another is chosen.
- **Pictures in a message being written (issue #10):** every picture showed as a thumbnail 72
  points tall, whatever its size, and couldn't be resized. A picture now shows at the size it's
  sent and keeps its shape, never wider than the text; one wider than a page (624 pixels, Outlook's
  6.5 inches) goes in at a page's width, and one wider than a third of that on a line of its own.
  Clicked, it's selected (Delete removes it); its corner handle drags it bigger or smaller, undo
  takes that back, and its right-click menu sets it to its original size, 75 %, 50 % or 25 %. It's
  sent at that width (`<img width>`). `compose.resizePicture` does it from the command line;
  `compose.insertPicture` takes `width` and `line`.
- **Files pasted and dropped (issue #10):** files copied in a file manager are attached when
  pasted into the message: on Linux and macOS, where the file manager puts their paths on the
  clipboard as text too, Ctrl+V attaches them and the paths stay out of the text. Explorer puts no
  text there, and the Craft apps' egui sends no paste for its Ctrl+V: on Windows the text's
  right-click Paste attaches them. Files dropped on the main window with nothing being written
  there start a message with them attached, as Outlook does; while they're held over it, it says
  so. Dropping on a message being written already worked: tried with the 0.28 `.deb` on Debian 12
  under X11, and this build on Ubuntu 25.10 with a real drag and a GTK 4 clipboard holding files as
  Nautilus does.
- **HTML mail's pictures at their sizes (issue #10):** pictures showed at their own size, so a logo
  made twice as sharp as it says showed twice as big, a picture capped by `max-width` went past it,
  one with a height alone kept its own, a spacer took no room, and the layout around them moved.
  They now show as Outlook and browsers show them: at the `width` and `height` the message sets
  (attributes, or its style's, which win), the other side keeping its shape when one is set, no
  wider than `max-width` or the room. Pictures sent inside the message are decoded at a size to read
  them at (they shared the attachment cards' 480-pixel thumbnails). Checked against Edge's
  rendering of the same newsletter, sent through GreenMail.
- **WebP attachments (issue #11):** MailCraft read WebP pictures (lossy, lossless, with
  transparency, animated) and saved them byte for byte, but an attachment the sender gave no name
  was saved as `attachment-1`, and one named without an extension kept none: the system couldn't
  open either, nor the file saved. An attachment now gets the extension its type has (WebP, BMP,
  TIFF, HEIC, AVIF, SVG, icons, CSV, vCards, ZIP, MP4, MOV, MP3, WAV, besides those before), or,
  sent as plain bytes, the one its first bytes say (PNG, JPEG, GIF, WebP, PDF); nothing that runs
  code is ever added. A picture sent as plain bytes with a picture's extension is a picture: it has
  a thumbnail and a preview.
- **Signatures with pictures (issue #11):** signatures kept their formatting (bold, colours, sizes,
  links, as pasted from Outlook or Word) but not pictures, and the editor showed no way to format.
  The signature editor now has a bar of its own (the message's formatting, a link, pictures), takes
  pictures from a file or pasted (up to 1 MB each, 10 at most), and keeps them in the signature's
  HTML (`data:`), so they come with it into every message and are sent inside it. A signature
  copied from Word or Outlook with its logo brings the logo when pasted, into the signature
  editor or a message, at the size it had there (only its text and look came): Word writes the
  picture into the copied HTML, and older Office versions point it at a file in their clipboard
  folder in the temporary folder, which the desktop app reads (that folder only, pictures up to
  1 MB, so copied HTML from a web page can't have a file of the user's attached). Tried with Word's
  own copy and paste on Windows.
- **Found on the way:** a picture at the end of a line had no room in the line (egui sizes a row by
  its letters), so a second one after it was squeezed into what was left; the scroll bar took
  the pointer from the corner of a picture as wide as the text; and a drag on a picture's handle
  without selecting the picture first lost the handle (#14's test found it). Attachments opened
  or dragged out failed where `$XDG_RUNTIME_DIR` names a folder that isn't there (WSL): the
  cache folder is used then (#14). A signature's pictures had one Content-ID in every message,
  where RFC 2392 asks for a unique one: each message gives them its own (#16).
