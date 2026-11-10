# Changelog

Every version of MailCraft that testers can install, newest first. What's still missing is in
[known gaps](docs/gaps.md); what comes next is in the [roadmap](ROADMAP.md).

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and MailCraft uses
[Semantic Versioning](https://semver.org/). Before 1.0, any version may change anything. Each
version is a milestone, named in italics under its heading. Numbers like (#11) are tester issues
on GitHub.

## 0.35.0

*Mail built from templates reads as it was designed, a mail file opens from inside MailCraft, and each account checks for mail as often as you choose.*

### Fixed

- Mail built from templates is no longer said to hide its text: a preview line's invisible padding isn't words, and `font-size: 0` on a column hides nothing when the text inside sets its own size (#47).
- A heading written in boxes of its own no longer shows `##` before its text, and a price no longer shows `^{$}`: raised and lowered text (`<sup>`, `<sub>`) sits above and below its line (#47).
- Newsletters and notifications built with MJML and similar templates are laid out however deeply their tables nest, with no more text tables of box-drawing lines in a monospace font; lines around cells and tables, dividers and spacers are drawn, and heading cells are bold and centred (#47).
- A row's shares no longer push out what's beside them: a progress step's dot stays between its bars and a logo beside a cell of 100% stays beside it. A picture held back keeps the size it says, so an icon's description no longer runs a letter to a line (#47).
- A box drawn around text or a picture keeps its colour, corners and lines, whatever its width: a progress step's round dot, or a note in a coloured card (#47).
- Text in a message is drawn at the size its sender set, so fine print is small and a price or title large (#47).
- On a POP account, a message you send shows in Sent Items at once, and a saved draft in Drafts, instead of after MailCraft starts again (#57).
- A message you send just before starting another no longer stays "still being sent" for good, and closing MailCraft no longer warns about it.
- New contacts and events have somewhere to go when the calendar server has only address books or calendars you can't change (a Nextcloud user who never opened its web page): they stay on this computer instead of being refused (#61).
- Calendars and contacts sync with a server that takes Digest sign-in only, as Baïkal does unless set otherwise: every request was refused as if the password were wrong (#61).

### Added

- File › Options › Safety can show web pictures as a message opens, unless it looks suspicious: in Junk, from a blocked sender, or from a sender your mail server couldn't confirm. Off at first; a picture's server can see your IP address and when you opened the message.
- File › Open & Export opens a saved `.eml` or Outlook `.msg` file to look at as plain text, without adding it to a folder (#56). Opening one shows who else it went to, what's attached and where a link really goes.
- File › Options sets how often each account looks for new mail: automatically, as before, or every 5, 10, 15, 30 or 60 minutes (#58).
- Add account can add calendars and contacts only, with no email, from any CalDAV and CardDAV server (Nextcloud, Baïkal, SOGo, Radicale and others): give its address, your user name and an app password, and they sync both ways. A server that schedules meetings sends their invitations (#61).

## 0.34.0

*One coloured title bar on Windows, dates written your way, and drafts never lost on their way to the server.*

### Added
- On Windows, MailCraft's top bar is the window's title bar, in its colour, as Outlook's is: its mark, the tabs and the search box share the top of the window with the window's own buttons, the bar's free space moves the window (a double-click maximizes it, and it snaps to the screen's edges), and the window resizes from its edges. **File › Options › Use the system title bar** brings the system's back, from the next start (#22).
- Drafts shows how many drafts it holds, in brackets (`Drafts [2]`), and a message whose reply or forward was begun and left in Drafts is marked with a red **[Draft]** before its sender in the list, as in Outlook (#52).
- **File › Options › General** chooses how dates are written (2026-10-28, 28/10/2026, 10/28/2026 or 28.10.2026), a 24-hour or 12-hour clock (`1:53 PM`), and whether the message list gives every message its whole date and time instead of the time today and the day this week (#46).
- Key tips on the ribbon, as Outlook's: Alt (or F10) shows a letter on each tab and on the search box, a tab's letters show its buttons', and a button's press it (Alt, H, R, P replies; Alt, J, S opens Send / Receive; Alt, Q goes to the search box). Escape goes back a letter, and a character typed with an Alt code shows none.
- The boxes you type a time into (an event's start and end, a task's reminder, Delay delivery) show it on the 12-hour clock when File › Options sets one, and take `1:30 PM`, `1pm`, `12 AM` or each language's own way of writing it, as well as `13:30`.
- A newsletter's buttons are drawn as buttons, with their round corners, and Outlook's at the size its copy gives them; columns that set no width are as wide as what they hold, as in a browser.

### Changed
- Outlook data files (`.pst`) are read and written by MailCraft's own code, written from Microsoft's published specification, in place of a third-party library. Files import as before, and those MailCraft makes or adds to open in Outlook.

### Fixed
- On Windows, maximizing and restoring MailCraft are smooth: they animate as any window's do, the taskbar no longer blinks and the window no longer flashes a blurred picture of itself.
- On Windows, all text in MailCraft is sharp again: the window's own title bar had the picture squeezed by one row of pixels, which softened every line from top to bottom. A stray black line along the top of the window is gone too.
- Removing an account while its `.pst` import was still writing could leave a new account added with the same id writing into the removed account's kept data file. The two now never share a file.
- Sending greets the server with this computer's own address (`EHLO [192.168.1.20]`, as Thunderbird does) instead of `[127.0.0.1]`, which some providers' spam filters took as a reason to drop the message after accepting it (#50).
- A reply or forward being written in the reading pane makes way for the message you select next, kept in Drafts, as Outlook does; the pane went on showing it while the other message was only selected in the list. One with nothing written in it just closes, and one in a window of its own stays open (#51). A reply or forward left as MailCraft made it, or a draft opened again and left unchanged, counts as nothing written: it leaves nothing new in Drafts, and closing it asks nothing, as in Outlook.
- A message closed with its changes saved (**Yes** to "Want to save your changes?"), or made way for in the reading pane, is held until its draft is safely in Drafts on the server or kept on this computer; if neither can keep it, the message opens again in a window with the reason, instead of being lost (#53).
- Messages sent from two MailCraft windows or computers started in the same second no longer share a Message-ID, which could make a mail service drop one of them as a copy of the other.
- A folder's unread count no longer jumps back up after a draft is saved: the folder list the server account refreshed then brought back the counts from before the mail you had just read reached the server.
- A draft or a sent copy filed in a Drafts or Sent folder deleted on the server (by another program) makes the folder again and goes in it, and one renamed there is found again under its new name. It was refused, and a draft kept on this computer was refused again at every connection.
- The ribbon's labels of commands you can't use yet (Delete, Archive… with no message selected), and the classic ribbon's group captions, read clearly: their grey was too faint to read comfortably on the ribbon (3.2:1), and is now as dark as text must be (4.5:1), still lighter than the commands you can use (#45).
- Newsletters lay out much as desktop Outlook shows them (#47): MailCraft reads the copy a newsletter gives Outlook (`<!--[if mso]>`, with its buttons and banner pictures), so columns built for other programs come out as Outlook's table; boxes side by side in a cell sit in a row when they fit; a row's cells are as tall as its tallest, their colours filling it, with what's in them at the top, middle or bottom as they say, and a height they ask for kept as the least; cells that each take a share of a table fill it as a browser shares it out; and the page's own colour no longer shows as a grey highlight behind every word.
- A folder of an Outlook data file being imported shows in the folder pane as soon as it's in, even when it's finished just after another: one finished within half a second of the last one shown (the Inbox, just after Deleted Items) waited for the next folder, or the import's end.
- Renaming or deleting a folder while MailCraft is still getting the mail of the folders before it no longer says "the server refused: no such mailbox".
- A folder renamed twice while offline keeps the mail already on this computer once the renames reach the server; it was downloaded again, and the name in between said "no such mailbox".
- A table's footer row (`<tfoot>`, as an invoice's total often is) shows at the end of its table, as a browser shows it: it was left out of the message and its text, and came first in a table laid out in columns.
- A draft saved just as MailCraft closed, or as the connection was lost, is in Drafts once after the next connection also on a server that adds a header to what it keeps. On a server without UIDPLUS, the copy a save replaces is marked deleted instead of the folder being expunged, so mail another program marked deleted in Drafts no longer goes with it.
- A forward's draft from a folder whose name has accented or non-Latin letters (`Entwürfe`) remembers the message it forwards when it's opened again.
- A message nesting tables by the thousand, as broken or hostile mail can, opens at once instead of after minutes: past what any mail nests, it's read as its words and links.

## 0.33.0 - 2026-11-05

*Replies as they came, and fixes shared with the other Craft apps.*

### Added
- Open a draft from Drafts to carry on writing it (**Edit**, or a double-click), as it was saved: its recipients and Bcc, formatting, signature, pictures, files and the original under a reply. Sending it marks the message it answers.
- Releases for FreeBSD on 64-bit ARM (aarch64).
- MailCraft updates itself however it was installed on Linux and FreeBSD: a `.deb` or `.rpm` through the system's package tool (after your password), a `.tar.gz` in place, and a Flatpak through your software installer, as the `.msi`, portable `.zip`, `.dmg` and `.AppImage` already did.
- MailCraft's releases (`v0.33.0`), not only its test builds, can be published, and they update themselves to the next release. A test build updates to the next test build or release, whichever is newer.

### Changed
- **File › Info › Mailbox settings** gives the data file's location, its Outlook copy and the clean-up each a row of their own.
- The **Open Outlook data file** dialog shows the file on a card with its size and contents, and a bar of the room its copy takes.
- **File › Options › Language** lists English first, then the other languages by their own names, accents set aside.
- The ribbon's tabs share the top bar with the search box, so the window gives one row less to its chrome; the app's name, which the title bar already shows, has gone from it. Send / Receive, Undo, dark mode and settings sit at the bar's end. In a narrow window the search box has a row of its own (#22).

### Fixed
- A server whose certificate comes from your organisation's own CA, as an on-premises Exchange often does, connects once that CA is in the system's certificate store (Windows, the macOS Keychain, the Linux bundle), for receiving and sending, instead of failing with "UnknownIssuer". Certificates Windows lists as untrusted stay refused, whether the server or a CA above it, and the password never reaches such a server (#37).
- Blocks of a newsletter stacked one on another touch, as in Outlook: each table keeps its own spacing and each block its padding, and a pale tint on a pale page still shows (#41).
- A block whose background is a gradient shows the colour Outlook shows for it (#41).
- The plain text of mail laid out in tables has no box-drawing lines around each table, and its preview keeps one block's words apart from the next's (#41).
- Reply, Reply all and Forward keep the original as it came, as Outlook does: mail laid out in columns shows under your text as the reading pane shows it, you change its words where they stand (typing, the ribbon and the shortcuts work in it), and it goes out with its own formatting and pictures.
- The message list's filters stay on one line in a narrow pane. Those that don't fit go into a **⋯** menu beside **By Date**.
- On the File page, a row's buttons and pickers never cover its text, and the page fits a narrow window instead of being cut off.
- The **Export to Outlook data file** card says calendars go inside the `.pst` file too.
- The View tab folds its buttons to icons in a narrow window, as Home does, instead of scrolling sideways.
- A message taken back out of the Outbox to change keeps its formatting, its pictures in place and, for a reply, the original under it as it was.
- Saving a draft again replaces its copy in Drafts on the server instead of adding another, and a server without a Drafts folder gets one.
- Sent from Drafts, a draft no longer leaves the list saying there's one more on the server.
- When the graphics driver resets (after sleep, say), MailCraft keeps every message being written in Drafts and closes, instead of freezing or crashing.
- Esc in a field on the File page leaves the field; the next Esc leaves the File page.
- A text file attached inline and named only by its file name, as Apple Mail sends one, is listed with the message's files, where it can be opened and saved, instead of being run into the message's text.
- A file whose base64 has a stray character in it opens as the file instead of as its encoded text, and a message whose own text has one shows that text instead of turning into a file.
- A file's name written in several pieces (RFC 2231) in a character set other than UTF-8 reads right instead of with `�`, and a message whose text has its boundary in the middle of a line is no longer cut short there.
- On Windows, MailCraft started from Task Scheduler or a remote shell opens in the window already running, not a second one.
- A file a program hands over unquoted, its path split at the spaces, still opens.
- The windows of messages being written, opened or shown as source carry MailCraft's icon on Wayland.
- On Linux, starting MailCraft no longer leaves a busy cursor, and the light or dark setting is followed even when the desktop changes it during start-up or doesn't answer.
- Screen readers can't close MailCraft by losing track of the focused control.
- Test builds install an update only once MailCraft has ended, and never one whose checksum file lists it twice with different sums.
- On a Mac, files MailCraft saves in a folder on a network share no longer fail to save.
- A picture opened again from a message no longer makes Photos ask "Is this file from a trusted source?": the "from the internet" mark an earlier version left on its copy is removed.
- Searching for a phrase whose words repeat ("the the") finds it after a word joined by a dot ("you.the the the"), where the text was checked.

### Security
- An Outlook data file's link to a device (`NUL`, `COM1`) or to a hidden file stream is kept as text, never read.

## 0.32.0 - 2026-11-04

*Pictures open without asking.*

### Added
- A local task list (**Tasks**, Ctrl+4) with due days, status, priority, reminders and repeats. A task with a due day shows in the calendar on that day.
- Tasks round-trip through `.pst` files and go into calendar files (`.ics`) as to-dos.
- Select and copy a message's text across links, tables and quotes. Ctrl+A selects the whole message, Shift with the arrow keys crosses lines, and a drag at the edge scrolls.
- `.pst` export keeps every item kind Outlook stores: flags, categories, pins, signed and encrypted mail, reports, attached messages, contacts, notes, tasks and calendars.
- Export adds to `.pst` files Outlook made, in its Unicode and older ANSI formats, and replaces items changed since the last export.
- `.pst` import brings in Rich Text bodies, mail in any code page, attached messages, linked files, RSS feeds, recalls, sharing invitations, meetings and the files on notes.
- **Mark complete** keeps a follow-up done, with a tick and the day it was done, through servers and `.pst` files.
- Search finds a word whatever its accents, and takes dates in `received:`, `sent:`, `date:`, `after:` and `before:`, including ranges such as `2024-01-01..2024-03-31`.
- Before an import, MailCraft shows what the `.pst` file holds and the room its copy needs. During it, a progress bar shows the time left.
- Pause, stop or continue an import. An import cut off by a crash or by closing MailCraft goes on where it stopped.
- **Compact now** in **File › Info › Mailbox settings**. MailCraft also compacts data files when left idle, and on close if you turn that on in **File › Options › Data files**.
- Turkish as an interface language (#34).
- Drop Outlook message files (`.msg`) or `.eml` files on MailCraft to put them in the folder open, as read mail, with their files and attached messages. A `.msg` opened from the desktop shows like an `.eml` (#38).
- React to a message with an emoji (**React**, beside **Forward**). The reaction goes to everyone the message went to (RFC 9078), and the reactions others send show under the message (#39).
- OpenPGP beside S/MIME: import your secret key file in **File › Info › Security** (its passphrase isn't kept; the key stays in the system keychain). Older keys stay to decrypt old mail (#36).
- PGP/MIME and inline PGP mail is decrypted for showing, and the reading pane says who signed it and whether the signature holds (#36).
- People's OpenPGP keys come from their mail's `Autocrypt:` header and attached keys, or from a key file you import. Your own key goes out with your mail, so answers can be encrypted (#36).
- Sign and encrypt with OpenPGP as PGP/MIME, to yourself too so Sent Items stays readable. With both a certificate and a key, the message uses the one every recipient can receive, or the one you pick (#36).
- The message you're writing says whether every recipient can receive it encrypted, and names those who can't (#6).
- **Encrypt when every recipient can receive it** in **File › Info › Security** turns encryption on by itself while that's so. It's off unless you turn it on (#6).

### Changed
- Opened or saved pictures and plain text files no longer carry Windows' "from the internet" mark, so Photos doesn't ask whether to trust them. Office files, PDFs, web pages, archives and programs keep the mark.
- Double-clicking an attachment only opens it. The reading pane keeps showing the message.
- Plain text moves from the link above a message to **File › Options › Reading** and to each message's **⋯** menu.
- Search runs in the background over an index that keeps up as mail changes. Typing never waits.
- Imports read on every core.
- An opened `.pst` file's copy (`.mailcraft`) goes beside it, instead of in Documents.
- Mail from a `.pst` file is listed by when it arrived, not when it was sent.
- Mail from a `.pst` file joins conversations by Outlook's conversation index when its ids don't place it.
- MailCraft reads and writes `.pst` files with one crate, from the fork [ZIDAN44/outlook-pst-rs](https://github.com/ZIDAN44/outlook-pst-rs).

### Fixed
- Big folders scroll, and long messages with many pictures open, without stutter.
- The window opens at once, even while a data file is checked. Data files reopen at once after a crash.
- Search results appear as soon as typing pauses.
- Pictures shown in a message's body keep their file names when sent or imported.
- Text attachments are saved and forwarded exactly as they came, in their own character set.
- The paperclip shows only on mail with real files, not on mail whose only pictures are in the body.
- Bounces and read receipts from a `.pst` file stay reports.
- A drive that fills during an import pauses it, instead of failing every message after.
- A sign-in that fails to save leaves no stray entries in Windows Credential Manager.
- Two MailCraft windows, or the CLI beside the app, can't overwrite each other's mail folders.
- Opening a message no longer waits for a folder, or the calendars, to finish syncing: the account's second connection brings it (#24).
- On IMAP accounts, a big message with attachments shows its text at once, its files listed; they download after (#24).
- Centred columns sized in points or ems, as Outlook and email tools write them, keep their width instead of filling the window (#41).
- A title centred in such a column stays centred and keeps its size, as Outlook shows it (#41).
- A reminder dismissed or snoozed in MailCraft is dismissed or snoozed on the server too, so Outlook doesn't show it again (Exchange, Microsoft 365, and CalDAV calendars) (#40).
- A reminder dismissed in another calendar program that says so in the event (RFC 9074) isn't shown again in MailCraft (#40).

### Security
- MailCraft never writes into an opened `.pst` file. Changes go into its own copy beside it, which belongs to one data folder.
- Mail goes only from an account that can send. With only a data file open, the message form offers **Add an account**.
- A `.pst` attachment that links to a file is read only from this computer's drives, never from the network or the web.
- A server's answer with lists nested a thousand deep can no longer crash MailCraft.

## 0.31.0 - 2026-10-31

*Signatures and meetings.*

### Added
- Several named signatures. Each account picks one for new messages and one for replies and forwards (#11).
- The **Signature** menu in a message lists the signatures and swaps the one in use (#11).
- Select several messages with Shift and the arrow, Home, End, Page Up and Page Down keys (#26).
- The right-click menu acts on every selected message, and the reading pane says how many are selected (#26).
- **View › Message source** shows a message as it came, headers and all. `mailcraft-cli source` does the same (#27).
- Keywords that other mail programs set show as categories. **Categorize › All categories…** lists them (#28).
- The category list exports to a file and imports back (#28).
- The message list can show a sender's address after the name: always, for strangers only, or never (#29).
- An optional External tag for senders outside your organisation's domains (#29).
- Each account's default calendar can be chosen (#30).
- **File › Options › Mail › After answering a meeting invitation**: leave it, move it to Deleted Items, or move it to a folder (#31).
- **Private**, **Show time only** and **Show as** in the event form (#32).
- Search in the Calendar finds events across every calendar shown, offline, with the mail search's syntax (#33).
- Keep your own BigBlueButton, Jitsi or OpenTalk rooms in **File › Options › Online meetings**. **Add online meeting** puts a room's link into an event (#25).
- **Join meeting** opens the meeting link of an event you received (#25).

### Changed
- Changing **From** while writing swaps an unchanged signature for the new account's (#11).
- A new category gets the same IMAP keyword Thunderbird gives a tag of that name, so both programs share it (#28).
- An invitation that arrives before the account's server calendars are found waits, then goes to the server (#30).
- An accepted invitation shows as busy, not tentative (#32).
- On servers that send invitations themselves (SOGo, Stalwart), guests get one invitation, not two.
- Signature settings saved by 0.31 aren't read by 0.30, which then shows no signature.

### Fixed
- In Calendar, People and Notes, the message list's keys don't act on hidden messages (#26).
- Shift+click selects the rows as they appear, pinned messages included (#26).
- Searching the server for a category finds it (#28).
- A folder that can't keep keywords says so, instead of dropping the category silently (#28).
- A new event goes to the account's default calendar, and two syncs in a row send it once (#30).
- One event that SOGo refuses doesn't stop the whole calendar sync.

### Security
- A sender whose name matches someone you know, from an address that isn't theirs, is warned of in the reading pane and marked in the list (#29).

## 0.30.0 - 2026-10-30

*Sent folder and centred mail.*

### Added
- Paste files copied in any file manager (#10).
- Signatures written as HTML code (#11).
- Alt+numpad character codes on Windows (#23).

### Fixed
- Sent mail goes to the folder the server marks as Sent (`\Sent`, RFC 6154) (#19).
- Fixed-width HTML mail keeps its column width (#10).
- Exchange messages show their text before the attachments finish loading (#24).
- A signature's table keeps its columns, so a photo stays beside the name (#11).
- The Flatpak attaches files dropped or pasted from anywhere in the home folder.
- A saved web-page signature finds its pictures in a folder with an accent in its name.

## 0.29.0 - 2026-10-29

*Pictures and pasted files.*

Thanks to Remo Laubacher for #12–#16.

### Added
- Pictures in a message being written show at the size they are sent. Resize them by the corner handle or the right-click menu (#10).
- Drop files on the main window to start a message with them attached (#10).
- Paste files copied in a file manager to attach them. On Windows, use the right-click **Paste** (#10).
- Signatures with pictures, from a file or pasted: up to 10 pictures of up to 1 MB each (#11).
- A formatting bar in the signature editor (#11).
- A signature copied from Word or Outlook brings its logo when pasted (#11).
- Replies and forwards can open in a window of their own (**File › Options › Mail**) (#11).
- **View › Favorites** hides Favorites from the folder pane (#11).
- `winmail.dat` attachments (Outlook's Rich Text mail) open: their files, pictures and formatted text show like any other message's (#18).
- Meeting invitations go on the calendar as tentative when they arrive, here and on the CalDAV server. **File › Options** turns it off (#20).

### Changed
- Exporting to an existing `.pst` file adds to it and skips messages it already has (#18).
- With the Unread filter on, an opened message stays in the list until you pick another (#13).
- Replies and forwards carry the original message's pictures (#10).

### Fixed
- The search box's text sits centred (#12).
- Pictures in HTML mail show at the sizes the message sets (#10).
- Attachments with no name or extension get the extension their type has, so the system can open them (#11).
- Servers without a Sent folder, or with one named in another language, get a copy of sent mail (#19).
- Closing MailCraft while a message is still sending asks first.
- `mailcraft-cli send` waits for its own message and for its copy in Sent.
- `mailcraft-cli` says when the app has the same data open, instead of showing empty folders.
- Mail in Shift_JIS, GBK, Big5 and EUC-KR shows its text.
- A message without a date shows the time the server received it, not 1 January 1970.
- Attachments open and drag out where `$XDG_RUNTIME_DIR` is missing, as on WSL (#14).
- A signature's pictures get a new Content-ID in every message (#16).

## 0.28.0 - 2026-10-28

*Web calendars.*

Some fixes are ported from the sibling Craft apps, with their authors credited.

### Added
- **Subscribe to a calendar…** takes an iCal or `webcal://` link. The calendar shows read-only and refreshes by itself (#7).
- Right-click a subscribed calendar to refresh or remove it (#7).

### Changed
- People found in Exchange's directory show their full name, not their alias (#9).
- A contact saved without a name takes the name the directory gives (#9).

### Fixed
- Events in read-only calendars can't be deleted.
- A subscribed calendar refreshes after the clock is set back.
- On Windows, a pinned MailCraft keeps its icon after an update. A pin that already lost it needs pinning again once.
- On macOS, the open and save panels follow the interface language.
- On Linux, the deb and rpm packages recommend `libxkbcommon-x11`, which the window needs under X11 and XWayland.
- Release builds always include the fallback fonts for Arabic, Hebrew, Chinese, Japanese, Korean and Thai.

## 0.27.0 - 2026-10-27

*Mirrored right-to-left.*

### Added
- In Arabic and Hebrew the panes are mirrored: icons on the right, counts and times on the left, scroll bars on the left.
- A Hebrew or Arabic paragraph being written starts at the right edge.
- Arabic and Hebrew mail can be selected and copied with the pointer.
- Screen readers name every button, toggle, folder and message row.
- **File › Options › Accent colour** offers eight colours and a picker for any other (#8).
- A **Plain top bar** option draws the top bar in the panes' colours (#8).
- A User-Agent of your own for each Exchange account, for servers that let in only mail programs they know (#9).
- **Select Names** searches the Global Address List, and To and Cc suggest names from it (#9).
- Exchange Online accounts search their organisation's directory, after a one-time sign-in that allows it (#9).
- Exchange servers that accept only Kerberos sign in.
- Simplified Chinese in the browser. The page fetches the font only when it needs it.
- **File › Info › Security › Keep unlocked in this browser** keeps the certificates' password sealed in the browser.

### Changed
- Long names are cut to one line with "…" instead of wrapping.
- The ribbon's buttons sit on a card with an edge. Menus and dialogs have a firmer edge and a soft shadow (#8).
- In dark mode the top bar is a deep shade with white text (#8).

### Fixed
- Closing from the taskbar or Dock while minimized asks before a message being written is lost. On macOS, ⌘Q asks too.
- Latin words inside Hebrew or Arabic text read in the right order, and the caret and word deletion work there.
- The message editor no longer crashes on a cursor past the end of the text.
- A data file that OneDrive or an antivirus holds for a moment is written again instead of failing.
- The System theme follows a Linux or BSD desktop's dark or light setting.
- Ctrl+V pastes a picture.
- A picture in a message sits where it is in the text.
- Each ribbon tab keeps its own scroll place, and file dialogs open where the last one was left.
- A calendar the Exchange server refuses doesn't keep the address books from updating.
- The RPM installs on openSUSE.
- Hebrew and Korean draw in the browser.
- A JMAP account the server made with no sending identity can send.
- A user name with a line break or tab in it is refused, instead of never signing in.
- A sample mailbox from an older build no longer stays on "Loading folders…".

## 0.26.0 - 2026-10-25

*Classic ribbon.*

### Added
- **View › Classic ribbon** shows big icons with their names underneath, in named groups (#8).
- **File › Info › Account settings › Calendar server…** takes CalDAV and CardDAV addresses with their own user name and password (#7).

### Changed
- Importing a renewed certificate keeps the old one to read mail sent to it. The newer one signs and encrypts (#6).
- **File › Info › Security** lists older certificates with **Use for sending** and **Remove** (#6).
- Calendar discovery follows RFC 6764 fully, so calendars such as Posteo's are found (#7).
- Dark mode sets panes, borders and fields further apart, and light mode's faintest text is easier to read (#8).

### Fixed
- Encrypted mail for a certificate MailCraft doesn't have says so, instead of trying every key.
- Signed mail stays valid through servers that strip `X-Mailer`, such as Postfix.

### Security
- Removing an account removes its certificate keys and calendar password from the system keychain.
- Calendar servers over plain http are allowed only on this computer.

## 0.25.0 - 2026-10-25

*Updates that finish.*

### Added
- A bar says when a test build's update is ready: **Restart now** or **Later**. A system notification says so when MailCraft isn't in front.
- The status bar shows the update download's progress.
- Closing MailCraft asks whether to install a downloaded update.
- An interrupted update download resumes, in this run or the next.

### Changed
- On Windows, the bar and the close question say that Windows asks for an administrator.
- An AppImage in a folder you can't write to is left alone, and macOS finds Craft apps in `~/Applications` too. See [how test builds update](docs/updates.md).

## 0.24.0 - 2026-10-24

*S/MIME.*

### Security
- Import a `.p12` or `.pfx` certificate in **File › Info › Security**. Its private key stays in the system keychain (#6).
- Sign and encrypt outgoing mail, by default or per message (#6).
- Signed mail says who signed it and whether to trust it. If a check fails, it says which.
- Encrypted mail is decrypted in memory only, and shows its attachments, previews and invitations.
- Replies to encrypted mail are encrypted.
- The message list marks signed mail with a seal and encrypted mail with a padlock.
- EC certificates sign, encrypt and decrypt.
- Use keys on smart cards (PKCS #11), in the macOS Keychain or in the Windows certificate store, including keys that can't be exported.
- S/MIME works in the browser, with the `.p12` file unlocked by its password.
- The subject, sender and recipients are signed and encrypted too (RFC 9788). An encrypted message's subject reads `[...]` in transit.
- Decrypted mail loads nothing from the web, and only a message protected as a whole counts as signed or encrypted.
- An optional revocation check of the signer's certificate, in **File › Options › Safety**, off by default.

## 0.23.0 - 2026-10-24

*Mail that can't fool you.*

### Added
- `mailto:` links, and `.eml`, `.ics` and `.vcf` files, open in MailCraft. The installers offer it as the mail app.
- `is:awaiting` finds mail you sent 2 to 30 days ago that nobody answered. A Search Folder and a Search tab toggle offer it.
- Repeat rules such as "the last weekday" and "Friday the 13th".

### Changed
- An event keeps its time zone and repeats at the same local time across daylight saving.

### Fixed
- Answering an update to one occurrence changes that occurrence, not the whole series.
- An expired sign-in token renews itself before MailCraft asks you to sign in again.
- Mail that arrives just after a sync shows at once, not up to ten minutes later.
- A message the SMTP server refuses with a mention of "authenticated" can be sent again.
- Ctrl+Q marks a message as read without closing MailCraft.
- Files dropped from the desktop onto a message are attached.
- Search results from several folders say which folder each is in.
- The 32-bit Windows build opens its window.
- `--help` prints the options.

### Security
- Photo location is taken out of JPEG and PNG attachments when you send. **File › Options › Safety** can keep it.
- A red bar warns of a sender whose domain imitates one you deal with, or whose name shows a different address.
- **Check before sending** holds mail to a look-alike domain, or mail that shows eight or more unrelated people each other's addresses.
- Tracking pictures never load, and a message says how many trackers it carries.
- Links open without redirect wrappers and tracking tags. **File › Options › Safety** can open them as they are.
- HTML mail that hides 25 words or more says so and shows the hidden text.
- Invitation updates and cancellations apply only when they come from the organizer.
- A hardening review: MailCraft doesn't open attachments that run code, marks saved attachments as downloaded on Windows, cleans unsafe file names and keeps message pictures off your own network.

## 0.22.0 - 2026-10-22

*Everyday tools.*

### Added
- Event reminders, from None to 2 weeks, with a Reminders window (Dismiss, Snooze, Open Item) and a desktop notification.
- **Block sender**, **Never block sender** and **Never block sender's domain**, with both lists in **File › Options**.
- **Clean up conversation** and **Clean up folder** move messages that a later reply quotes in full to Deleted Items. Undo brings them back.
- Search Folders: saved searches in the folder pane.
- `larger:` and `smaller:` in search, and several addresses in `from:` and `to:`.
- Quick Steps, from the ribbon or Ctrl+Shift+1 to 9.
- The Work Week view.
- **Insert date & time** and **Word count** while writing.
- The search box's scope menu (Current Folder, Subfolders, Current Mailbox, All Mailboxes), recent searches and Advanced search.
- Write several messages at once, each in its own window.
- Hovers, selections and panes move smoothly. **Play animations** in **File › Options** turns this off.
- Test builds update themselves: they download the next test build, check it and install it on close. Signed releases don't.

### Changed
- Searching the server goes newest first, shows results as they come and stops when you clear the box (#4).
- The ribbon is one height on every tab, and drops labels from its end when the window is narrow.

### Fixed
- **Show more** works on servers with QRESYNC, such as Dovecot (#4).
- Settings are saved every few seconds, so a crash doesn't lose them.
- Closing MailCraft with messages being written asks to save them.
- Changes made with `mailcraft-cli run` reach the server at the next `sync`.
- The CLI doesn't crash when its output is closed.
- Ctrl+Shift+digit shortcuts work with Shift and AZERTY keyboards.
- Notifications from a portable copy show MailCraft's name when an installed copy is on the computer.

## 0.21.0 - 2026-10-21

*One MailCraft at a time.*

### Added
- Starting MailCraft again brings the running copy to the front instead of opening a second window on the same data. A different data folder runs its own copy.
- **Show more** at the end of a folder's list brings in older mail beyond what is kept on this computer (#4).
- **Search the server** at the end of a search finds mail that isn't downloaded, for IMAP, Exchange, Microsoft 365 and JMAP accounts, and in the browser (#4).

### Changed
- Shared mailboxes are listed as mailboxes of their own, in the folder pane and the calendar. Drag them, or use **Move up** and **Move down**, as with accounts (#1).
- Exchange invitations show their notes as the invitation does, with each link under its words and long addresses shortened. **Edit notes** opens the text box (#1).
- Every dialog fits inside the window and scrolls when it has more (#1).
- **Online only** explains what is sent straight away and what stays on the server (#4).
- Closing with unsent mail or unsaved changes on an online-only account asks first (#4).
- A portable copy asks for each account's password once, when MailCraft starts (#4).
- **File › Options** says where the account list and settings are kept (#4).
- In a narrow reading pane, the date moves under the sender and the header's buttons wrap.
- An account shows as online once its mail is synced. Calendars and contacts follow.
- Connecting tries IPv6 and IPv4 in turn, so a missing route doesn't hold up the account.

### Fixed
- JMAP folders keep as much mail as you chose, not just one request's worth.
- Flag changes, moves and deletes made during a sync show at once and survive quitting.
- Exchange Online mail fetched in batches isn't downloaded again on the next sync, and server search results stay.
- Moving one message window no longer pulls the others to the same place.
- Exchange appointments that share an ID are no longer merged into one.
- An Exchange birthday saved without a year shows as a day and month.
- Hidden Exchange folders stay hidden.

### Security
- What you type into **Search the server** can't change the query sent to the server.

## 0.20.0 - 2026-10-20

*A window on every Windows PC.*

### Added
- **Online only**, per account: none of its mail is written to this computer, and a message downloads when you open it (#4).
- Portable mode: a `portable.txt` or `MailCraft.portable` file beside the app keeps all settings and data in a `MailCraftData` folder there, and nothing in the registry (#4).
- With several accounts, the calendar lists each account's calendars under its address, in its colour and with the name the server gives them (#1).
- A shared mailbox's calendar shows with the mailbox (#1).
- **Join meeting** opens a meeting's Teams, Zoom, Meet or Webex link, and the links in its notes are listed to open (#1).
- **Check for updates** in **File › About** finds the newest build and offers its installer. Nothing installs by itself.

### Fixed
- The window opens on Windows PCs without a graphics card. MailCraft uses DirectX 12, picks a real GPU first and falls back to Windows' software renderer (WARP).
- A window that can't open shows a message with the reason and where the log is.
- A Microsoft 365 sign-in is kept in Windows Credential Manager, however long its token (#5).
- A meeting shown in two calendars opens (#1).
- A window saved maximized opens once, maximized, without flickering.
- Windows return to the screen when their monitor is gone, and shrink to fit a smaller screen.
- Moving a window to a monitor with another scale, and typing with an input method (Korean, for example), work as expected.
- On Linux, MailCraft draws on the desktop's GPU, opens through XWayland when it can so attachments drag in and out, and never freezes on a hung clipboard.
- Intel Macs and graphics cards with lower limits start normally.
- Odd command-line arguments, a broken error stream and quitting on macOS no longer crash the app.
- MailCraft is sharp on high-DPI Windows monitors.
- The Windows installer says MailCraft needs Windows 10 or later.

### Security
- Files handed to other apps (an opened attachment, video frames) go in a folder of your own, not a shared temporary folder.

## 0.19.0 - 2026-10-20

*Shared mailboxes of their own, and SMTP's own sign-in.*

### Added
- The outgoing server can have its own user name and password, kept in the system keychain (#3).
- A shared mailbox's menu has **Send / Receive**, **New folder**, **Remove** and **Add shared mailbox** (#1).

### Changed
- A shared mailbox appears after the account's folders as a mailbox of its own, with its Inbox, Drafts and Sent Items in their usual places (#1).
- Exchange's special folders keep the names the server gives them, such as "Postvak IN" (#1).
- The conversation above a message shows only while **View › Conversations** is on (#1).

## 0.18.0 - 2026-10-19

*Shared mailboxes and the contact card.*

### Added
- Exchange shared mailboxes. Those Autodiscover names open by themselves, and you can add others in **File › Info › Account settings › Shared mailboxes** (#1).
- A contact card: click a name in a message's header to send mail, add or edit the contact, or copy the address (#1).

### Changed
- Removing an account keeps its data file on disk. The **Remove** dialog names the file, and adding the address again starts a new one.

### Fixed
- Enter submits the password, shared calendar and shared mailbox dialogs.

## 0.17.0 - 2026-10-19

*Copy and paste, and public folder contacts.*

### Fixed
- Copy and paste work in every box, including formatted signatures pasted from Word (#1).
- An organisation's shared address books and calendars in Exchange public folders show (#1).

## 0.16.0 - 2026-10-19

*Contacts as Outlook keeps them.*

### Added
- Contacts keep each phone number's kind, separate business, home and other addresses, and a birthday, across vCard, Exchange, Microsoft 365 and `.pst` files (#1).
- The contact form has three email fields, a field per number and one address at a time (#1).
- People lists each account's address books and the shared ones. Click one to show it alone (#1).
- Signatures keep the font sizes and typefaces of pasted text (#1).
- A log file, `mailcraft.log`, in the data folder. It never holds message text or passwords.
- Export and import rules from **Open & Export** and the **Rules** dialog.
- The **Rules** dialog shows one rule per line, with a find box and **Run all**.
- Select several attachments with Ctrl+A or Ctrl+click and save them together.

### Changed
- Sending and automatic replies use a second connection, so they never wait behind a sync (#1).
- Backspace after a recipient removes the whole address.
- Address suggestions are as wide as the longest one, and a click anywhere on a row picks it.
- **File › Options** is laid out as cards of settings with switches.
- **Export** has one list of accounts and one **Export…** button. The file is named after the account's address.
- **Print** asks where to save the PDF, then opens it.
- New mail no longer keeps the Windows taskbar button highlighted.
- Large `.pst` files import and export much faster, use less memory and take less disk space.
- Big folders stay quick to move, delete and mark read in, and repeated searches of a large mailbox are fast.

### Fixed
- Another person's Exchange calendar is editable when its events allow it (#1).
- Public folders on Exchange 2013 and later list and sync their mail. A server that won't list them says why (#1).
- French text never reads "de Anna" (#1).
- One bad message no longer stops the rest of a folder downloading on Exchange and Microsoft 365.
- Replied and Forwarded marks are set when the reply is delivered, not when it is queued.
- A search like `from: anna` (with a space) works.
- Deleting a contact asks first.
- vCard 2.1 values in quoted-printable show decoded.
- On Gmail, permanently deleting a message in a label folder removes it.
- The To and Cc of a reply end with `; `, so typing adds a new address.
- Clicking an address suggestion keeps the cursor in the box.
- Ctrl+A in a message's text selects only that text.

## 0.15.0 - 2026-10-18

*What testers asked for, again.*

### Added
- Text pasted from Outlook, Word or a web page keeps its bold, italics, colours and links (#1).
- The signature editor is a full message editor. Signatures keep their formatting in new messages, replies and **Insert signature** (#1).

### Changed
- Side by side shows each calendar in its own pane, under a heading in its colour with × to hide it. Clicking in a pane adds the event to that calendar (#1).
- **Select Names** opens with the address book you chose under "Always open with this address book" (#1).

### Fixed
- Another person's Exchange calendar opens editable when you are its editor (#1).
- Public folder contacts and calendars on Exchange 2013 and later can be read. One that still can't says why (#1).
- Long names and addresses in **Select Names** end with "…" instead of overflowing (#1).

## 0.14.0 - 2026-10-17

*What testers asked for.*

### Added
- Choose how much mail each folder keeps: the newest 1,000, 5,000, 20,000 or all, with 40 to 1,000 downloaded ahead (#1).
- Choose when mail is marked read: when shown, when you move to another message, or only when opened in its own window (#1).
- **Insert signature** and **Signatures…** on the **Message** ribbon (#1).
- The search box searches People and Notes when you are there (#1).
- Calendar events show in their category's colour. Categories sync with Exchange, Microsoft 365 and CalDAV (#1).
- Tick calendars to show or hide them, view them side by side in Day and Week, and see up to six months in the side pane (#1).
- People's List view lets you choose columns and sort by any of them (#1).
- An organisation's shared contacts and calendars in Exchange public folders, read-only (#1).
- Exchange automatic replies can send a different reply outside the organisation (#1).
- Notes arrange by title, colour or last change (#1).
- **Open folder** opens it in the file manager (#1).

### Fixed
- A Hotmail account added with a password switches to **Sign in with Microsoft** from the status bar and keeps its mail (#1).
- "User is authenticated but not connected" says what to do. **Use Microsoft Graph** moves the account off IMAP (#1).
- Automatic replies on an account that can't connect no longer leave it at "Connecting…" (#1).
- Generated HTML mail keeps its pictures, style sheet colours, headings, column widths and stripes (#2).
- Typing in search stays quick on a big mailbox (#1).
- Exchange folders sync past the first 1,000 messages (#1).

## 0.13.0 - 2026-10-17

*The open gaps, again.*

### Added
- Outlook.com, Hotmail and Live accounts sign in with Microsoft in test builds.
- Edit another person's calendar when its owner made you an editor. Calendars you can't edit say "read-only".
- HTML mail shows background pictures behind tables, cells and boxes, held back like other web pictures.
- HTML boxes placed with `position: absolute` sit over the box that holds them.
- Importing a `.pst` file brings in its notes, with their colours and times.
- Right-click Arabic or Hebrew text to copy the line, or the whole message.
- Move accounts up and down from their right-click menu, or drag them.
- A POP account can start from a `.mailcraft` file.
- **File › Open a data file** opens a `.mailcraft` file as a mailbox of its own, without changing the file.

### Changed
- MailCraft opens on the first account's Inbox. All Inboxes stays in Favorites.
- The status bar sums up all accounts ("All folders are up to date", "2 accounts disconnected"). Hover to see each account.
- The folder pane remembers which of Favorites, accounts and folders you collapsed.

### Fixed
- Text in HTML mail takes its cell's or table's colour, so white text on a dark band stays white.

## 0.12.0 - 2026-10-17

*Microsoft sign-in that works.*

### Changed
- Accounts that sign in through the browser say **Sign in again**, not **Password needed**, and the link opens the sign-in.
- An account the server refuses stops with the reason until the next **Send / Receive**.
- Each Microsoft sign-in asks for the permissions it needs. An SMTP refusal says whether the sign-in covered sending.

### Fixed
- On Windows, **Sign in with Microsoft** opens the sign-in page, and links in mail open.
- Sent Items shows the sent copy a few seconds after sending on Exchange and Microsoft 365.
- Small dialogs open at their own size, with their buttons in place.
- Opening People no longer widens the folder pane everywhere.

## 0.11.0 - 2026-10-17

*Notes, previews and Exchange beyond the mailbox.*

### Added
- Click an attachment to preview it in the reading pane: pictures, PDF pages, text, attached mail, and Word, Excel and PowerPoint files (#1).
- Notes (Ctrl+5): coloured sticky notes, kept in an Exchange mailbox or on this computer (#1).
- People in three layouts: a list with an a–z index, business cards and a phone list (#1).
- Open another person's Exchange calendar by address, read-only (#1).
- Public folders on on-premises Exchange (#1).
- A Dutch interface.

### Changed
- Adding an Outlook.com or Hotmail account says it needs Microsoft sign-in instead of asking for a password again and again (#1).

### Fixed
- The list's filter tabs no longer run into **By Date** in long-worded languages (#1).
- The status bar no longer counts mail while People or Calendar is open (#1).

## 0.10.0 - 2026-10-16

*Windows, folders and rules as Outlook keeps them.*

### Added
- The main window reopens where it was and at its size, maximized if it was. Message and compose windows open where the last one was.
- Run a new rule at once on the mail already in the Inbox, or use **Run now** in the rules list.
- Attachments as small chips with a thumbnail, name and size. Double-click opens one; ▾ or right-click offers **Preview**, **Open**, **Save As** and **Save All Attachments**.
- Notifications for new mail that rules filed into folders.
- A **Search** tab while searching: where to look, **From**, **Sent To**, **Subject**, **Has Attachments**, **Unread**, **Categorized**, **Flagged**, **Important** and **Close Search**.
- **Follow Up**: flag for today, tomorrow, this week, next week or no date, **Mark complete** and **Clear flag**. The reading pane shows when it's due, in red once overdue.
- The reading pane shows when mail came with high or low importance.
- **Delay delivery** holds a message in the Outbox until its time, while MailCraft is running.
- **Replies to** sets a Reply-To address while writing.
- **Quick print**, **Ignore** and **Find related** on a message; **Reply with meeting** and **Forward as attachment** under the reading pane's **⋯**.
- **Delete all…** and **Properties…** on a folder; **Open file location** and **Account settings…** on an account.
- Rows, buttons, tabs, calendar events and colour swatches light up under the pointer.

### Changed
- **New folder** in the account menu starts in the folder that is open.

### Fixed
- A folder made in the Inbox shows under the Inbox.
- **Open** on an attachment opens it, and on Windows apps such as Acrobat accept the file.
- A message window's attachments act on its own message.
- Japanese, Korean and Chinese show in the language menu and in mail whatever the interface language.
- "Pin", "Unpin" and "Always move to Focused/Other" are translated.

## 0.9.0 - 2026-10-15

*Outlook's everyday commands.*

### Added
- Choose how long POP mail stays on the server: kept, removed after a number of days (14 for new accounts), or removed once deleted from Deleted Items.
- **View › Arrange by** (date, sender, recipient, subject, size, flag, attachments) and **Reverse sort**, with headings.
- A **Send / Receive** tab with **Send all** and **Work offline**, which is kept across starts.
- Undo (Ctrl+Z) for moves, deletes, flags and categories.
- **Filter** by categorized, important, or to or Cc me.
- **Ignore** a conversation, under **Delete**.
- **New items** for an event or a contact.
- A folder pane that minimizes.
- **File › Save As**, **Save Attachments**, **Print** and **Exit**.
- High and Low importance and read receipts when writing. The list shows incoming mail's importance.

### Fixed
- A draft that can't be saved while offline is kept and saved when the account connects.
- Grouped list headings are spaced as headings, not messages.
- The ribbon keeps its last buttons in a narrow window.

## 0.8.0 - 2026-10-15

*POP and Outlook's own data files.*

### Added
- POP accounts (RFC 1939, over SSL/TLS or STARTTLS). New mail downloads once into folders on this computer and leaves a copy on the server unless you choose otherwise.
- **Use an existing data file**: a new POP account can start from an old `.pst` file.
- Each account keeps its mail in one data file (`.mailcraft`) in **Documents\MailCraft Files**. An opened `.pst` file gets one too.
- **File › Options › Data files** shows the data files' folder and moves it.
- Each account's mailbox settings show its data file and save a copy as a `.pst` file.

### Changed
- Mail kept by earlier versions moves into the data files by itself.

### Fixed
- A `.pst` file that Outlook has open says so.
- Opening an `.ost` file (Outlook's offline copy of a server account) explains what it is.
- Importing a `.pst` file skips the RSS Feeds folder and an empty Outbox.

## 0.7.0 - 2026-10-15

*The open gaps closed.*

### Added
- HTML mail laid out with CSS (flexbox, grids, inline-block, floats) keeps its columns.
- On-premises Exchange calendars and contacts, both ways, through Exchange Web Services (EWS).
- Autodiscover finds the Exchange server from the email address.
- In Arabic and Hebrew the window mirrors, with folders on the right and the reading pane on the left.
- Right-to-left paragraphs line up on the right while you write.
- Importing a `.pst` file brings tasks with a due date into the calendar.
- Exporting to `.pst` also writes the calendar and contacts beside it as `.ics` and `.vcf` files.

## 0.6.0 - 2026-10-15

*Outlook's reach.*

### Added
- New mail can open in a window of its own, and replies pop out.
- A tall **Send** button beside **From**, **To** and **Cc**. **To** and **Cc** open the address book (**Select Names**).
- Formatting from the **Message** ribbon or keyboard shortcuts: font size, bold, italic, underline, strikethrough, colours, lists, alignment, indent, links, pictures and undo.
- Replies keep the original message under a rule, with its own formatting.
- "Want to save your changes?" when closing an unsaved message.
- Exchange Online calendars and contacts, both ways, through Microsoft Graph.
- On-premises Exchange accounts through Exchange Web Services (EWS), signed in with NTLMv2 (Windows sign-in) or Basic: folders, sync, drafts, sending and automatic replies.
- Importing a `.pst` file brings in contacts and calendar items, recurrences included.
- Export any account's mail to a new `.pst` file.
- Newsletters laid out with tables keep their columns, widths, coloured cells and centring.
- Arabic and Hebrew: right-to-left text in mail and the interface reads in order, joined and aligned right, with a fallback font when needed.
- In the browser: push, and a cache kept between visits.
- Video previews without ffmpeg, from the system's own thumbnailer.
- Drag attachments out on Linux and the BSDs under X11.

## 0.5.0 - 2026-10-15

*Beyond mail.*

### Added
- Open a message in its own window with a double-click or **Open in new window**.
- The **File** page: account information, server settings, automatic replies, space used, emptying Deleted Items and Junk, rules, **Options** and **About**.
- A calendar with day, week and month views, recurring events and time zones, synced with CalDAV.
- Contacts, synced with CardDAV.
- Meeting invitations by mail: accept, tentative, decline, and invite guests.
- **File › Open & Export** opens a `.pst` file as a mailbox on this computer.
- Exchange Online accounts through Microsoft Graph: folders, sync, drafts, sending and automatic replies.
- A web build: the same window in the browser for JMAP accounts, with a `?sample` mailbox.

## 0.4.0 - 2026-10-15

*Media and Craft integration.*

### Added
- Attachment previews: text, attached mail, a PDF's first page and video frames (through ffmpeg).
- Open an attachment in the Craft app for its file type, or **Save all**.
- Drag attachments out to other apps on Windows and macOS.
- **Print** (through a PDF), **Save as PDF** and **Save as `.eml`**.
- Ten interface languages besides English: Spanish, French, German, Brazilian Portuguese, Italian, Russian, Japanese, Korean, and Simplified and Traditional Chinese. MailCraft follows the system or your choice in **Options**.
- JMAP accounts (RFC 8620/8621), with push.
- HTML mail keeps its colours and tables, adjusted for contrast.

## 0.3.0 - 2026-10-15

*Modern sign-in.*

### Added
- Sign in to Gmail and Microsoft 365 through the browser (OAuth 2.0 with PKCE), for IMAP and SMTP.
- Account settings found from the email domain (autoconfig, the Mozilla ISPDB, MX and SRV records).
- Faster folder opening on servers with QRESYNC, which sends deletions and flag changes as the folder opens.

## 0.2.0 - 2026-10-15

*Daily driver.*

### Added
- Create, rename, move and delete folders, and choose your favourites.
- An Outbox: mail that can't go out waits and sends on reconnect. A server's refusal stays with its error.
- Recipient suggestions from mail you have seen.
- Desktop notifications for new mail, and the unread count in the window title.
- Colour categories, pinning, snooze, inbox rules and Focused/Other.
- HTML mail shown formatted. Pictures from the web load only when you ask, remembered per sender.
- Rich-text writing with inline pictures, sent as HTML with a plain-text copy.

## 0.1.0 - 2026-10-15

*Read and send.*

### Added
- IMAP and SMTP accounts, with passwords in the system keychain.
- A unified inbox.
- A local cache that fetches only what changed, gets new mail as it arrives, and sends offline changes when you reconnect.
- A message list grouped by date, with conversations, filters, multi-select and drag to a folder.
- A reading pane on the right, at the bottom or off, with the conversation and picture previews.
- Reply, Reply all and Forward, with quoting and signatures.
- Drafts.
- Delete, archive, junk, move, flag and mark read.
- Offline search, with field operators such as `from:`.
- Dark mode.
- A command-line client.
- A sample mailbox.
- Packages for Windows, macOS and Linux.
