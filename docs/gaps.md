# Gaps

What MailCraft lacks compared with the everyday Outlook mail workflow, ranked by how often people
hit it. v1 scope follows [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177);
the items it deferred (calendar, contacts, Exchange, PST import, a web build) are now in, with the
limits below. What each version brought is under [Done](#done); the milestones are in
[ROADMAP.md](../ROADMAP.md).

_Status: v0.32, 2026-10-31._

## Open

| # | Gap | Notes |
|---|---|---|
| 1 | **Browser version: accounts other than JMAP** | The web build syncs JMAP accounts over `fetch`, with push, and keeps its cache in IndexedDB; the server must allow the page (CORS). A browser can't open IMAP or SMTP connections, so IMAP accounts would need a relay server that holds the password and speaks JMAP to the page: a server product of its own, not built. |
| 2 | **Right-to-left: inside the editor, dialogs and menus** | The mail window mirrors, inside its panes too, and right-to-left mail and interface text read right to left and can be selected with the pointer and copied. Not yet: dialogs, Calendar, People and Notes aren't mirrored inside; egui's menu items stay on the left (the Craft apps' egui mirrors where menus open and their arrows, but lays their items out with `top_down_justified(Align::Min)`: a one-line change in the shared fork, storytold/egui's `containers/menu.rs`); in the message editor a line mixing right-to-left and left-to-right words shows in the order egui lays it out. |
| 3 | **Outlook data files: what the `.pst` libraries can't do** | Mail (signed and encrypted mail as it was sent, meeting requests with their invitation, recalls, task requests and sharing invitations, which say what they share; an attachment that is a link brings its file when the file is on one of this computer's drives (by a plain path, through no link or junction, within the room the item has left), else a short text file naming the path or address, never fetched; RSS feeds' posts, in their folders under RSS Feeds), contacts (found in any folder but Deleted Items, as calendar items, tasks and notes are: what was thrown away stays out) (with their pictures and files), contact groups, calendar items (with their files, and a repeating one's changed occurrences, which an appointment attached to a mail keeps hidden), notes, journal entries (as notes) and tasks (every one, due or not, into MailCraft's task list, with its files) come in; the files on cards, events, tasks and notes (a journal entry's on its note, opened or saved from the Notes pane) are kept up to 1 MB each and 8 MB in all (together), leaving room for the rest of the `.ics` and `.vcf` they go out as (an exported calendar, a server's address book), each read back to 16 MB (and at most 20,000 events and 50,000 cards, so a bigger calendar or address book doesn't read back whole); a note's files go with it into an exported `.pst`, and Exchange's notes sync without files (MailCraft reads an Exchange note's text and colour, and a note with files only comes from a data file, whose account has no server); export writes the mail as `.pst` (a new file, or added to one already there in the Unicode format of Outlook 2003 and later, encoded as Outlook encodes its files or not; Outlook 97–2002's ANSI files can't be added to), with the account's calendar, contacts, contact groups, notes and tasks inside it as Outlook's own items in its Calendar, Contacts, Notes and Tasks folders (the ones a file added to records; an event as an appointment with its zone, reminder, organizer and guests with their answers (a meeting someone else organizes as one the user received, with the user's own answer), an all-day one at this computer's midnights as Outlook keeps one, a repeating one with its pattern and its deleted and changed occurrences; what Outlook has no field for, a fourth address, a phone of a kind it hasn't or a task's rule its pattern can't say, kept in the item's text as a card or to-do of its own, which the import reads back; each item marked with its uid and a digest of what went in, so a file added to again gets the ones changed since, and those the file has as they are aren't written twice), and meeting requests, cancellations and answers as Outlook's meeting mail carrying the meeting's own properties; an attached message goes in as the message it is (with its own files and attached messages, 32 deep), signed and encrypted mail as Outlook keeps it (its signature still verifying and its content still decrypting once imported), delivery reports and read receipts with Outlook's report classes, and files of any size up to the format's 2 GB (one over 4 MB read from the cache as it's written, so a batch holds one big file at a time): nothing goes beside the file. An event whose rule Outlook's recurrence pattern can't say (hourly, minutely or secondly, `BYSETPOS` past "the nth weekday", several months or days of the month, `BYHOUR`, `BYMINUTE` or `BYSECOND`, or dates added with `RDATE`) goes in as its occurrences, each an appointment of its own (up to 2,000, the series' files on the first only, a changed occurrence none of them takes on its own, and counted in the export's notice): MS-OXOCAL has no place for the rule, so that is a limit of the format. MailCraft never writes into an Outlook data file it opens, by choice: a file opened as a mailbox keeps MailCraft's copy beside it (`archive.pst` → `archive.mailcraft`, or with the other data files when its folder can't be written), every change made in MailCraft is in that copy, opening the file again goes on with it, and a file changed since is read again into it (what it has new added, nothing of MailCraft's changes undone) or the copy opened as it was, as the user says (a copy made before MailCraft noted what came in from its file can only be opened as it was). Not yet: a copy beside a file doesn't record which MailCraft data folder it is an account of, so a second one (the CLI's, a `--data` run) can take it on while the first has it closed. The reader and writer are MailCraft's fork of `outlook-pst-rw` 1.2.14 (Microsoft's reader with a writer added; `ZIDAN44/outlook-pst-rs`, branch `craft/rw-1.2.14`, with the reader's fixes and the writer's own). The writer writes each message with what it is: read or unread, a draft as unsent, flagged for follow-up (with the day it's due when one was set), answered or forwarded, its categories, importance and sensitivity, its header block, what it answers, when it was sent beside when it was received, and its Bcc; the account's Sent, Trash, Drafts and Junk folders go to the file's Sent Items, Deleted Items, Drafts and Junk Email (the ones a file added to records, whatever their names). What the import leaves out, for good: Outlook's rules, hidden items of the Inbox (associated contents) in Outlook's own format (MS-OXORULE: conditions and actions as binary restrictions naming entries and folders by id), which MailCraft's rules can't hold whole, and part of a rule would file mail differently from what it says; a link's file on another computer or the web (never read: a data file is hostile input, and a network read from it could hand the user's sign-in to its server), kept as a text file naming it; a drive letter the user mapped to a network share is read as a local drive is (it is the user's own share, not a server the file names, and telling it apart takes a system call MailCraft has no safe wrapper for). Not yet: a follow-up marked complete (MailCraft forgets the flag), pinning, and the due day on the way back in (the import reads the flag, not the day); an event, contact, note or task changed since an earlier export into the same file goes in beside its earlier copy, which stays (the writer can't take an item out of a folder yet). With the reader's fixes, attached messages read and come back as the file's own message type; the import brings each in as an attached `.eml` with the files inside it, down to 32 attached messages deep (a guard against a damaged or hostile file; search reads the text of the first four). |
| 4 | **Drag attachments out on a Wayland-only desktop** | Works on Windows, macOS, and Linux and the BSDs under X11 (XDND), and on Wayland desktops through XWayland, which the window opens on by default. With `MAILCRAFT_WAYLAND=1`, or on a desktop without XWayland, the window is native Wayland, where the Craft apps' winit (0.30, storytold/winit) can take drops but not start a drag. Upstream winit 0.31 adds it (`ActiveEventLoop::start_drag`, rust-windowing/winit#4571); it comes to MailCraft when egui and the Craft forks move to 0.31 (still in beta, and egui on 0.30.13). |
| 5 | **Public folders on Exchange Online** | Public folders list and sync on on-premises Exchange through EWS. Exchange Online reaches them only through EWS, which Microsoft blocks there from October 2026 and removes in April 2027, and Microsoft Graph has no public folders; its suggested replacement, Microsoft 365 Groups, needs permissions only a tenant's administrator can grant. |
| 6 | **S/MIME: what isn't done yet** | The browser can't check revocation (authorities' servers don't answer web pages) and has no smart cards or system store (Chrome's Web Smart Card API is for Isolated Web Apps only). Its certificates are locked again after a reload unless *Keep unlocked in this browser* is on. |
| 7 | **Updates: a signature, signed releases, macOS without an administrator** | Test builds update themselves ([docs/updates.md](updates.md)); signed releases don't yet. A download is held to the SHA-256 GitHub lists for it, which proves it arrived whole but not who made it: an Ed25519 signature checked against a key built into MailCraft (as Tauri and Sparkle do) needs a signing key kept by the release workflows, the owner's to create. On macOS a bundle in `/Applications` on a standard account isn't replaced (it would need an authorization prompt). No staged rollout, and no smaller downloads (zsync, deltas). |
| 8 | **Searches that find tens of thousands: the first search of a session just opened on the big file, and the index's size** | Measured on the made-up mailbox of 50,000 messages (`sample::big`; release build, `search_big::how_long_searches_take`, which fails if a budget met slips; the machine shared with other builds, so the ranges are of several runs). Met: a word, 8–15 ms all told and 1 ms a step warm, and in a session just opened, searched alone or after the other queries in the same process, 52–77 ms all told against 90 ms and p99 2.0–2.8 ms a step against 3 ms, its worst step the same, against 4 ms (its index opened by the window's first tick, which takes 2–4 ms and isn't counted in a search's time; its 50,000 summaries' texts made about 4x as fast; 152–182 ms and p99 3.3–5.0 ms before); a one-letter word the first time 75–81 ms all told against 300 ms (each lexicon page decoded once, not once a unit: 383–401 ms before); the common phrase, now settled from its words' places in the index (no candidate's text read), 89–113 ms in frames in a session just opened against 150 ms (322–454 ms before) and 11–21 ms searched again; the searches that find tens of thousands, now that the folders' lists, the list shown and the search share their summaries rather than copy each one found (`SharedSummary`), the search's sets hold no strings, a key is hashed from its folder's hash and the list is put together at the start of a step: `is:unread`'s 15,000 4–7 ms all told against 15 ms (20–43 ms before), a one-letter word's 47,387 found 20–25 ms all told searched again against 60 ms, its steps 3.0–3.6 ms at p99 and at worst against 6 ms (8–13 and 11–22 ms before), and every frame of `is:read`, `is:unread`, the domain and the one-letter word, searched again or just opened, at most 6.5 ms against 8 ms (10–16 ms before); every step reads at most 16 store pages. On the 41 GB file (34,691 messages, `bench::queries`, the 21 queries of its ground truth from a file outside the repo, from the frame that sets the query to the last busy frame): in a session in use (its summaries' texts made by a search before) each query's first search takes 1–59 ms and again 6–18 ms, the phrase 30–58 ms the first time (0.71–1.10 s checking its 1,485 candidates' texts, the same 449 found) and a common two-word phrase 36–59 ms (3.3–3.4 s, the same 13,718), the negated word's 32,454 found 14–18 ms (109–127 ms) and `to:` a domain's 20,713 11–14 ms (67–71 ms): every aim met. Not yet: the first search in a session just opened, 134–235 ms for any query asking for words (the phrase 162–197 ms), nearly all of it its 34,691 summaries' texts made (a query the index alone answers in the same session, `has:attachment`, 3–8 ms). The places the index now keeps made its segments 6.7 times as big on that file (9.1 MB written before them, 60.8 MB with them, 52.1 MB of it places; the data file a fresh import makes 29 MB bigger, 37.75 GB to 37.78 GB, as the store packs its pages), and 3.1 times on the made-up mailbox (about 2.4 MB to 7.3 MB): far past the half again aimed at, the owner's to accept or not (keeping no places for the parts of dotted words, which no phrase reads, would make them smaller). |
| 9 | **Read marks on a big data file: now and then slow to reach the disk** | A mark-read is written on the store's thread for marks, durably, in about 5 ms. On the 41 GB file's data file, marked read and unread from the ribbon every quarter second for 40 s with nothing else running, now and then the commit's flush took 50–490 ms, so a mark took 69 ms at p99 (206 ms at most) from the window to the disk; during an index pass, 97–124 ms, though it waits for none of the pass's writes (the pass lets marks go first). Measured again with 250 toggles each: during the first index pass 5 ms at the median, 73 ms at p99 and 137 ms at most; with nothing else running 5, 189 and 305 ms. Every one of the window's durable commits then had no batch of the pass's waiting to be written with it, and the slow ones (51–304 ms) came with and without a pass: the time is the system flushing the file, which a durable commit waits for whatever it writes. The plan's 60 ms is not met there, and without writing marks behind (which the owner ruled out) it can't be. A window frame of 50–120 ms came with some of those slow commits (5 in those 40 s; none over 42 ms in the 500 toggles since), with or without a pass, though nothing logged a wait of the window's for the marks or the lists' lock: what the window waits on then isn't found yet (a read of the data file held by the flush is the likeliest). Durable index batches, readers that also give way, and four readers instead of eight changed none of it. |
| 10 | **The time left of an import, from a tenth done** | Aimed at: within 20% from a tenth done. Timed from the run's first progress now, a fresh import of the 41 GB file in the window was 4% off at worst from a tenth done, but replayed over nine recorded imports of it the worst from a tenth done was 24–66% off (36% at the median; 41–155% and 85% before): not met. The 10% from half done waits on the owner (no estimate from a rate can meet it on this file; see the import notes in v0.32). |
| 11 | **An import's peak memory, indexing as it goes (the owner's call)** | Aimed at: no more than the 789 MB an import took before it indexed as it went (in 677 s; about 145 s now). In the window an import now peaks at 1.0–1.08 GB (1.39–1.42 GB before); by the benchmark at 843 MB, against 770 MB for the same import without indexing, so the rest is the faster import and the window (about 220–300 MB), not the index. Whether that is accepted for an import four times as fast, or the target changes, is the owner's to decide. |

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
    change of the list, in the engine, which each step through the list used to build again too.
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
- **Two options (issue #11):** replies and forwards can open in a window of their own each time
  (File › Options › Mail; `view.replyWindow`), where they started in the reading pane and had to
  be popped out; and Favorites can be hidden from the top of the folder pane (View › Favorites;
  `view.favorites`), where it could only be collapsed.
- **Outlook's Rich Text mail (issue #18):** Outlook sends mail written in Rich Text Format, and
  some it forwards from Exchange, as a `winmail.dat` (`application/ms-tnef`) holding the files and
  the formatted body; MailCraft showed one `winmail.dat` nothing could open and the plain text.
  It now reads it (MS-OXTNEF, from Microsoft's open specifications): the files are listed in its
  place, saved, previewed and forwarded like any other; pictures in the body show in it; an
  attached Outlook message opens as a message; and the body shows formatted: the HTML Outlook
  keeps, or its compressed RTF (MS-OXRTFCP) as the HTML it wraps (MS-OXRTFEX), or, for RTF of its
  own, with its bold, italics, underlines, links and line breaks; text in the fonts' code pages
  and as Unicode. Search finds the files' names and the text. A `winmail.dat` that can't be read
  stays one. Tried with RTF and attachments Outlook itself saved, sent through GreenMail and read
  over IMAP.
- **Adding to an Outlook data file (issue #18):** exporting to a `.pst` that's already there
  replaced it. It's added to now, as Outlook's export does: its mail stays, the account's goes into
  folders of the same names, messages it already has (by `Message-ID`) aren't written twice, and
  the file is written as a copy that takes its place only once it's done. A file that isn't a data
  file, or is in Outlook's older format, which the writer can't add to, is left alone and said so.
- **Pictures in replies and forwards (issue #10):** a forwarded or answered message's pictures
  came out as "[picture]". Every one now goes with it, as in Outlook: those sent in the message
  (by `cid:`, by `Content-Location` or name) and those written into its HTML (`data:`) are sent
  inside the reply; one on the web keeps its address (never loaded here: the message being
  written shows its place); tracking pictures are left out. A picture sent inline beside plain
  text, which the quote can't show, goes with a forward as a file.
- **Sent mail on the server (issue #19):** an IMAP server with no Sent folder (GreenMail, a new
  account on many hosts) got no copy of sent mail, and nothing said so; nor did one whose Sent
  folder has another language's name (`Gesendet`, `Envoyés`, `Elementos enviados`…) without
  the server marking it. A Sent folder is made the first time something is sent, as Thunderbird
  and Evolution make one: `Sent`, or `INBOX.Sent` on a server that keeps every folder in the Inbox
  (cPanel, Courier); folders named Sent Items in other languages are known; and when no copy can
  be kept, it's said. Tried with GreenMail and with Dovecot laid out as cPanel's.
- **Invitations on the calendar as they arrive (issue #20):** a meeting invitation by mail (iMIP)
  reached the calendar only once it was answered, so its time showed free and could be booked
  twice. It's on the calendar as tentative the moment it arrives, as Outlook's "process requests
  on arrival" puts it, in the account's calendar and on its CalDAV server so other clients see it
  too; nothing goes to the organizer until it's answered, and answering it keeps one copy. An
  update from the organizer changes it (keeping the answer once there is one), a cancellation
  takes out one never answered and marks an answered one cancelled; one already on the calendar
  (the server put it there) isn't added twice, and deleting the invitation leaves the meeting.
  Mail from anyone can bring a new invitation, so it only ever arrives tentative, nothing is
  sent, and changes still have to come from the organizer (or be the very invitation already
  there, as a calendar service sends one for the organizer). File › Options › Inbox and
  notifications turns it off (`calendar.tentativeInvites`). Mail whose body isn't downloaded yet
  (online-only accounts) is looked at once it is. Tried with GreenMail and Radicale (CalDAV) and an
  invitation laid out as Outlook sends one.
- **Found on the way, again:** closing MailCraft while a message was still on its way lost it
  (it isn't in the Outbox until the server refuses it): closing asks first. The command line's
  `send` stopped before the copy was filed (so it kept none), and could take another message's
  word for its own (one waiting in the Outbox, sent as the account connected) and stop before
  its own went: it waits for its own, then for the copy. With the app open on the same data, the
  command line showed every folder empty: it says the app has it open. `account.exportPst`'s
  help still said a data file already there is replaced.
- **Found on the way:** mail in Shift_JIS, GBK, Big5 and EUC-KR came out as `�` (mail-parser
  decodes those only with its `full_encoding` feature, now on); a reply or forward showed the
  pictures in the message it quotes as "[picture]" (they go with it now, as in Outlook; pictures on
  the web stay a word, never loaded); a message without a `Date` was quoted, and read on the
  command line, as sent on 1 January 1970 (it's the time the server took it in, as the list shows).
- **Found on the way, earlier:** a picture at the end of a line had no room in the line (egui sizes a row by
  its letters), so a second one after it was squeezed into what was left; the scroll bar took
  the pointer from the corner of a picture as wide as the text; and a drag on a picture's handle
  without selecting the picture first lost the handle (#14's test found it). Attachments opened
  or dragged out failed where `$XDG_RUNTIME_DIR` names a folder that isn't there (WSL): the
  cache folder is used then (#14). A signature's pictures had one Content-ID in every message,
  where RFC 2392 asks for a unique one: each message gives them its own (#16).

### v0.30 — Sent where the server keeps it, mail in its column, files pasted from any file manager
- **The Sent folder the server names (issue #19, again):** at a cPanel host the copy of sent mail
  went in another folder: the first one in the list with a Sent role took Sent Items, and Dovecot
  lists the server's own `INBOX.Sent` (flagged `\Sent`, RFC 6154) after folders other programs
  made (`INBOX.Sent Items`) or a `Sent` of the user's own further down (`INBOX.Clients.Sent`).
  The flagged folder wins now, then one that can be opened, then the first listed; a name says
  Sent Items only at the top or right in the Inbox. An account already set up moves over at its
  next sync, and the log says where each copy went. Tried with Dovecot laid out as cPanel's.
- **Mail in its column (issue #10, again):** a message set in one centred column of a set width
  (a 600-pixel table, or `max-width: 600px; margin: 0 auto`) was stretched across the window: a
  layout was kept only for columns side by side. A box or table of a set width keeps it now,
  centred where the message centres it (a cell that centres it too, as browsers do), and
  `max-width` holds a `width: 100%`.
- **Files pasted from any file manager (issue #10, again):** Ctrl+V attached files only when the
  file manager put their names on the clipboard as text too: egui-winit takes Ctrl+V and reads
  only text or a picture, so files copied in PCManFM (LXDE's) or Explorer reached nothing but the
  right-click Paste. The clipboard keys are MailCraft's now, on every window (a viewport hook;
  not in a native Wayland window), and Ctrl+V in a message's text looks for files first; PCManFM's
  file list ends with a NUL byte, which hid its last file. Tried on Debian 12 under X11 with GNOME
  Files, Nemo, Caja, Thunar, Dolphin and PCManFM, dragging too, and on Windows; text paste, copy
  and cut checked against v0.29. The Flatpak couldn't attach a file dropped or pasted from
  outside Downloads (the paths a file manager hands over weren't there in its sandbox): it reads
  the home folder now (`--filesystem=home:ro`), still writing nowhere but Downloads.
- **A signature in HTML code, a business card in columns (issue #11, again):** a signature with
  the photo beside the name and numbers couldn't be had: a signature is text with a look, which
  holds no columns, and a pasted card's table came out as lines drawn in a fixed-width font. The
  signature editor takes HTML code now (Formatted or HTML code; pasted, or From a file… for one
  saved as a web page, as Outlook keeps its `.htm` signatures, with the pictures in the folder
  beside it), sent as written into each message's HTML while the signature stands as put in;
  its pictures go as parts of the message, each under a `cid:` of its own. Scripts, frames,
  forms, `on…` handlers and `javascript:` links are left out. While writing, and in the text
  part, it reads as its cells one after another, and so does any signature's table now. Read
  back, a column holding only a picture of a set width is as wide as the picture and stays
  beside the text (it took half the table, and then stacked). Sent through GreenMail and read
  back in MailCraft and Outlook, which shows the card as written.
- **Found on the way:** a picture in a folder with an accent in its name (`Andr%E9_files`, as
  Outlook writes it) wasn't found; the fuzz targets didn't build (the fuzz lockfile resolved a
  newer `sspi` than the workspace's, and the `tnef` target used functions mime didn't export).
- **Alt+numpad codes on Windows (issue #23):** É, È and Ç, which a French keyboard has no keys
  for, are typed as Alt+0201 and the like, and nothing came: Windows makes the character once Alt
  is let go, as a character message with no key of its own, which the window library drops. The
  digits typed with Alt alone held are collected on every window's input and the character typed
  when Alt comes up, in any text: a leading 0 reads the code in the system's ANSI code page, none
  in its OEM page (850 in Western Europe, 437 for US English, 866 Cyrillic), above 255 as
  Unicode; another key ends a code with nothing typed, as on Windows. After WordCraft's (#961).
- **A message's text first on Exchange (issue #24):** opening a message with a big attachment
  waited for the whole of it before a line showed (20 s for 20 MB at 2 MB/s). Its body, headers
  and the list of its files (names, types, sizes, no bytes) come first in one quick request (EWS
  `GetItem`, Graph `$expand=attachments`), shown with "Downloading the attachments…" by the
  files; the whole message follows on the account's second connection, so the next one opened
  shows at once. Open, Save, Preview and Forward wait for the files and say so. EWS does this
  for messages over 1 MB with files, Graph (no sizes) for all with files. grommunio sends the
  whole source when the request names a body type or the headers, so neither is asked for.
  Tried against grommunio behind a 2 MB/s link: the text in 0.01 s, the rest 15 s later.

### v0.31 — Signatures for replies, several messages by keyboard, the message source, the sender check, shared categories, private events, invitations on the server, online meetings

- **Several signatures (issue #11, again):** an account had one signature, put into new
  messages, replies and forwards alike, and changing From while writing kept the first
  account's. Signatures now have names ("Full", "Short"), as in Outlook's Signatures and
  Stationery; each account picks one for new messages and one for replies and forwards, either
  of them "(none)". The message's Signature menu lists them, ticks the one in the message and
  swaps it. Changing From swaps an unchanged signature for the new account's (and takes it out
  where that account has none); one the writer changed is left alone. The one signature an
  account had becomes "Signature", for both. Found on the way: a reply to one of the user's own
  messages with a picture signature took the quoted picture's Content-ID for its own, and moving
  an account to Microsoft Graph kept only the plain text of its signature. Settings moved to
  several signatures aren't read by v0.30, which then shows none.
- **Several messages by keyboard (issue #26):** Shift with Up, Down, Home, End, Page Up and Page
  Down selects from where the selection started, and going back selects less, as in Outlook;
  Shift+click goes by the rows on screen (a pinned message's range was wrong). With several
  selected the reading pane says how many, and the right-click menu acts on them all (it used
  to open the one clicked and act on it alone). The list's keys no longer run in Calendar,
  People and Notes, where End read a hidden message and Delete deleted one.
- **View message source (issue #27):** View › Message source, More actions and the list's
  right-click menu open the message as it came, headers and all (or the header only), in a
  window of its own, selectable and copied whole; `mailcraft-cli source`. A message not
  downloaded yet is fetched. Text in an 8-bit charset is read in it; control characters show
  as their pictures and marks of text direction by name, so a subject can't reorder what's
  around it; the first 2 MB is shown, the rest saved with Save as.
- **Categories other programs share (issue #28):** a new category gets the keyword Thunderbird
  gives a tag of the same name ("Rückfrage" → `r=c3=bcckfrage`, "To do" → `to=20do`); a
  category's keyword can be set (`$label6`, as SOGo numbers its tags) and is checked as an IMAP
  atom; categories made before keep theirs. Keywords other programs set show as categories
  (system ones such as `$Forwarded`, `NonJunk` or `JunkRecorded` aside), and Categorize › All
  categories… lists them with Add to the list. The list goes out and comes back as a file.
  Searching the server for a category asked for its name, not its keyword; it asks for the
  keyword now. A folder that can't keep keywords (PERMANENTFLAGS without `\*`, as a shared
  folder the user may only read) refused them silently, Dovecot answering OK: MailCraft
  refuses them itself and says so. Tried on Dovecot with a folder shared by ACL.
- **The sender check (issue #29):** "Jane Director" <jane.director1984@gmail.com> passed: a real
  address at a real mail service. The list now shows the address after the name of a sender the
  user doesn't know (an option: always, strangers, never), and a name that matches a contact,
  someone written to, a person from the directory or the user (case, accents, look-alike and
  styled letters, digits for letters, titles and word order folded) from an address that isn't
  theirs is warned of in the reading pane and marked in the list. Lists, single words and names
  such as "IT Support" don't count. An External tag for senders outside the organisation's
  domains is an option, off at first (outside a company nearly everything would carry it); a
  mail service is never taken for the organisation.
- **Invitations on the server (issue #30):** an invitation that came before the account's
  calendars on its server were found (the check failing, a CalDAV address given by hand later)
  went into the calendar on this computer and stayed there. It waits now, in the cache, until
  they are, and goes there; after three minutes it goes here and moves once they're found.
  Other events kept here are offered to move. Each account's default calendar can be set; else
  its own calendar comes before calendars shared with it (SOGo's `personal` first). Found on the
  way: a new account's first message in an empty Inbox wasn't taken for new, and two calendar
  syncs back to back sent new events twice; a new event in the form went to the account's first
  calendar, not its default; an answer sent the organizer the pieces of its alarm it had cut.
- **Answered invitations out of the Inbox (issue #31):** File › Options › Mail › After answering
  a meeting invitation: leave it, move it to Deleted Items (as Outlook does; the setting for a
  new install) or to a folder. It applies to answers from the message and from the calendar,
  takes the meeting's older invitations along (never a newer update not taken in yet), and
  cancellations once applied; guests' answers optionally. Settings from before keep the mail
  where it is. An invitation answered while it's read goes on to the next message, as after a
  delete. A rule condition "is an invitation" isn't there: rules run on the header, before the
  body that says so arrives.
- **Private events and Show as (issue #32):** the event form has Private (a lock in the views),
  Show time only and Show as (free, working elsewhere, tentative, busy, out of office), written
  as CLASS, TRANSP and Outlook's X-MICROSOFT-CDO-BUSYSTATUS, EWS's Sensitivity and
  LegacyFreeBusyStatus and Graph's sensitivity and showAs; one set elsewhere is never dropped
  (Exchange's Personal included). An accepted invitation shows as busy, not tentative. An event
  without a subject reads "Private appointment". Whether a server then hides it from a
  colleague is the server's: SOGo wasn't there to try.
- **Calendar search (issue #33):** in the Calendar the search box searches events across every
  calendar shown, offline: subject, place, notes, organizer, guests and categories, with the
  mail search's syntax (`location:`, `organizer:`, `attendee:`, `category:`, `is:private`,
  `after:`, `before:`). The results list upcoming then past events (or one of them), a series
  once at its next occurrence; a click goes to the day and opens it; a meeting is found by its
  link too. `calendar.search` too.
- **Online meetings (issue #25):** the user's own rooms (a BigBlueButton, Jitsi or OpenTalk room's
  link, access code and dial-in lines) are kept in File › Options › Online meetings; Add online
  meeting in the event form puts the link in the place, an RFC 7986 CONFERENCE line in the event
  and a block in the notes, and Remove takes them out again; an option adds one to every meeting
  with guests. Join meeting knows rooms by their shape, the rooms' servers and hosts the user
  adds, and a received event's CONFERENCE. Sign-in to a room server and rooms made from MailCraft
  wait for those servers' APIs (PILOS's is a draft, Greenlight has none); a Teams meeting through
  Graph waits for a Microsoft 365 tenant to try it on.
- **On Mailcow (issues #28, #30, #31, #32, #25, #33):** tried against a Mailcow in Docker, Dovecot,
  Postfix and SOGo 5.12, as the issues' reporter runs it. SOGo's labels and MailCraft's
  categories pass both ways, in a shared Inbox too. An event SOGo gives a colleague only the time
  of (titled "(Private event)", `X-SOGO-SECURE:YES`) is a private appointment, not to be edited.
  A meeting in the user's calendar and a colleague's subscribed one keeps the user's copy. SOGo
  refusing one event (403, its roles per classification) no longer stops the calendar sync as a
  wrong password; a 412 or 409 over a meeting SOGo already filed keeps both sides' changes (SOGo
  ignores `If-None-Match: *`, so a new meeting is looked up first), and SOGo's re-stamped
  invitation can be answered.
- **One invitation when the server schedules too:** SOGo and Stalwart say `calendar-auto-schedule`
  and email guests themselves, so MailCraft's own invitation made two. MailCraft keeps sending
  (it is the only way outside guests hear on a stock SOGo, and Stalwart doesn't answer for the
  user): it marks what it stores `SCHEDULE-AGENT=CLIENT` (RFC 6638; SOGo reads the organizer's,
  Stalwart the guests') and mails once the server holds the change, with the server's SEQUENCE.
  An answer to a meeting the server delivered is left to the server; Graph sends its own. Counted
  per mailbox on Mailcow with SOGo's mail on and off, Stalwart, Radicale and an outside domain.
  Open: changing an answer already given to an outside organizer on a SOGo that mails sends it
  twice (SOGo ignores every opt-out there; its "Disable e-mail notifications on CalDAV" stops
  it); a meeting moved into another account's calendar is judged by that account's address;
  Graph meetings untried on a real tenant.
- **The weekly fuzz run, running:** it had never fuzzed anything (`cargo-fuzz` as installed is a
  musl build and built the targets for musl, where the address sanitizer can't link). Running,
  it found a photo whose place a second pass still took out (a second GPS pointer, or an offset
  the wiping itself changed), now gone in one pass; and the signature target, which had no
  dictionary. `cargo xtask fuzz` checks every target has its seeds and dictionary.

### v0.32 — Pictures open without asking

- **Selecting a message's text:** a message is drawn in pieces (a formatted line's words and
  links, a table's cells, a quote, a right-to-left line, a long text's chunks), and each was an
  egui label with a selection of its own: Ctrl+A took only the paragraph under the selection,
  Shift with the arrows (Home and End too) stopped at the edge of a piece, and a selection
  dragged past the top or bottom of the reading pane neither scrolled nor grew (egui lets go of a
  selection whose ends it doesn't draw). The pieces are now one text with one selection
  (`reading/text_selection.rs`), in the reading pane and in a message's own window, for every
  kind of body: Ctrl+A selects the whole message and a copy has it in reading order, a line break
  between lines; Shift with the arrows, Home, End and Ctrl goes on across lines and blocks and
  keeps its end in view; Shift+click, double- and triple-click work across them; a drag held at
  or past the view's edge scrolls, faster the farther out (PdfCraft's edge scroll; not on the
  text before the edge, where the message ran away under a still pointer), the wheel scrolls
  during a drag, and the selection keeps growing either way. The shading of a line of several
  pieces (a bold word) runs on without a seam: each piece's met the next's a fraction of a pixel
  over, a darker line down the selection. The scroll bar beside the text leaves the selection as
  it is. Clicked into, the text has Ctrl+A, Ctrl+C and Shift with the arrows; a click in the
  message list gives them back to it (Ctrl+A there selects every message), and so does another
  message shown in its place (going back to it by the keyboard brought the old selection back,
  keys and all). A message's own window gives them to its text from the start, unclicked. On a
  touch screen (the browser on a phone or tablet, a touch laptop) a finger dragged over the text
  scrolls it, as over egui's labels there, instead of selecting.
- **Sent with no account to send from:** with only an Outlook data file open (no account set
  up), a new message said "Message sent" and went into the file's Sent Items: the engine took
  any account without a server, or none, for the sample mailbox, which files what it sends in
  Sent. Mail now goes only from an account that can send it (`SendBlock`): none set up, a data
  file, a mailbox kept only on this computer, or an account without an outgoing server can't.
  `compose.send` is off and says why, and sending itself refuses (`Session::deliver`, so the
  Send button, Ctrl+Enter, Alt+S, Send anyway, Delay Delivery, a reply or forward, the CLI's
  `send` and `--cmd` all stop there): nothing goes in Sent or the Outbox, nothing says it went,
  and the message stays open as it was. The message says so over its form with Add an account,
  and Send, drawn off, says it again when clicked; Ctrl+Enter no longer closes that alert the
  moment it opens. As in Outlook, a new message, reply or forward in a data file's folders is
  written from the first account that can send, and From offers only those; a forward as an
  attachment takes that account's signature too. An account added while such a message is open
  (the bar's own Add an account) takes it and any set aside: they go From it, its signature in
  their place, and can be sent. Send now in the
  Outbox, its timer and an answer to a meeting invitation from a data file don't send either
  (the answer is kept in the calendar and the window says it wasn't sent). Siblings checked:
  no Craft app sends mail, so there was nothing to port.
- **Exported mail keeps what it is:** a `.pst` export wrote every message read, as an ordinary
  `IPM.Note` with nothing but its text, addresses and files, into folders named as the account
  names them. It now keeps what the store knows of each: unread, a draft (in Drafts or flagged
  `\Draft`) as unsent, flagged for follow-up as Outlook flags it (`PidTagFlagStatus`, the red
  flag, "Follow up", and the day it's due as the task's start and due dates when one was set),
  answered or forwarded (the last verb and its icon), its categories (`Keywords`), importance and
  sensitivity, its whole header block, In-Reply-To and References, its `Date:` as the time it was
  sent beside the time it was received, SMTP address types and the sender it was sent as, and a
  sent copy's Bcc. The account's Sent, Trash, Drafts and Junk folders, and the folders inside
  them, go to the file's own Sent Items, Deleted Items, Drafts and Junk Email, recorded as such so
  Outlook shows them as its own; a file added to keeps the special folders it records, whatever
  their names. MailCraft's fork of the writer now takes a message's class and any tagged or
  named property (the names added to the file's name-to-ID map, hashed as Outlook hashes its
  own), counts unread mail in its folders, makes folders with a container class and records
  special folders (`PATCHES.md` 13–15). The round trip through the import keeps all of it but the
  due day, and `pst_append` fuzzes adding such a message to a damaged file's name map.
- **Attached messages and big files in an exported `.pst`:** a message forwarded as an
  attachment went into a `.pst` as an `.eml` file, which Outlook shows as a file and not as the
  message it is, and files over 100 MB were left out, since every file of a batch was held whole
  until the batch was written. A `message/rfc822` part now goes in as an attached message
  (`afEmbeddedMessage`, its subject its name, cut to 255 characters so its properties fit their
  block), with its own recipients, files and attached messages in turn, 32 deep; one deeper, or
  one that doesn't parse, stays an `.eml` file. A file over 4 MB is left in the cache, read to
  learn its size and let go of, and read again when the writer comes to it, which writes it as
  it reads it (the fork's `UnicodePstAttachmentSource`, `PATCHES.md` 16 and 17, with attached
  messages); a batch counts only what it holds. Exporting two messages with a 120 MB file each in
  one batch grew the export's peak working set by 290 MB (one source and its file decoded beside
  it), where holding both files with their batch took 411 MB; both files are in the data file
  byte for byte, and the import, which keeps files up to 100 MB, brings both messages in and
  counts their files as too big. A message attached three deep with a file at the bottom is read
  level by level from the file with the writer's reader (method 5, no `.eml`) and comes back
  through the import down to the file, and the `.pst` fuzz target's file now carries an attached
  message.
- **Signed and encrypted mail and reports in an exported `.pst`:** signed and encrypted mail went
  in as notes with its signature or its encrypted content as files, so neither Outlook nor the
  import could check the signature or decrypt the message, and delivery reports and read receipts
  were notes. They now go in as Outlook keeps them, the reverse of what the import reads
  (`pst/protected.rs`, `pst/report.rs`): a clear-signed message is `IPM.Note.SMIME.MultipartSigned`
  with one attachment, `smime.p7m` (`multipart/signed`), holding the whole signed entity, its
  `Content-*` header fields and all; an opaque-signed or encrypted one is `IPM.Note.SMIME` with
  `smime.p7m` (`application/pkcs7-mime`) holding the CMS (MS-OXOSMIME); the body is the text
  MailCraft shows (for an opaque signature, the content it carries; none for encrypted mail). A
  delivery report is `REPORT.IPM.Note.NDR`, `.DelayedDR` or `.DR` by its recipients' `Action`, a
  read receipt `.IPNRN` or `.IPNNRN` by its `Disposition` (RFC 3464, 8098), its returned message
  attached as a message. The round trips through the import: a message signed with a test
  certificate, whose signature verifies over the same bytes after it; one encrypted to it, which
  decrypts; a made-up opaque signature, kept byte for byte; a meeting request with its calendar;
  and a bounce and a read receipt, which come back as `multipart/report`, the bounce's message
  attached as a message. `mime_parse` fuzzes how an export reads a message (S/MIME, a report's
  status, attached messages).
- **A task list:** an Outlook task came in from a `.pst` only when it had a due date, and then
  as an all-day event on that day (a done one's title marked "✓"), its status, how far along it
  was, its start, priority and how it repeats lost; one with no due date was left out. MailCraft
  now has Tasks beside Calendar, People and Notes (Ctrl+4, `view.tasks`): a list with a box to
  mark each one done, its due day (red when it's late), status and priority, sorted by due day,
  priority, title or last change (a column's heading sorts by it), the done ones last and hidden
  when asked, and searched from the box at the top (words, whatever their accents). A task opens
  in a form with its start and due days, status and percent complete, priority, reminder,
  repeat, categories and notes. Marking a repeating task done keeps a done copy and moves the
  task to its next due day, as Outlook does (one set to end after so many times counts down,
  and is done after its last). A task with a due day shows in the calendar that
  day, drawn from the task (a box before its title) and opened as one, not copied into the
  events, and its reminder rings in the Reminders window. Tasks are kept with the account's
  calendar and contacts on this computer: no server keeps them (no CalDAV or Microsoft To Do
  sync, by the owner's choice). The import brings every `IPM.Task` in as a task, due or not
  (MS-OXOTASK: its status, percent complete, start, due and done days, its recurrence pattern
  read into a rule as an appointment's is, its importance as priority, its reminder, categories
  and files), and its report counts them. An exported `.pst` carries them inside (below), and a
  calendar file as to-dos (`VTODO`, RFC 5545 §3.6.2): one opened brings its to-dos in as tasks (one with no title, as Outlook allows, among them; opened again, a file's to-dos take
  the place of those with their uids instead of coming in twice); Waiting and Deferred, which
  iCalendar has no word for, go as `X-MAILCRAFT-TASK-STATUS`.
  Commands: `task.list`, `task.create`, `task.update`, `task.complete`, `task.delete`,
  `task.arrange`, `task.hideDone`. The `ical` fuzz target reads, writes and reads back to-dos
  (every one of them comes back),
  and `pst_recurrence` makes a task of its pattern; a file the writer makes with a Tasks folder
  is imported whole in a test (the writer had no 64-bit floats then, so percent complete is
  tried on the item's values alone).
- **Contacts, groups, notes and tasks inside an exported `.pst`:** an export wrote the account's
  address book beside the data file as `… Contacts.vcf` and its tasks as to-dos in
  `… Calendar.ics`, and left its notes out. They now go inside it as Outlook keeps them, in the
  file's Contacts, Notes and Tasks folders, made with their container classes and recorded as the
  file's own (MS-OXOSFLD 2.2), or the ones a file added to records, whatever their names: a card
  as an `IPM.Contact` (MS-OXOCNTC: its names and File As, three email addresses, a field for each
  kind of phone Outlook has, a work, a home and an other address, company, title, birthday, web
  page and note, its picture as an attachment marked as one, its files), a group as an
  `IPM.DistList` (its members as one-off addresses, and wrapped, with their checksum), a note as
  an `IPM.StickyNote` (MS-OXONOTE: its colour, Outlook's size) and a task as an `IPM.Task`
  (MS-OXOTASK: status, percent complete, start, due and done days, priority, reminder, categories,
  files, and how it repeats as a recurrence pattern). What Outlook has no field for (a fourth
  address, a phone or postal address of another kind, an address's kind, a birthday without its
  year, a card's or to-do's other properties, a rule no pattern says the same as) stays in the
  item's text after its own, as a card or to-do of its own (`BEGIN:VCARD` … `END:VCARD`), which
  the import reads back, so nothing is lost; a group's note that lists its members, as the import
  writes one, names them. Each item is marked with its uid in the account and a digest of all
  that went in (a named property, `X-MailCraft-Item`): added to again, one the file has as it is
  now isn't written twice (counted apart, as mail already there is), one changed since goes in
  again beside its earlier copy (the writer can't take an item out of a folder yet), and the
  file's own items, unmarked, never stand for the account's, however alike their names and text.
  The block of what Outlook has no field for starts with a line saying it's MailCraft's
  (`X-MAILCRAFT-REST:1`), so a note that ends in a card or to-do of its own stays text, and a
  task's is read as a to-do (`ical::parse`), so a line in it can't give the task another uid or
  end the to-do early. A rule's `WKST` is the pattern's FirstDOW, and the import says it back for
  a rule every so many weeks. A mail folder of a file added to named as one of Outlook's (Notes)
  keeps its mail, the notes going to `Notes (2)`. An account
  with no mail but these makes a file of them. The calendar went beside the file until events
  were mapped too (the owner's decision: what is inside isn't written beside it; below); no `.vcf`
  is. MailCraft's fork of the writer now writes 64-bit floats, records the Calendar, Contacts,
  Journal, Notes and Tasks as special folders and makes a file with no message (`PATCHES.md`
  20–22). Tested by a round trip through the import, field by field (cards with a picture, a
  file, nine phones and five postal addresses, two groups, two notes, four tasks: a pattern, a
  rule no pattern says, a to-do's own property, a file), and by adding to a file with its own
  Contacts folder under another name and a mail folder named Notes (its own, and the
  account's), a contact of its own of the same name, and items changed between two exports; the
  `pst_pim_text` fuzz target reads hostile text back into a card and a task (a task's kept lines
  never saying its uid, title or days, each component they begin ended by name) and writes rules
  as patterns.
- **The calendar inside an exported `.pst`:** an export wrote the account's calendar beside the
  data file as `… Calendar.ics`. It now goes inside, in the file's Calendar folder (made
  `IPF.Appointment` and recorded as the file's own, or the one a file added to records), each
  event an `IPM.Appointment` as MS-OXOCAL keeps one: its start and end (the appointment's own and
  the item's), length, all day or not, how it shows its time, place, version, its zone as
  Outlook keeps a zone (`PidLidTimeZoneStruct` and the start's, end's and a series' time zone
  definitions, built from its `TZID` with the zone's offsets and when its clocks change, keyed by
  Windows' name for it; its `TZID` as the zone's description, which the import reads back), its
  first reminder (Outlook has one an item), privacy, categories, files, its uid as the meeting's
  global object id (one that is Outlook's in hexadecimal goes back as itself, any other inside
  it, `vCal-Uid`, as Outlook keeps a meeting another calendar made), and its organizer and guests
  as the item's recipients, the organizer flagged as such and each guest with its answer. A
  repeating event is one appointment with its AppointmentRecurrencePattern: the rule's pattern
  (daily, weekly, monthly on a day, its last day or the nth weekday, yearly), its end, the day its
  weeks start, the days of its deleted occurrences (`EXDATE`s and occurrences called off) and of
  its changed ones, and each changed one's ExceptionInfo and ExtendedException (writer version
  0x3009), and as an item attached to it, hidden as an exception, with its own text, times,
  place, reminder and guests and the occurrence it replaces. A pattern goes in only when the
  import reads it back as the same occurrences and the same ones deleted; a rule it can't say
  goes in as its occurrences, each an appointment of its own named `<uid>/<start>`, up to 2,000
  (the series' files on the first only, not copied into each, and a changed occurrence none of
  them takes on its own), counted apart in the export's result and its notice
  (`Exported::occurrences`): a limit of the format. A meeting someone else organizes (by the
  account's addresses, or the calendar keeping it as a guest's copy) goes in as one the user
  received (asfReceived) with the user's own answer, one the user organizes as such; an all-day
  event starts and ends at this computer's midnights, in its zone, as Outlook keeps one (they were
  UTC's, the wrong evening in Outlook west of UTC), and the import puts an all-day series' deleted
  days on their days. MailCraft's calendar now repeats at a rule's `BYHOUR`, `BYMINUTE` and
  `BYSECOND` (they were ignored: an hourly rule at three hours made 24 a day) and reads, keeps
  and writes `RDATE`'s added dates. A message carrying a meeting's request, cancellation or
  answer goes in as Outlook's meeting mail (`IPM.Schedule.Meeting.Request`, `.Canceled`,
  `.Resp.Pos`, `.Neg`, `.Tent`) with the meeting's properties as its appointment has them (its
  times, place and when it last changed for its guests and organizer too), received or the
  user's own as the meeting is, its calendar still among its files. The import reads
  more of an appointment for it, Outlook's own too: the organizer a recipient flags is the
  organizer and not a guest, each guest's answer is its `PARTSTAT`, the zone it names is its
  `TZID` (so a series' occurrences and its deleted ones fall on their wall-clock times across a
  change of summer time, where they were an hour off before), a yearly pattern's month is its
  first local day's (its UTC start was taken, a month late for an evening one west of UTC), and a
  meeting another calendar made keeps its own uid. MailCraft's calendar now repeats hourly,
  minutely and secondly rules too (every so long from the start, on the days the rule allows),
  where it showed none of their occurrences. The writer in MailCraft's fork now writes a
  recipient's flags and track status (`PATCHES.md` 23). Tested by a round trip through the import
  (a single event with everything Outlook keeps, an all-day event, a weekly series in New York
  across the change of summer time with a moved, a retitled, a deleted and a called-off
  occurrence, events in Tokyo and Berlin, a meeting with its organizer and three guests' answers,
  and an hourly series), its occurrences compared over three years and exported again with
  nothing written twice; a meeting request and an answer, read back with their class, the
  meeting's id, times, place, pattern and reminder; a property test of 400 rules of every kind
  Outlook's pattern says, in four zones and none, all day or not, with occurrences deleted, read
  back with the same occurrences over three years; and the `pst_recurrence` fuzz target writes a
  pattern from its input's text and reads it back.
- **A double-clicked attachment only opens:** a double-click on an attachment opens it in the
  app for its kind, as Outlook's does, and the reading pane goes on showing the message; its
  first click had shown the file there already, left behind the app it opened in (in a
  message's own window too, and for every kind of file). A click alone still previews it, once
  the double-click time is out, as PDFCraft's full screen waits before it turns a page.
- **Opened pictures:** every attachment opened or saved on Windows carried the mark that it came
  from the internet, and Windows' Photos now asks "Is this file from a trusted source?" before
  every picture with it. A picture whose bytes are the picture its name says, and plain text,
  aren't marked any more; Office files, PDFs, web pages, SVG, archives and programs keep it (so
  SmartScreen, Protected View and Office's block on internet macros still apply). Tried on
  Windows 11: Photos asked for PNG and WebP with the mark and not without; Media Player (MP3,
  MP4) and Notepad don't ask either way, so those keep it.
- **Rich Text mail from a data file:** an item Outlook kept only in Rich Text Format came in from
  a `.pst` with an empty body (996 in a 41 GB file), and plain-text items lost the HTML their
  RTF wrapped. The compressed RTF (`PidTagRtfCompressed`) is now read when an item has no HTML,
  the same way as in a `winmail.dat`: as the HTML it wraps, or, when there's no plain text, its
  own formatting or its text; the pictures its HTML shows are in the body instead of listed as
  files. A plain body stays over RTF of its own, whose tables and pictures the text keeps better,
  and over RTF the item says is older than its text (`PidTagRtfInSync`). Rich Text that can't be
  read is counted in the import's log, and its simple HTML stops at the same 16 MB as the RTF.
- **Outlook data files in their own code pages:** an imported `.pst` message's HTML body was
  read as UTF-8 or Windows-1252 only, so Cyrillic, Greek, Chinese, Korean and Japanese mail
  came in as mojibake (and Japanese mail with its ISO-2022-JP escapes showing). It is now read
  in the code page the item names (`PidTagInternetCodepage`), with GB18030, EUC-JP/KR,
  ISO-2022-JP, UTF-7, UTF-16BE, KOI8-U, Mac Cyrillic, DOS Latin 1 (850) and more ISO-8859 pages
  added to the table winmail.dat already used; every page in it is tested with its own bytes.
  A body that is valid UTF-8 stays UTF-8 unless a `<meta>` tag near its top declares another
  8-bit charset (Outlook labels some UTF-8 mail with the sender's code page; `charset=` in a
  comment or a link, US-ASCII and Word's "unicode" don't count), though UTF-16 is always read as
  UTF-16; an HTML body's byte-order mark decides first. An older (ANSI) data file's subjects,
  names and file names are read in the message's code page (`PidTagMessageCodepage`) rather than
  as Windows-1252, except UTF-16 and UTF-7, which 8-bit text can't be in (RTF's `\ansicpg` too).
  Windows-1252's five undefined bytes still come through as the C1 controls they number rather
  than U+FFFD, and UTF-7 is read by MailCraft itself, so text ending inside a base64 run keeps
  its last characters, and a `+` that starts no character (`C++`, `1 + 1`) is kept. ISO-2022-KR
  has no decoder: its ASCII is kept, and the import report counts the messages (with attached
  ones) that lost text that way. A new fuzz target, `pst_props`, reads property text in a page
  of each kind.
- **Down held in a big folder:** every press queued while a frame was drawn read, parsed and
  marked read a message of its own, so holding Down in a folder of 10,941 messages drew a frame
  every 727 ms (median) and marked read every message it passed. The presses of Down or Up (J,
  K, with Shift too) queued in one frame are now one step of so many messages (`list.next
  {"by": n}`): only the message landed on is read and marked read, as in Outlook's list, and
  those passed over keep their flags. With nothing focused, such a step lands where as many
  single Downs would; Page Down still goes to the top row. A step also built every row of the
  list again to find where it was (1.5 to 11 ms on 20,000 messages, by the arrangement); the
  rows are now kept with the list and a step looks its row up. Marking the message landed on
  read built the whole list again as well (each message copied out of the folder, filtered,
  arranged, its conversations and rows worked out): a read mark, a flag or a category now
  changes the list as it stands, counting its row's unread again, taking out what the filter
  now leaves out (shown Unread only, the message read goes once another is opened), arranging
  the list again only when it's arranged by flag. Opening and marking read 100 messages in a
  made-up folder of 10,941 took 2.1 s, now 0.5 s. With a search open the list is still built
  again.
- **Down in a big folder of a server account:** each read mark was then written to the cache on
  the window's thread (the folder's whole list read and written, 28 ms on 10,941 messages), and
  when the cache said the folder had changed, the window read all of it again and built the list
  again (24 ms more), for a change it had made itself. The change is now written on a thread of
  the pool's own, in the order changes were made, and the session knows its own read marks,
  flags and categories coming back: the folder in memory and the list are kept as they are, and
  only a change from the server among them (or one the cache couldn't take) reads the folder
  again. A Down step on a made-up folder of 10,941 messages costs the window 0.4 ms (median;
  52 ms before). Closing MailCraft waits for the changes made so far to be on disk, and an
  export to a data file starts with them. A message whose body is fetched on the way said the
  folder had changed once it was here, which read the folder again and built the list (16–24 ms
  on that folder); it now comes back as its summary filled from the source (its preview, files
  and conversation), which the folder in memory and its row take, in 5 µs by median, the list
  built again only when its conversation, or a filter or arrangement by files, size or
  importance, depends on what changed.
- **A message with many pictures:** each thumbnail read and parsed the whole message again, in
  the frame drawing it (a 33 MB message with 36 pictures took 2.6 s to show), and past 64 the
  thumbnails were thrown away and made again every frame, so the window froze for as long as the
  message was open. The message's files are now taken out once, by one parse, kept while it's
  open, and its pictures decoded on a thread of their own, each shown as soon as it's ready (in
  the browser, one a frame); the picture opened big, a `cid:` picture in its HTML and a PDF's
  pages come from that same read, and the picture opened big and the pages on screen are
  decoded before the thumbnails waiting. What's kept is bounded (96 MB of pictures, the least
  recently shown let go first), a PDF draws only the pages on screen, and moving to another
  message stops the work on the last one's. A picture that breaks the decoder fails alone, and
  one that keeps it busy 30 s after its message was left no longer holds up the pictures after
  it. Thumbnails are made at 160 pixels (they're shown at 30 points), not 480.
- **Long messages:** a message's text was laid out whole in the frame it opened, so a message of
  127,655 characters took a frame of 179 ms to show. Text over 20,000 bytes now goes in chunks of
  at most 4 KB (cut at a line's end, or inside a longer line at a character's edge), and formatted
  mail of more than 2,000 lines in blocks of 64 lines (a newsletter's tables one by one): only
  those near the view are laid out, the others keep their place by their height, guessed and then
  measured a couple a frame so the scroll bar settles. A selection keeps both its ends laid out
  wherever it's scrolled, so Shift+click far below extends it and a copy takes every line between;
  Ctrl+A in the text, then a copy, takes the whole message. Long plain text no longer has a text
  box's cursor: it's selected as formatted text is.
- **Attached messages and empty files from `.pst` files:** a message attached to another one in
  an Outlook data file was left out (the reader stopped at its property type, looked for it in
  the wrong place, then would have waited on itself), and so was a zero-byte file. Both come in
  now, the attached message as an attached `.eml` with the files inside it and the messages
  attached to it in turn (three levels deep at first; 32 now, in **Attached messages as deep as
  they go**, below). One item's files,
  its attached messages' text and files included, come to at most 128 MB, at most 1,000
  attachments of its attached messages are read, and an attached message that holds a message it
  is attached to (a damaged file's loop) is counted and left out rather than read again. The
  reader is built from MailCraft's fork of it with these fixes, and it no longer trusts
  a damaged file's trees: a value, a table's rows or a heap is read up to the size it declares
  (blocks named over and over add nothing past it), a value the file says is over 256 MB (a
  message's, an attachment's, the store's, a folder's or the name map's) is an error before any
  of it is read, a table's index levels can't be walked without end, and a data, subnode or
  B-tree page that points back at itself is an error rather than a crash.
- **Files a `.pst` couldn't hand over:** an attachment the reader failed on was left out with
  only a line in the log, and one that is just a link to a file elsewhere (attached by reference)
  went without a word. Links, files over 100 MB, and files and attached messages that can't be
  read are now counted by reason in the import's report, apart from the messages it couldn't
  read. When the reader fails on one, its row in the message's attachment table is read: a
  zero-byte file kept with no value would come in empty, named from the row (at least its short
  name) and typed from the name. That row is a backstop: the reader MailCraft carries already
  reads such a file as one with no bytes, and the 7 zero-byte files of a 41 GB test file all
  come in that way, with no attachment the reader fails on.
- **The paperclip on mail from a `.pst`:** an imported message got a paperclip (and matched
  `has:attachment` and Has files) for any file the item held, the pictures its HTML shows and
  S/MIME's signature included: 27,917 of the 34,691 messages of a 41 GB file. It now follows
  the rule the reading pane and search read a source with, through the same test: a picture the
  HTML shows by its `cid:` (matched whatever its case, as the reading pane matches it, so such a
  picture also goes in the body rather than the list of files) and S/MIME's own part aren't
  files; a `winmail.dat` is one when it holds a file. A file whose Content-ID is only the name of
  a file the HTML links to stays a named file. That file now gives 12,696, the count its rebuilt
  sources give read back one by one, and the paperclip, the Has files filter and `has:attachment`
  agree on the same 12,696 (`bench::files_agree`, 21,995 without). The 12,983 an earlier import
  gave counted the pictures its Rich Text bodies listed as files, which are now shown in the text.
- **Pictures in the body keep their names:** a picture the HTML shows went out with no name
  (76,712 of them in a 41 GB `.pst`'s import), so another client, or a saved copy of the message,
  listed it as `attachment-1`. Sent mail, signatures and imported mail now write its name
  (`filename=` and `name=`, as Outlook does; a pasted picture gets its type's extension). It
  stays inline, and MailCraft gives it no paperclip while the HTML shows it.
- **Files keep their Content-ID, and hidden ones come in:** a file from a `.pst` whose Content-ID
  the HTML doesn't show (3,499 in a 41 GB file) went out without it; it now stays a listed file
  with its Content-ID. A message's attachments Outlook marks hidden came in as nothing; they now
  come in as files, except hidden attached messages that are a series' changed occurrences:
  those marked as one (afException) in any message, a meeting request's (its invitation carries
  them) and those of an appointment attached to a mail (its bookkeeping) are left out. A contact's picture
  goes on its card, and an appointment's changed occurrences come in as events (below).
- **Bounces and receipts from a `.pst` stay reports:** a delivery report, read receipt or
  feedback report that came in by mail (`multipart/report`, 11 in a 41 GB file) was rebuilt as
  `multipart/mixed`. When its original header says `multipart/report` and its files are the
  status and at most one returned message or header, it is a report of its `report-type` again,
  status first; anything else stays mixed with every part. A report's status now goes as 7bit
  text when it is (RFC 3464), not base64.
- **Search reads accents and dates:** a word is found whatever its accents, typed or written
  (`resume` finds "Résumé", `résumé` finds "resume", `strasse` "Straße"; Cyrillic keeps й, and
  other scripts are looked for as written). `received:` (also `sent:`, `date:`) takes a day, a
  month, a year, `today`, `yesterday`, `"last week"` and the like, a range
  (`2024-01-01..2024-03-31`) or a comparison (`>=2024-03-01`), in the user's time zone;
  `after:` and `before:` take the same dates (from its start on, and before it starts).
  `received:2024` was two words that matched nearly everything; a date that isn't one
  (`2024-13`) is now left out rather than looked for (the search box doesn't say so yet: the
  query keeps the problem for it to show), while words that only follow the name
  (`before: launch`, `date: friday`, `after:party`) are still looked for as words. `attachment:`
  asks for a word in the files' names (for now the message's text is looked through with them);
  a server can only look through the whole text for them, so `-attachment:` isn't asked of it
  and what it finds is checked here. What a server finds is checked against the dates, flags
  and replies it wasn't asked about; the words it was asked stay its own to match (IMAP finds
  `from:oana` in "Joana", which the list doesn't). A lone accent typed is no longer a word
  that every message starts with.
- **Mail from a `.pst` that came without headers** (sent items, drafts, mail Outlook made
  itself): a forwarded message shows as forwarded (by Outlook's last action on it, or its
  forwarded icon), high and low importance come through, and the ids it answers are read
  however the file separated them (`<a><b>`, commas, spaces), so its replies thread. Its
  conversation index and topic and its sensitivity become `Thread-Index`, `Thread-Topic` and
  `Sensitivity` headers (on mail whose own header lacks them too), each cleaned of line breaks
  and control characters, capped and encoded; the subject, names, addresses (one with a line
  break or other control character in it, or longer than 254 bytes, is none) and ids it gets
  from the file are cleaned and capped the same way, its Bcc names encoded, and a blank line in
  its own header no longer ends it early, so a damaged file can't add header lines. A recipient
  Outlook kept with only a name, its address inside the item's own record of it (a one-off
  entry id), now comes in with that address instead of being left out; one with no address at
  all is still left out, and counted among what the import couldn't bring in. And mail keeps
  its categories from files where Outlook names that property by its name (`Keywords`, as in
  files from other people's Outlook) rather than by a number.
- **Data files after a crash:** a data file is sealed (where its free space is, recorded) once its
  writes have been quiet for two seconds, when an import ends or stops, and as MailCraft quits
  (and anything still being written as it quits is sealed as it goes in); a crash after that opens
  it again at once (25 ms on a copy of a 37 GB file whose full check took 305 s). Not every commit
  is sealed: on that file it tripled a small one's time (4.6 to 15.6 ms), so a crash within two
  seconds of a write still costs the full check.
- **The window comes at once:** the data files open on a thread of their own, so MailCraft's
  window shows straight away, even while a data file not closed properly is checked (that took
  minutes with no window at all). A check, or opening that takes longer than a moment, is shown
  in the window (which file, how far, and of how many); a start with nothing to check shows
  nothing extra. A data file that can't be opened is said there, with Continue, and the others
  open; a check holds up only its own account. The CLI opens a data file when a command first
  uses it (`help` waits for none) and says "Checking" on standard error when that is a check.
- **Importing while reading:** an import (a `.pst`, or a MailCraft data file brought in) compresses
  each message before it takes the data file, and writes them in batches of at most 2 MiB (a
  bigger message alone) that don't wait for the disk (the folder lists written after them do, and
  so does a batch once 8 MiB are waiting). A message marked read during an import waits a few
  milliseconds for the writer (p99 6 to 31 ms in a measurement against a writer going flat out),
  where a 16 MiB batch held it for about a tenth of a second. The write itself still waits for the
  disk the import keeps busy (7 to 11 ms by median, a p99 of 45 to 230 ms on a busy machine and 78
  to 102 ms measured again, 13 ms with no import), but the window no longer makes it: a read mark
  is written on the store's thread for marks. A Down step marking mail read in a folder of 10,941
  while an import writes 2 MiB batches into the same data file as fast as it can took the window
  0.6 ms at p99 (`down_steps_during_an_import`), under the 60 ms aimed at.
- **The data file's cache during an import:** a data file's write transaction keeps what it
  changes in half the file's 16 MiB cache, and past that writes pages out and reads them back;
  with the import's 16 MiB batches this was thought to be the extra 28 GB read and 25.6 GB written
  on the 47 GB file. The batches are 2 MiB since, and an import now gives its data file a 32 MiB
  cache while it writes (`Store::bulk_begin`), closed and opened again as the import starts and
  ends, or the first time it's asked for once nobody holds it. Measured on a made-up file of
  600,000 messages (7.9 GB): 7.17 GB written for a 6.87 GB data file (1.04 times), against
  7.22 GB with the small cache; pages read back by a transaction 58% fewer, for about 22 MB more
  at the peak. 48, 64, 128 and 192 MiB took 1 to 5 points more off for up to 225 MB more, and
  4 or 8 MiB batches read back 1.7 and 4 times as many pages. The close at the end (16 ms, the
  next open 6 to 9 ms) also gives back the room the file grew by ahead of its contents: 8.59 GB
  came down to 6.87 GB at once, rather than when MailCraft closed. On the 41 GB file (34,691
  messages), twice each way: 40.05 GB written for a 37.69 GB data file (1.06 times, against
  45.30 GB with the small cache), 58.45 GB read where the reader itself read 56.50 GB (1.03
  times, against 63.7 GB), 74% fewer pages read back, a peak 4 to 26 MB higher (751 MB against
  725 to 747), and the same time (213 to 222 s; one 172 s run with the small cache, on a
  machine busy with other work). In the app, a new account's import of it took 189 s to its
  end, wrote 40.05 GB and peaked at 1.05 GB, against 198 s, 45.30 GB and 1.03 GB built with the
  small cache, whose data file stayed 38.65 GB instead of 37.69 GB until MailCraft closed. The
  app had nothing holding the new file as the import began, so its cache changed at once.
  Outside an import the cache stays 16 MiB: opening the biggest folder, 40 Downs and a
  mark-read read back no page with 16, 32 or 48 MiB, read 2,745, 2,395 and 2,111 from the file
  in the same time, and the first search, reading every message, within 1% as many. An import
  ended by a panic leaves the file open with its cache until it's next asked for: closed while
  a thread panics, the database writes nothing, and the file would need its full check.
- **Big attachments from a data file:** an imported message's files were copied on their way in
  (into the message being built, again into its parts, its source written out twice to put the
  original header back, and the item kept until it was compressed), so a message with an
  attachment of A bytes held about 4.7 A at its peak. The item's files now move into the message,
  whose parts read them where they are, the header is put back in the source's own buffer, the
  item is let go of once the message is written, and the compressed copy starts at 0.8 of the
  source (what base64 attachments come to) rather than 0.9, each buffer grown only by what it
  needs (not doubled) and kept at that while the message waits in its batch: about 2.5 A while
  it's compressed, and 2.7 A for a moment while a received message's longer header is put back
  (worked out from the copies, not measured on a big file yet). The text a message's preview
  reads is made once, for its text part and its preview both; the preview reads all of it, so
  words after more than 64K characters of quoted lines are found in a message made of HTML too.
- **Reading a `.pst`:** the import asked to write the data file as well as read it, and read it a
  block of at most 8 KiB at a time (about 4.5 million reads for the 47 GB file, one after
  another), reading the index's root page again before every block. It now opens the file to read
  only, and while it reads, Windows lets no other program write it (Outlook opened then can't
  change it under the import; a file Outlook has open is still refused, with the same message).
  Reads go through a 4 MiB cache: a page read again (the index's root) comes from it, reads in
  order read ahead 256 KiB, and others read just what's asked. outlook-pst also kept every index
  page it read until the end (the reader grew from 38 to 210 MB over the big file): the handle
  that reads items is now opened again once its pages are thought to hold 16 MiB, starting with
  what the cache holds. On the 41 GB file, one lane, on a machine busy with other work: 1.8
  million reads instead of 4.5 million, 52.9 GB read (1.29 times the file: reading ahead 256 KiB
  overshoots; 16 KiB chunks read 45.3 GB in 2.8 million reads, in the same time), the item handle
  reopened 23 times, and the folder handle, never reopened, held about 12 MB at the end. The
  whole process stayed at 60–190 MB, rising and falling with the messages read (it was 546 MB at
  its peak, the reader alone growing 38 → 210 MB). Reading items still took 373–437 s of the
  1,015–1,112 s, short of the 160 s aimed at for attachments; what's read is to come within 1.1
  times the file.
- **Importing on every core:** a `.pst` was read one item at a time on one thread (69% of one
  core on the 41 GB file), each item read, rebuilt, compressed and written before the next was
  read. A planner now reads the file's folders first (their items and the bytes the file gives
  them, so the progress has its totals from the start) and hands the items out to lanes, as many
  as the computer has cores less one, up to 12, each reading through a handle of its own and
  making the message, its summary and its compressed source; the writer keeps them in the
  file's order, so an import comes out byte for byte the same whatever the number of lanes,
  and an item with no message id gets one from the file and its place in it, the same when the
  file is imported again. What the lanes hold at once is bounded (288 MiB, charged by the size
  the file gives an item and again by what it is once read, given back once it's written);
  the item the writer needs next always goes ahead, so the import can't wait on itself. Each
  folder goes into the account's folder list as it finishes, and the window's own count saves
  change only counts in that list, so neither takes out what the other added. The reader
  panicking on an item costs that item (counted as unreadable, the lane's handle opened
  again); 32 in a row stop the import as a damaged file, keeping what came in; a fault in the
  planner or the writer stops every thread and keeps what was written. A folder listed under
  itself is read once, and the folders and items planned are capped.
- **Stopping an import:** an import couldn't be stopped: removing its account let it go on
  writing. It can now be cancelled (what came in is kept, the folder it was part-way through
  too) or paused (the same, but that folder stays out of the list); removing the account stops
  it within a second or two and it writes nothing more. A MailCraft data file brought in is
  copied whole or not at all: stopped part-way, none of it is kept, and a big one stops while
  it's still being copied, not only once it has been. A file opened again at once into the same
  account waits for the import being stopped, rather than being refused, and the end of an
  import thrown away is never taken for the next account given the same id (adding an account
  right after removing one said "0 messages imported" and took it offline). Closing MailCraft
  pauses an import and waits up to 10 seconds for it to write what it holds before the data
  files are sealed. The folder pane gets each folder as it finishes, not all of them at the end,
  and an Outlook data file looked at before it's imported is walked once, not again by its
  import. Removing an account while its file is imported now waits for the import to stop and
  then deletes what it brought in (a part of a file that's still there is worth nothing), so no
  partial data file is left behind; the account stays listed meanwhile and its name isn't given
  to the next file opened, and a removal MailCraft closed in the middle of is finished as it
  opens. Where each account's mail came from is kept (`imports.json`): an import interrupted by
  MailCraft closing goes on by itself as it opens (a POP account connecting once it's in), and
  one stopped waits to be continued, kept as it is, or started again; until then, as while it
  runs, the account's folders can't be made, renamed, moved, deleted, emptied or cleaned up and
  its mail isn't moved, deleted or snoozed (snoozed mail comes back once it's done), since the
  import writes them, and the window says so instead of doing nothing. The mail an import brings into an Inbox, stopped and continued too, is never taken
  for new mail (no rules ran on it, nothing was announced). Opening a data file that's already
  open as an account (the same path, however it's written, or the same file under another name)
  is caught before it's imported a second time, unless another copy is asked for. An import no
  longer says "N messages imported so far" in a notice every few seconds. A data file's account
  left with nothing in it by an import that never ended (before this was kept) is found as one to
  import again or remove. The folders a file holds are shown ahead as an outline: as soon as
  an import (or a run of one going on) has planned the file, the folders not in yet are listed
  faint after the account's, indented as deep as each will be, and each becomes a folder like
  any other, in its place, as it finishes: in the window, importing the 41 GB file, 42 of its 46 folders were outlined
  5 s in, 9 a minute in, and none once it was done.
- **Going on where an import stopped:** continuing an import read the file again from its start
  (the same messages coming out the same way, so a 41 GB file paused near its end took as long
  again). An import now notes where it got inside the account's data file, in the same commit as
  the messages it names: the folders done, the one part-way through (its next item, its next
  message number and its list so far), the items the reader failed on, and the counts. It's
  written at least every 10 seconds, before finished folders are shown, and as the import
  stops or ends, each time a commit that waits for the disk and reopens at once after a crash.
  Paused (MailCraft closing), cancelled, or cut off (the computer turned off), it goes on from
  that note: what was written after it is written again under the same numbers, an item the
  reader failed on is never read again, and a cancelled folder, shown since, keeps its messages
  and numbers. The file is checked first: the same size and time, or else the same record key
  and the same items and bytes in each folder (Outlook opening a file changes its time and
  nothing it holds), and one that changed or went waits on the user instead of being mixed in.
  An import done answers at once without reading the file. The note in the data file is what
  decides: an import the user cancelled isn't gone on with by itself even if MailCraft closed
  before it said so, and one whose record was lost is found from it. A MailCraft data file
  brought in leaves its own import's note behind. The checkpoint read back is checked (a fuzz
  target, `import_checkpoint`). Keeping what a paused import brought in (Keep what's imported,
  for one paused, or waiting on a file that changed or went, or on room) left the folder it was
  part-way through out of the folder list, its messages in the data file but shown nowhere: that
  folder, and the Sent Items folder made to be read last, now come into the list with the
  messages the note names, numbered as they came in (those written past the note, before a
  crash, go), and the import's note says it's done. In the window, an import of the 41 GB file
  paused at 98% (reading its Sent Items folder, last) and then waiting on the file another
  program held: Keep brought that folder into the list, 46 folders and 34,267 messages listed
  against 45 and 33,292 before it; the same on the build before this, paused at 80%, left 38
  folders and 18,136 messages as they were.
- **Seeing an import go:** opening a `.pst` said "Opening {file}…" in the status bar for as long
  as it took (13.6 minutes on a 41 GB file), with no total, no percentage, no time left and no
  way to stop it, and the folder pane said "Loading folders…". A file chosen is now looked at
  first: how many items in how many folders, the size of MailCraft's copy and where it goes, the
  room free there (no Import when it won't fit), and the account it's already open as (go to it,
  continue it, or import another copy). While it's read the status bar has its bar, from the
  folders' item counts and sizes, so an item weighs its bytes plus as much as 128 KB would (a
  folder of small messages isn't shown as nearly free), with the percentage from the first
  progress and "About 40 min left" once 20 s and 2% are behind it, smoothed over about 30 s and
  revised only when off by more than 10% or 15 s (under a minute, minutes up to an hour and a
  half, then hours and minutes to five). A click shows its details: messages and data done of
  their totals, the folder being read, the time so far, what couldn't be brought in so far,
  where the copy is (Open file location), and Stop importing, which asks first and keeps what
  came in. The account's line in the folder pane has a thin bar and the folder being read; its
  menu stops, continues or keeps a stopped import, and a stopped one can be continued from that
  line. Closing MailCraft while a file is read says it goes on the next time MailCraft opens,
  then pauses it; removing its account asks whether to stop and delete the part imported. At the
  end a summary says how many messages came into how many folders, with the contacts, calendar
  items and notes, how long it took, and one line for each reason something was left out; it's
  there again as the account's Import report. An import that waits is "paused" when something
  stopped it (the file gone or changed, the drive full) and "stopped" when the user did, alike
  in the status bar, the folder pane and the account's status; one whose data file's place was
  lost isn't offered Continue. What couldn't be brought in so far shows its reasons on hover
  once a run has stopped and said them (a run going on only counts them). A file picked again
  is looked at again, so room freed since lets it be imported. A bar without a total (a MailCraft data file
  brought in) slides at most ten frames a second, and stands still with animations off. The
  welcome screen offers "Open an Outlook data file…" beside adding an account, and sizes are
  written in GB once they reach one. On the 41 GB file in the window (release build, scratch
  data folder): "Counting items..." from the window's first frame, the percentage 2.2 s later
  once the folders were counted, folders appearing in the pane as they finished (from 1 to 50
  rows over the run), 34,691 messages in 46 folders in about 2.5 minutes, and the window's
  message loop never held more than 23 ms (5,354 checks 20 ms apart, none over 50 ms; peak
  working set 999 MB). The folder pane's line names no folder while a `.pst` is read, since
  its folders are read a few at a time.
- **Room for an import:** the window looked at a chosen `.pst` before importing it, but never
  learnt how big MailCraft's copy would be or how much room the drive had, so it couldn't warn
  that the copy wouldn't fit; and a drive that filled during an import failed every batch after,
  counting thousands of messages as couldn't be saved. A file looked at now says the size of
  its copy (98% of its messages' own sizes: 38.65 GB for 39.5 GB on the 41 GB file), the room
  asked for (130% of that and 2 GB besides, for the file's growth while it's written) and the
  room free on the drive of the data files' folder (or the nearest folder above it that's
  there); Import is offered only when it fits, and a room that can't be told doesn't stop it.
  An import asks for the room as it begins, with every checkpoint and every 64 MB it writes:
  with less than 1 GB left, besides what the data file can grow by in one step (up to 4 GB once
  it's past 4 GB), it stops as a pause does, keeping and listing what came in, nothing counted as
  left out, and waits saying the drive is too full; Continue asks again, and goes on from where
  it stopped once there's room. A write the drive refuses all the same (something else filled
  it) stops it the same way, going on from its last checkpoint, where it used to count every
  batch after as lost; so does any other write to that account's data file failing at the disk
  meanwhile, rather than going on and losing what it had written since its last checkpoint. A
  MailCraft data file copied in asks for the room too, counting the copy it's read from (made on
  the same drive, so about twice the file's size), and waits with none of it kept when the drive
  fills. A file that makes the reader fail as it's looked at is said to be one that couldn't be
  looked at.
- **Long folders:** moving through a folder of thousands of messages finds each one through an
  index built with the list, instead of walking the list several times a step; the conversation
  shown above a message is worked out once with the list, not again for every message opened
  (with conversations on and the list arranged by sender, each Down cost about 46 ms on a
  made-up folder of 10,941 messages, now about 6 ms; on an imported folder of that size, mail
  already read, a Down takes 18–60 ms with the message read in, about 10 ms of it the list's and
  the rest reading and parsing the message); and every message selected (Ctrl+A) is no longer
  copied three times a frame to draw the ribbon and menus. The rows are now built with the list
  too, not again for each step; the message is read on the reading lane (below), and a read mark
  is written on a thread of the store's own (below), where it cost about 0.2 s a step in the
  frame. Measured since on that imported folder: a Down's longest frame is 1.5 ms by median and
  4.5 ms at most on unread mail (each mark on the disk in 4 ms by median), and the same on mail
  already read (5.1 ms at most), within the 25 ms a step aimed for.
- **Text files saved as they came:** a text attachment (`.txt`, `.htm`, `.csv`…) was handed out
  as the text the parser read from it, in the charset it named or, with none, as UTF-8 with
  every other byte replaced, so a file in Latin-1 or Windows-1252 was saved, opened and
  forwarded changed, from any kind of account (a `.pst`'s Rich Text items carry such `.htm`
  files). Its bytes are now taken from the message's source, only the transfer encoding
  (base64, quoted-printable) undone, in an attached message too; its preview (in the reading
  pane, and the invitation card's calendar file) is still read in the charset it names, and a
  forward, or an Outbox message taken back to change, sends it with that charset on its type.
  The size the list shows is still the text's.
- **Opening a big message:** whether a message is signed or encrypted was found by parsing all of
  it, then it was copied and parsed again to show it, so a message of 50 MB of pictures spent
  about 0.1 s being looked at for a signature it didn't have. Its header alone says now (12 to
  18 µs on that message); only signed or encrypted mail is read further, which includes a `.p7m`
  sent as `application/octet-stream`, named in its Content-Type or only in its
  Content-Disposition. The message shown is parsed once for the reading pane and its own window
  together, and drawing them no longer copies it every frame; a reply takes its own copy.
- **An import whose last save fails:** when the folder list or the contacts and calendar
  couldn't be saved at the very end of a `.pst` import, the account was removed with the data
  file it had written, mail and all. What came in is now kept (the folder list is saved again,
  so it's there after a restart; if that fails too, the notice says to import the file again)
  and the error is said with the count. The import also counts what it leaves out that it used
  to skip silently: a journal or another kind of folder MailCraft has no place for, with its
  items (folders under it are still read), and a contact or calendar item that couldn't be read
  is told apart from one that was read but can't be used. The progress bar moves through items
  left out and contacts as it does through mail.
- **Reading while moving down the list:** the message shown was read from the cache and parsed in
  the frame that showed it, so the biggest messages (0.1 s) and the longest ones held the window
  up each time one was landed on. Where the app has a thread for it, that work now goes to the
  reading lane: the pane shows the subject and a spinner for the frame or two it takes, only the
  message landed on is read (not each one passed), and its pictures are taken out of the source
  read for it rather than read again. A command run meanwhile (reply, forward, save) finishes it
  first, so it acts on the message shown. Signed or encrypted mail, the command line and the
  start-up `--cmd`s still read it at once.
- **Mail from a `.pst` is listed when it came:** the list placed a message by its `Date:`, when
  it was sent; Outlook places received mail by when it was delivered, and 1,177 messages of a
  41 GB file were more than an hour apart on the two. A `.pst` message now keeps its delivery
  time beside its date: the list's order, its date column and headings, conversations, the
  new-mail notice, `received:`, `after:` and `before:` go by it; the reading pane, `sent:` and
  `date:` still go by when it was sent.
  Lists written before read as they were (IMAP mail has no delivery time kept yet).
- **Conversations by Outlook's conversation index:** a message the ids leave without a parent,
  or hanging only from ids no listed message carries (Outlook keeps no Message-ID for sent mail,
  so a reply names one the folder doesn't hold, or none), goes under the message
  its `Thread-Index` answers, whatever its subject, or else joins the conversation that index
  names when its subject is the conversation's. An index of the wrong length, of more than 50
  replies or with no conversation GUID is passed over. It is kept with the list as three hashes,
  so working out conversations never decodes it; mail already listed has none (a `.pst`
  imported again gets them).
- **Sent mail's own ids from a `.pst`:** Outlook keeps no Message-ID for the mail it sent, so a
  sent item came in with one MailCraft made up, and the replies to it named a message the
  account didn't hold. The Sent Items folder is now read after every other (it's still made in
  its place, under its name, and read even when the reader failed on a folder before it): a
  reply read before it whose conversation index is a sent item's and one block more gives that
  item the id the reply answers, when the item has none of its own (and no original header). Nothing is guessed: replies
  answering two ids give none, an id a message read has already isn't given, and one is given
  once. Up to 16 MB of replies and ids are kept, with the import's checkpoint (an import gone on
  with gives the same ids); past that, sent items keep the ids made up for them. Replies in Sent
  Items itself (to your own mail) aren't used, and drafts get none.
- **Signed and encrypted mail from a `.pst`:** it came in with its signature or its encrypted
  content as a file, `smime.p7m`, that nothing could check or open. Outlook keeps that content
  whole; it is now put back as the message's content, under the message's own header fields, so
  the signature is checked and encrypted mail opens with the certificate it was sent to. A
  content that isn't what the item says, or whose header would add fields to the message, stays
  a file, as before. The 41 GB file measured has no such mail, so it's tried on made-up items only.
- **More of what a `.pst` holds:** a meeting request, cancellation or answer carries its
  invitation again (`text/calendar` with its method), so it can be accepted or declined from the
  reading pane. Posts and documents in a mail folder come in as mail, and so do recalls and
  sharing invitations now (**Recalls, sharing invitations and contacts in a mail folder**,
  below); a contact there goes to the address book. Contact
  groups come in as group cards with their members' addresses (members that are only contacts
  of the file aren't listed), journal entries as notes with their time, and calendar items keep
  their categories, Show As, privacy and reminder. A repeating appointment's changed occurrences
  (moved, retitled, with a room, text, guests, Show As, reminder or files of their own) come in
  as events of their own that replace their occurrence (`RECURRENCE-ID`), from its pattern's
  ExceptionInfo and its attached exception items (MS-OXOCAL 2.2.1.44, 2.2.10); before, a moved
  occurrence was simply gone. An exception item no ExceptionInfo matches (its pattern cut short
  or damaged) still replaces the occurrence it names, and one that says no time is counted as
  not usable. A meeting request carries them in its invitation too; an appointment attached to
  a message keeps them hidden, as the message's bookkeeping. The files of
  contacts, calendar items and tasks come in on the card, event or task: an event's and a task's as `ATTACH`es with
  their bytes, a contact's picture as its card's `PHOTO` and its other files as
  `X-MAILCRAFT-ATTACH` (vCard has no property for a file), an attached message as its `.eml`.
  The contacts and calendar are one document, written whole at every change and exported as one
  `.ics` and one `.vcf`, so a file is kept up to 1 MB and all of them, cards' and events'
  together, up to 8 MB (as written, in base64); one past that is counted as too big. A calendar
  or address-book file is now read to 16 MB, as much as one opened is (4 MB and 1 MB before), so
  the files kept leave room for the rest (a calendar past 20,000 events, or an address book past
  50,000 cards or 16 MB, still doesn't read back whole), and the calendar borrows its events to draw them instead of copying them, files and
  all, at each frame, as People does its cards (`Session::people` hands out `Person`s that
  borrow theirs, the status bar, list and reading pane's contact card copying none). MailCraft doesn't show these files yet; they go with the event or card
  when it's exported (`.ics`, `.vcf`). A note's and a journal entry's files come in too now (**A note's files**, below). The 41 GB file
  measured has none of these (its calendar, contacts and tasks folders are empty, and its
  34,691 items are all mail), so they're tried on made-up items and on a file the writer makes
  (`pst/tests/pim_written.rs`: a repeating appointment with a changed occurrence attached to it
  as Outlook keeps one, and a contact with its picture and a file, written with the writer's
  named properties, container classes and attached messages, and imported whole). A contact's
  picture past the 100 MB cap on an attachment is counted as too big, not dropped unsaid.
- **The search index is built in the background:** each account's mail is now indexed for
  search as it comes (a sync, a message downloaded, a move or delete, an import's end), on a
  thread of its own, with the messages read and their text taken out on all but two of the
  computer's processor threads (at most eight); asks close together wait a second and a half
  and make one pass. A pass reads only what changed (a folder whose list and kept mail are as they were is
  passed over, a moved message isn't read again), writes the index into the account's data file
  in short transactions, each after the read marks handed over before it, so a message marked
  read meanwhile waits for one of them at most (on 20,000 messages in one folder, indexed in
  9.8 s, a mark-read took 3.6 ms by median and 11.7 ms at p99, against 3.4 and 3.7 ms with
  nothing running; within the 60 ms the plan sets), and stops between two
  messages when an import starts or the account goes, what it did kept and nothing written for
  an account removed. A message whose text can't be taken out is counted and kept with no
  words. No pass runs for an account a data file is being imported into. The browser builds its
  index the same way, a few messages a frame.
- **Search never waits on the mail:** the search box read and parsed every message in the folders
  it looked in (every attachment decoded with it) on the window's thread, and threw that away when
  a message was marked read, a folder changed or another view opened: 2½ minutes on a 41 GB
  `.pst`'s mailbox, again after opening an unread result. It now reads the index: the summaries
  answer flags, dates, senders and subjects, the index the words, the text it keeps the phrases,
  and no message is read for a search in the window. It runs a few bounded steps a frame (a page
  of a folder's list, 512 summaries, two pages of the index, 256 KiB of a text), so typing and
  moving about never wait for it; what's found shows as it's found, newest first, and is put into
  conversations at the end. Mail the index doesn't have yet may be missing (counted, for the
  search box to say) and comes in by itself once a pass has indexed it, without searching again. Marking a
  result read or flagging it weighs that message alone; mail coming into a folder has that folder
  looked at again; another view keeps each folder's folded text. `has:attachment` counts the
  files the index found, leaving out pictures the text shows (on mail indexed). The CLI and
  scripts still wait for a search, reading from its source what the index doesn't have yet
  (`search.run` takes `"background": true` to not wait).
- **The search box says how a search is going:** a search takes 6 ms of each frame, the time
  checked after each step of it, and starts a quarter of a second after the last key (0.3 s
  before). The list says "Searching…" while it runs (over an empty list, rather than "No
  results"), "Some results may be missing: still indexing (NN%)" while mail it may match isn't
  indexed yet, and "Couldn't read the date …" for a date that isn't one (`received:2024-13`).
  The status bar shows "Indexing NN%" while the search index is built, with how many messages on
  hover. Advanced search has Received after and Received before (days, as `after:` and
  `before:`). A search that fails part-way ends, and the index it was reading is left alone
  until it's written again, so the next search can't fail on it too.
- **A `.pst` import indexes as it goes:** a data file's messages were indexed only after its
  import, by a pass reading every one of them again (157 s for the 41 GB file measured, the
  app holding 631 MB at its peak). The import now puts each message in the account's
  search index as it writes it, its words made from the message as rebuilt on the threads that
  read the file, and marks each folder as indexed as it finishes it: the pass at its end reads
  no message. The index is written in short transactions of its own, after the sources it
  names; one that can't be written, or an index another pass still holds 5 s after the import
  began, costs the import nothing, and the pass after it adds what's missing. It isn't free: on
  the 41 GB file the import took 197–245 s with indexing against 161–219 s without it, run
  in pairs on a machine busy with other work (12–37% more, about a fifth in the middle), for a
  pass of 157 s it no longer needs; the aim, within 5%, isn't met. The extra time is the index's own
  writes into the account's data file, which the import's writes wait for: an index filed and
  written on a thread of its own, its writes sealed or not, measured no faster.
- **The import measured whole:** on the 41 GB file (34,691 messages), every aim of the import
  work was measured again in one sitting (`pst_bench::import`, which now also pauses an import
  at random or at a place, goes on after a kill, and judges the time left the window would have
  said). In the app a new account's import took 172 s, the window answering within 12 ms all
  along (none of the 50 ms frames aimed at), its folders shown as each finished and its
  percentage from the first seconds, for a 37.75 GB data file, 1.42 GB at its peak; one cut off
  2 minutes in (its process ended; twice) and opened again checked its data file for 57–59 s, the window
  answering all the while (its slowest frame 336 ms), then went on into the file it had open and
  finished with every message, 1.33 GB at its peak. By the benchmark it took 173–214 s with
  indexing, most runs over the 180 s aimed at (154 s without), wrote 40.3 GB (1.07 times the
  data file), and read 58.5 GB where the reader read 56.5 GB (1.04 times); the data file's bigger
  cache while an import writes (32 MiB, 16 MiB otherwise) raised the peak by 18 MB, within the
  32 MiB aimed at, and saved 5.3 GB of writes (45.4 GB, and a 38.65 GB file, with 16 MiB). One
  thread took 611 s, 326 s of it reading, its working set flat as aimed at (324 MB a fifth of the
  way, 336 MB at nine tenths). Twelve threads and one came out the same byte for byte (a digest
  of the folders, their lists and the sources), and so did an import paused 30 times at random
  and one killed 5 times at random, gone on with each time: 34,691 messages, every one parsing,
  with 107,328 attachments (the 107,134 files aimed at, with the 187 attached messages and 7
  empty files that come in since attached messages do). The checkpoints cost under 1%: 29 to 42
  of them took 1.2–1.9 s, the batches they went with included, and one every 10 s besides those
  at each folder's end changed nothing by the clock. A pause took 0.3 s to stop by median, but 5
  of 114 took 10 to 17 s (16.8 s at the 99th, under 2 s aimed at), all in one run at one place,
  where the import gone on with stopped again before it got any further, the process waiting on
  something rather than working; a stop that slow outlasts the 10 s MailCraft waits for an
  import as it closes. That tail didn't come again, so what caused it isn't known. One wait
  with no bound was found and is gone: a stop waited for each item being read to be read to its
  end (a 37 MB one's reads took 1.2 s with another program writing to the drive, and on a drive
  kept that busy, or a slow one, many times that). The threads reading the file now ask before
  each read whether the import is stopping, and let the item in hand go (it's read again going
  on), as does the walk over the folders, whose plan cut short is no longer taken for a file
  that changed: 20 pauses in the first 7,000 messages with another program writing to the drive
  all along stopped in 0.3 s by median and 1.2 s at worst (0.6 s and 1.7 s before; in a test
  whose reads take a second each, 11.6 s before and within 2.5 s now), and 18 where the tail
  was, 31,600 to 33,800 messages in, three runs each going on and stopping again before it got
  further, with the drive written to all along, in 0.6 s by median and 1.3 s at worst. Whether
  the slow read was what held those 5 stops isn't confirmed: the 2 s aim is met where it was
  measured again, not shown met for whatever held them. In the window, on a fresh import (release build, scratch data
  folder), Stop showed the import stopped 0.39–0.64 s after it was pressed (0.30–0.55 s on the
  build before), and Close MailCraft ended the process 0.46 s after (0.40–0.43 s): a pause there
  was quick before too, its slow ones rare. The writer was busy 58–61% of the
  time with indexing (55–58% without), 85% aimed at: it waits on the threads reading the file.
  The peak was 1.17–1.22 GB with indexing (0.76 GB without), its working set growing by a third
  from a fifth of the way to nine tenths, where 1 GB and flat were aimed at. Opening the data
  file after a kill took 1.7 to 55 s (the file checked whole), over the 5 s aimed at: between
  the checkpoints, which are sealed, the import's batches past 8 MB, its folder lists and their
  pages were made durable unsealed. While an import writes, every commit of its data file that
  waits for the disk is now sealed, and its batches wait for the disk 256 MB at a time (sealing
  each 8 MB cost 35 s and 8.5 GB of writes): killed four times 25–40 s into a run, it opened in
  12–126 ms, and the import took 132 s writing 42.4 GB against 160 s and 40.2 GB the same day. A
  mark saved meanwhile waits longer for the disk (147 ms by median after 256 MB of batches, 10 ms
  after 8 MB), on its own thread, never the window's. In the window, killed at 81% and 90% of a
  fresh import, MailCraft showed the account 0.54–0.58 s after it started and the import was
  going again 1.4–2.2 s after; on the build before, killed at 68% and 72%, it went on 50–56 s
  after, its data file checked whole meanwhile. After an
  import paused and gone on with, the next index pass looks at the lists of most folders again
  (30,342 messages' worth, none of them read). And the time left, which waits on the owner (or
  the plan): from half done it was 4–26% off (10% aimed at), and before that up to twice the
  time it took (20% aimed at; less since, in **The time left from a tenth done**, but not met:
  open, #10), and no
  estimate from a rate can meet the 10% on this file. Even
  the run's own average over the whole of it, known ahead, would have been off by up to 13% from
  half done (the import going faster and slower than its average, folder by folder, by more
  than their sizes tell), and weighing each item by its size (its bytes raised to a power, or a
  share growing with it) did no better in a replay of five runs (13% at best); from a tenth done
  that average would have been within 13%, but the speed seen by then is a fifth below it, the
  first folders, of big messages, going slower. By the design's rule an item should now weigh
  nothing in the bar: fitted folder by folder as the design asks (26 folders, r2 0.975, on a run
  slowed by other work) as over the progress, an item costs less than nothing (-0.4 to -2.2 ms),
  which says that a big item costs more for each byte, not what an item costs of its own. What
  an item costs of its own was then measured where it shows, on two files made for it and
  imported twice each: 40,000 messages of about 4.6 KB in 1.46–1.55 s and 6,000 of about 1 MB in
  11.4–11.6 s, which is 29 µs an item and 1.8 ns a byte, about 16 KB (the 128 KB an item weighs
  came from the import that read one item at a time, at about 2 ms an item). Replayed over eight
  imports of the 41 GB file, the time left with 16 KB was off by up to 16% from half done and
  146% from a tenth (19% and 155% with 128 KB, 20% and 147% with bytes alone), but in the window,
  one fresh import each way, 16 KB was off by up to 125% from a tenth and 19% from half done
  where 128 KB was off by 50% and 7% (one run each says less than the runs differ by), and the
  compaction's and the search index's bars weigh their items the same way, unmeasured with
  16 KB: an item still weighs 128 KB. A weight for each kind of folder or each size of item,
  fitted over the replayed runs, wasn't tried: those runs kept their progress, not the size of
  each item in the order it was read, which such a fit needs. The aims stay unmet, and no single
  weight meets them: on this file the speed goes up and down by a half, the same way in every
  run, from one stretch of folders to the next (the first twentieth half as slow again, the next
  stretch twice as fast); an estimate that starts from the run's own average and turns into the
  30 s one by half done was off by up to 64% from a tenth but 21% from half done, so it wasn't
  taken. Meeting them needs another aim or another way to tell the time left: the owner's to
  choose.
- **What a `.pst` import keeps, measured whole:** every item and attachment of the 41 GB file
  (34,691 messages in 46 folders) was compared with what an import of it holds, read from the
  file a second time, and the benchmark's own count of what came in (`pst_bench::import`'s
  `kept:` line, with `MC_VERIFY`) agrees, as does the import test on a real file
  (`real_files::imports_the_file_in_mailcraft_pst`: 34,691 messages, none skipped, 107,328
  attachments, attached messages 187, 8 and 2 deep with 742 files inside). Every message is where
  it should be (the 33,291 with a Message-ID of their own at the same folder and number, every
  folder's count the file's), and the import's report leaves nothing out, for any reason. HTML
  bodies read in their code page: 33,608 of 33,610 give the text their code page gives (2,144
  came in as mojibake before), and the other 2, valid UTF-8 labelled ISO-8859-1, read as UTF-8,
  as meant; the 34 with U+FFFD hold it in the file. The 996 items kept only in Rich Text, and the
  3 plain-text items whose RTF wraps HTML, all have their HTML; the 35 messages with no text are
  HTML bodies with none (pictures alone, mostly). The 187 attached messages come in, with the 10
  attached inside them and their 741 files, and one part more: the invitation MailCraft writes
  for a meeting request, here one attached and kept only in Rich Text. 107,139 of the 107,141
  files come in byte for byte and under their own name, all 105 text files and the 7 empty ones
  among them (the comparison before the fidelity work couldn't match 2 either). All 78,518
  pictures the text shows are pictures in the body with their Content-ID, and all 1,690 files
  with a Content-ID the HTML doesn't show keep it. The design's 3,499 unshown files were counted
  on the file's own HTML: 1,809 of them belong to items kept only in Rich Text, whose HTML now
  shows them. (A first count paired a file with another of the same bytes in its item, which
  1,290 files have, and took 42 such mix-ups for lost Content-IDs and names: there are none.)
  Forwarded (119), high importance (991, 12 of them on items with no header) and the References
  of the 1,099 items with no header that have them all come through, as do the 11 reports and
  the read and answered marks. The list places every message within an hour of its delivery
  (1,177 were placed by the time they were sent, more than an hour off), and 34,662 of 34,691
  have a conversation index to thread by (29 have one of the wrong shape). Sent mail's ids: the
  1,400 items without an id of their own (1,399 of them in Sent Items) keep the ones made up for
  them, as no reply in this file answers one of them (no reply's conversation index is a sent
  item's and one block more), so the import has none to give. It costs what the import measured
  whole did: 220 s on a quiet machine (250 s beside other builds) against 173–214 s, 40.3 GB
  written and 58.6 GB read as then, 1.20 GB at its peak against 1.17–1.22 GB, for the same
  37.75 GB data file.
- **The paperclip on mail kept from before:** a `.pst` imported before its paperclips followed
  the source (and any mail whose summary says otherwise than its source) kept the old flag, so
  Has files and the paperclip still counted pictures the text shows while `has:attachment` didn't.
  The search index now puts those summaries right as it reads the mail, once for mail already
  indexed (every folder is looked at again by the first pass), and the list shows it at once, so
  the three agree. Only the pages of a folder's list holding a message put right are written. A
  message the index couldn't read, or couldn't read all the parts of (a file nested past its
  limits), keeps what its summary says, for `has:attachment` too.
- **Searches that find tens of thousands:** measured on a made-up mailbox of 50,000 messages in
  40 folders (the Inbox 12,000; `sample::big`, for the tests and `bench.rs`), a search finding a
  few messages a step rebuilt the whole list each step, and each one found was first looked for
  over the whole list: a common phrase took 14 s from a session just opened, and showing 35,000
  found held frames for 30–65 ms. A long list now takes what's found every so many steps (one
  for each 2,048 messages it holds; a short one at once), at the start of a step where the
  window's clock counts it; its index keeps the keys' hashes rather than hashing them again;
  the end of a search adds what's left to the list it streamed into rather than copying every
  message found again; a phrase's candidates are checked in the index's order (103 pages read
  rather than 1,015); the list a search replaces is let go of a slice a frame; and the search
  command takes 4 units rather than 16 before the window's frames take over. In the window
  (release build) a word takes 25 ms warm and 130 ms just opened (its summaries' text made),
  the common phrase 0.3–0.4 s, `is:read`'s 35,000 70–80 ms and a one-letter word's 47,000
  180–270 ms, a word's frames spending at most 8 ms on the search; Down through the 12,000
  builds no rows (8 ms a frame at p99). Not met: the frame that shows 35,000–47,000 found
  builds the list at once, 10–12 ms; `is:unread` over 50,000 takes 35 ms against the 15 ms
  aimed for over 35,000; the common phrase's 16,667 found, each checked against its text, take
  longer than the 150 ms aimed for a phrase (Open, 8).
- **Clearing a search:** emptying the search box dropped everything the search had shown and
  held at once, then copied the folder's summaries out of memory again, arranged them and worked
  out their conversations and rows: on the 10,941-message folder of the 41 GB file's data file
  that held the frame for 18–36 ms after a search of a few results and 56–91 ms after one of
  7,481 to 32,574. The folder's list on view when a search starts is now kept, with its
  conversations and rows, and shown again when the search is cleared, unless anything it was
  built from changed meanwhile (a mark, a move or new mail in a folder, another folder opened,
  the arrangement, the filter or Focused); what the search showed and held is let go of a slice
  a frame. On the made-up mailbox (release build), clearing 35,000 found over the Inbox's
  12,000 takes 1.6 ms; on the real file's folder no clear's frame took over 5.9 ms (0 over 50 ms
  in 60 clears), and typing the 20 queries of its ground truth a character at a time no frame
  over 10.8 ms (2,246 frames).
- **Searching the 41 GB file, measured:** each search now logs, when it's done, what it spent
  its time on by kind of work. Warm, on the file's 34,691 messages, a word's search is almost
  all checking the summaries (25–50 ms) and looking the word up (2–24 ms); a word found in
  thousands copied each summary found twice on its way to the list, and now once: the negated
  word's 32,574 found take 140 ms from the query to the last frame (203 ms before), `to:` a
  domain's 20,713 75 ms (111 ms). Most word searches take 41–56 ms, a word found in 7,654
  messages 80 ms, against the 50 ms aimed for; the phrase is 0.7 s of checking kept texts. Every
  count matches the file's ground truth.
- **Searching the 41 GB file, quicker:** what each warm search spent its time on named what to
  cut. A summary is now looked through once for a word, a quick look most fail, before each of its
  parts word by word; the index's words for a search are found without hashing them for each
  summary, a folder's number looked up once a slice, and a folder's first scan doesn't ask whether
  what it doesn't find was found before (checking the summaries of a word search: 6–26 ms, from
  17–50). A word looked up lately is taken as the same generation of the index gave it (9–11 ms
  before, for a word in 7,654 messages). A phrase's candidates take their summary's text from the
  scan rather than folding it twice more, and the texts they're checked against are kept folded
  between searches (32 MiB), so the phrase typed on or searched again reads and folds nothing: the
  file's 1,452 candidates are checked in 7 ms. The index reader keeps its text pages apart from its
  lexicon and postings, a unit makes at most 128 summaries' texts in a session just opened, a short
  prefix sets about two pages' worth of docs a call, and what a phrase's candidates said is let go
  of a slice at a time. Warm, from the query to the last busy frame: a word found in 7,654 messages
  32 ms (77 ms before), other words, a prefix, accented and CJK words and two words 6–21 ms (23–75),
  the phrase searched again 16–24 ms (958), `to:` a domain 67–71 ms (76), the negated word
  109–127 ms (140); still short of the aims (docs/gaps.md Open #8): the phrase searched the first
  time, 0.79–1.17 s, as each candidate's text is read and folded, the negated word and `to:` a
  domain, copying the summaries found. No frame of the three passes took over 15 ms, and typing the
  20 queries a character at a time none over 9.8 ms (2,157 frames). A frame's share of the search
  stays 6 ms: given 10 ms when nothing was typed, each frame waited for a second refresh of the
  120 Hz screen, and the negated word took 198 ms. Every count matches the file's ground truth. A
  text kept folded is kept with the doc it was read from, in the index's epoch: a key that comes
  back as another message (a local folder deleted and made again numbers its mail from 1 again) is
  checked against its own text, and a folder's texts go when it leaves the listing. A summary
  filled in by its download while no search showed kept its old text there, its uid unchanged at
  its place (a word only its new preview had went unfound, one it no longer had still found):
  that text is made again now, the folder's others kept. The app's runs
  on the file were built before the last search change (what a phrase's candidates said let go of
  a slice a tick, which only moves work out of the step that starts the next search); the made-up
  mailbox's figures are from the final tree.
- **A phrase from its words' places:** a phrase was settled by reading and folding each
  candidate's text kept in the index, which was nearly all of a common phrase's first search
  (its 16,667 candidates over the made-up 50,000 messages, 322–454 ms in frames; the 41 GB file's
  phrase 0.79–1.17 s). The index now keeps where each word of a message's text and files' names
  is among its words, in places pages of their own beside each segment's postings (a word looked
  up never reads them), and a phrase is settled from them, a few pages a unit once its words are
  looked up, for the messages with all of them: its words one after another, the last as a word's
  start, in one file name (the names' places are a word apart), never starting at a part of a
  dotted word. No candidate's text is read for it; it's checked against the text only where the
  places can't tell (a word of it too long to index, or one too long to index just where its last
  word would be). A phrase settled lately is taken as the index's same generation gave it, as a
  word is. An index from before has its words made again from the texts it keeps when its writer
  opens (10.4 s on the 41 GB file, no message read); merges and new ids carry the places. Made-up
  mailbox: the common phrase 89–113 ms in frames in a session just opened (150 aimed at), 11–21 ms
  searched again, no text page read. The 41 GB file: the phrase the first time in a session in
  use 30–58 ms and again 8–11 ms, the same 449 found as by its texts (0.71–1.10 s); a common
  two-word phrase's 13,718 36–59 ms (3.3–3.4 s); just opened, 162–197 ms, as every word search
  then is (its summaries' texts made). The places made the index's segments 6.7 times as big on
  that file (9.1 to 60.8 MB as written; a fresh import's data file 29 MB bigger in 37.75 GB) and an
  import no slower (100 s against 107 s). A message's files' names are now read for its words as
  they're kept (an empty name was taken for the end of them, the names after it left out and the
  message counted as cut short).
- **Holding Down while marks are written:** a hold of Down over unread mail on the 41 GB file's
  data file once had frames of 0.5 and 1.8 s, and the window answered nothing for about 7 s,
  while two read marks took 3.1 and 3.5 s to commit (the disk flushing). Reading a folder's list
  from the disk, or the folder list, waited for every mark handed over before; now only for
  those of that folder (of the account, for the folder list), so another folder opened, a draft
  filed or a search reading the other folders doesn't wait, and a search reads the folder whose
  marks are being written after the others. A local account's folders counted again after a
  move or a draft are written on the marks' thread too, after the marks before them, rather than
  waiting for them in the window's frame; and the window's settings, flushed to the disk every
  few seconds when they changed (5–15 ms of a frame, 197 ms once while the disk was busy), are
  written on a thread of their own. A wait for the marks, or for the lock every change of a list
  takes, over 50 ms is logged with what waited, and the log's lines are written on a thread of their
  own too (a line logged from the window's thread waited there for the disk). A folder whose marks
  are still being written isn't read from the disk by the window at all (opened again after the
  cache let go of it, its read waited for their commit): it's read a moment later, once they're
  written (one said changed meanwhile keeps its list, the message read with it, until then; Delete
  All and the like wait for them), and a folder list the window writes whole goes on the marks'
  thread.
  Measured on the real file: 40 read/unread toggles had every mark written within 5.3 ms. The frames
  of 21–57 ms and the 0.6–0.8 s without a frame measured beside a disk flushing writes came, as far
  as could be found, from the measuring: its probe appended a line to a file from the window's
  thread every frame, and the tool that took the window's stack when it stalled suspends the whole
  process for seconds while it loads symbols. Timed with the probe's lines handed to a thread of its
  own, and eframe's painting split into its parts, 16 ten-second holds of Down over unread mail (213
  marks each, 8 of them beside a disk flushing) had no gap between frames over 24 ms and no frame
  over 16 ms but one of about 23 ms in 4 of them: egui-wgpu creating the texture of a 1058×730
  picture in the frame (20 ms; under 3 ms in the other holds). The first scroll of the 36-picture
  and the 127,000-character messages after start-up stalled for 0.5–1.9 s in the earlier checks,
  which also wrote from the window's thread: in 14 first scrolls since (two with AccessKit turned on
  by UI Automation) no frame took over 14 ms, the wait for the display's refresh included, and the
  window always answered within 10 ms. Open: a big picture's texture made in the frame (egui-wgpu's
  `update_texture`, the Craft apps' fork); and the session's own documents (the rules, the snoozes,
  the marks owed to originals), still written and flushed to the disk on the window's thread when
  one changes (the holds changed none).
- **A data file read on handles of its own:** on Windows every read and write through one handle
  waits for the one before it, so the search index's eight readers, the marks' commits and
  their flushes and the window's own reads all queued on the data file's one handle: building
  the index of the 41 GB file took 110 s with eight readers, 119 s with sixteen and 164 s with
  two. A data file's database now reads through ten handles of its own, each thread through its
  own while it's free (so the system reads ahead of a thread reading the file through), and
  writes, flushes and locks through its own as before; its lock against another copy of
  MailCraft starts at its lock bytes rather than at the file's start, where it held back every
  other handle's reads. The index of the 41 GB file now builds in 55 s (the readers spend 188 s
  reading, 543 s before), at most 156 MB above the window's own, with no frame over 5 ms; the
  check after a kill takes as long as before (86–95 s, 90 s with the old reads).
- **The search index's pass in less memory, out of a read mark's way:** where the pass's
  readers spend their time was measured on a copy of the 41 GB file's data folder
  (`index_bench`, headless): of 165–180 s reading sources on eight readers, 63–75 s at the disk
  and 100–105 s uncompressing, 0.1 s on the data file's lock and read transactions; read in the
  data file's order instead of newest first it took as long (29.4 s against 29.3 s), so the
  newest are still read first. A source kept compressed in pieces was put together compressed,
  then uncompressed into a second buffer, so a big one was held one and three-quarter times over;
  it's now uncompressed as its pieces are read: the pass's peak went from 125 to 93 MB above the
  process before it, and the pass from 37 to 29 s. A read mark is written under the lists' lock
  and through the data file's writer, which the pass takes for each folder's list it puts right
  and each of its writes, one after another without handing either on: it now lets the marks
  waiting go first before each, and puts the lists right at most every 5 s (835 list writes in
  the first pass over the file, 177 now). In that first pass a mark-read took 197 ms at p99 and
  774 ms at most; now 24 and 73 ms. In the app, on a fresh copy with the 10,941-message folder
  open: the pass took 33.3 s (52.2 s before), its peak working set was 470–491 MB (506–530 MB
  before) against 400 MB for the app with its index built, and no frame took over 3.6 ms. With
  the open message marked read and unread from the ribbon every quarter second, a mark took
  6 ms by median and 97–124 ms at p99 from the window to the disk (188 ms before), waiting for
  none of the pass's writes: the rest is the mark's own commit (Open #9).
- **Attachments a build before 0.32 kept raw:** a data file those builds made keeps most base64
  attachments uncompressed (34.9 GiB of a 47 GB file). The store can now compress them again in
  the background (`Store::recompress_data_file`): a batch of at most 2 MiB at a time, each
  compressed before it takes the file, stopped between any two and gone on with later (a kill
  loses nothing), a message written again meanwhile left as written, one moved behind it
  compressed too, and an account removed meanwhile left removed. On a made-up file the base64
  came down to 0.76 of its size; beside it, a whole mark-read from the window took 9 to 12 ms at
  p99 among 90 KB messages. An attachment over 2 MiB is a batch of its own, so a mark-read may wait
  for one's write: 0.1 to 0.18 s at most for one of 60 MiB (p99 24 to 100 ms with a few of 16
  MiB and one of 60 MiB); those over 64 MiB are left raw and counted. The app runs it as the first part of
  compacting the file in place (below). On a data file made like the 47 GB one from a copy of the 37 GB data
  file (15,430 files kept raw, 35,035 MiB, where the app's file had 15,100 and 34,897 MiB), a
  recompress took 231 s (304 s in a second run), read 45.7 GiB and wrote
  30.4 GiB with 118 MiB at most in memory, and kept 9,104 MiB less live. Its health had said
  13.1 GiB would be saved, where 8.8 GiB was: the sample overstates what compressing again gives.
  Killed twice on that file (a third of the way, and about four fifths after going on), it lost
  nothing: every message read back the same, after a whole check of the file on opening (89 to
  107 s), as its commits aren't sealed. The app never recompresses a file that big in place: it
  copies it instead (below), and a mark-read during that copy was measured there.
- **What compacting a big data file costs, measured** (`packed/tests/compaction.rs`, ignored tests
  run on copies, release build, NVMe drive reading 3.6 to 4.6 GiB/s and writing 1.5 to 1.6 GiB/s
  past the cache). Each run starts cold: after the file's health is read, the file is replaced by a
  copy of itself made past the system's cache. That file, compressed again (44,817 MiB with 8,490
  MiB to give back by its health), compacted in place by redb in 125 s to 35,685 MiB: it read 1.9
  times the file (83 GiB), moved 8.7 GiB, held 134 MiB at most and shrank only 3 times (the first at
  61 s); an earlier run of the same, not made cold, took 182 s (other work on the machine, likely,
  but not shown). Compacted again, cold, with nothing to give back, it still took 63 s and read the
  whole file once: a compaction reads all of it however little it moves. From the two cold runs it
  costs about 1.8 s for every GiB looked at plus 5.3 s for every GiB moved, five to six times what
  the drive's rates alone give; what the health says is to give back went at 68 MiB/s, not the 300
  MB/s the design assumed, and a cold compaction of a file this big never fits 15 s at idle nor 30 s
  on close. Copied cold into a new file instead (in key order, batches as a background writer makes
  them), the same file took 127 s to 35,686 MiB, read and wrote 43 GiB with 88 MiB at most in
  memory: no faster than in place, and it needs the room for the copy (35 GiB) free beside it. Every
  file read back the same after the recompress and both compactions. These numbers decide how
  compacting is offered (below).
- **Compacting, run by the app:** a data file is now compacted by MailCraft (`dataFile.compact`,
  `dataFile.idle`): in place when the part that has the file to itself is estimated, from the
  measured cost and how this file's last compactions went, to fit how it was asked (15 s at idle,
  30 s on closing, any length by hand), compressing again first what an earlier build kept raw; a
  file kept raw that's too big for that is copied into a new file in the background when its drive
  has room for the copy, and importing it again is suggested when it hasn't. Left alone ten minutes,
  MailCraft measures each data file at most once a day and compacts those with a gigabyte and a
  fifth of the file to give back (or a gigabyte to save by compressing again); a recompress stopped
  part way goes on at the next idle. While the file is had to itself the account waits: its jobs,
  its events and commands on it, a search that reads it, and a removal; the window is never kept
  waiting for it (what it reads of the file is refused at once), and no import runs beside a
  compaction. A copy keeps every file or isn't made: one with damaged files is refused. On a made-up file of deleted mail the space came
  back to the drive with every file reading back the same; a copy took in what was written while
  it ran. On the 41 GB file's data files, in the app: a copy of its import with two thirds of
  its files kept raw (47 GB, made as arc 57 made it; the import itself had nothing to give
  back) was copied by **Compact now** in 106 s (2.3 s of it with the file to itself) to 37.48 GB, every message
  reading back the same; killed at three and at seven tenths of a copy, the part copied was
  removed on opening, in about a second and with no check. The 37.8 GB import with two folders
  deleted (13.4 GB to give back) was compacted in place; killed at three tenths, the file was
  already down to 25.7 GB and opened after a 38 s check, every file the same. Meanwhile the window
  answered within 39 ms, a mark-read within 14 ms, and left alone it began a compaction (a copy)
  613 s after starting, which a move of the mouse stopped within 2 s, the copy discarded. Not run
  on them: compacting on close within 30 s (a file this big never fits) and a drive without the
  room for a copy.
- **Compacting, in the window:** File › Info › Mailbox settings says, beside what an account keeps
  on this computer, how much its data file could give back (measured once a visit, from 64 MB) and
  has **Compact now**; afterwards it says what that came to (before → after, what was reclaimed
  and how long it took), or why nothing was done: too long for idle, stopped, no room for a copy
  (importing the file again is suggested), or nothing to give back. A compaction shows its part
  (measuring, compressing again, copying, finishing with about how long is left) with a bar, in a
  dialog that can be hidden and in the status bar, where it goes before the search index; the
  part that has the file to itself can't be stopped or put away, and is shown over the window
  whatever began it. The window tells the idle policy every 30 s how long it has been left alone,
  and anything the user does, in the main window or a message's own window, stops an idle
  compaction where it can stop. That tick only runs while the window is drawn: minimized,
  MailCraft draws nothing, so no idle compaction starts until it's shown again. **Compact data
  files on close** (File › Options › Data files) is off unless chosen: on, closing compacts each
  data file last measured as worth it when that takes 30 s at most, waiting for all of them, with
  Close now, and then offers a longer one by hand, with its estimate (or that it's copied, which
  takes longer), or closing without it. Closing while a compaction has its file to itself waits
  for it. Checked on a 37.8 GB data file with 13.4 GB to give back, **Compact, then close** in
  that offer once closed MailCraft at once with nothing compacted (the answer looked for the
  account in the dialog, already put away when it ran); the answer now carries the account, and
  on that file (with its two largest folders deleted, 13,801 messages, 13.2 GB to give back) it compacted to 23.98 GB
  and then closed, 104 s after the click, opening again in under a second with no check.
- **Down in a big folder of a data file:** a read mark on a `.pst` account's message saved the
  page of 4,096 summaries it's in (about 2 MB) and then the folder counts, two durable commits
  of the 38 GB data file in the window's frame: 24 ms by median, up to 1.4 s when the disk was
  slow, every Down that landed on an unread message. A list kept in pages now keeps the flags
  marked since its pages were written beside them (512 at most), and the marks and counts go in
  one commit, written on a thread of the store's own in the order they were made: still saved at
  once, but not while the window waits. What reads a folder's list from disk, a move included,
  and closing wait for the marks handed over before, so a message marked read and deleted at once
  keeps its mark. In a made-up folder of 10,941 messages a mark writes under 64 KB and nothing on
  the window's thread.
- **A search step that held the window:** typing a query into the search box on the 41 GB file
  once held one frame for 2.9 s inside a search's step, though a step runs its units for 6 ms a
  frame; it wasn't seen again in three tries, nor in 60 queries typed a character at a time
  since (the window answered within 100 ms throughout, 4 of 14,033 samples over 50 ms). Each
  part of a step now says what it was as it ends (a page of a folder's list, an index opened, a
  word looked up, summaries checked, a kept text checked, the list put together, what's found
  shown), and the window logs any that takes over 50 ms with its kind, so the next one is named.
  A folder's list isn't read while read marks wait to be written (the read would wait for them
  on the window's thread): the step goes on and reads it once they're on disk.
- **Memory while the search index is built:** building the index of the 41 GB file's 34,691
  messages took the app from about 395 MB to 784 MB. Eight reader threads each read a source
  whole (up to 33 MB) with 64 texts waiting behind them, and the index's writer held 32 MB of
  words, counted, before writing them, which took several times that. The readers now share
  48 MB for the sources being read and the texts taken out of them until they're added (a bigger
  source is read alone), and a pass's writer writes at a quarter of what an import's holds: the
  build peaks at 563 MB (170 MB above the window's own, against the 150 MB aimed for) in 110 s,
  as before (40 to 90 s is aimed for; the readers spend most of it waiting for the data file).
- **The invitation card read every message twice:** each message opened spent 20 to 37 ms of
  its first frame in the reading pane's header, 149 ms for a 33 MB message with 36 pictures and
  104 to 117 ms for one of 127,655 characters, though their bodies are parsed off the window's
  thread: the meeting card read the message's source again and parsed all of it on the window's
  thread to look for a calendar. It now looks in the message as parsed to be shown; one without a
  calendar is neither read nor parsed again, and while a big message's files are still coming,
  the card waits for them.
- **Words after a dot:** nine messages of a 41 GB file hold a word only as "xx.word", and the word
  found none of them: UAX #29 keeps letters joined by `.`, `:`, an apostrophe or `_` as one word,
  and only that was indexed. Their parts of two letters or more are now indexed and matched as
  words too ("site.word" gives "site" and "word", "l'avion" "avion", "shot_list_v2" "shot" and
  "list"; "10,000" isn't split); a phrase isn't found across the parts kept after a summary's
  words. The index is made again from the texts it keeps, reading no message. On that file the
  20 measured queries now match a ground truth that splits words the same way, all 9 found.
- **Searches said "still indexing" with the index complete:** a message of pictures and files
  only, read whole, was taken as unknown for any word, so `-word` missed 658 of them on a 41 GB
  file (31,930 of 32,588) and every word search said "Some results may be missing: still
  indexing" (658 to 1,031 counted). Such a message now has no word. A message too long to index
  whole (past 1,000,000 characters, or parts left unread) is still unknown past its cut, but its
  kept text, which its words were indexed from, isn't read again for every word searched: it's
  counted apart, and the list says "Some long messages were searched only in part" instead, more
  quietly. `-word` now finds 32,574, matching that ground truth.
- **Memory of a `.pst` import that indexes as it goes:** the app held 1.39–1.42 GB at its peak
  importing the 41 GB file (1.20 GB by the benchmark, 0.76 GB without indexing). The search
  index's writer holds the words and texts of the messages it adds until it writes them, and
  counted them at a sixth of what they took: each message's kept text was given 64 KiB of room
  more than its length (the room left for files' names, not the room they took), and a new word's
  place in the map and its list's spare room weren't counted. A kept text now takes the room its
  bytes need, and the words are counted as the heap holds them (checked against a counting
  allocator: 0.93–1.04 times, from 5.6), so the writer writes at what it says it holds, for an
  import as for a pass. By the benchmark the import now peaks at 843 MB, 73 MB over the same
  import without indexing (770 MB; `MC_PEAK_MB` now fails a run over a limit), in 150 s (175 s
  before). Held to a quarter, as a pass's writer is, it peaked at 790 MB in 146 s; it isn't,
  as fewer, bigger segments are written for the searches after. In the app, an import closed
  at 31% and gone on with after MailCraft opened again peaked at 1.0 GB and 1.08 GB (1.39 and
  1.42 GB before) in 144.5 s of import time for a 37.75 GB data file, and the 20 searches after
  it found what they found before. That is still over the 789 MB an import took before it
  indexed as it went, three times slower: the import alone, without indexing, peaks at 770 MB
  by the benchmark, and the window adds to it.
- **The time left after an import goes on:** an import stopped and continued, or gone on with
  after MailCraft was closed, said "Less than a minute left" from 36% with about 126 s left, and
  from 56% with about 86 s. The time left goes by the rate of the run, which counted the work of
  the runs before as done in its first second. The import now says where its run began and the
  time the runs before took, so the rate is this run's own, and the details' time so far counts
  every run. On the 41 GB file, gone on with from 31% after a close, it said "About 2 min" from
  55% to 60% (67 to 58 s left) and "Less than a minute" from 64% (54 s left).
- **What the lists in memory take, and sharing them:** `Summary::heap_bytes` counts what a
  summary holds besides its own 360 bytes (its strings and lists by capacity, rounded to the
  allocator's 16), and `bench::big_mailbox` counts it by part, in the list, the folders cached,
  the list kept while a search shows and the lists searches replaced (let go of a slice a step
  and a tick), with the biggest folder open, after 500 Downs (each message opened, then the
  largest, 117 KB; their read marks undone after, so the searches that follow find what the
  mailbox was made with) and with searches shown, beside the process's private bytes. On the
  made-up mailbox of 50,000 messages the 62,000 summaries held with the Inbox's 12,000 listed
  take 49.4 MB of the process's 143 MB, the same after the walk (a Down 0.02 ms in the middle,
  1 ms at most): 21.3 MB the summaries themselves, 14.4 MB addresses, 7.6 MB previews. Sharing
  whole summaries between the folders kept and the list (`Arc<Summary>`) would save 6.6 MB of it
  (13%); interning keys' account and folder 2.8 MB (6%) and addresses (sender, To and Cc) 6.6 MB
  more (13%), 19% in all, To being one address here (the user's own, as mostly in real mail).
  With a search of 35,006 shown (97,006 summaries, 73.5 MB, the process 298 MB) sharing would
  save 30.4 MB (41%), interning 14.8 MB (20%). Interning isn't done: it saves less than the 20%
  it had to with a folder open and reaches it only while a big search is shown, for a change to
  every address and key the engine, store and sync build. Sharing is done since (**Searches
  that find tens of thousands, without a copy of each**, below), and measured as estimated: the
  summaries held take 42.8 MB with the Inbox open (6.6 MB less, 13%) and 43.1 MB with the
  35,006 shown (30.4 MB less, 41%), the process 129 MB and 232 MB (143 and 298 MB before);
  interning would now save keys 2.3 MB (5%) and addresses 5.3 MB (12%) more. Of the 155 MB the
  process grew by from the folder view to that search before, 24 MB was summaries and 8.4 MB
  the keys of what the searches before found (100,284), still to be let go of because the bench
  doesn't tick (gone after the ticks the app would run: 288 MB); those keys are gone too (a
  search's sets hold no strings), and the rest is the index built in between and the search's
  own.
- **One writer an account's search index, across processes:** an account's index had one writer
  at a time within a process only. A second copy of MailCraft on the same data folder (the CLI
  and the app, or a second store on it) opened a writer of its own, whose clean-up removed the
  first's files not yet committed; their metas then fought, the index was found damaged and made
  again. A writer of the folder layout's or a data file's index now also holds a lease, the
  system's lock on `.locks/search-<account>.lock` in the data folder, let go however its process
  ends (a killed holder's is free a moment later); the second is told the index is busy. Its
  indexer tries again every 5 seconds and says so once in the log, its searches read the index as
  the holder's commits leave it, and an index it finds damaged is left to the holder to set
  aside. Where the file system can't lock, the log says so and only the guard within a process
  applies. Tested with two stores on one folder, a second process holding the index and then
  killed (its committed docs read, what it left half written removed), and a lease file left
  behind.
- **Mail added to data files Outlook made:** Outlook encodes its data files (permute, unless
  told otherwise) and the writer added only to unencoded ones, those MailCraft had made, so an
  export into a file Outlook made failed, saying it wasn't in Outlook's current format (it was).
  The writer is now built from MailCraft's fork of it (`craft/rw-1.2.14` of the reader's
  repository, its PATCHES.md listing the fixes) and adds to permute- and cyclic-encoded files,
  writing the new data blocks encoded the same way. It also carries the reader's fixes, and reads
  a file it adds to as the reader does, within the reader's 256 MB limit: a damaged one is an
  error rather than a stack overflow, an assert, a walk that never ends or gigabytes built from a
  few blocks. New blocks go where the file's allocation map marks space free only when every map
  page reads (its CRC too) and none of that space holds a live block or page; otherwise they go at
  the end of the file, where a damaged map that still said it was valid had them written over
  live mail. Fuzzed by `pst_append`: a damaged file added to, every message that read before the
  append reading the same after it (40,000 random cases, none lost). The export replaces the file
  it added to only once the new one is whole (renamed over it), and if that fails it says the
  mail is in the `.pst.part` file. A file in
  Outlook 97–2002's ANSI format now says so, and one that can't be read says that. On a copy of
  the 41 GB file (permute-encoded), 240 messages from the sample mailbox went in in 128 s, the
  export all told (the 34,691 messages' ids read, the file copied, eight folders committed), the
  file not growing (the writer fills space its allocation map marks free; the map passes the
  check above, its 507,372 free ranges checked in under a second), then 4,332 more in
  98 s; the copy read back with 34,931 messages and then 39,263, every one readable, as all
  34,691 were before, its attachment and recipient rows up by the ones added.
- **A crash during a long move opens at once:** a local move of more than a batch of messages now
  writes its batches without waiting for the disk and seals the data file each time it does wait
  (every 256 MB, and for its last batch, lists and counts), and seals a file written since its last
  seal as it begins (asked with the file's writer held, so a background commit that began before the
  move and wasn't sealed is sealed after it). Before, each batch waited for the disk unsealed, so a
  crash in the middle needed the whole file checked: killed 4–6 s into a move of 10,941 messages out
  of the 37 GB copy's biggest folder, MailCraft's window came at once but the check took 98–105 s.
  Killed 4, 60 and 120 s into the same move, it now opens in 0.7–1.2 s with no check. The move took
  164 and 175 s before and 164 and 138 s after: two runs each, the disk's cache swaying them more
  than the change, so whether sealing adds 2% or less to a move isn't shown by them. Its commits
  that waited for the disk went from 855 unsealed (18.6–23.1 s in all) to 52–53 sealed (9.0–11.7 s),
  a seal by itself costing 12–19 ms by median and 23 ms at most on that file. Each now writes out up
  to 256 MB, so the longest took 1.2–2.3 s against 0.5–0.8 s: the 60 ms limit for a write on the
  window's thread isn't met by a long move, before or after, as a local move runs on that thread and
  waits for the disk there (sealing more often would make each wait shorter but not short enough,
  and the move longer); only moving it off the window's thread would. An import's commits were
  sealed already: killed 90 s in, and again 60 s into the run that went on, the file opened in 0.7 s
  with no check, as before. Not yet: a crash within two seconds of an ordinary write (a read mark, a
  short move) still checks the whole file (83 s after a kill 3 s into a session of read marks);
  emptying or deleting a big folder writes a commit per message, each unsealed.
- **How far the check of a data file is, in percent:** the card shown while a data file not
  closed properly is checked says how far it is ("42% checked") under its bar. The database says
  so only as each of its passes over the file begins (0, 60 and 90%), so the bar stood still for
  most of a check of minutes; now it moves on by the share of the file each pass has read. On the
  37 GB copy it counted from 1% to 89% over the 83 s of a check, a point every 0.7 s through the
  first pass and every 1.4 s through the second (the database gives them 60 and 30 points), and
  never goes back.
- **A search just after the window opens:** a word's first search in a session folds each summary's
  subject, senders, recipients and preview as it checks them, and that was almost all of it: over
  the made-up 50,000 messages 152–182 ms all told, p99 3.3–5.0 ms a step against 4 ms, the step that
  opened the index often the slowest. A summary's text is now made into one string as its words are
  read, rather than a string for each part, word and piece: an ASCII text is split into words a byte
  at a time by what UAX #29 says of ASCII, an ASCII word only lower-cased, the parts of joined words
  split out only where a word has a joining character, and the addresses folded a name and an
  address at a time; byte for byte the same text, which proptests hold against the old way and
  against unicode-segmentation (about 4x as fast on plain summaries, 1.8x on random Unicode ones).
  The window's ticks open each account's index before anyone searches, one a tick. A word now takes
  52–77 ms all told and p99 2.0–2.8 ms a step, its worst step the same, searched alone or after the
  common phrase and the domain in the same process (runs 40–60% slower after them were the
  machine's other builds, which slow every query alike). The tick that opens the index takes 2–4 ms
  there, and the index stays open, in memory, whether or not anyone searches. A one-letter word the
  first time took 383–401 ms, most of it decoding the same lexicon page again for each unit that
  looked through it (177 ms of 681 units for `k`): the reader keeps the page it read last decoded,
  and `k` takes 119–227 ms. The bench fails if a word in a session just opened takes over 90 ms or
  4 ms a step at p99, or `k` over 300 ms.
- **A folder layout in use isn't brought into data files, nor shared by two windows:** opening
  data files brings an account's folder from before into its data file and then removes the
  folder. The CLI always opens data files, so run beside the app on the folder layout (the sample
  mailbox, or the app's fallback when its data files can't open) it moved the folders the app was
  writing, its search index among them, and removed them; and two app windows on one folder
  layout, where the single-instance check can't keep the second out (across Windows sessions or
  displays, or when its socket can't be made), each wrote the folder states and `ops.json` whole,
  so one's change could be lost to the other's. Every store on the folder layout now holds a
  lease for its life, `.locks/folders.lock` in the data folder (let go with its last clone, or
  however its process ends): shared by stores that only read and write it (tests, tools), alone
  by the app's. The bring-in takes it alone first: while another store has it, nothing is
  brought in or removed, and the error says the mail "is open in another copy of MailCraft". The
  CLI adds "close MailCraft first"; the window says its mail couldn't be moved and opens the
  folder layout, as for any bring-in that fails; a second window on a folder layout the first has
  runs without its cache, as one does when the first has its data files open. That closes the
  gap of one writer across processes (Open, 8 before). Tested in one process and across two (the
  other killed, after which the folders come in within the retry), with the folders' files
  compared byte for byte, the app's lease against a second store of either kind, the CLI's
  message, and `.locks` left out of data files and accounts.
- **Plain text out of the way:** "Show as plain text" was a link right above an HTML message's
  text, where it was clicked by mistake, and it turned every message plain. It's now where
  Outlook keeps the same thing: File › Options › Reading has "Read all mail as plain text" (the
  setting it always was, said the way round it works), as does View › Message › Plain text. One
  message is switched by itself, the setting left as it is: a check in its More actions (⋯), in
  the reading pane and in the toolbar of a message's own window, and, above a message shown as
  text, "Shown as text." with Show formatted (as Outlook shows one message as HTML while all mail
  is read as plain text). Those choices last while MailCraft is open, and are forgotten when the
  setting changes. The command (`view.html`), its use from the CLI and `--cmd`, and the setting
  are as they were. Headless tests find no link over the text; switch one message from More
  actions and Show formatted with the setting unchanged, on and off; switch the setting from the
  View ribbon and the Options switch; and switch a message in its own window from its toolbar.
- **A `.pst` import that keeps its writer busy:** on the 41 GB file the import's writer stood
  idle 40 to 44% of the time, waiting for one big item (20 to 40 MB) at a time: a lane whose
  charge didn't fit the byte budget waited while the lanes after it took the room as it came
  back, so the big item went only once the writer needed it, alone. Room is now given in the
  items' order (an item charged more than the whole budget only waits for its turn and holds no
  one up); a block read within 4 MiB of the last block read reads the 64 KiB chunk around it, as
  a big attachment's blocks lie close together in no order (1.04 to 1.09 million reads, against
  1.97 million, for 61.0 to 61.7 GB against 56.6 GB); the search index is written on a thread of its own,
  handed each batch's words once the batch is written; and the budget is 288 MiB (384 MB
  before), the reader's handles reopened once their pages are thought to hold 4 MiB (16 MiB
  before). Measured on a machine shared with other builds: by the benchmark the import took
  130 to 179 s (173 to 214 s before) with the writer 78 to 84% busy (56 to 61% before), and 143 to
  144 s without the index, so indexing as it goes adds no time beyond the runs' spread; the
  data file is the same (same digests of every message, folder and list, and the same kept
  bytes as before). In the app, four imports took 111 to 152 s with the writer 84 to 90% busy,
  peaking at 818 to 860 MB of working set (958 to 994 MB of private bytes), growing by 40 to
  170 MB from a fifth of the way to nine tenths. Not met: the writer 85% busy in every run, a
  peak within 850 MB in every run and flat, and reading the items within 160 s (the lanes spend
  515 to 690 s reading, summed over 12 of them that wait on the disk at once).
- **Searches that find tens of thousands, without a copy of each:** every summary a search found
  was copied into its finds and again into the list shown, its key's two strings into each set
  it went into, and each key hashed whole (SipHash, 130–150 ns) as the list took it: the frame
  that showed 35,000–47,000 found took 10–16 ms against the 8 ms of a frame, `is:unread` 20–43
  ms against 15 ms, and a one-letter word's steps 8–13 ms at p99 against 6 ms. The folders'
  lists in memory, the list shown, the list kept while a search shows and what a search found
  now share each summary (`SharedSummary`, an `Arc<Summary>`; on disk it's the summary itself,
  byte for byte), copied only when one changes while another list holds it, a flag changed on a
  result then giving the list the folder's own again; a search's sets hold a message by its
  folder's place, validity and number (`RunKey`), nothing on the heap; a key's hash mixes its
  folder's, made once a search, with its number (the list's index hashes the rest about 7x as
  fast); the list's date headings are worked out a day at a time; and the list a run shows at
  its end is put together as a step's first unit, not after others late in a frame. Measured
  (release, `search_big::how_long_searches_take`, three runs and each query alone, all now
  budgets it holds): `is:unread` 4–7 ms all told; a one-letter word warm 20–25 ms all told, its
  steps 3.0–3.6 ms at p99 and at worst; every frame of `is:read`, `is:unread`, the domain and
  the one-letter word, warm or just opened, at most 6.5 ms. The memory it saves is in **What the
  lists in memory take, and sharing them**, above.
- **Search results as soon as the typing pauses:** the search box runs its search a quarter of a
  second after the last key, through a command that takes only the search's first few units
  after the frame was drawn; the frames after step the rest. That frame asked for its next in
  20 s, so on the 41 GB file the results waited for something else to wake the window (its
  settings tick, every 5 s): from setting the query to the first frame searching took 2.4 s at
  the median and 5.3 s at most, and in 9 of 76 queries the list still showed the folder 4 s
  later, though the search itself takes 3–24 ms. A frame that runs a search command, or has a
  search still under way, now asks for the next at once. A headless test runs the window's
  frames only when it asks for them, as the desktop app does, and finds the results of a word
  over 50,000 messages drawn 0.25 s after the key (20.25 s before). On the 41 GB file, the 20
  queries set whole twice in each of two sessions: from setting the query to the first frame
  searching 0.25–0.61 s (0.26 s at the median: the quarter-second pause), no query left the
  folder shown, and every count the window showed was the CLI's.
- **The time left from a tenth done:** the rate the time left goes by was seeded with the run's
  average at its first second, which held the walk for the totals (1.9 s of a real file's
  import), and the moving average (30 s) had come only half way from it to the real rate when
  first shown, 20 s in: "About 4 min left" with 96 s left. The rate is now timed from the run's
  first progress, the plain average since then until a time constant has passed and the moving
  average after. Replayed over nine imports of the 41 GB file, the worst from a tenth done went
  from 41–155% off (85% at the median) to 24–66% (36%); from half done it stays within the runs'
  spread (1–44% off against 4–42%). A test with a walk of 1, 2 and 5 s, then a steady rate, is
  within 20% from the first time said (53% off before). In the window, a fresh import of the
  file (112 s, 1.7 s of it the walk) first said "About 2 min left" at 13% done with 90 s left, and
  each time said after was within the minute it named (4% off at worst from a tenth, against
  87–89%). Better, but the 20% from a tenth done is not met: that one run was within it, the
  nine replays all over it (open, #10).
- **Holding Down, timed by part:** five ten-second holds of Down on the 41 GB file's data file
  (6,496 frames) had 11 frames over 16 ms and none over 31 ms. Five were the reading pane's first
  frame of a message with attachments or pictures (17–24 ms), one the list's (16 ms), and the
  others went in the rest of the frame, outside the ribbon, the list and the reading pane. Not
  fixed yet: which of a message's attachment chips or pictures costs that first frame isn't
  known. Frames that seemed slower came from the measuring: a probe line appended to a file from
  the window's thread took 20–270 ms now and then.
- **Holding Down: a video's first frame:** timed again by part with probes that only hand their
  lines to a thread of their own, the one frame over 16 ms left in holds of Down on the 41 GB
  file's data file was the first of a message with eight attachments, one a 1.7 MB video: its chip
  read the message (6.5 ms), took the video out of it (5.1 ms) and wrote it to a file for the
  platform's decoder (0.7 ms), all in that frame (16.9 ms), and the frame the platform made was
  decoded in a frame too. Both now go to the pictures lane (`reading/videos.rs`): the chip shows
  its kind's icon meanwhile, and work for a message left is cancelled. A test hands the lane's work
  to itself and finds no file written and nothing decoded in a frame (both were before), the
  thumbnail shown once the work ran, and the work of a message left doing nothing.
- **Searching: the frame of the search command:** timed by part on the same data file, no frame
  of the 20 queries set whole, typed a character at a time or cleared went over 13.5 ms but those
  that ran the search command, after the frame was drawn: its four units, bounded by pages and
  summaries rather than time, took 16.1 ms once (a word's look-up read cold, 10.7 ms of it) and
  19.2 ms for the first search after a fresh index pass. In the window the command now takes
  none of the search, and the next frame, asked for at once, steps it within its 6 ms share (the
  list drawn in the frame of the command is the one before it, so nothing shows empty
  meanwhile); a test finds no unit taken by the command and the search done by the frames after.
  Clearing a search after a result was marked read or unread, which builds the folder's list of
  10,941 again rather than showing the one kept, took 2–5 ms. The 357 ms frame of round two's
  first search after an index pass and its 177 ms frame at a clear weren't seen again in three
  sessions on fresh copies, the first search right after the pass or after holds of Down: those
  probes appended a line to a file from the window's thread at the search command and at every
  step, which alone made frames of 20–270 ms.
- **Holding Down and searching, every frame under 16 ms:** measured again on fresh copies of the
  41 GB file's data file (release build, the probes handing their lines to a thread of their own),
  from the first index pass on: five ten-second holds of Down (7,352 frames) had none over 6.9 ms
  (3.2 ms at p99); the 20 queries set whole, each with a result opened, marked and the search
  cleared (3,687 frames), none over 10.7 ms (a long message's first frame, its formatted text laid
  out); and five prefixes typed a character at a time right after the pass (288 frames, and 176
  as each was cleared), none over 9.9 ms, the search command 2.9 ms at most. Besides the window's
  first frame, one frame went over 16 ms in all these sessions: the first message opened in a
  session started on a fresh copy, once (16.2 ms, 11.3 ms of it in the reading pane before the
  message was ready to show), not seen in the sessions after, on fresh copies or not. Its row in
  Open is closed.
- **Attached messages as deep as they go:** a `.pst` item's attached messages came in three
  levels deep, and a fourth was counted as too deep. The cap is now 32 (`pst.rs` `MAX_NESTING`), a
  guard against a damaged or hostile file that no real one comes near: the whole item stays
  bounded by its 1,000 attachments of attached messages, its 128 MB and the search-key check that
  stops a loop. Each level is an `.eml` sent as it is (7bit or 8bit) inside the one holding it,
  which mail-parser reads without recursing (it recurses only into an attached message sent in
  base64 or quoted-printable, three deep, and the import never sends one so); the reading pane
  opens each on its own, and the rebuild stops at the same depth whatever item it's handed. Search
  reads the text of the first four (`index_text`'s depth of 8, counting each one's multipart):
  deeper ones are shown, not searched. Tested on made-up chains six and forty deep, each level
  parsed and opened, a rebuild handed one a hundred deep, and a message attached to itself.
- **Recalls, sharing invitations and contacts in a mail folder:** a recall (`IPM.Outlook.Recall`,
  and its report), a sharing invitation (`IPM.Sharing`) and a task request in a `.pst`'s mail
  folder were counted as items that couldn't be used, and a contact there too. The first three
  now come in as mail, with their subject, text and sender; a sharing invitation's text begins
  with what it shares, read from its sharing properties (MS-OXSHARE: the folder's kind and name,
  who shares it, an internet calendar's address), in its HTML too, so it says so whatever
  Outlook's own text says. A contact, contact group, calendar item, task, note or journal entry
  found in a mail folder goes to the address book, calendar, tasks or notes as one in its own
  folder does, except in Deleted Items (the store's `PidTagIpmWastebasketEntryId`) and the folders
  under it: Outlook moves what's deleted there, and a deleted meeting, contact, task or note isn't
  brought back to life; it is counted, as before. Tested on made-up items of each kind, on a file
  the writer makes with a contact in the Inbox and a contact, an appointment, a task and a note in
  Deleted Items, and on sharing properties with line breaks, HTML and values that aren't
  addresses; the `pst_item` fuzz target maps its properties 0x8001 to 0x8006 to the sharing ones,
  so its invitation seed, and what it's mutated to, reaches the lines and their place in the HTML;
  the whole-file seed's invitation carries them too.
- **Attachments that are links:** an attachment Outlook kept as a link to a file elsewhere (by
  reference, resolved, reference only, or on the web: PidTagAttachMethod 2, 3, 4, 7) was only
  counted, and the message showed nothing of it. When its path (PidTagAttachLongPathname, else
  PidTagAttachPathname) names a file on one of this computer's drives that can be read, within the
  room the item has left (an attachment's 100 MB, the item's 128 MB, and for a contact, calendar
  item, task or note what its 1 MB file line holds), the file comes in under the attachment's name;
  a bigger one isn't opened. Otherwise the message carries a short text file, `<name> (link).txt`,
  holding the path or address (its room taken from the item's too), so what was there is seen,
  and the report still counts it as a link. A web reference is never read, nor is a network path
  (two slashes at its start, which would reach another computer and hand it the user's sign-in),
  a path with `.` or `..` in it, one through a link or junction (which could lead to a share), a
  link itself, or on Unix the system's own files (`/proc`, `/sys`, `/dev`): a data file is hostile
  input, and a read across the network from it is not allowed. A drive letter the user mapped to
  a share is read as a local drive is: it's the user's own share, and telling it apart takes a
  system call MailCraft has no safe wrapper for. One item looks for at most 64 links' files, and
  once four reads have taken two seconds or more (a drive that is gone answers only when it times
  out) the import reads no more of them. Tested with a file in a scratch folder, one missing, one
  bigger than asked, a folder, a path through a junction, network paths and web addresses, an
  item with 5,000 links naming one file, and through the import's writer; the `pst_item` fuzz
  target has a network link and a web link among its seeds.
- **RSS feeds from a `.pst`:** Outlook's RSS Feeds folder (`IPF.Note.OutlookHomepage`) was left
  out with the folders under it counted as of a kind MailCraft has no place for. It and its
  feeds' folders now come in as mail folders of their posts (`IPM.Post.Rss`, mail already), each
  feed under RSS Feeds. Outlook's rules stay out: they aren't items of any folder but hidden ones
  of the Inbox (its associated contents, which the import doesn't read), kept in Outlook's own
  rule format (MS-OXORULE: conditions and actions as binary restrictions, many naming Exchange
  entries and folders by id), which MailCraft's rules can't hold whole; bringing in part of a
  rule would file mail differently from what it says. Tested on a file the writer makes with an
  RSS Feeds folder, a feed's folder of two posts, and a recall, a sharing invitation and a contact
  in a mail folder, read by the import itself.
- **A note's files:** a note or journal entry in a `.pst` that carried files came in without
  them: MailCraft's notes had nowhere to keep one. A note now keeps its files as an event and a
  task keep theirs (`Note::files`: each an `ATTACH` line with its bytes inline, the same 1 MB a
  file and 8 MB in all, cards', events', tasks' and notes' together; an attached message as its
  `.eml`), and the import keeps a note's and a journal entry's. The Notes pane shows a paperclip
  and their number at a note's foot, whose menu lists each with its size, to open in another app
  (never one that could run a program) or save where asked (`note.saveFile`; in the browser,
  downloaded); the list reads names and sizes without decoding the bytes, and the notes it shows
  borrow theirs instead of being copied each frame. A note's files go back into an exported
  `.pst` as its attachments, and come back from it. Exchange's notes sync without files:
  MailCraft reads an Exchange note's text and colour, not its attachments, and a note with files
  comes only from a data file, whose account has no server. Tested on made-up notes and journal
  entries (a file, an attached message, one too big), on a file the writer makes with a note and
  its file, through an export and its import back, on the command (listed, saved, refused to be
  opened when it's a program), and on headless frames of the Notes pane with a note's file in
  light and dark.
- **An opened data file keeps its copy beside it:** File › Open a data file (and the welcome
  screen's "Open an Outlook data file…") made its copy in Documents\MailCraft Files, and the
  Outlook file was only ever read. It's still only read (the import opens it read-only, shared
  with readers only; a test opens, changes in MailCraft and closes one, then reads it again, and
  its bytes and modification time stay the same throughout), but its copy now goes next to it,
  `archive.pst` → `archive.mailcraft` (`archive (2).mailcraft` when that name is another file's,
  or the first copy's account is still open), named in `datafiles.json` by its whole path
  (`store/src/beside.rs`, `packed/beside.rs`). MailCraft never writes into an Outlook data file:
  the owner's choice, in place of writing changes back; everything changed in MailCraft is in the
  copy. A folder that can't be written (read-only media, a protected folder, a share refusing
  writes, tried by making a file of MailCraft's own beside the copy's name and removing it at
  once, so a crash between the two never takes the name) has the copy go with the other data
  files, and the dialog and a notice say where; the folder is looked at without the data files'
  names held, as a slow share would hold up every account. Opened again after its account was
  removed, the file goes on with its copy (found by its import's note, read from it read-only:
  made from a file of that name, no account's now, and writable; one that can't be written is
  passed over), under the account's own id when that's free, its
  folders and messages claimed for a new one otherwise; an import it had stopped waits as it did,
  one cut off goes on, and a run of it that fails before more comes in keeps the account and its
  copy, waiting to go on (it was removed with its file before). Import another copy makes a copy
  of its own, never one found beside the file, and a path named from where MailCraft runs (the
  CLI's) is in that folder. When the file changed since (size or time against the note), the dialog
  says so and offers Open as it was or Read the changes in (`account.importPst` `changes`
  "keep" or "read"; asked for when neither is given), and Read the changes in is offered too when
  the file's account is still open. Read again, the file goes into a store of its own beside the
  data files, then what it has new is added to the copy (`Store::merge_from`): a message neither
  in the account (in any folder) nor brought in from the file before (`import-seen.json`, added to
  as each import ends or stops part-way, so what's deleted before it goes on stays noted), into
  the folder of the same path (its list read and written under the lists' lock, so a mark made
  meanwhile stays), and contacts, calendar items, notes and tasks the same way by what they hold.
  A message the file gives no Message-ID is known by its node in the file: the one the import
  makes up for it names the account and the file's tag too, which change from one read to the
  next. An account with no such note (opened before MailCraft kept one) isn't read into, as
  everything deleted in MailCraft would come back: the dialog says so and offers the copy as it
  was, or another copy. Nothing in the copy changes otherwise: a message read,
  moved or deleted in MailCraft stays so, and a deleted one doesn't come back; an item the file
  changed comes in beside MailCraft's (a message, the same by its Message-ID, doesn't). A read
  stopped, paused or failed adds nothing, and is asked for again from the start; removing the
  account meanwhile keeps the copy. Compaction copies left beside an opened file by a crash are
  removed as the data files open, as in MailCraft Files. MailCraft doesn't open `.ost` files (it
  explains what one is): an `.ost` is a server account's offline copy, and adding the account
  shows its mail. Outlook 2007 and 2010's share the Unicode `.pst` layout (512-byte pages), so
  they're refused by that choice, the reader turning them away only by their client magic (`SO`
  for `SM`); Outlook 2013's and later need a reader of their own (4 KB pages, bigger blocks with
  their own trailers and a changed index layout, none of which MS-PST describes), the scope of a
  later arc.
- **One crate reads and writes `.pst` files:** the import read with Microsoft's `outlook-pst`
  (MailCraft's fork, `craft/1.2.0`) and the export wrote with `outlook-pst-rw` (its own fork,
  `craft/rw-1.2.14`), two copies of the same reader with the same fixes. The writer's fork already
  carried each of the reader's fixes with its test, so the import now reads with it too and
  `outlook-pst` is gone: one fork to keep, one `[patch.crates-io]` pin, one reader for what is
  read in and what is added to (`craft/1.2.0` stays, marked superseded, since earlier versions pin
  it). On the 41 GB real file, imported twice with each, the messages, folders and skip reasons
  were the same, and so was the digest of every message, folder and list read back; the import
  took 87–91 s with either, in 600–630 MB at its peak.
