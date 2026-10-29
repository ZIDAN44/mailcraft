# Roadmap

**Stage:** v0.30 — Sent where the server keeps it, mail in its column, files pasted from any file manager (2026-10-30). Each milestone below is a version: v0.1 was
the first working client, and every later one is named for what it added.
Scope: [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177) (lightweight,
local-first, media-aware v1); the fuller request is
[storytold/artcraft-services#80](https://github.com/storytold/artcraft-services/issues/80).

## Headline numbers

| | |
|---|---|
| Crates | 8 libraries + 3 apps + xtask |
| Commands | 214 (`mailcraft-cli commands`) |
| Tests | 700 (unit, in-memory IMAP, JMAP, Graph, EWS (mail, calendars, contacts, Autodiscover), CalDAV/CardDAV and ManageSieve servers, NTLM test vectors, SMTP and HTTP stubs, data files, TCP end-to-end, an X server drag, headless UI) |
| Languages | 14 (English, Spanish, French, German, Dutch, Brazilian Portuguese, Italian, Russian, Japanese, Korean, Simplified and Traditional Chinese, Arabic, Hebrew) |
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
8. **v0.8 — POP and Outlook's own data files** ✅ POP accounts with folders on this computer,
   starting from an existing `.pst`, and real Outlook `.pst` files checked end to end.
9. **v0.9 — Outlook's everyday commands** ✅ From an audit against Outlook: Arrange by, a Send /
   Receive tab with Work offline, Undo, more filters, Ignore, importance and read receipts, File's
   Save As and Print, POP copies removed after days; drafts no longer lost offline.
10. **v0.10 — Windows, folders and rules as Outlook keeps them** ✅ Remembered window places and
   sizes, subfolders under their parents, rules run at once, attachments in message windows; from
   a second audit: the Search tab, Follow Up dates, Delay delivery and Replies to, Outlook's menu
   items, hover everywhere.
11. **v0.11 — Notes, previews and Exchange beyond the mailbox** ✅ From issue #1: attachments
   previewed in the reading pane (Word, Excel and PowerPoint too), Notes, three People layouts,
   other people's Exchange calendars, public folders, Dutch; Outlook.com and Hotmail say why
   they need Microsoft sign-in.
12. **v0.12 — Microsoft sign-in that works** ✅ Microsoft sign-in tried end to end: the sign-in page
   opens in the browser on Windows, accounts that sign in that way ask to sign in again rather
   than for a password, an account Microsoft refuses says why, sent mail shows in Sent Items at
   once; dialogs and the folder pane keep their sizes.
13. **v0.13 — The open gaps, again** ✅ From an audit of [docs/gaps.md](docs/gaps.md): other
   people's Exchange calendars can be changed where they allow it, HTML mail's background
   pictures and boxes placed over others, text in its cell's colour, Outlook's notes from a data
   file, right-to-left mail copied as written; calendar boxes that fit their lines.
14. **v0.14 — What testers asked for** ✅ From issues #1 and #2: Hotmail accounts with a password
   switch to Microsoft sign-in, a mailbox Microsoft won't open over IMAP says why (or moves to
   Graph), generated HTML reports as Outlook shows them, quick search, reading and signature
   options, calendar colours, ticks, side by side and more months, List columns, public contacts,
   Exchange's outside replies, how much mail is kept.
15. **v0.15 — What testers asked for, again** ✅ From issue #1's next round: calendars side by
   side each in a pane of its own, other people's Exchange calendars editable by their editors,
   signatures and text pasted from Outlook with their look, public folder contacts on Exchange
   2013 and later, the address book to open with, and long names in it.
16. **v0.16 — Contacts as Outlook keeps them** ✅ From issue #1's third round: contacts' numbers
   by kind, their business, home and other addresses and birthday (vCard, Exchange, Graph, data
   files); signatures pasted with their sizes and typefaces; a wrong recipient gone with one
   Backspace; someone's Exchange calendar editable when its events say so; public folders routed
   on Exchange 2013 and later (listing and mail too), and People's address books to find them.
   A second connection per account for sending and automatic replies, so neither waits for a
   sync; diagnostics in mailcraft.log; French values kept away from words that elide; address
   suggestions that fit and click anywhere. Big data files and mailboxes: a 64.5 GB `.pst` of
   800,000 messages imports in 13 minutes under 850 MB of memory, and a mailbox that size opens,
   searches and changes without reading all of it. The File pages laid out as settings
   (Options, Account information, Open & Export), rules exported, imported and run all at once,
   Print asking where the PDF goes, Ctrl+A for the text or the attachments being looked at.
17. **v0.17 — Copy and paste, and public folder contacts** ✅ From issue #1's fourth round: the
   clipboard back (nothing could be pasted or copied since egui 0.37), so signatures paste from
   Outlook again; the organisation's shared contacts and calendars found in Exchange's public
   folders, and the log saying how many.
18. **v0.18 — Shared mailboxes and the contact card** ✅ From issue #1's fifth round: an
   Exchange account's shared mailboxes open under it by themselves (the ones Autodiscover names)
   or by hand (Info › Account settings › Shared mailboxes); a name in a message's header opens a
   contact card (send an email, add or edit the contact, copy the address) and its right-click
   menu has the same; Enter submits the one-field dialogs again. A removed account's data file
   stays, as Outlook keeps a removed account's `.pst`.
19. **v0.19 — Shared mailboxes of their own, and SMTP's own sign-in** ✅ From issue #1's sixth
   round and issue #3: an Exchange shared mailbox shown as a mailbox of its own, Exchange's
   folders under the names the server gives them, the conversation above a message only when
   the list is grouped by conversation, and an outgoing server signed in to with a user name
   and password of its own.
20. **v0.20 — A window on every Windows PC** ✅ Windows draws with DirectX 12 alone: a real GPU
   first, Windows' software renderer (WARP) on a PC without one, where the window never opened;
   a window that can't open says why; and the other Craft apps' fixes to shared ground that
   apply to MailCraft (GPU device limits and errors, Intel Macs, Linux's GPU, XWayland for drag
   and drop, clipboard reads that can't hang, a maximized window that doesn't blink, the DMG's
   strict signature, Windows 10 at install), and from WordCraft, PdfCraft, DeckCraft and
   GridCraft's desktop glue: winit's input-method and monitor fixes, windows back on screen,
   arguments and quitting that can't crash, DPI awareness, files for other apps kept private;
   and from issue #1, calendars told apart by account, shared mailboxes' calendars, Join and
   links in meetings, Check for updates; and from issue #5, a Microsoft 365 sign-in kept in
   Windows' Credential Manager whatever its length; and from issue #4, accounts kept online only
   (none of their mail written to the computer) and a portable mode that leaves nothing behind.
21. **v0.21 — One MailCraft at a time** ✅ Started again, MailCraft brings its window forward
   rather than opening a second one on the same data; and from issues #1 and #4, shared
   mailboxes as mailboxes of their own, invitations that read with their links in dialogs that
   fit the window, older mail shown and searched on the server (every kind of account, and in
   the browser), and online only made clearer (a password asked for at start, a warning before
   closing loses anything).
22. **v0.22 — Outlook's everyday tools, and test builds that update themselves** ✅ What a 1:1
   look at Outlook's Home ribbon and calendar found missing: reminders that pop up (snooze,
   dismiss, open), blocked and safe senders, Clean Up, Search Folders, Quick Steps
   (Ctrl+Shift+1…9), the Work Week view, Insert date & time and Word count. A build from the
   unsigned test-build workflow fetches the next test build in the background, checks its
   SHA-256 against the release, and installs it when MailCraft closes (or at once with Restart
   to update), the way that copy was installed: `.msi`, portable `.zip`, `.dmg` or
   `.AppImage`. Signed releases are unaffected.
23. **v0.23 — Mail that can't fool you, and a hardening review** ✅ Mail that keeps its
   reader safe: photos sent without where they were taken, a sender passing for someone the
   user deals with (a lookalike domain, or another address in the name) said plainly, a check
   before a message goes to a lookalike address or shows strangers each other's addresses,
   tracking pictures never loaded and links opened without their trackers, text a message hides
   brought out, and `is:awaiting` for mail still waiting for a reply. Before them, a hardening
   review area by area, each problem reproduced first: attachments that run code, pictures kept
   off the local network, IMAP answers capped, invitations changed only by their organizer,
   OAuth renewed, files written whole; calendar time zones; fuzzing and dependency checks;
   MailCraft as the system's mail app.
24. **v0.24 — Signed and encrypted mail (S/MIME)** ✅ From issue #6: a certificate imported
   from a `.p12` or `.pfx` file (its private key kept in the system's keychain), mail signed and
   encrypted as it's sent and read signed or encrypted as it comes, with who signed it and whether
   to trust that said above it; certificates learnt from signed mail to encrypt to; drafts of
   encrypted mail encrypted too; header protection (RFC 9788); EC certificates for encryption
   (ECDH); revocation checks, opt-in; certificates on smart cards (PKCS #11), in the macOS
   Keychain and in the Windows certificate store, their keys staying there; S/MIME in the
   browser, its `.p12` kept locked with its password. Checked against OpenSSL both ways, through
   a real server, against SoftHSM 2 and against Windows' CNG.
25. **v0.25 — Updates that finish** ✅ A test build's update no
   longer waits in silence for MailCraft to close: once it's downloaded and checked, a bar offers
   Restart now (and the system's notification says so when MailCraft isn't in front); closing
   asks whether to install it, and says when a download is still going. The download carries on
   where it stopped, in the same run or the next, and checks go on hourly while MailCraft stays
   open, sooner after a failure ([docs/updates.md](docs/updates.md)). On macOS and Linux an app
   in a folder the user can write updates itself, without a password.
26. **v0.26 — Renewed certificates, calendars anywhere, the classic ribbon** ✅ What issues
   #6, #7 and #8 asked for. A renewed S/MIME certificate keeps the old one, so mail encrypted to
   it still opens: the newer signs and encrypts, the older is kept for reading, and either can be
   made the one in use. Calendars and contacts are found as RFC 6764 says (the SRV record's TXT
   path, which Posteo's needs), or at an address given by hand with a password of its own (an app
   password). A classic ribbon (big buttons in named groups) is a View setting, and dark mode tells
   its panes and borders apart.
27. **v0.27 — Arabic and Hebrew mirrored, colours of your own, Exchange's directory** ✅ In Arabic and Hebrew the mail window and
   the message being written are mirrored inside the panes too (the ribbon from the right, icons
   and avatars on the right, scroll bars on the left), and egui draws right-to-left text itself:
   MailCraft builds egui and winit from the forks the Craft apps share, which also count
   right-to-left letters right. Closing a minimized window and ⌘Q ask before a message being
   written is lost. Screen readers hear what every drawn control is. A whole-window soak test
   plays the app by a random hand, and found two crashes, both fixed. The cache survives a file
   another program holds, file dialogs reopen where they were left, the System theme follows a
   Linux desktop, and the packages install on openSUSE and ship their licences. From issues #8
   and #9: an accent colour of one's own (eight, or any colour, made to read in either theme) and
   a plain top bar, edges on the ribbon, buttons and menus; an Exchange account can name the
   User-Agent its server lets in, and Exchange's directory (the Global Address List) is searched
   as names are typed, on premises and online, so someone new is found without a restart.
28. **v0.28 — Calendars from the web, full names from the directory** ✅ From issue #7: a calendar
   is subscribed to by its iCal or `webcal://` link (Outlook's Add Calendar › From Internet),
   shown read-only beside the account's own, read again every few hours (as often as it asks)
   and at once on Refresh; the CLI reads it too. From issue #9: people found in Exchange's
   directory show by their full names where the server names mailboxes by their aliases, and
   someone known only by an address takes the directory's name. An event in a read-only calendar
   can no longer be deleted. From the sibling Craft apps: a pinned MailCraft keeps its icon after
   an update on Windows, macOS's open and save panels speak the interface's language, the Linux
   packages bring `libxkbcommon-x11`, a release build without craft-fonts fails, and CI is
   leaner and grants write access only where it's needed.
29. **v0.29 — Pictures at their size, pasted files, signatures with pictures** ✅ From issue #10:
   a picture in a message being written shows at the size it's sent (it was a thumbnail 72 points
   tall), goes in at a page's width at most and on a line of its own when it's wide, and is made
   bigger or smaller by its corner handle or its right-click menu; files copied in a file manager
   are attached when pasted, and files dropped on the main window start a message with them, as in
   Outlook; HTML mail shows its pictures at the sizes it sets (a picture made twice as sharp
   showed twice as big, and the layout around it moved). From issue #11: an attachment with no
   name, or no extension, gets the one its type has (a WebP saved without one couldn't be
   opened), and a picture sent as plain bytes is still a picture; signatures take pictures, and
   the signature editor has a bar of its own, and a logo copied with a signature from Word or
   Outlook comes along when pasted; replies and forwards can open in a window of their own each
   time, and Favorites can be hidden from the folder pane. From issue #17: Reading pane › Bottom
   shows the message below the list (the list took the whole height). From issue #18: Outlook's
   `winmail.dat` gives its files, its attached messages and its formatted body (Rich Text, and the
   HTML Outlook wraps in it), and exporting to a `.pst` already there adds to it, nothing twice;
   East Asian mail reads, and replies and forwards keep the pictures they quote. From issue #10
   again: every picture of a forwarded message goes with it (in the message, written into its
   HTML, or on the web by its address). From issue #19: sent mail gets its copy on servers with
   no Sent folder (one is made) or with one named in another language; closing while a message
   is still being sent asks first. From issue #20: meeting invitations are on the calendar as
   tentative as they arrive, before they're answered (on the CalDAV server too). Also the search box's text centred, and the message
   being read kept in the list under the Unread filter. With Remo Laubacher's pull requests
   #12–#16, which the fixes above build on.
30. **v0.30 — Sent where the server keeps it, mail in its column, files pasted from any file manager**
   From issue #19 again: sent mail's copy goes in the Sent folder the server names, wherever it
   lists it. From issue #10 again: mail set in a centred column of a set width keeps it; Ctrl+V
   attaches files copied in any file manager.
31. **Next:** what's left in [docs/gaps.md](docs/gaps.md): a relay for IMAP accounts in the
   browser, bidirectional editing, Wayland drag-out, Kerberos-only Exchange, what the `.pst`
   libraries can't do yet.

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
- **2026-10-15** — **v0.8**: POP accounts (downloaded mail in folders on this computer, a copy left
  on the server), "use an existing data file", and real Outlook `.pst` files checked: open files,
  `.ost` files, RSS Feeds and the Outbox handled; each account in a `.mailcraft` data file in
  Documents\MailCraft Files.
- **2026-10-15** — **v0.9**: an audit against Outlook: Arrange by and Reverse sort, a Send / Receive
  tab with Work offline, Undo, Filter (categorized, important, to me), Ignore a conversation, New
  items, a minimizable folder pane, File › Save As / Save Attachments / Print / Exit, importance
  and read receipts, POP server copies removed after days or once deleted; drafts that can't
  reach the server are kept.
- **2026-10-15** — **v0.10**: windows open where they were (maximized too), a folder made in the
  Inbox shows under it, a new rule runs at once on the Inbox, attachments as Outlook's chips that
  open with a double-click, and notifications for mail rules file away.
- **2026-10-16** — **v0.10**: a second audit against Outlook: a Search tab, Follow Up with dates,
  Delay delivery and Replies to, Forward as attachment, Reply with meeting, Find related, Delete
  all and folder Properties, hover on every control; CJK names drawn in any interface language.
- **2026-10-17** — **v0.11**: issue #1: attachments preview in the reading pane (pictures, PDF
  pages, text, Word, Excel, PowerPoint), Notes, three People layouts, other people's Exchange
  calendars, public folders, Dutch; Outlook.com and Hotmail say why they need Microsoft sign-in.
- **2026-10-17** — **v0.12**: Microsoft sign-in tried end to end (Outlook.com over IMAP, a
  personal Microsoft account through Graph): the sign-in page opens on Windows, "Sign in again"
  instead of a password, refusals say why, Sent Items catches up at once; dialog and pane sizes.
- **2026-10-17** — **v0.13**: the gaps audited again: shared Exchange calendars editable where
  allowed (Graph and EWS), background pictures and placed boxes in HTML mail, cell colours, notes
  from `.pst` files, right-to-left text copied from a right-click; Exchange Online's public
  folders wait on Microsoft (EWS is being retired there).
- **2026-10-17** — **v0.14**: issues #1 and #2 reproduced and fixed: Microsoft sign-in's dead ends
  (passwords, "authenticated but not connected", a stuck "Connecting…"), HTML reports (pictures,
  style sheets, columns), search speed and People/Notes search, reading pane and signature
  options, notes order, calendar categories, ticks, side by side and months, List columns,
  public-folder contacts, outside replies on Exchange, mail kept offline, Open folder.
- **2026-10-18** — **v0.15**: issue #1 again, each reproduced first: side by side as Outlook shows
  it (a pane per calendar), an editor's rights on someone's Exchange calendar read as Exchange
  gives them, public folder content routed to the mailbox holding it, HTML on the clipboard pasted
  with its look and signatures kept formatted, Select Names opening with the address book chosen.
- **2026-10-19** — **v0.16**: issue #1's third round, each reproduced first: contacts' phones by
  kind, addresses and birthdays through every kind of account and the contact form, People's
  address books (an organisation's shared ones among them), pasted fonts and sizes, Backspace on a
  whole recipient, an editor told by the events' rights, public folder hierarchy and content
  routing with errors shown.
- **2026-10-19** — **v0.16**, more: a message sent, or automatic replies asked for, during a big
  mailbox's first sync went only after it (20 s and more in a test holding each folder 4 s): they
  go on a second connection now, sharing the sign-in. Diagnostics had nowhere to be written:
  `mailcraft.log` keeps them. French sentences keep filled-in values away from « de », « le »
  and other words that elide, and a test checks the catalog.
- **2026-10-19** — **v0.16**, big mailboxes, measured on generated data files (3 GB, 9.4 GB of
  800,000 messages, 64.5 GB): imports wrote a transaction per message (386/s), held every folder's
  list, parsed each message twice and stored values rounded up to powers of two (2.1x the file);
  now 6,000/s, 624 MB at most for the 9.4 GB file, a cache smaller than the file (compressed, in
  page-filling pieces), lists written a page at a time. In the app: deleting 8,000 messages took
  65 s (quadratic), one flag 4.7 s, a search 80-150 s and 3.5 GB; now 0.2 s, 0.5 s, 2 s again and
  2.5 GB, with folder lists kept in pages and only the changed ones written, a bounded cache of
  lists and of search text, and the message list's rows made once per change.
- **2026-10-19** — **v0.16**, a round of everyday fixes: Graph and EWS prefetch skips a message
  the server errors on instead of abandoning the folder; Replied/Forwarded flags are owed until
  the Outbox delivers; search accepts `from: zoe`; removing a contact needs confirmation; vCard
  2.1 quoted-printable is decoded; no taskbar highlight for new mail; permanent delete on Gmail
  moves to Trash and expunges there; prefilled reply recipients end with `; `.
- **2026-10-19** — **v0.16**, the File pages and the reading pane: Options, Account information
  and Open & Export as cards of settings, each with its control or button on the right
  (switches, choices side by side; Info's account actions no longer hidden in a tile's menu);
  Export says which account it writes, in a list, names the file after its address and offers
  `.pst` as its type; one stray stroke fixed in the Outbox/Export icon; From's menu drawn with a
  chevron (the font had no ▾). A suggestion picked with the mouse keeps the cursor in To, Cc or
  Bcc. Print asks where the PDF goes. Ctrl+A in a message's text no longer selects the whole
  list too; in the attachments it selects them all (Ctrl+click picks some), and several are
  saved together. Rules go to a file and come back (an account by its address, a missing
  category made), the Rules dialog lists them a line each, with a find box past six, and Run
  all runs every rule that's on.
- **2026-10-19** — **v0.16**, dependencies brought up to date, each changelog read and the change
  checked: egui and eframe 0.37 (ids, `scope_id`, `Role`, text alignment; its emoji fonts
  turned back on, and missing characters, ✓ ▾ or Devanagari, now found among the system's fonts),
  quick-xml 0.42 (text and attributes as `str`; a test of entities, CDATA and references), hmac
  0.13 with md-5 and md4 0.11 (the NTLM test vectors pass), redb 4.4 (same file format: a 4.3
  data file read and written by 4.4 and read back by 4.3), dirs 7 and miniz_oxide 0.9. skrifa,
  windows-registry and x11rb stay on the versions egui, ipconfig and winit use, so each is built
  once; keyring stays on 3.x, as its 4.0 asks of existing applications.
- **2026-10-19** — **v0.17**, issue #1's fourth round: nothing could be pasted (a signature from
  Outlook, or text anywhere) or copied since egui 0.37, which made the clipboard a feature of
  eframe's own, left out with its defaults; named now, with a test that it stays. And the
  organisation's shared contacts and calendars never appeared: Exchange answers each folder in
  an element of its kind (`ContactsFolder`, `CalendarFolder`…), and only `Folder` was read.
  Every kind is read, the fake Exchange answers that way (its test failed until then), and the
  log says how many public address books and calendars were found.
- **2026-10-19** — **v0.18**, issue #1's fifth round: shared mailboxes didn't show for Exchange
  and couldn't be added. Autodiscover's `AlternativeMailbox` entries (delegate and team
  mailboxes; archives left out) open by themselves, others are added by address, and each is
  listed through EWS with its `Mailbox` and the `X-AnchorMailbox` header; the fake Exchange has
  them (with no-access and by-legacy-name cases) and its test lists and syncs them. A name in a
  message's header opens a contact card, its address a new message to them, and right-click ›
  Add to contacts a filled-in contact. Enter hadn't submitted the password, shared calendar or
  shared mailbox dialog: they took the focus back before seeing the Enter that let it go.
  Removing an account deleted its `.mailcraft` file; as Outlook leaves a removed account's
  `.pst` on disk, the file now stays (closed, no one's), the same address added again gets a
  file of its own beside it, and Open & Export's picker starts in MailCraft Files to open it.
- **2026-10-20** — **v0.19**, issue #1's sixth round and issue #3, each reproduced in the app first
  (against a fake Exchange made in Dutch with a shared mailbox mapped to it): a shared mailbox
  showed as a folder of the account's own mailbox; it's a mailbox of its own now, after the
  account, its Inbox, Sent Items… with their icons and places, collapsible, with Add and Remove
  in its menu and the account's. Exchange's special folders showed MailCraft's English words
  ("Inbox") where Outlook shows the server's ("Postvak IN"); they keep the server's names (EWS
  and Graph). The reading pane showed a message's whole conversation above it though the list
  wasn't grouped; only when grouped now, as Outlook. And an outgoing server can sign in with a
  user name and password of its own (issue #3), its password kept apart in the keychain.
- **2026-10-20** — **v0.20**: on some Windows PCs without a GPU the window never opened; the
  process stayed in the background holding the data files (so later launches were told another
  copy had them) and crashed when Windows shut it down. wgpu started Vulkan and OpenGL beside
  DirectX 12, and without a GPU those find only leftover or overlay drivers and Windows' OpenGL
  1.1. Only DirectX 12 starts now, the adapter picked by MailCraft (a real GPU first, WARP when
  there's none), its shaders compiled with FXC; a window that can't open says why in a message.
- **2026-10-20** — **v0.20**, more: the sixteen sibling Craft repositories were cloned and their
  fixes to what every Craft app shares were read and checked against MailCraft. Adopted, each
  tested: device limits from the adapter, wgpu errors logged after start-up, Intel Macs without
  indirect-call validation, Linux on the desktop's GPU and through XWayland (attachments drag on
  Wayland desktops; a stale DISPLAY falls back), clipboard reads with a time limit, no blink
  from a maximized window at start-up, the DMG built so its app passes `codesign --strict`, and
  the MSI's Windows 10 condition.
- **2026-10-20** — **v0.20**, the four closest siblings again, every commit to their desktop apps,
  packaging and build tools (367) read rather than searched: winit 0.30.13 vendored as PdfCraft
  carries it (Windows 11 monitor scale, input methods on every platform, windows created
  maximized: now shown once, maximized), windows brought back on screen and fitted to it,
  non-Unicode arguments, broken standard error and quitting on macOS made safe, per-monitor DPI
  in the manifest, and attachments for other apps in a folder of the user's own.
- **2026-10-20** — **v0.20**, issue #1's calendars, reproduced against a fake Exchange first: each
  account's calendars under its address in its colour and the server's name, a shared mailbox's
  calendar read with it, Join meeting and the notes' links (a long Teams link read whole from
  the HTML), a meeting in two calendars that opens again, and File › About's Check for updates.
- **2026-10-20** — **v0.20**, issue #5: an Office 365 sign-in couldn't be kept on Windows (its
  refresh token, about 2,000 characters, is over the 1,280 a Credential Manager entry held as
  the keyring crate wrote it); kept as UTF-8 over as many entries as it needs, reproduced and
  tested on the real Credential Manager.
- **2026-10-20** — **v0.20**, issue #4: an account can be kept online only (its mail held in
  memory, each message downloaded when opened, its data file gone) and `portable.txt` beside the
  app keeps everything in MailCraftData there, passwords in memory only; reproduced with the app
  against a fake IMAP server (a 557 KB data file holding every subject and body, then nothing),
  the host's folders, Credential Manager and registry compared before and after.
- **2026-10-21** — **v0.21**: started again, MailCraft brings its window forward (minimized or
  behind other windows) instead of opening a second one on the same data, as PdfCraft does;
  another data folder (`--data`, `--sample`, a portable copy) still opens its own.
- **2026-10-21** — **v0.21**, issues #1 and #4 again, reproduced with the app against fake IMAP
  (1,500 messages) and Exchange servers first: a shared mailbox in the folder pane and the
  calendar as a mailbox of its own, moved like an account; a Google invitation's notes read with
  their links, and every dialog within the window (its title and buttons were off a 2160-pixel
  screen); Show more at the end of a folder and Search the server at the end of a search, for
  mail past what's kept (IMAP); online only explained again, its password asked for at start,
  and a warning before closing would lose unsent mail or changes.
- **2026-10-21** — **v0.21**, the gaps audited again (craft-fonts still has no Hebrew or Korean
  font, the `.pst` crates no new release): gap #7 closed, Search the server for JMAP, Microsoft
  Graph and EWS accounts, their finds kept while the server has them; the browser searches its
  server and shows more; JMAP folders keep more than one `Email/get` (they kept 100 to 500); the
  reading pane's header fits a narrow pane. Checked against fake Exchange and JMAP servers, in
  the app and the browser.
- **2026-10-21** — **v0.21**, a reliability round, each problem first reproduced on the fake
  servers or in the app: edits made mid-sync are now persisted on the spot and replayed
  over refreshed folders (they could be undone on screen, or lost on exit); Graph `$batch` entries
  now request immutable ids (mismatched ids caused endless re-downloads and dropped server-search
  hits), are capped at 4 and retried on 429, and large first syncs read summaries from the listing;
  sibling message windows no longer drag each other around; non-recurring appointments sharing a
  UID stay separate; the 1604 placeholder year is read as "no year"; hidden Exchange folders are
  skipped; Online is shown once mail is current; dead IPv6 routes cost 3 s, not 20 s.
- **2026-10-22** — **v0.22**, Outlook's everyday tools, from a side-by-side look at Outlook's
  ribbon and calendar: calendar reminders (VALARM, mapped to EWS and Graph) with a Reminders
  window, snooze, dismiss and a desktop notification; blocked and safe senders applied as mail
  arrives (and to Junk, whose high-water mark is now set as soon as it's known: against a real
  IMAP server, mail filed there before Junk first changed was never seen arriving); Clean Up
  (messages a later reply quotes in full, conservative by design); Search Folders (with
  `larger:`/`smaller:` and `from:a,b` in the search language, sent to IMAP and JMAP servers too);
  Quick Steps with Ctrl+Shift+1…9 (digit shortcuts now read by key position, so Shift or AZERTY
  don't break them); the Work Week view; Insert date & time and Word count while writing. Found
  on the way: settings were written only on a clean close, so a crash or a forced quit lost
  every change (now kept every few seconds when changed); the log's terminal echo panicked once
  standard error went away (MailCraft ended at the next log line), and the CLI did the same
  writing into a closed pipe; the ribbon's last buttons were cut off at 1336 px (labels now drop
  from the end as needed); a big calendar's expansion was quadratic. Tested against a GreenMail
  IMAP server in Docker as well as the sample mailbox.
- **2026-10-22** — **v0.22**, test builds update themselves (`apps/mailcraft/src/self_update.rs`),
  modelled on how the Craft launcher replaces its own copy and on Office's install-on-close: a
  check a minute after start and then about hourly with jitter (`If-None-Match`, so an unchanged
  list costs no quota), never an older or the same build, the installer for how this copy was
  installed, refused unless its size and SHA-256 match GitHub's per-file digest (or the release's
  `SHA256SUMS.txt`), installed by a detached helper after MailCraft exits (logged to
  `updates/install.log`), and a day-long marker against reinstall loops. Only
  `release-wip.yml` turns it on (`MAILCRAFT_UPDATE_CHANNEL=test`); `release.yml` is unchanged.
  Checked on Windows from a portable copy posing as 0.20.0: test build 18 downloaded, verified,
  unzipped over the folder (data kept) and reopened.
- **2026-10-22** — **v0.22**, issue #4 again, reproduced first against Dovecot 2.3 in Docker
  with the build before the fix: Show more did nothing on a server with QRESYNC (that way of
  syncing never took in the larger window), and Search the server ran each folder whole, showed
  nothing until it ended and didn't stop on Esc. A folder is now searched newest first a range of
  message numbers at a time, its finds shown as they come, and clearing the box, a new search or
  leaving the folder stops it between two ranges. The search box gains Outlook's scope menu
  (Current Folder, Subfolders, Current Mailbox, All Mailboxes), its recent searches and Advanced
  search. Also found on the way: two doc comments the file split had left on the wrong item.
- **2026-10-23** — **v0.23**, a hardening review, area by area, with the Craft family's apps
  and shared repositories as the yardstick for what the apps share; each problem reproduced first
  and fixed with tests: an
  attachment that runs code is never opened, pictures reach the internet only, IMAP answers are
  capped, a one-occurrence invitation reply no longer deletes the series, only the organizer
  changes a meeting, OAuth tokens renew themselves, mail before IDLE isn't late, Ctrl+Q stays,
  the 32-bit build opens, files are written whole and private. Checked on Dovecot and GreenMail
  in Docker too.
- **2026-10-24** — **v0.23**, mail that can't fool you, each piece tested in the engine and
  looked at in the app on the sample mailbox: photos leave without their GPS data; senders imitating a domain
  the user deals with, or showing another address in their name, get a red bar; Send holds a
  message to a lookalike address or one showing many strangers each other (Put everyone in
  Bcc); tracking pictures are counted and never loaded, links open out of redirect wrappers and
  without reader tags; hidden text in HTML mail is brought out; `is:awaiting` finds sent mail no
  one has answered. A fuzz target for photos, and the html and search targets cover the rest.
- **2026-10-24** — **v0.24**, S/MIME (issue #6), on RustCrypto in a crate of its own
  (`mailcraft-smime`): `.p12`/`.pfx` files of every kind (PBES2 with AES, SHA-1 3DES, RC2-40), the
  key in the keychain; signatures made (SHA-256, RSA or ECDSA, RFC 8551's attributes) and checked
  (detached and opaque, PSS, ECDSA, a chain to the system's roots or one's own, dates, email use,
  the sender's address); mail encrypted AES-256-CBC, or AES-GCM for recipients who read it, and
  decrypted (CBC, GCM, 3DES, PKCS #1 v1.5 and OAEP) in memory only, never loading anything from the
  web. Tested with OpenSSL 3 both ways in Docker, and end to end against GreenMail (a legacy `.p12`,
  OpenSSL's mail decrypted, MailCraft's sent, fetched back and read by OpenSSL as its recipient).
  Then the rest of it: a seal and a padlock in the list; header protection (RFC 9788); ECDH for
  EC recipients (RFC 5753); revocation by OCSP and CRL, opt-in, against OpenSSL's own CA,
  responder and list; keys that stay where they are (`mailcraft-keys`: PKCS #11 smart cards,
  tested on SoftHSM 2, and the macOS Keychain); and the browser, which keeps the `.p12` locked
  with its password and signs, encrypts and decrypts with the same code. Then the Windows
  certificate store (`rustls-cng` for listing and signing, as PdfCraft does, and one isolated
  `unsafe` file for CNG's decrypting and EC agreement, as PhotoCraft's tablet crate is), and the
  macOS Keychain's EC decrypting the same way; tested in CNG and in the real store.
- **2026-10-25** — **v0.25**, updates that finish (`apps/mailcraft/src/self_update.rs`,
  [docs/updates.md](docs/updates.md)): the installer downloaded to a partial file and carried on
  with HTTP range requests (`mailcraft_net::updates::download`, a streamed GET in
  `http/streamed.rs`), as ArtCraft Launcher fetches the Craft apps; a bar and a notification when
  it's ready, a question on closing (install, keep for next time, or a download that carries on
  next time); checks hourly while open, backing off from 5 minutes after a failure. Tried for
  real: a test build updated a portable copy to `v0.24.0-test.21` from GitHub through the bar,
  the close question and Restart now, and resumed a 54 MB download from 20 MB.
- **2026-10-25** — **v0.26**, issues #6, #7 and #8. S/MIME keeps older certificates for reading
  (`addressed_to` picks the key a message names; each older key its own keychain entry; imports
  take turns; the browser keeps the older `.p12` files the same way, `smime::own`); removing an
  account now removes its certificates' keys too. Calendars: a TXT
  lookup in `net/src/dns.rs` and the search from the SRV record's path, then its well-known
  address and root; `account.dav` for an address given by hand (followed from the server, a home
  or one calendar) with its own password as `dav` in the keychain. The classic ribbon is drawn by
  the same buttons (`widgets::RibbonFit::classic`), and dark mode's tokens were raised, held by a
  contrast test. Tried for real: Radicale and GreenMail in Docker through the pool (a calendar
  address as Posteo writes it, its own password, a wrong one; mail to last year's certificate),
  Posteo's DNS records, and the app's File › Info with both certificates and the calendar.
- **2026-10-27** — **v0.27**, after `v0.26.0-test.23`. egui and winit from storytold's forks by
  revision in `[patch.crates-io]` (vendor/winit gone); `bidi.rs` keeps only a paragraph's
  direction and font coverage, and mirrors MailCraft's own rows (`bidi::Mirror`,
  `row_from_start`); values in translated sentences go between isolates. The close is decided in
  eframe's logic pass, so a minimized window asks; on macOS the app menu is MailCraft's own (muda)
  and quits through `terminate:`. AccessKit roles and names on every drawn control, held by a
  test. `tests/soak.rs` (3 seeds of 300 actions with the tests, `MAILCRAFT_SOAK` for more) found the
  editor's cursor past the end and egui's right-to-left count. Atomic writes retried on Windows
  sharing violations; relative `APPDATA`/`HOME`/`XDG_DATA_HOME` ignored; COM0/LPT0 and superscript
  ports reserved; textures within the card's limit; the desktop portal's color-scheme on Linux.
  Packaging: soname requirements and libm's GLIBC_2.35 in the RPM, pinned and hash-checked
  appimagetool and signing client, licences in the AppImage and app bundle, macOS `verify.sh`,
  craft-fonts b076417 (Hebrew and Korean in the browser); CI through craft-common-actions'
  `gate.yml`.
  Issues #8 and #9: `Tokens::themed` makes any accent read (darkened or lightened until text in it
  passes on the panes; the tints and the top bar worked out from it), held by the contrast tests
  over every named accent, five hard picks and both bars; the ribbon's buttons on a card; menus
  with a firmer edge and a shadow. `AccountConfig::user_agent` names every EWS request,
  Autodiscover's too; `Client::search_directory` (ResolveNames, `SearchScope="ActiveDirectory"`)
  behind `Job::SearchDirectory`, its answers kept five minutes ("no one" half a minute) and
  forgotten by Refresh; the EWS calendar and contacts read apart, so one refused doesn't keep the
  other old. Tried for real: HAProxy in Docker refusing unknown clients with 403, as the tester's
  server, in front of the fake EWS server: MailCraft's own name refused (the issue's "Exchange
  answered HTTP 403"), Evolution's let in, through the CLI, the worker and the app; someone added
  to the directory 45 seconds in found by the next search. GreenMail and Radicale in Docker for the
  rest of the real-server suite. Gap 7 closed: Exchange Online searches `/users` (then
  `/me/people`) with a token of its own, from a sign-in asking for `User.ReadBasic.All` and
  `People.Read` alone (`oauth::DIRECTORY_SCOPE`, `Job::AllowDirectory`), its refresh token kept as
  `directory` in the keychain; the mail's sign-in and refreshes ask for what they did. Gap 5 closed
  (Kerberos for EWS: `exchange/kerberos.rs` on sspi, the KDC over our own TCP, found by DNS SRV;
  tried against Samba's AD DC and mod_auth_gssapi in Docker), gap 2 narrowed (right-to-left mail
  selects; Simplified Chinese fetched in the browser when wanted), gap 7 narrowed (certificates kept
  unlocked in the browser, sealed by WebCrypto, tried against Stalwart and OpenSSL); JMAP accounts
  without an identity get one.
- **2026-10-28** — **v0.28**, after `v0.27.0-test.24`: issues #7 (subscribe to a calendar by its link:
  `session/pim/web.rs`, `net/src/protocols/web_calendar.rs`; tried against nginx, Radicale and
  Thunderbird's published holiday calendars) and #9 (full names from the directory: EWS
  `ResolveNames` with the full contact; tried against grommunio in Docker, which names a mailbox
  with no display name by its alias). `session/pim.rs` split: the sync merge in `pim/sync.rs`.
  Ported from the sibling Craft apps (authors credited in each commit): the Windows pin icon
  (df0f10a), macOS `CFBundleLocalizations` (7dc8276) and a private folder for the signing
  certificate (7ce73f5), `libxkbcommon-x11` in the deb and rpm (3c2a9fa), `CRAFT_FONTS_REQUIRED`
  enforced (6df62e5), job-level release permissions (0619fe9), FreeBSD and Windows ARM64 off the
  push path (b21aa6a).
- **2026-10-28** — **v0.29**, after `v0.28.0-test.25`: issues #10 (pictures in the editor at their
  size, with handles: `rich_edit/pictures.rs`; files pasted and dropped on the main window:
  `app/dropped.rs`; HTML pictures at the sizes they set: `mime::ImageSize`) and #11 (attachment
  extensions from their type or first bytes: `mime/src/files.rs`; pictures in signatures, kept in
  the signature's HTML as `data:`). Tried for real: the mail through GreenMail in Docker, the 0.28
  `.deb` on Debian 12 and this build on Ubuntu 25.10 under X11 with a real file drag and a GTK
  clipboard holding copied files, a newsletter checked against Edge's rendering of the same HTML.
  `compose.resizePicture` is new; `compose.insertPicture` takes `width` and `line`. The first
  outside contributions: Remo Laubacher's #12 (search box text centred), #13 (Unread filter keeps
  the message being read), #14 (pictures at their own size, pasted files, the WSL runtime-folder
  fallback), #15 (one two-way table of extensions and types) and #16 (signature pictures with
  Content-IDs of each message's own), squash-merged; v0.29's commits after them fix what testing
  them found (a second wide picture past the editor's edge, big pictures sent at full width, a
  drag on a handle that lost it) and add what they left out.
  Issue #18: Outlook's `winmail.dat` read (`mime/src/tnef.rs`, MS-OXTNEF; compressed RTF and the
  HTML or text in it in `tnef/rtf.rs`, MS-OXRTFCP and MS-OXRTFEX), its files listed through one
  index with the message's own (`mime/src/attachments.rs`); a fuzz target `tnef`; exporting into
  a `.pst` already there adds to it (`pst_export.rs`). Tried with compressed RTF and attachments
  Outlook itself saved, packed as its transport packs them, sent through GreenMail and read over
  IMAP by the CLI and the app. Issue #10's forwarded pictures (`mime::quoted_pictures`) and
  issue #19's Sent folder (`worker/send.rs`, made where there's none; other languages' names in
  `FolderRole::from_name`), tried with GreenMail and Dovecot set out as cPanel's. Issue #20's
  invitations on arrival (`session/invite.rs`, `take_invitation`; `calendar.tentativeInvites`),
  tried with GreenMail and Radicale.
- **2026-10-30** — **v0.30**, after `v0.29.0-test.27`: issue #19 at a cPanel host (one Sent Items per
  account, the server's flag first: `protocols/imap.rs`, `one_per_role`; names further down aren't
  roles), issue #10's centred columns (`mime/src/layout`, a box or table of a set width) and
  Ctrl+V of files (`ui-egui/src/app/dropped.rs`, `clipboard_keys`, run by a viewport hook in the
  app with eframe's clipboard shortcuts off). Tried with Dovecot as cPanel's, GreenMail, and on
  Debian 12 under X11 with six file managers.
