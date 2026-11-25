# Changelog

Every version of MailCraft that testers can install, newest first. What's still missing is in
[known gaps](docs/gaps.md); what comes next is in the [roadmap](ROADMAP.md).

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and MailCraft uses
[Semantic Versioning](https://semver.org/). Before 1.0, any version may change anything. Each
version is a milestone, named in italics under its heading. Numbers like (#11) are tester issues
on GitHub.

## 0.42.0

*What testers met in their first days with 0.41.*

Thanks to kzoli and kovadam69 for the Hungarian translation and dates written year first (#198).

### Added

- **Snooze** beside a message's Reply, Reply all and Forward (#274), and **Choose a date…** in every Snooze menu: the message comes back on a day and at a time of your own.
- **Archive** can be among the buttons a message in the list shows under the pointer (#274): **File › Options › Reading › Buttons on a message in the list** picks up to four of Delete, Archive, Flag, Mark read or unread and Pin. Delete, Flag and Mark read until you choose.
- **Recipients as chips** in To, Cc and Bcc (#269): each person by name (the address on hover), × to remove them. A click selects one, Ctrl+click adds one and Shift+click those between; with nothing typed, ← and → go along them (Shift+arrow selects more) and Backspace or Delete removes those selected. Dragged, they move to another box or to another place in theirs; a right-click menu moves, copies, edits or removes them. An address that isn't one is outlined in red, a list pasted a person a line becomes a chip each, and a box you aren't writing in shows one line of them and how many more. What's sent is unchanged.
- A message dragged from the list onto a message being written is attached to it, as an `.eml` (#270), as Outlook does; several selected go together. The drag says **Attach** while it's over the message, in the reading pane or a window of its own.
- Tables pasted into a message stay tables (#268): cells copied in Excel, or a table copied from a web page or another message, show in the message as they will be sent (their rows and cells, lines, colours and alignment) and go out as a table, with each row a line of text in the plain text. Backspace after one takes it out; a draft keeps its tables. Its cells can't be edited in MailCraft yet.
- **Text size**, in **File › Options › General** (#267): everything in the window larger or smaller, from 80 % to 200 %, kept for the next time MailCraft starts; **Ctrl** and **+** or **−** change it too, and **Ctrl+0** puts it back. Windows keep their size and place on the screen as it changes.
- **Font size for new messages**, in **File › Options › Mail** (#267), beside the font: new messages, replies and forwards are written in it, and text you set a size on keeps its own. A signature is shown in the font for new messages while you edit it, as it goes in them.
- Accounts signed in with Google sync their calendars and contacts too (#262, #263), in a copy of MailCraft given a Google client of your own ([docs/oauth.md](docs/oauth.md)). One signed in before is told to sign in again, to allow them.
- Everything with one person in one place (#178): **Everything with them…** in someone's card above a message, on their name's right-click menu and in the People pane, and **Find related › Everything with sender…** on a message. It shows the mail you and they have exchanged in every account, by conversation and with the folder each message is in; the files and links in it; the meetings they organise or are invited to; and the others on that mail, each a click away, with an overview of a few of each first. A search narrows every part, and a box at its top finds someone else. Phone numbers in their signature are offered for their contact, never put there by themselves.
- A new account starts with the calendar and contacts servers its provider's settings name (#176), as Thunderbird's autoconfig files can (a Nextcloud's, for one); **Calendar server…** shows the servers MailCraft found by itself, where its fields looked empty.
- Tasks sync with a CalDAV server's task lists (#201): Nextcloud's Tasks, SOGo and Mailcow, Stalwart, Radicale. Made, changed, marked complete or deleted here or on another device, they follow; **Task lists** in the Tasks side pane shows every task or one list's, and the task form has **Task list** where there are several (a new task goes in the list shown). Tasks kept on this computer before go to the server's task list. A change made here to a task changed elsewhere meanwhile gives way to that one, and you're told. A list you may only read is shown, not changed.
- Colleagues' free/busy time from a calendar server that schedules meetings (#242), as from Exchange: an account whose CalDAV server does (Nextcloud, SOGo and Mailcow, Stalwart) has **Open shared calendar…** and **New calendar group…** in the Calendar, and each colleague's busy, tentative and away times show among your events, asked of your server while their calendar is shown. They need to have let you see their free/busy time on that server.
- Rules can run on an IMAP server (#243), as an Exchange account's do: **New rule on the server…** in Rules writes them in Sieve through ManageSieve (Dovecot, Mailcow, Cyrus, Stalwart…), so mail is sorted as it arrives even while MailCraft is closed or your phone reads it first. They're kept beside your automatic replies, and filters you keep elsewhere (a webmail's) run first and are left as they are; Rules says when that program's filters have taken over. A rule for mail you send stays in MailCraft.
- One occurrence of a repeating event can be changed, moved or answered on its own (#241), as Outlook's "this occurrence": opening one asks **Open this occurrence** or **Open the series**, and dragging one in the calendar moves it alone. It stays part of the series (on CalDAV servers in the series' own item; on Exchange the series' own occurrence), the others as they are. The guests of your meeting are sent that occurrence alone; as a guest, your answer to it goes to the organizer, the series keeping its own.
- Hungarian as an interface language (#198): **File › Options › General › Language › Magyar**, and a Hungarian system picks it.
- Polish as an interface language (#226): **File › Options › General › Language › Polski**, and a Polish system picks it.
- Mark all as read can be undone (#248), as a move, a delete or a flag can: Ctrl+Z, or Undo on the top bar, marks the mail it marked unread again. Unread mail on a server that wasn't on this computer yet, which the server marks read itself, stays read.
- People can be sorted by last name (#244): **Sort by › Last name** on the People ribbon. Names still read as they do (Zoe Park), filed under their last name's letter in the A–Z index; a company's card under its name.
- Dates can be written year first, with dots (2026.10.28), as Hungary writes them (#198): **File › Options › General › Dates**; a date typed in a form is read so, and MailCraft's own 2026-10-28 still is.
- Invitations in the Inbox… in the calendar (#237) finds the meetings waiting for your answer there that your calendar hasn't got (mail from before the account was added here, which nothing took in): still to come, not called off since, not answered already, each in its newest version. Those you tick go into the calendar unanswered, on its server too, with no answer sent.
- Forward on a meeting you're invited to (#234), on its invitation and in its form: the organizer's invitation goes to the people you name, with them among its guests, so their program shows it as a meeting to answer; your own answer stays. Their answer reaches the organizer, whose copy offers to add them. A meeting whose organizer doesn't allow forwarding isn't forwarded.
- Delegate on a meeting you're invited to (#254), on its invitation and in its form, as SOGo's web mail offers: someone else goes in your place, people suggested as you type who (in Forward too). They're sent the organizer's invitation as your delegate, and the organizer your answer naming them; your copy stays in the calendar, shown as free. A series is delegated whole, or the occurrence its form was opened on. As organizer, the guest list says who handed their place to whom, and the delegate's answer is recorded as it arrives. Checked with SOGo (Mailcow, both in MailCraft and in SOGo's web mail), Nextcloud, Stalwart and Radicale. An account with calendars only doesn't delegate: its server would have to tell the one going instead, and Nextcloud and Stalwart don't.
- A meeting's guests are typed as a message's To is (#253): **Invite** in the event form suggests people as you type, Ctrl+K checks the names typed (someone who could be several people is asked about under the box), and **Invite…** opens the address book. The guests of the meetings in your calendars are suggested as people you write with, though no mail went to them (a meeting made in a web calendar, or organized by someone else).
- An organisation's profile (#245): one file, or Group Policy and Intune on Windows, or a configuration profile on macOS, gives everyone the same MailCraft, each setting a default or locked (greyed out, **Set by your organisation**). The look (theme, accent, top bar, language), the organisation's name and logo in **File › About**, the servers of its mail domains (an address and a password are then all an account needs), which ways servers are looked for, accounts of its own domains only, its own Microsoft 365 registration, a signature filled from each person's details (**File › Info**, and Exchange's directory) and a notice under every message sent, encrypted connections only, no certificate trusted by hand, web pictures that never load by themselves, AI agents off, read only or asking before each command, and any other setting. Several institutions in one file, by mail domain. **File › Info** says what it sets and what it couldn't take. `packaging/windows/policy/` has the Group Policy template; see [docs/organisations.md](docs/organisations.md).
- MailCraft updating itself can be turned off, or kept to releases (#207): **File › Options › General › Update MailCraft by itself**.
- AI assistants on this computer can use MailCraft (#255): `mailcraft-cli mcp` is an MCP server for Claude Desktop, Claude Code and other clients, off until **File › Options › AI agents › Let AI agents use MailCraft**. They read the mail, calendar, tasks and people as the open window has them (or the data folder, MailCraft closed), find mail as the search box does, read a message without marking it read, see what the window shows and take a picture of it. With **Let AI agents ask to run commands** on, they run MailCraft's commands, each asked about in a small window of its own (Allow, Always allow, Deny) unless you always allow it. An account can be kept from them, and **File › Info** lists what they read and ran. See [docs/mcp.md](docs/mcp.md).
- **Always allow every command**, in **File › Options › AI agents** (#281): an AI agent's commands run without asking, but sending mail or invitations to other people (a message, a reaction, a meeting's invitations, changes or cancellation, an answer to one), which is still asked about. An event with no guests mails nobody, so it runs.
- Propose new time, on a meeting invitation (#238), as Outlook's: the time you'd rather have goes to the organizer, with Tentative or Decline beside it if you like. As organizer, a proposal shows who proposes what, with Accept proposal (the meeting moves, and every guest is sent it) and Decline proposal (the guest is told). An organizer who takes no proposals isn't offered one.
- People's photos show where their initials were (#223): a contact's own photo, as the card in your address book has it, and on on-premises Exchange the photo your organisation has for them, in the message list, above the message, on the contact card and in People. A photo kept at a web address isn't fetched.
- Rules can be for mail you send (#202), as Outlook's can: Applies to › Mail you send in the rule editor. As each message's copy reaches Sent, the rule files it in a folder, archives, categorizes, flags or deletes it; Run now runs it on what's in Sent already. Mail you move into Sent yourself is left alone.
- Check Names, as Outlook's (#211): Ctrl+K in To, Cc or Bcc, or Check Names beside Address Book, puts in the one person a name typed is, from the people you know and the organisation's directory (Exchange, Microsoft 365); a name that could be several people offers them to pick from, and one found nowhere is said. A name the directory is still being asked about is checked again once it answers.
- On an IMAP server that offers NOTIFY (Dovecot, so Mailcow too), mail the server's own rules file into another folder shows as it arrives (#232), not at the next look: only the Inbox was watched. A server without it (Gmail, Exchange…) is looked at as before.
- Guests' answers to your meetings are recorded in the calendar as they arrive (#236), as Outlook records them: Record guests' answers in File › Options (on). One from someone not on the guest list, or to an earlier version, still waits for Record the answer, and an answer's card says when it's recorded (the button said nothing once clicked).
- An event can have several reminders (#224): each comes in turn, so Google's "an hour before and 2 minutes before" reminds twice (only the first reminded), and the event form shows each, with Add reminder and a button to take one away. Not on an Exchange calendar, which keeps one an event, as Outlook does.
- A reminder of an online meeting (Teams, Zoom, Webex, your own rooms…) has Join meeting beside Open Item (#239).
- A calendar group's menu has Add people…, to put more people in it (#209): it had only Delete.
- Picking people for a calendar group or a shared calendar shows them as names, in rows that grow and then scroll, each with its × (#210): the box kept one size and showed names and addresses.
- The Address Book shows people's job titles and departments where the organisation's directory gives them (Exchange, Microsoft 365), as Outlook's does (#213).
- In the calendar's Day, Work week and Week, dragging over the hours makes a new event that long (#212): a click made half an hour's, and a longer one needed its end changed in the form.
- The Schedule view has Only work hours: its time line over the hours anyone shown works, from the earliest start to the latest end, as Outlook's Scheduling Assistant can show (#206).
- On Exchange, a colleague's calendar opened here (on its own, or in a calendar group) is named as the directory names them, in the calendar list and the Schedule view (#222): it was labelled with their address.
- MailCraft can start as you sign in to the computer (#87): **File › Options › General › Start MailCraft when you sign in**, on Windows, Linux and the BSDs, and a Mac. With **Start MailCraft in the notification area** on as well, it starts there, checking for mail with no window coming up. Not offered in a portable copy (it leaves nothing on the computer), nor in the Flatpak or a Snap.

### Fixed

- Clicking MailCraft's notification of new mail brings MailCraft forward with the newest of that mail open (#273), from a minimized window or the notification area too; it did nothing. On Windows, one clicked later in the notification centre does too, as the installer registers MailCraft's own `mailcraft:` links for it (a portable copy's, while it shows: [gap 26](docs/gaps.md#26-a-portable-copys-notifications-in-the-notification-centre)); on Linux and macOS, a click on it, in the notification list too.
- A mailing list's **Unsubscribe**, above its message, is a link (#272): it opens the list's own page for leaving it when the list gives one (its address otherwise), with the whole address on hover and **Copy link** on a right-click. It was a line of text cut off at the pane's edge.
- A blank line inside a heading of an HTML message (paragraphs apart by an empty line in a large title, as some survey mail has them) shows as a blank line, where it showed `###` (#271).
- A table whose cells share their lines (`border-collapse`), as web pages and pasted tables have them, is drawn with one line between two cells, as a browser draws it, where it was twice as thick as the table's others: in mail read, a reply's original and a pasted table (#268).
- A table cell's italics, and a colour written by its name (yellow, navy, teal…), show as written, in mail read, a reply's original and a pasted table (#268): a total Excel sets in italics was upright, and its yellow headings were white.
- A contact deleted here is deleted on the server only while its contacts are found there, and an event or task only while its calendars are: on a server holding both, where one of them wasn't found (or Google refused it), the deletion was sent anyway. It waits until they're found. Checked on Radicale, Nextcloud, Stalwart and SOGo.
- A calendar or contacts server that wasn't found, or that Google refused, is looked for again a day after it was looked for, where an account syncing more often than once a day never looked again until its servers or password changed. Checked on Radicale, Nextcloud, Stalwart and SOGo.
- A change made on SOGo (Mailcow) to a calendar, contacts or tasks in the very second MailCraft read it is read at the next sync (#201): SOGo's collections say when they last changed, in seconds, and such a change was missed until something else changed there.
- An event or task deleted here that was changed on the server meanwhile comes back, and you're told (#201), where its deletion was tried again at every sync and refused each time.
- An on-premises Exchange account signs in where Exchange's Extended Protection is on (#249), as it is by default since Exchange 2019 CU14: MailCraft's NTLM and Kerberos sign-in now carry the secure connection's binding, and without it Exchange refused them, which MailCraft took for a wrong password and asked for it again and again.
- On a Windows PC with two GPUs, MailCraft draws its window on the one Windows chooses for it (Settings › System › Display › Graphics: the power-saving one unless you chose otherwise), not always the dedicated one (#246). `WGPU_POWER_PREF=high` still asks for the dedicated GPU.
- The message list's Pinned heading is in your language: it showed in English in every language (#198).
- The labels of the event, task and contact forms no longer run into their fields (#198): the labels' column is as wide as the longest label in your language (Hungarian, German, Spanish, Polish, Russian, Slovak and Arabic had some too long for it, and English its Repeats every).
- Compact now gives back what File › Info says can be reclaimed, and the room stays given back (#221): a data file under 4 GB is copied into a new file (compacted in place, its database doubled it again at its next write, so it grew: 1.5 GB became 1.8 GB), and what the card offers is only what compacting gives back for good (it offered room the database fills again at once, then said "Nothing to give back").
- Mail moved to another folder no longer leaves a copy of itself behind in the data file (#221): each move kept the moved messages' old copies for good, in no folder, so the file only grew (2,453 messages moved to Deleted Items and emptied kept their 2 GB). Any account, Exchange, IMAP or a data file kept here.
- A message written in Calibri, or another font with built-in pictures of its letters for some sizes, shows every letter (#166): most of Calibri's letters were drawn in white, whatever the text's colour, and didn't show on the page.
- On Exchange, a repeating meeting you accepted shows as accepted when it was sent to another of your addresses (#144): Exchange's answer was put only on your account's own address, so a meeting sent to an address you added to the account, or to the name you sign in with, kept waiting for your answer and was drawn as one.
- On Exchange, a single occurrence of a repeating meeting moved or cancelled shows as it is (#204): MailCraft kept the series' usual time on that day, and a cancelled occurrence stayed. A moved one is read with its own time, subject and guests, in place of the occurrence it stands for.
- On Exchange, a repeating meeting with every occurrence deleted no longer brings a red notice at each start (#229): Exchange won't give such a series, and MailCraft counted it as an appointment it couldn't read. It has nothing to show, and is left out quietly.
- On Exchange, a task deleted in MailCraft is deleted on the server too (#230): Exchange refused the deletion ("AffectedTaskOccurrences attribute is required for Task items"), the task stayed there, and the refusal came back as a red notice at each start.
- The To-Do Bar's appointments are from your own calendars (#205): a colleague's calendar opened here put their events among yours, a holiday of theirs under Today, and made their days bold in the bar's month. They stay in the Calendar.
- On Exchange, the people of a new calendar group, or a colleague's calendar opened, show in seconds (#208): they were read with the next whole calendar pass, minutes later on a big mailbox, after any pass under way. Their free/busy times are asked at once too, not after a pass.
- On Exchange behind a load balancer, mail an Inbox rule files comes with the next look for new mail, every time (#145): the server's word on which folders changed was kept by one of its servers, and a look that reached another was told it wasn't found; what came meanwhile waited for the next full sync. MailCraft now asks to keep to the server that has it, as Exchange wants.
- Send / Receive's progress shows an Exchange account too (#231): its folders as they're read (n of m) and its Outbox, as an IMAP account's. The window said nothing had been sent or received while the status bar said its folders were being updated.
- A search's heading above the message list is cut short where it's too long, the whole of it on hover (#200): a long query widened the list as far as it goes, and the heading still ran past its edge.
- In Rules, a rule made in another program is cut short before its row's buttons (#214): its note ran under ↑, ↓ and Delete, and past the dialog's edge. The local rules and Categories were the same.
- The read, flag and delete buttons on a message's row act on that message alone (#216): they selected it too, and what was selected and open before was lost.
- The subject above a message can be selected and copied, as its text can (#217).
- Follow Up › Custom… is as tall as its fields, OK and Cancel at its bottom (#220): it opened as tall as the screen, its buttons halfway down. The contact form's buttons were the same.
- In Russian, the message list's weekday headings start with a capital (Понедельник, under Сегодня and Вчера), and a month after its day number takes the form it has there (четверг, 17 декабря 2026); in Slovak too (17. decembra) (#215).
- In the dark theme, a pale highlight behind text stays (#227): Jira's pale pink and green marks of what changed were dropped with the grey boxes, and the old and new values read as one. Grey and white boxes still go.
- An Apple Mail link card (a shared album, a web page) shows its picture and opens its link when clicked, with Open link and Copy link on its menu (#225): only an empty box was drawn. Pictures written into a message's HTML show as those sent with it do, and what a link around a box holds opens it.
- In the To-Do Bar, an appointment's "In N min" sits above its time, and its title keeps all its room (#195): at the row's end it hid the end of the title.
- An invitation's card says how it repeats (Repeats · Every week), and draws the invitation on a line of its own above your events, its name and theirs no longer written over each other (#194).
- The guests of a repeating meeting told that one occurrence is called off are told which, by its date and time (#235): the message read as if the whole series were cancelled.
- On Windows, `mailcraft --help` and `mailcraft --version` print in the console MailCraft was started from (#257), where they printed nothing.
- Where MailCraft's window can't open on Windows Server 2016 or a Windows 10 older than version 1709 without a graphics card to draw with, as over Remote Desktop (#256), the message says that Windows' own software renderer is too old to draw MailCraft there, and no longer suggests the setting that draws with it.
- What **Compact** says it gives back is what it gives back again, and a compacted data file no longer grows at its next write (#259). Data files are kept with redb 4.6.0, which no longer opens a data file damaged in its last save as an empty one.
- What `mailcraft-cli` prints can't drive the terminal: a control character in a subject, a sender's name, a message, a folder's name or a server's error (an escape that recolours, retitles or clears it, a lone carriage return that rewrites a line) is shown as an escape, as `\u{1b}`.
- A damaged system font no longer stops MailCraft as it starts: a font file for its interface, Chinese, Japanese and Korean, or Arabic and Hebrew that's empty, cut short, too large or not a font is passed over for the next.
- On Windows, the Explorer, PowerShell and rundll32 that MailCraft starts (to open a folder, a file or a link, and to install an update) are Windows' own, by their full path: a program of the same name beside a portable MailCraft no longer runs in their place.
- On a Mac, data files kept on a network share (SMB), and Outlook data files exported or changed there, are written: the full flush the Mac asks for is refused there, and every change failed; a plain one is made then.
- The browser version's sample server settings send `Cross-Origin-Opener-Policy: same-origin`, so a page opened from MailCraft can't reach its tab, and Apache's sends `nosniff` as the others did.
- Ctrl+E while typing in the search box selects the search, so what's typed starts a new one, and Rename folder and Edit category open with the name selected, so typing replaces it (it went on the end).
- A double-click counts as the desktop counts one, up to half a second between the clicks: a relaxed double-click on a message was taken for two clicks, and it didn't open.
- Double-clicking a one-letter word in a message you're writing (the "a" in "Take a walk") selects that word alone: it selected the word before it too, and typing replaced both.
- On Windows and on a Mac, a window opens inside the part of the screen the taskbar, the Dock and the menu bar leave free: one whose title bar was under a taskbar along the top comes down below it, and one too big is made as big as that room (it was kept a fixed margin from the screen's edges, too little beside a tall taskbar and more than needed beside one that hides).
- A list or a pane with more than it shows has a thin scroll bar at its edge while the pointer is elsewhere too, beside what it scrolls: it showed only under the pointer, so nothing said there was more.
- The first start after MailCraft updated itself says so in a bar, or, when the update didn't install, that it's offered again (it said nothing either way).
- On Linux and the BSDs the top bar is the window's title bar, as on Windows: it moves the window, and minimize, maximize and close are at its end, with no second bar from the desktop above it. **File › Options › General › Use the system title bar** keeps the desktop's.
- On a Mac, ⌘A, ⌘C, ⌘V, ⌘X and ⌘Z work in a file dialog's fields (a Save As name, an Open dialog's search): MailCraft's menu bar had no Edit menu for them to go through, so ⌘A did nothing there.
- On Windows, MailCraft and its tests no longer load OpenGL, which they don't use, as they start: each start and close is about 65 ms quicker.
- On Windows, an attachment or a note's file opened again while its app still has it open (a document in Word) opens again, its app coming to it (#260): MailCraft said the file was "being used by another process". Another file of the same name, from another message, opens beside it.

## 0.41.0 - 2026-11-20

*What testers met in their first days with 0.40.*

Thanks to Peter Páleník for reviewing the Slovak translation (#133, #137).

### Added

- Send / Receive shows how it goes, as Outlook does for POP and IMAP accounts: at its end, in place of the accounts' "Updating…", the status bar says which message is downloading and how much is left ("Downloading: 10 of 158 (27.6 MB of 210 MB)", "Sending: 1 of 3", an IMAP account's folders one by one), and clicking it, or **Send / Receive › Show progress**, opens a small window you can move anywhere: a line for each account's sending and receiving, how far each has come (what's left on hover), and why any failed. **Cancel all**, or a task's own stop button, stops between two messages, keeping what came; the next Send / Receive goes on from there. With **Open this window when Send / Receive starts** (off, as in Outlook), F9 opens it too, and it closes once all is done well, or stays open when something failed. Exchange, Microsoft 365 and JMAP accounts, which sync on their own all the time, aren't in it.
- An IMAP or POP account's email address can be corrected in **Server settings**, as Outlook lets it be: one mistyped as the account was added was the sender of every message, which the server turned away, and only removing the account changed it.
- An Exchange account's **File › Mailbox settings** shows how much space the mailbox takes and its limit, as Outlook does (#143): it said the server doesn't report how much space is used. With no limit set, it shows the space used.
- An Exchange account's own rules on its server show in **Rules**, under MailCraft's (#139): turned on or off, moved up or down, deleted, and changed where MailCraft can say what they do (a word in the sender, recipients or subject, attachments; moving, deleting, marking read, a category), and new ones made there, which the server runs even while MailCraft is closed. A rule made in another program that does more is kept as it is; Outlook's own copy of the rules is removed only when asked.
- **Undo Send** (#135): in **File › Options › Mail**, a message you send can wait 5 to 30 seconds in the Outbox before it goes, and a bar at the bottom of the window counts down with **Undo**, which opens it again as you wrote it. Off as it starts; closing MailCraft lets a waiting message go at once.
- A conversation shows your own replies from Sent Items (#148), as Outlook's do: in its list in the reading pane, by date with the others, dimmed and with Sent Items' name, a click opening one in its own window. Deleting or moving the conversation in the Inbox leaves them where they are.
- An Exchange or Microsoft 365 colleague's calendar shared for its free/busy time alone, as a company's usually are, or for its subject and place too, opens with **Open shared calendar** (#138, #140): it shows when they're busy, tentative, away or working elsewhere beside your own, with what and where when they share that much, read again every few minutes while the calendar is open. When their times can't be had (they don't share them, or aren't found), it says so beside the calendar's name. It said "not on the server any more".
- **Calendar groups** for an Exchange or Microsoft 365 account (#140), as Outlook's: **New calendar group…** in the calendar's side pane names a group and picks its people from the address book, your organisation's directory among them. Their calendars are listed under the group's name, shown and hidden together by its box; right-click deletes the group. **Open shared calendar** picks from the address book too, several people at once.
- The calendar's **Schedule** view (#141), beside Day, Week and Month: a day's hours across, a row for each calendar shown and one for all of them together on top, to see when several people are free at once. Clicking or dragging across free time on a colleague's row starts a meeting with them invited; on the row of all together, with everyone shown; on your own calendar, an event in it.
- A right-click on a row of the To-Do Bar's task list opens its menu, as Outlook's (#160): **Open**, **Follow up** › Today to Next week, **No date** or **Custom…** (a day picked from a month), **Mark complete**, and **Clear flag** for flagged mail in any folder; a task's **Categorize** and **Delete**.
- **Start in**, in **File › Options › Mail** (#172): MailCraft can start in **All Inboxes**, the folder open when it last closed, or any account's folder, as Outlook's "Start Outlook in this folder"; it always opened the first account's Inbox, which stays the default.
- What a search found is marked in the open message (#173), as Outlook marks it: in its text, formatted or plain, and its subject, and in the list's subjects and lines of text. A word is marked where the search found it, inside a longer one too (`J006965361` in `201124-J006965361-S/N000164396`).
- An on-premises Exchange account's tasks sync both ways with its mailbox's Tasks folder (#161), as Outlook keeps them: those made in Outlook show in **Tasks** and the To-Do Bar, and one made, changed, marked complete or deleted here is so in Outlook too, with its days, status, how far along, priority, reminder, categories and notes. Tasks made here before go up at the next sync.
- An Exchange or Microsoft 365 account's follow-up days are kept on the message, as Outlook keeps them: a day given to a flag in Outlook shows in MailCraft (in the To-Do Bar, under its day), and one given here, changed or taken off shows in Outlook. **Mark complete** is complete in Outlook too on an on-premises Exchange account, and one completed in Outlook shows complete here: it was only unflagged there. They stayed on this computer before.
- A flagged message can remind you, as in Outlook: **Follow up** › **Add reminder…** (or a reminder in **Custom…**), on the ribbon, a message's menu or the To-Do Bar. It comes up in the Reminders window with events and tasks, to snooze, dismiss or open in its own window; **Clear flag** and **Mark complete** take it off. On an Exchange or Microsoft 365 account it's kept on the message, so a reminder set in Outlook reminds here and one dismissed or snoozed here is in Outlook too; an IMAP account's are kept on this computer, and come in from and go out to `.pst` files.
- Spelling is checked as you type (#174), as in Outlook: a misspelled word in a message is underlined in red, and right-clicking it offers suggestions, **Ignore all** and **Add to dictionary**. It's on unless turned off in **File › Options › Mail › Check spelling as you type**. The text is checked in the interface's language: English always, others when a Hunspell dictionary for them is on the computer (Linux's own, the Mac's `~/Library/Spelling`, or LibreOffice's). The original under a reply is left alone.
- **F6** and **Shift+F6** go from pane to pane (#175), as in Outlook: the folders, the message list (on its message), the reading pane, the To-Do Bar and the ribbon, each in turn, so the keyboard and a screen reader get to each pane without going through every control on the way with Tab.
- A screen reader is told what was done where the keyboard isn't (#175): each notice as it comes (a failure at once), a setting switched with its key (**Ctrl+F1**: "Hide the ribbon: on"), and **Ctrl+B**, **Ctrl+I** and **Ctrl+U** while writing ("Bold on", "Bold off").
- With a screen reader, the open message is one text to read by line, word and letter, as Outlook's reading pane is (#175): Tab or **F6** gives it the keyboard, and the arrows, **Home** and **End** (**Ctrl** too) move through it, **Shift** selecting, while the reader reads the line, word or letter reached. It was read a block at a time.
- A **Font** box beside the font size while writing (#166), as Outlook's: any font installed on the computer, for the text selected or typed next, shown in it as you write and sent with the message. Text pasted in another program's font (Calibri, Tahoma) now shows in it too, when the computer has it. Not in the browser, which reaches no fonts of the computer's.
- **Font for new messages**, in **File › Options › Mail** (#166), as Outlook's: new messages, replies and forwards are written in the font chosen, which goes with them even when nothing else in them is formatted. Text you set a font on keeps its own.
- **Clean up old items** (#179), as Outlook's Archive, in **File › Info › Mailbox settings**: an account's mail received before a day, in a folder and the folders in it or in all of them, moves into an Outlook data file on this computer (`archive.pst` or any other), into folders of the same names, and is then deleted from the mailbox, which gets its space back. Mail on the server that isn't kept on this computer is found there and downloaded first, in rounds for a big mailbox; nothing leaves the mailbox before the file has it. The file then shows in the folder list, where its mail can be read, searched and moved back, and Outlook opens it too. Archiving into it again adds to it.
- **AutoArchive** (#180), as Outlook's, in **File › Options › Data files › AutoArchive settings…**: every so many days, asking first unless told not to, each folder's mail older than an age (6 months to start with) is archived to a data file as **Clean up old items** does, or deleted for good. A folder's **Properties** give it settings of its own, or keep it out; **Apply these settings to all folders now** puts every folder back on the default ones. Off as it starts.
- **View › Folder density** (#184): **Spacious** (as it was), **Medium** or **Compact**, for the folder pane's text size, row height and indent a level together. **Compact** shows about one and a half times as many folders in the same height, about as many as Outlook for Mac.
- Folders in an order of your own (#183), as in Outlook: a folder dragged to the top or bottom edge of another goes before or after it (a line shows where), and **Move up** and **Move down** in its menu move it among those beside it. Dropped on another's middle, it goes inside it. **Sort folders A to Z** in the account's menu puts them back in alphabetical order. Special folders keep their places.
- **Work time** (#185), as Outlook's, in **File › Options › Calendar**: the days you work and the hours between, Monday to Friday 08:00 to 17:00 to start with. **Work week** shows those days (Sunday to Thursday, say, or Monday, Wednesday and Friday alone), and the day, week and Schedule views shade the hours outside them. A colleague's calendar opened from Exchange or Microsoft 365 shades their own work hours, in their time zone, as their server gives them with their free/busy time, and the Schedule's row of all together the hours when not everyone works.
- **Next free time** and **Previous free time** on the calendar's ribbon (#186), as Outlook's AutoPick: the next (or earlier) time when everyone whose calendar is shown is free for the **Meeting length** chosen (30 minutes to start with), inside everyone's work hours and days, your own events and colleagues' free/busy times counting, tentative ones too. The calendar goes to it and marks it; a click on it starts a meeting then with them all. Each click finds the next one, from the days shown on, never in the past; someone whose times can't be had counts as free, and it says so.
- A heading of the message list folds away (#189), as Outlook's groups collapse: a click on **Today**, **Last week**, a sender's name or any other heading hides its mail, the heading then saying how much it hides (in the accent when some is unread), and a click again shows it. Its right-click menu, and the list's **By Date ⏷** menu, have **Collapse all groups** and **Expand all groups**. What's folded is kept for each folder and arrangement, after a restart too; new mail under a folded heading stays hidden and is counted, and the arrow keys go over it. **Select All** still selects all of the folder.
- An event dragged in the calendar moves (#192), as in Outlook: in **Day**, **Work week** and **Week** to another time, in quarter hours, and to another day, and in **Month** and along the all-day row to another day, all of it and as long as it was. Where it would go is outlined with its new times while it's dragged, and **Esc** leaves it where it was. A meeting you organize sends its guests the change, as when it's saved in its form. One that repeats, or a meeting someone else organized, is changed in its form as before.
- **Mark all as read** on the Home tab (#197), in the menu beside **Mark read**, as Outlook has it on its Folder tab: the folder open, all of it, whatever a search or the **Focused** and **Other** tabs show of it. On a server holding more of the folder than is kept here, its unread mail there is marked too (on Exchange 2010 and grommunio a page at a time), and the folder's count goes. A folder's own right-click **Mark all as read** marks it where it is, without opening it.
- The line between the To-Do Bar's **Appointments** and **Tasks** is dragged up or down to give either more room (#196), and stays where it's left, a restart too: the appointments had half, always. With the keyboard on it, Up and Down move it.
- The To-Do Bar's appointments say how long each is under its time, **30 min** or **1 h 30 min**, and one starting within the hour how soon, **In 20 min**, counting down (#195), as Outlook for Mac's day list.
- A meeting invitation says how it fits your calendar (#194), as Outlook does on a meeting request: what it clashes with, by name and time, or what it's right before or after, under its time, with a strip of the hours around it showing your events and the invitation outlined among them; a click on one of yours opens it. Your own calendars count, shown or not, not a colleague's; events shown as free don't clash.
- With the **Focused Inbox** on, only new mail that goes to **Focused** is announced (#199), as in Outlook: newsletters and automatic mail coming into **Other** arrive quietly. **File › Options › Inbox and notifications** has **Notify of mail in Other too** to have them announced as well.

### Changed

- The Slovak translation reviewed by a native speaker (#137): the strings added in 0.40 and 0.41 translated or corrected, short day and month names in lowercase as Slovak writes them, and a few sentences reworded.
- The reading pane's header (the subject, who it's from and to, its buttons and attachments) stays at the top while the message's text scrolls under it, as in Outlook, in its own window too (#151); in a pane too short for that, all of it scrolls as before.
- A POP account set up with an existing `.mailcraft` data file keeps its mail in that file, where it is, as Outlook uses an existing `.pst`: it's no longer copied whole into Documents › MailCraft Files first (a big one sat at "Counting items…" for as long as that took), and removing the account leaves the file there. A file that can't be used where it is (open in another program or not closed properly, read-only, or still an account in another MailCraft data folder) is copied in as before, and MailCraft says why.
- A POP account's new mail shows in its Inbox as it downloads, a batch at a time, instead of all at once at the end: a big download's Inbox stayed empty while it came.
- A message the outgoing server turns away for its sender says which address that was ("the server refused the sender, …"): "550 Invalid Sender" alone didn't say which.
- A calendar or contacts address given by hand that holds none now says what the server answered there (#147), as "PROPFIND answered HTTP 403" where it didn't let these credentials look (a provider whose calendar user name isn't the email address, set under **The calendar server has its own user name and password**): it said only that none were found.
- A server that ends the connection as it's being secured is now named, with what's most likely wrong (#153): it may offer only older encryption than MailCraft uses, TLS 1.0 or 1.1 or TLS 1.2 without its newer ciphers, which its administrator can turn on. An Exchange 2013 server that did said only the system's "an existing connection was forcibly closed".
- An Exchange account's look for new mail every two minutes reads every folder the server says changed, not only the Inbox (#145): mail an Inbox rule files into a folder of its own comes within minutes, and mail marked read or unread in another program there changes here as soon, where both waited for the full sync a quarter of an hour on.
- An Exchange account's calendars and contacts no longer hold up its mail while they're read, which takes minutes with colleagues' calendars (#145): new mail is looked at meanwhile, and **Send / Receive** reads the mail before they're done, the status bar saying so, where it waited for them to finish and showed nothing.
- The times in MailCraft's log end in `Z`, saying they're UTC (#145): read beside a clock, they were taken for local.
- A day clicked in the To-Do Bar's month lists that day's events under it, in **Appointments**, and the mail stays on screen (#154): it opened the Calendar, losing your place. Clicking the day again, or today, brings back what's coming up; a double-click opens the Calendar on that day.
- The To-Do Bar's **Appointments** list every event of the coming week, scrolling when they don't fit, and its flagged mail is listed without `RE:` or `FW:` before the subject, as Outlook's (#163): it stopped at 8 events, today's and tomorrow's on a busy calendar.

### Fixed

- An Exchange 2010 account syncs (#149): its first sync failed with "The request failed schema validation", MailCraft asking for its folders together with three that Exchange 2013 brought (Archive, Conversation History and Sync Issues), which Exchange 2010 doesn't know. On a later Exchange they're found as before.
- With the reading pane off, mail chosen in the list, clicked or passed over with the arrow keys, is no longer marked read (#146): it isn't shown, so only opening it in its own window reads it.
- Tables written in Outlook or pasted from Excel keep their rows and columns in a reading pane narrower than they are (#152): their columns narrow, down to their widest words, where every cell went on a line of its own. A table of figures too wide even for that scrolls sideways.
- On a Mac, scrolling stops at the top and end of the folder pane, the message list and the reading pane (#150): a smooth-scrolling mouse or a trackpad pulled them a third of the pane further, leaving it blank, and they stayed there.
- An Exchange meeting you accepted shows as accepted, a recurring one too (#144): the calendar took your answer from the meeting's list of guests, which keeps the one the invitation came with, where Exchange keeps yours apart; some accepted series showed as tentative or not answered.
- An attachment dragged out of a message opened in its own window keeps its name and extension (#120): it was dropped as a file called "attachment", and Save Attachment there suggested that name too.
- In Calendar, People, Tasks and Notes, the ribbon's **Send / Receive** and **View** tabs show their own buttons (#142): Send / Receive as in Mail, View how the module shows what it holds (the calendar's Day to Month, how tasks and notes are arranged, people's views) and the folder pane; both showed Home's. **Update folder** (Shift+F9) there reads the calendars, contacts, tasks and notes again, where it read the mail folder behind them.
- An Exchange account's mail says what it answers (#148): a reply joins its conversation even when only its headers are here, where it stood alone.
- In the dark theme, the segment of a switch under the pointer (File › Options › Theme, Reading pane and the others) no longer looks chosen: it was lighter than the segment that was.
- MailCraft's log no longer fills with the HTML reader's "foster parenting not implemented" (#156): written for each table in a message whose text sits outside its cells, it took most of a tester's log while mail was being indexed, hiding the lines that matter. The HTML readers' warnings, about a message's markup and not MailCraft, are left out; their errors still go in.
- The Search tab stays while the search box is being typed in (#158), and goes with a click elsewhere: clicking into the box took its focus away again as the tab appeared, so the tab went as soon as the pointer left the box, before its buttons could be reached.
- In the calendar's day and week views, overlapping events share the column only among themselves (#155), as in Outlook: every event of a day was as narrow as the most that overlapped anywhere in it, a morning meeting a third of the column for three in the afternoon.
- Mail sends through IONOS's outgoing servers (#165): every message was refused with "503 Bad sequence of commands", the server refusing the second greeting MailCraft sent to see which sign-ins it takes. It's sent only when the first didn't name a password sign-in, and a refusal of it no longer stops the message.
- The message list keeps the width it was dragged to, and the folder pane its own, when MailCraft starts again (#171): both went back to their first sizes. The list above the reading pane keeps its height too.
- With messages grouped by conversation, **Delete** on a conversation deletes all of it in the folder (#170), as Outlook does: it deleted the newest message alone, one press for each. A message picked inside the conversation, in the reading pane, is still deleted by itself, and one **Undo** brings the whole conversation back.
- A Gmail account refused because Google wants an app password (2-Step Verification on) is told so plainly, in your language, with a link to make one (#164): MailCraft passed on Google's English "Application-specific password required" and asked for the same password again. Sending refused the same way says so too, and adding a Gmail account, or one on Google's mail servers, says beforehand that an app password goes in the password box.
- The task form's **Start date**, **Due date** and reminder day have the month to pick a day from beside them, as the event form's dates have (#162), and the date boxes of both forms show and read dates in the order chosen in **File › Options › Dates**: they were always `YYYY-MM-DD`, which is still read.
- On a Mac, a window closed to the Dock (**Keep MailCraft running when its window is closed**) answers again when it's brought back (#169): it came back showing its last picture and took no clicks or keys until MailCraft was quit. A Mac started with its window put away was the same.
- With a screen reader (NVDA, Narrator), the message list is read as you go through it (#175): Up and Down take the keyboard to each message, read with who sent it, its subject, when, and what's marked on it; a click on one does the same, and after **Delete** the next one is read. Nothing was read: the keyboard stayed on the window. The list is one stop for Tab, which goes on to the reading pane: its buttons, its attachments (read by name and size; they were "custom") and the message's text, which is read when it gets the keyboard and couldn't be reached before.
- A drop-down's arrow keys choose, as on Windows (#175): Up and Down pick the choice before or after, Home and End the first and the last, and Alt+Down opens its list on the choice made, where the arrows go through the choices and Enter picks one; the keyboard stays on the drop-down. Up and Down went to the field above or below instead, so in the account form the kind of server (IMAP or POP) could only be changed by finding the keys that open the list. Esc in an open list closes the list, not the window it's in with what was typed there. The account form's **Mail server** and **Sign in with** are read by their names.
- Sending mail uses lettre 0.11.24, which closes six flaws in sending over SMTP (RUSTSEC-2026-0560 to 0565): replies a server sent forged by someone in between when the connection turns encrypted (STARTTLS), a line break alone in a message able to slip in SMTP commands, mail for an address with a quoted name delivered elsewhere, and three ways a hostile server could stop MailCraft or use up its memory.
- OpenPGP: a signed message put together from someone's public key is no longer shown as signed by them (rPGP 0.21.1, GHSA-h834-358q-rx97). Every copy of a public key carries its owner's signature on their name and address, and that signature, lifted out with what it covers as the "message", read as a good signature from them on mail.
- Searching the server for a date alone (`before:2024-01-01`, `after:`, `received:2023`) finds the mail of those days that isn't kept on this computer, on IMAP, Exchange, Microsoft 365 and JMAP accounts: the server was asked without the date, which left nothing to ask. With words too, IMAP and JMAP servers are asked for the days as well.
- Searching the server right after deleting or moving mail no longer finds that mail again: the search could reach the server before the change did.
- A folder dragged onto another folder in the folder pane goes into it (#181): the drop was lost, though the folder under the pointer lit up. A folder dropped on a shared mailbox's top row goes into it too.
- **Move folder** (#182): a click picks where the folder goes and **Move** moves it there, as in Outlook; it moved at once, and **Move** never turned on. A double click still moves it at once. Its list, and **Move to folder**'s, are as wide as the dialog, their scroll bar at the edge rather than in the middle.
- A search ended by opening a folder, or by going to the Calendar, People, Tasks or Notes, leaves the search box empty (#187), as in Outlook: its words stayed, as if the search were still on, and going to another module didn't end it. They're among the recent searches in the box's list.
- The folder pane starts right under the ribbon (#188): its **‹** (Minimize the folder pane) sits at the end of the first row, the Favorites heading or with Favorites hidden the first account, as in Outlook. It had a row of its own, leaving an empty strip above the folders.
- A big message with attachments on an on-premises Exchange server opens at once (#191): its text, asked for ahead of its files with a field Exchange doesn't know, was refused with "The request is invalid.", and that showed in the reading pane in place of the message until it was opened again. Its text comes first again, and from a server that refuses that, the whole message comes instead, never an error.
- On a Mac, a reminder or a new message no longer opens a "Where is use_default?" window to cancel, one for each notification at once (#193): MailCraft told macOS no application to send them as, and the notification library asked the user to find one by that name. They go out as MailCraft's, from its app.

## 0.40.0 - 2026-11-17

*What testers met in their first days with 0.39.*

Thanks to Peter Páleník for the Slovak translation (#124), and to flash7777 for #125–#130.

### Added

- Slovak as an interface language (#121): **File › Options › General › Language › Slovenčina**, and a Slovak system picks it.
- Attachments selected together (Ctrl+click, ⌘-click on a Mac) drag out together (#120), onto a folder to save them there or into another app, as **Save Selected** saves them: only the one under the pointer went.
- Choose how MailCraft signs in to an account's mail servers (#123), under **Server settings** when it's added or changed: **Sign in with**, under each server, Automatic, Password, or NTLM (Secure Password Authentication, as a company's Exchange may ask for it: the password never crosses the network), and for the outgoing server No sign-in. Left on Automatic, a server that offers NTLM and not a password is signed in to with NTLM.
- The message list can leave out its headings, such as Today and Yesterday (#129): **Show in groups** in the menu of how the list is arranged (By Date ⏷), on as before.
- The ribbon can be hidden, for more room for mail (#127): **File › Options › General › Hide the ribbon**, Ctrl+F1, or a double click on one of its tabs. The tabs stay on the top bar: a click on one shows its buttons until you click elsewhere.
- The message list can show a message on one line, as a table's row (#125): the sender, the subject, its markers and the time, so many more fit in the window. **View › Single line**, or **File › Options › Mail › Single-line list**. Its head is one line too: the folder's name, with the filters (All, Unread…) in a menu beside it.

### Changed

- **Add an email account** is shorter and says more: what it works with and where your password goes, at the top; **Server settings** folds away, naming the servers it found; opened, the kind of mail server is picked from a list, with a line saying what each is, and a POP account's copy on the server and data file to start from fold away too, saying what they're set to; an Exchange (Microsoft 365) or Outlook.com account goes on with **Sign in with Microsoft**, its only way in; and an account for calendars and contacts alone is a link under the form, to a form of its own. **Server settings** for an account already added are laid out the same way, each server and how it's signed in to.

### Fixed

- On Linux and the BSDs, attachments drag out of MailCraft on a desktop running Wayland, such as GNOME's or KDE Plasma's (#120): drag one onto a folder in your file manager to save it there. The window opens through XWayland so that they can, but dragging them was still left off there, and nothing happened.
- Newsletters that put a table around each section for Outlook alone, as HubSpot's do, show their text (#122): every section came out empty, and only the background and pictures showed. A title in a quote in a colour of its own no longer starts with `> ##`.
- A table whose rows are inside a form, as a survey in a newsletter has them, shows its rows: it came out empty.
- Mail goes out through an outgoing server that takes it without a sign-in, as a company's own may (#123): MailCraft signed in anyway, and couldn't send. A server that offers only sign-in methods MailCraft doesn't speak is said to, naming them.

## 0.39.0 - 2026-11-16

*What testers met in their first days with 0.38.*

### Added

- A mail server whose certificate no authority vouches for can be trusted, as other mail apps let you (#105): a company's own, or one set up at home. MailCraft asks, showing whom the certificate is for, who issued it, until when, and its fingerprint for whoever runs the server to confirm, and **Trust this certificate** keeps it for that account. Or the certificate file they give you, the server's own or the authority that issues it, is trusted under **File › Info › Server settings… › Trust a certificate file…**, where **Stop trusting** takes one back. Only for the account's mail servers, only a certificate in date and for the server's name, never for a major mail service's servers, and MailCraft asks again if it changes.
- macOS: the Dock icon shows how much unread mail is in the Inboxes, as the window's title does, and nothing once it's all read (#93).
- An account can look for new mail only when you ask (#104): **File › Options › Mail › Checking for new mail › Only when I ask**. It isn't signed in as MailCraft starts nor looks by itself; **Send / Receive** on the account looks (and now looks at that account only: it looked at all of them), Send / Receive for all leaves it out, and sending or opening a message not downloaded yet still works.
- A picture in a message has a menu on a right-click (#110): **Copy picture**, for another app to paste, and **Save picture as…**, for one the message carries or one from the web that's been shown.
- In the dark theme, messages can be shown on a light background, in the colours they were written in (#109): **File › Options › General › Messages on a light background**, or the sun above a message to switch.
- An event's repeat can be set as you need it (#103): every so many days, weeks, months or years, on the weekdays you pick, on a day of the month or its nth weekday (the third Thursday, the last Friday), and ending never, after so many times or on a date. A series from elsewhere opens with its rule to change, unless it says more than the form shows. A small month beside each date picks a day, and the end follows the start, keeping the event's length.
- One of an account's other addresses can be the one new mail is written from (#88): tick **Write new mail from this address** when you add or change it under **File › Info › Other addresses**. A new message then starts from it, with its name and signature, and **From** still offers the others. An answer goes from the address the mail came to, as before, and from the chosen one when the mail came to none of the account's addresses (a list's mail, a hidden copy). For a mailbox that signs in as one address and writes as another, such as a Gmail mailbox with a domain of your own.
- Files can be sent from where they are (#96): **Send to › MailCraft** in Windows' Explorer, **Send with MailCraft** in Dolphin's, Nemo's and PCManFM-Qt's menus for files (from the .deb, .rpm and FreeBSD packages), and on a Mac any file dropped on MailCraft's Dock icon start a new message with them attached, in the MailCraft already open if there is one. `mailcraft --attach FILE…` does the same from a script.
- Ctrl+Enter asks before it sends the message you're writing (#115), so a slip of the keys sends nothing unfinished: **Don't ask again** keeps the answer, and **File › Options › Mail › Ctrl+Enter in a message** sets it to ask first, send, or do nothing. Alt+S and Send send as before.
- MailCraft can start in the notification area (#116): with **File › Options › General › Keep MailCraft running when its window is closed** on, **Start MailCraft in the notification area** starts it there, checking for mail and announcing it with no window coming up (minimized where a window can't be hidden, as on a Mac). Started to open a link or a file, it opens the window.
- MailCraft's icon in the notification area shows how much unread mail the Inboxes have, in its corner (#117), as the window's title does; **Show the unread mail on its icon** turns that off.

### Removed

- Windows 32-bit (x86) builds are no longer released, as the other Craft apps' aren't: 64-bit Windows runs the x64 or ARM64 build. A 32-bit copy already installed keeps working but gets no more updates; install the x64 one to keep up.

### Fixed

- **Print** shows the message as it will print at once, in your PDF viewer, to look over and print from (#97): it asked first where to save a PDF, as Save as PDF does.
- A signature in HTML code with pictures from the web keeps them when you look at it under Formatted and go back (#91): Formatted can't show them, and coming back made the code anew without them. Formatted now says so. While writing, the signature is drawn as it's sent, its web pictures shown when you click Show pictures.
- Enter opens the message selected in the list in a window of its own, as a double-click does, and a draft to go on writing it (#92). It did nothing, on every system.
- A folder named archive inside the Inbox, with folders of its own, shows with them under it (#94): MailCraft takes it for the Archive, as it does on servers that don't mark one, and its folders were drawn among the Inbox's, under the wrong parent.
- A meeting series' first invitation, once the organizer has changed one of its occurrences, says it's out of date (#101). Its answer buttons were still there, and answering it put the series back without the change.
- Declining one changed occurrence of a meeting series works on SOGo (#101): it was refused (HTTP 403) and nothing reached the organizer. The occurrence now keeps your answer instead of being taken out of the organizer's meeting, and isn't shown in your calendar. An answered invitation goes out of the Inbox only once the server has the answer: one it refuses stays to be answered again.
- Your meeting saved again without a change no longer sends the guests an update (#102), nor does a change only you see: its reminder, its categories or how it shows your time. Each update asked the guests to answer again.
- An event moved to another start keeps its length: a half-hour meeting moved became an hour long.
- A recurring meeting moved to another time keeps the occurrences taken out of it out (#100): one deleted came back, for you and the guests, and an occurrence changed on its own stood beside the series' new one that day. Each moves with the series, and the guests' update carries the changed occurrences along.
- Meeting invitations, answers and cancellations sent by MailCraft arrive in Exchange and Outlook as meetings, with buttons to answer (#98), not as a message with an `invite.ics` file: the calendar now goes inline beside the text, with its method and character set, as iMIP (RFC 6047) asks.
- A certificate file (`.p12`, `.pfx`) saved by Thunderbird or Firefox is imported (#89): they write the file another way than OpenSSL and Windows do, and every one was refused as a wrong password, old files and new. What stops an import is now said as what it is: a wrong password, a file locked in a way MailCraft doesn't read, or a file that isn't a certificate file.
- An account with calendars and contacts only is no longer listed in the mail folder pane (#86), where it opened to nothing and several of them filled the pane. Its calendars and contacts are in Calendar and People as before, and the account under **File › Info**. **All Inboxes** and **All Sent** show only when more than one account has mail.
- A signature set out in columns keeps them (#90): a logo, a name and an address side by side went one under the other in a reading pane of the usual width, and the line between them was gone, unless the window was wide. Short lines now wrap a little beside columns that say how wide they are, as in a browser. A signature written as HTML code also shows that way in the message you're writing, where it was its text, one part under another: its columns, colours, lines and pictures, as it's sent. It's one thing in the text there: the cursor passes it, and Backspace after it takes all of it. Pictures it names on the web load when you ask (**Show pictures**, under it), as for a message you read. Change it under **Signature**.
- **Sign in again** says which user name the account signs in with, when that isn't its address (#88): asked for "the password for" the address, it read as signing in with the address.
- In the browser, a picture or subscribed calendar larger than MailCraft takes is no longer downloaded whole before it's refused: reading stops once it passes the limit, or before it starts when the server gives its size. One answering with gigabytes could take the page down.
- In the browser, a backup the browser's storage won't take (full, say) is said to be refused, and the page goes on keeping what you do. It used to say nothing, and nothing done afterwards was kept: it was gone at the next visit.
- Rules, blocked senders and ignored conversations sort a lot of mail at once without holding up the window: **Run Rules Now** on a big Inbox, or the first sync after a long time away, changed each message on its own and asked the server once per message (20,000 messages filed by a rule took over two minutes). Mail they file together is now one change, as when you drag it.
- Mail moved out of a data file or local folder whose copy was filed but which couldn't then be taken out where it was (the disk refused the write) is said to be in both places. It was said to be moved, with an **Undo** that couldn't put it back, or, moved to a server, a bare disk error followed by "moved".
- With **Focused Inbox** on, mail arriving in the Inbox on screen no longer has every folder of every account read again to sort it: the people you write to, as counted in the last ten minutes, sort it, as they do for the sender check.
- Replying to or forwarding a message with a link stuffed with `javascript:` no longer hangs MailCraft: each one was taken out by going over the whole link again (a megabyte of them took five minutes).
- A big folder of a JMAP account (Fastmail, say) syncs without looking each message up in the whole folder: in a folder of 40,000 that was nearly two seconds of every sync, even when nothing had changed, and it grew with the square of the folder's size.
- A calendar invitation or subscribed calendar with a very long list of skipped dates (`EXDATE`) is read at once: each date copied the whole list, so 100,000 of them took 40 seconds.
- An event whose notes are full of web addresses, or of brackets after one, is opened and its meeting link found at once; a crafted invitation's notes could hold MailCraft up for minutes.
- A contact from a CardDAV server that you edit goes back with what MailCraft doesn't show: saving it from **Edit contact**, even with nothing but its job title changed, cleared its last and first names (and dropped a middle name, title and suffix), took the type off its email addresses, and dropped its numbers' other types, which one is preferred and the label beside them (iPhone's "Studio fax", say).
- Exporting into an Outlook data file that's damaged so that a folder is listed inside itself no longer hangs before it starts: each folder is looked through once, as an import does.
- An export to an Outlook data file says so when a folder couldn't be read and its mail isn't in the file; it was left out with nothing said.
- With an Exchange account (EWS or Microsoft 365), a calendar item, contact or note the server refuses to delete (access denied, say) is said not to be deleted and is asked again at the next sync; it was taken for already gone. A contact or note changed here whose old copy the server wouldn't delete is no longer made again beside it, twice over. One whose id the server can't read is said once, not at every sync.
- A big folder of an Exchange account (EWS or Microsoft 365) syncs, and is searched on the server, without looking each message up in the whole folder, as a JMAP account's now does.
- Automatic replies can be read and set for an account that signs in with OAuth on a server with ManageSieve: its sign-in there was encoded twice, and always refused.
- A message with a line longer than 64 KB comes whole from a POP account: the line was broken in two, and a piece of it that was a lone `.` cut the message short there.
- Turning on **Online only** for an account when one of its files can't be read says so and changes nothing; that file was left behind and then deleted with the rest, changes waiting for the server and the Outbox among what could go.
- A move of the data files to another folder that stops, because a file of the same name turned up there meanwhile, leaves that file alone; it was deleted as the move was undone.
- When the list of which data file is each account's (`datafiles.json`) is damaged, each account's data file is found again by its name, and the damaged list kept aside as `datafiles.json.bad`; one that can't be read at all is said, and tried again at the next start. Either was taken for an empty list: accounts showed no mail, and the next mail to come in started a second data file beside the first, which was then forgotten. Two accounts with one address each get their own file back, never the other's.
- A rule with more than the rule editor shows (several conditions, any of which will do, several actions, or one account it's for: one brought in from a file, or made with a command) keeps all of that when you change it in the editor. Saving it, even only to rename it, kept just its first condition and first action, for every account, and a rule moving mail to Junk came back moving it to Archive.
- A contact saved from **Edit contact** keeps the postal addresses the form has no place for (a second home address, say) and the kind of a number or address it shows under Other (a pager, a postal address): the extra addresses were deleted and those kinds dropped.
- **Email** on a contact's card writes to that one person when the name has a comma, as Outlook's "Last, First" names do: the name was split into two recipients there, the first of them not an address, and the message couldn't be sent until it was put right.
- In the browser, a change made while the account is signed out (a flag, a move, a delete) waits with the mail kept in the browser, so it reaches the server once you sign in again even after the page was reloaded or closed. It waited in the page only, and was lost with it: the next sync put the message back as the server still had it.
- In the browser, a message popped out of a folder whose name has "sample" or "webgl" in it (`Samples`, say) opens in your own mailbox: its window opened the sample mailbox instead, and couldn't find the message.
- An update that couldn't be installed (a portable zip that wouldn't unpack, a Mac disk image that wouldn't open, an administrator prompt declined, or MailCraft still running) is offered again at the next check, from the download already made. The download was thrown away, and that version wasn't offered again for a day.
- `mailcraft-cli` takes `--headers` and `--checked` anywhere on the line, says which option was given no value, and says when `--limit` or `--timeout` isn't a number. `source --headers KEY` lost its KEY, an option without its value took the next option for it, and a bad number was quietly replaced by the default.
- In the browser, a file attached or a picture put into a message no longer takes many times its size in memory on the way in: a 25 MB file took about 600 MB.
- A category given twice to an event or task is kept once.
- An account's address changed in its settings, when it signs in with its address, is used at once: the account went on signing in with the old one until MailCraft was started again.
- `account.add` with a data file whose path is refused no longer leaves the account added.
- In All Inboxes, one Undo puts back what was done to mail of several accounts at once (marked read, flagged, categorised, moved or deleted): it took one Undo per account, in no set order.
- Archive or Move on mail of several accounts (All Inboxes) moves nothing when one of them has no such folder, instead of moving some and failing.
- Copying a lot of mail, or moving it from another account, into a folder of a data file or a local account no longer holds up the window: each message was read twice and written on its own, and 5,000 of them took over ten seconds.
- Deleting one changed occurrence of a repeating event from a calendar on Exchange or Microsoft 365 deletes it on the server too: it was taken out here only, and came back.
- A colleague's copy of a meeting in a calendar you may only read is no longer shown beside your own, and doesn't come back at the next sync.
- Moving more than 2,000 messages into an Inbox or Junk on a server no longer has some of them announced as new mail, or filed again by rules, when the server shows them there.
- A dismissed reminder stays dismissed: one for an event that runs for weeks, or for a task whose reminder came long before its due day, came back at once, and snoozing another reminder for days could bring back those dismissed in the last month.
- With a damaged key among your own OpenPGP keys, mail is no longer signed with another key than its address's.
- Writing from another address of an account that has only an OpenPGP key for it, the message bar says whether OpenPGP reaches the recipients: it went by the account's certificate, turned encryption on by itself, and sending then failed. A recipient's certificate withdrawn is taken into account at once, not up to an hour later.
- A message quoted in a reply or forward, and a signature's HTML code, lose every `on…` handler and script link a browser would run: one written without a space before it, after a `>` inside a quoted value, or as `javascript:` hidden behind character references went out as it came. Nor does one inside a tag written to end later than a browser ends it.
- A message whose style sheet is longer than MailCraft reads keeps the part read, instead of losing its colours and sizes.
- A picture written `<img`, a tab, `src=…`, or `src = …`, is blocked or shown with the rest, and an address inside another attribute's text isn't taken for one.
- A recurring event begun long ago still shows: a daily one started more than about 55 years back, or an hourly one more than five, was missing from the calendar, and a calendar of years-old series is drawn much faster.
- An event whose notes hold a long run of `<` brackets opens at once: the app froze for seconds on every frame it was open.
- An event or task holding a huge part MailCraft doesn't read keeps only its first few hundred lines: all of it was saved with the calendar, and an event's read through each time the calendar was drawn. An event's reminder is still found after such a part.
- An Outlook message file saved in a code page rather than Unicode keeps the accents and letters of its attachments' names: they were read as UTF-8 and lost, while its subject read right.
- Clean Up leaves a message with a category another program gave it (`$label7`, shown as a category) where it is: every keyword starting with `$` was taken for a system mark, and such a message could be moved.
- The search box says when part of a search was left out: a size or a yes-or-no it can't read (`larger:abc`, `isread:x`), or words past the 64th and names past a list's 50th. These were dropped without a word, and the search found more than was asked.
- Exporting into an existing data file part of which can't be read back stops and leaves the file as it was, instead of adding contacts, calendar items, notes and tasks a second time.
- Account setup still finds a provider's servers when the computer's first DNS resolver fails: the others weren't asked. Its lookups now carry a random id, so an answer can't be forged by guessing it.
- Signing in with Google or Microsoft no longer fails when the browser opens an extra connection to MailCraft before the redirect.
- Messages with unusual line endings (bare line feeds, or a mix) have their header read correctly when exported to or imported from an Outlook data file, or re-dated.
- Exchange accounts with shared mailboxes or public folders no longer send a request again to every one of them when something isn't found: each folder's requests go where the folder is kept.
- Changes made offline reach the server much faster when many are waiting (a thousand: 7 s instead of 25 s).
- Syncing a very large calendar or address book is faster: the first read of 20,000 items took twice as long, and sending changed events was far slower.
- Drafts kept on this computer while offline take a fraction of the room and time: each is a file of its own, and those kept by an earlier version are moved over the first time.
- A POP account with more than about 130,000 messages left on the server downloads its mail instead of failing every check.
- Checking a POP account that leaves many messages on the server, and moving or deleting many of them, is much faster.
- The first download of a large POP mailbox is many times faster, and one stopped part-way never loses a message or brings one in twice.
- A mail server sending a message date with an impossible year no longer crashes MailCraft; the date is ignored.
- Large Exchange and calendar replies sent in small pieces are no longer refused as too big, and take less memory.
- Exchange accounts with public folders sync faster: the public folder list is read once a sync instead of twice.
- Turning an account's Work online only setting on or off no longer needs twice the account's size in memory.
- Printing or saving as PDF a message with deeply nested tables is faster.
- Removing an account while it searched the server or downloaded a message no longer brings its files back.
- Syncing a very large IMAP folder kept whole is faster on servers without CONDSTORE.
- A message sent from the Outbox is no longer sent again when its files can't be removed, and deleting or editing a queued message that can't be taken out of the Outbox now says so instead of sending it anyway.
- When the disk fails to read MailCraft's cache, folders are no longer emptied or downloaded again, and an import says its folder list couldn't be read.
- A damaged search index is rebuilt straight away over every folder, so search doesn't stay empty or partial.
- A message whose source starts with bytes MailCraft uses inside its data file now reads back correctly, and no longer stops the data file from being compacted.
- A signed message whose certificate is encoded in an unusual but valid way is now trusted when its issuer is, and shows the same fingerprint as other mail programs; one changed after it was signed is no longer accepted.
- Signed mail sends your certificate exactly as your certificate authority issued it, so recipients can check it even when it's encoded unusually.
- A message an Outlook file read again couldn't give is tried again on the next read, instead of being left out for good.
- Flagging or categorising a message in a very large folder no longer rewrites megabytes of the folder's list.
- An attachment opened while its message was still downloading now shows its preview once the file arrives, and previews no longer pile up in memory.
- A POP account's Server settings now warn when its server doesn't use encryption, as adding the account already did.
- When a dialog's OK is refused, MailCraft now says why, even in Select Names.
- In the browser, a reply written in a message's own page now lists your contacts in Select Names.
- Move to and Move folder find a folder by the name they show in your language.
- Select Names adds someone whose address is part of another address, or of a name, already in the field.
- Two video attachments with the same name now each show their own thumbnail.
- An import no longer stops on its own when its account is switched to or from keeping mail online only.
- A Microsoft or Google sign-in service that is busy or failing no longer makes MailCraft ask you to sign in again; it tries again later. A server that takes a renewed sign-in and then refuses it again is asked a few times, then at longer waits.
- Exporting into an existing data file that can't be written to no longer loses track of its own Sent Items, Deleted Items, Drafts and Junk folders.
- Exporting to a large Outlook data file is faster.
- In the task list and the To-Do Bar, due dates are written in the order chosen under Dates, with the weekday.
- On Windows without ffmpeg, video attachments now show their frame on a busy or slow computer too.
- On Windows, changes made while an account syncs no longer briefly disappear from the folder it just synced.
- Text with many unfinished links (`[a](` over and over, pasted or quoted in a reply) no longer slows the message editor down.
- A reminder's event that is changed (renamed or moved) now shows so in the Reminders window at once.
- The folder pane stays fast with thousands of folders.
- People, Tasks and Notes stay fast with thousands of contacts, tasks or notes.
- Picking Bold, a colour or another look with nothing selected no longer takes away Redo or makes the next Undo do nothing.
- Undo no longer uses hundreds of megabytes of memory in a long message.
- The message editor stays quick with a long selection while the ribbon or a menu is in use.
- Writing a long message is smoother: the editor does much less work on each frame.
- In the browser, a password-locked backup is refused, rather than locked with weak randomness, when the browser can't supply secure random numbers.
- Replied and Forwarded marks owed by replies waiting in the Outbox are no longer lost when their file can't be read at start. MailCraft says when it couldn't be read.
- On Linux, a data file whose name differs from an open one only in case is no longer refused as already open.
- A refused edit of a message (an importance other than high, normal or low) no longer changes its recipients or text.
- An Outlook message file over 16 MB opens even without its .msg extension.
- Save all attachments no longer writes over a file when every numbered name is already taken.
- A search from the command line no longer spins while read marks are being saved.
- Dismissing a reminder for an event whose ID contains a "|" now sticks and reaches the server.
- A directory search that failed is tried again after 30 seconds, not 5 minutes.
- Flagging a whole folder at once no longer stalls the window when the To-Do Bar is open.
- Push notifications from a JMAP server that ends its lines with a lone CR are understood.
- Two JMAP folders whose names come out the same no longer swap their mail when the server lists them in another order.
- A picture whose address is written with `&#38;` now loads when pictures are shown.
- A file whose name appears in a picture's description no longer loses its paperclip.
- A message can't set its text's size or typeface with the private characters MailCraft marks text with.
- Pictures and buttons in newsletters' Outlook layouts take the size their style finally sets.
- A page's background is taken out of a quoted message when its style isn't quoted, and `data-bgcolor` is left alone.
- Pictures in pasted HTML are read from `file:` links on this computer only: one naming another computer was read as a path here.
- Video attachments' thumbnails are made a couple at a time instead of all at once, and their frames no longer stay in memory for the whole session.
- On Windows installed outside C:\Windows, the interface and PDFs use Segoe UI again, and Arabic and Hebrew text gets its font.
- `--cmd` with bad JSON says so for the window's own commands too (ui.addAccount and others) instead of opening the dialog empty.
- Started twice at the same moment, MailCraft no longer ends up opening a second window for every later launch.
- Dragging an attachment out on Linux no longer floods the drop target with messages, and works for file names that aren't UTF-8.
- In the browser, turning off "Keep unlocked in this browser" while a password is being sealed no longer leaves it kept.
- `mailcraft-cli --help` now lists `--jmap`, `--cc` and `--timeout`.
- With many web pictures loaded, the pictures of the open message and of the signature being written no longer disappear when others are shown.
- An encrypted message on screen no longer asks for its key again after many other encrypted messages have been opened.
- Reading a message in a very large folder is smoother: the reactions under it are no longer gathered on every frame.
- A picture whose name starts with "signature" is no longer lost from a message when a signature is inserted.
- An appointment from a damaged Outlook data file no longer comes in with a repeat rule calendars can't read; it comes in as a single event.
- Commands on a very large selection (Select All, then Delete, Flag, Mark as Read or Mark Complete) are much faster.
- Blocking a sender when the blocked list is full no longer drops them from Safe Senders, and a move to Junk that fails is now said. Block Sender on several messages says when the list was full for some of them.
- A rules import that fails no longer leaves new categories behind.
- When a server numbers a folder again, its old mail is no longer announced as new or run through rules again.
- Clean Up is faster on folders where much of the mail is unread or flagged.
- Ignoring a conversation when many are already ignored keeps it ignored.
- Automatic replies on a mailbox kept on this computer no longer call it "the sample mailbox", and a failed move or copy names the folder as it's shown.
- Searching the address book is faster with many contacts.
- An event whose repeat rule has a frequency calendars don't define (such as FORTNIGHTLY) now still shows in the calendar, once.
- Setting up an account whose address is typed with a trailing dot now finds the provider's servers.
- Printing a message with a very long word followed by many accent marks no longer hangs.
- Printing a message with huge text and a very long word no longer leaves near-empty pages.
- A link that puts a name and password before its address is counted as a tracker, or not, by its real host.
- A message attached inside a winmail.dat now opens the same way every time, and one with no date of its own no longer shows the time it was opened.
- IMAP messages from servers that send bare line feeds no longer go missing.
- Automatic replies work on servers that have a Sieve script named ok, no or bye.
- A folder change no longer briefly empties the folder pane when the folder list can't be read.
- Folders with millions of messages sync noticeably faster.
- Automatic account setup no longer picks a server that a provider's settings file has commented out.
- Exchange automatic replies containing <, > or & are kept as typed.
- Exchange servers or proxies that name NTLM in lower case can be signed in to.
- In Microsoft 365 accounts, pictures shown in a message's body no longer appear as attachments while the message is still downloading.
- Public folders holding many subfolders now list them all.
- Reminders written in unusual forms by other calendar programs are now found, changed and dismissed.
- Calendar search is faster on large calendars.
- Events with oddly written reminders from other programs can now be saved to the calendar server.
- A message the server refused for good shows the server's reason in the Outbox, and no longer reads as one that will be tried again.
- Sent mail is kept for outgoing servers whose names only end like Gmail's, and Gmail's own server is recognised however its name is written.
- Signing or encrypting a message, and exporting, no longer wait forever when the computer can't start another task.
- Clean Up keeps the message the list shows last: a reply from a computer whose clock is behind now counts as later than the message it quotes.
- Mail deleted in MailCraft from a very large Outlook file no longer comes back at random when the file is read again.
- Moving the data files to a network folder no longer fails when the copy was fine.
- Adding messages that carry many custom properties to an Outlook data file is faster.
- Reading large Outlook data files (importing from one, exporting into one) is faster.
- Importing an Outlook data file with thousands of folders no longer stalls before the first message comes in.
- An attachment whose long name is blank comes in under its short name instead of a name of spaces.
- Messages attached side by side without their own ids no longer come in with the same id.
- PDF previews draw text in bold when the PDF names a bold font it doesn't include.
- The Read / Unread button's tooltip on the Home tab is now shown in your language.
- Clicking a mailto: link in a message now fills in its subject and copies, and decodes characters such as %2E.
- Save all in the browser now says which attachment couldn't be downloaded.
- Accounts with thousands of folders, such as an imported Outlook file, update folder counts and delete folders faster.
- S/MIME mail encrypted with AES-192-GCM can now be read.
- Signed messages are checked faster.
- A setting saved by another version that can't be read no longer resets all the others.
- An event whose guest has a comma in their name can be saved again.
- People's List view names an address book from the server the way the side pane does.
- People's List view highlights the row under the pointer.
- In a small window the month view shows all six weeks.
- Searching a large mailbox for a phrase, and checking long result lists against the text kept, is faster.
- Insert link reads an existing email link's subject and address however the link spells them.
- Long messages open in several windows at once keep their scroll bars steady.
- Newsletters with many centred lines and links draw faster.
- Mail laid out in tables no longer slows down while its web pictures load.
- A message with hundreds of pictures no longer slows down every frame of the reading pane.
- In a very long formatted message, a picture inside a box over a deeply nested table is no longer shown again after the text.
- A meeting whose invitation's id holds an invisible control character is found again on the calendar server.
- An account removed while it was syncing no longer has its folder list come back.
- An account removed while it was connecting or making a change no longer has its files come back.
- On Linux, a MailCraft started by another program packaged as an AppImage (a `mailto:` link clicked in a browser, say) no longer puts its update over that program's AppImage: the program passes on where its own file is, and the updater took it for MailCraft's.
- On a Linux desktop running Wayland without X11's libraries (which the `.deb` and `.rpm` only recommend), MailCraft opens on Wayland instead of failing to start: it asked for XWayland, where it opens by default, without looking for them first. The log names the ones missing.
- On macOS 26, Chinese in the interface and in mail is drawn in PingFang again, which macOS moved: it came out in a Japanese or Korean font. Traditional Chinese takes STHeiti first, as PingFang's first face there is Simplified.
- Chinese, Japanese and Korean words in a subject, a name or a message sit on the same line as the Latin letters and digits beside them, as in a browser: they were centred on it, a little higher or lower.
- In the Flatpak, a screen reader (Orca) reads MailCraft's window, new mail is announced by the desktop's notifications, and the icon in the notification area is there: the sandbox let MailCraft reach none of them.
- An internal error while the window or one of its windows was being drawn (a message, a dialog, a pane) no longer closes MailCraft with what's being written in it: it's said, the dialog it may have come from is put away, and MailCraft goes on. Only a window that fails frame after frame still closes, and keeps what's being written in Drafts first.
- A window that was on a monitor since unplugged opens on screen when another monitor of the same size is left: it opened where no monitor was, out of reach. MailCraft now looks at where the monitors are, not their sizes alone, and brings a window it couldn't be dragged back by its top edge onto the main monitor.
- On Windows, MailCraft started again in the instant the running one sets up to answer launches no longer waits five seconds and opens a second window: a launch that asked too early asks again, and Windows sometimes didn't tell the running MailCraft the first ask was gone, so it waited for it while the second one timed out.
- On Windows, a pen or a finger moves the window by MailCraft's own title bar and resizes it from its edges, and drags a maximized window out restored, under the pen where it took it. It did nothing, and afterwards the mouse couldn't move the window either until MailCraft was restarted. A resize stops at the window's smallest size, as the mouse's does.
- A screen reader says what each drop-down is for: **Font size** while writing, **When mail** and **Then** in a rule, **Show as**, **Reminder** and **Repeat** on an event or task, **Language** and **Dates** in **File › Options**, and the others with them. They were read only by what they showed ("combo box, 15 minutes"), and one showing nothing by nothing at all.
- On Linux, a mail file MailCraft is started with opens in front of the main window: its window opened behind it, as the main window took the focus once it had drawn.
- Typing above a signature with a picture in it no longer makes the text under it jump at every letter (#114).
- Ctrl+A in a reply or forward selects all of it, the original under your text included (#118): it stopped at a paragraph, selecting only your text or the block of the original with the cursor. Copy and Cut take all of it, Delete removes all of it and typing replaces it, and Undo brings it back.
- A right-click on a calendar in the Calendar's list of calendars opens its menu, from its name or anywhere on its row (#119): its name took the click, so **Remove this calendar** for one you subscribed to, **Refresh** and **Use as default calendar** couldn't be reached.
- On a Mac, Control-click opens the right-click menus, as in other Mac apps (#119): it was taken as a plain click, so on a Mac with no second button or two-finger click no menu could be opened. Command-click still adds a message to the selection.

## 0.38.0 - 2026-11-13

*Messages as they look, on paper and from a file.*

### Added

- An account can have other addresses to send from (#81): the aliases and identities a provider gives one mailbox (iCloud, Gmail's "Send mail as", Exchange, Fastmail). Add them under **File › Info › Other addresses**, each with the name shown on its mail, where replies to it go and one of the account's signatures; a JMAP account's are found on its server with **Find on the server**. **From** lists them when you write a message, mail sent to one is answered from it, and the mailbox is still read and kept once. A mail server that refuses the address is said so, with the address. Mail from another address is signed with the certificate or OpenPGP key that names it: import it with **Add another…** under Security, where it's kept beside the account's own and says which address it signs for; with none that names the address, the message isn't signed as someone else.
- Mail can be moved or copied into a folder of another account (#82): from a mail server into an Outlook data file opened in MailCraft, from the file onto a server, or between two servers. Drag it there (hold Ctrl to copy), or pick the folder in **Move to** or the new **Copy to…**, which list every account's folders under the messages' own. It keeps its read mark, flag and categories. A message that isn't downloaded yet is downloaded first, and one that's moved is deleted where it was only once its copy is in the folder. Dropped on another account's folder, mail did nothing.
- In the browser, everything MailCraft keeps can be saved to a backup file and brought back (#83): **File › Open & Export › Back up…** downloads the accounts and their settings, rules, signatures, calendars and contacts, and the downloaded mail when asked, as one file, locked with a password if you give one; **Restore…** (there, and on the welcome page of a browser that has nothing yet) brings it into this browser or another. The browser keeps all of it for the site only, so clearing its data lost it for good. Passwords are never in the file, and nothing goes through a server.

### Fixed

- A mail file opened with MailCraft opens in front: its window came up behind the main window.
- A mail file's pictures from the web can be shown: **Show pictures** loads them for that message, when you ask. They never load by themselves, whatever its sender says, and asking doesn't make its sender one whose pictures always load.
- **Print** and **Save as PDF** show a message as it's laid out on screen: its columns side by side, its backgrounds, lines and buttons, the pictures it carries and those from the web that have been shown, and links that can be clicked. Tables came out as rows of text between lines of dashes, and pictures as their names.
- A shop's notice reads as it was sent (#47): the steps of an order drawn with their dots and lines, words set bold by the box around them (a step's name, a total), what a cell puts at its right standing there, an item's picture at the size its style sheet gives it on a wide screen, a divider no wider than its column, and a faint tint behind a picture.
- Paragraphs stand as far apart as in a browser: an empty line between them was half as tall. Room a box asks for above it (`margin-top`) is kept too: a footer's first line stood against the top of its box.
- A mail file's links say where they go only when their words name another place: with every link's address after it, a message's columns no longer fitted.
- A signed message is checked for the sender it shows. When the signed part carries its own From, that's the one shown, and the signature was checked for the one outside: mail signed by one person read as signed by the one it named inside.
- The certificate kept for someone, to encrypt to, isn't replaced by a lesser one: any signed mail naming them did (one from no authority this computer trusts, or the certificate they had before, when an old message was read again). Certificates are asked about at their authority only when it's one this computer trusts; an OpenPGP key from a message dated years ahead no longer stays in place of every later one.
- A draft kept encrypted is still sent encrypted when it's opened again (it went in the clear unless the account encrypts everything), and opens only once it has been decrypted.
- A signed or encrypted message taken back from the Outbox opens as it was written, to be signed and encrypted again as it goes, at the time it was to go: it opened as the signature's or the encryption's own file, and the message that waited was gone. What was decrypted for one Outbox message is no longer shown for the one that takes its row.
- Looking for an account's calendars and contacts, the mail password goes only to a server of the account's own that asks for it. It went with the first request to every place tried: the address's domain (a web site, for most), a host a DNS answer named, wherever a redirect led. A calendar server at another name than the address's and the mail server's isn't found by itself any more: give its address under **Calendars and contacts**. A sign-in no longer follows a redirect to another site either.
- On Windows a link or a file opens MailCraft with nothing after it read as an option: a `mailto:` link with a `"` in it could run commands as the window opened, point MailCraft at another data folder, or have another file opened.
- What a crash said is no longer written to the log or shown: only where it happened.
- A calendar rule that never matches (`FREQ=DAILY;BYSETPOS=2`: a day has no second day) no longer freezes MailCraft: an invitation or a calendar file with one was gone through to the end of time, and the calendar and its reminders never came back.
- From an Outlook data file, appointments on the last weekday of a month come once a month (not on each of its last five weekdays), those on the 31st fall on the last day of shorter months, and every-other-week ones keep their weeks where Outlook's weeks start on Sunday (`WKST` is read).
- A calendar rule with an end that isn't a date no longer ends an Exchange account's connection at every sync. An Exchange server that closed an idle connection no longer has the password asked for again; nor does a POP mailbox that's busy.
- Folders whose names have a `"` or a `\` in them open, and take mail: they were shown and asked for with the server's escapes still in.
- Mail moved or copied into another account keeps its categories and the day it came; a slow server no longer gets it twice; one that never got an answer can be moved again; and put into another account's Inbox it isn't taken for new mail (a rule filed it straight out again). While it's on its way to a server that's said (#85), and again when it's there.
- **Undo** that can't be done any more (the mail was deleted for good since) is dropped, not kept in the way of every Undo after it. Moving a message to the folder it's in leaves it open.
- A server that fails after signing in is tried less and less often, not every five seconds for good; a connection that got an answer it couldn't read is closed, not used again.
- Automatic replies on a server without them read "Not available: this server doesn't offer automatic replies", without "the server refused:" in the middle.
- From one of an account's other addresses (#81): **Put everyone in Bcc** puts that address in To, not the account's main one; a renamed signature stays the address's; a follow-up to your own mail from it comes from it; and an invitation sent to it is answered from it (answered from the main address, the organizer's program took none of it).
- A meeting answer counts only from the guest it's for: one that named its sender in another part of the meeting set someone else's answer.
- What's being written isn't lost: a message closed while it was being signed opens again when signing fails; removing its account leaves it open, to go from another (with no account left, it's kept as MailCraft closes and is back when it opens); and a draft saved on a server no longer opens a second time while it's still being written.
- A recipient after a name with an unbalanced quote is said to be wrong: it was left out of the message without a word.
- `mailcraft-cli send` says so at once when a message is held to be checked (`--checked` sends it), and waits for one that's signed first (it said "timed out" after the message had gone).
- A message with a megabyte of stray `<` opens at once (it took minutes). A file with a Content-ID (a PDF from some invoicing programs) is listed with the message's files; colours written with spaces (`rgb(34, 34, 34)`), a style rule after `@import`, a picture's address written without quotes and the last of two declarations are read.
- An attached message saved with bare line feeds goes with whole line ends: mail servers refused it.
- Deleting from Deleted Items asks first wherever it's asked from: a row's bin, the menu, a message's own window and the reading pane didn't. Ctrl+Enter sends only the message being written beside the list, asking about an empty subject as its Send button does, and not from another box beside it.
- **Move to folder** names other accounts by their address (each had the same name: yours), and Tab leaves its search box. A rule's **Run it now** runs only once the rule is saved, and what the rule dialog asks for is said in your language.
- File › Info no longer redraws without end with a renewed certificate or key listed, and doesn't offer an expired key for sending.
- The status bar says "1 account disconnected" for one (it read "1 accounts").
- In the browser, **File › Open & Export** offers what works there: **Open a data file** and **Export to Outlook data file** did nothing (a page reads and writes no data files), and are no longer shown. Rules and categories go to a file and back there too: **Export…** downloads the file and **Import…** takes one from the browser's picker, where both buttons did nothing.

## 0.37.0 - 2026-11-12

*Fixes found by testing against real mail servers and awkward real-world mail.*

### Added

- **Keep MailCraft running when its window is closed** (File › Options › General, off unless you turn it on): closing the window puts it away, and MailCraft goes on checking for mail and announcing it. On Windows, and on Linux desktops with a notification area, the window is hidden and MailCraft's icon there opens it again; under Wayland, without a notification area, and on a Mac, the window is minimized instead. Starting MailCraft again brings it back too. **Exit** (the icon's, or the File page's) and a Mac's **Quit** end it (#67).
- **A mail server on this computer with a certificate of its own can be trusted**: a program that connects a mail service to mail apps (a bridge) makes its own certificate, which MailCraft refused. Now it asks, showing the certificate's fingerprint, and **Trust this certificate** keeps it for that account, for receiving and sending. Only for a server on this computer; MailCraft asks again if the certificate changes, and a server anywhere else is checked as before (#77).
- **To-Do Bar** beside the mail (View › To-Do Bar), as Outlook's: a month to go to a day from, what's coming up in your calendars in the next week, and your tasks with the mail flagged in any folder, under when they're due. Type a new task into it, mark one complete, click a flagged message to open it, click a day to open the calendar on it. Each part is shown or closed on its own, and the parts and the bar's width are remembered (#79).
- The Calendar's small months show the days with something on them in bold.

### Fixed

- A message whose text comes in several parts shows all of it: text under an attachment (a note, a mailing list's footer) was left out.
- Text its sender wrapped softly (`format=flowed`) reads in whole paragraphs, not cut at each wrapped line.
- A subject or sender's name written in Latin-1 or another old character set, unencoded, reads as written, not as `caf�`.
- A reply or forward keeps the `[tag]` its subject starts with (`Re: [Ticket#123] Printer`), which a help desk matches answers on.
- Two notifications with the same `[tagged]` subject are no longer shown as one conversation.
- A reply to a message that names only the message it answers keeps the whole conversation together in other mail programs.
- A message can't mark itself as checked: only the result your own mail server put at the top counts, where one written by the sender lower down was believed (and its pictures could load by themselves).
- A draft keeps a recipient whose name has a comma (`"Park, Zoe"`) whole when it's saved.
- Addresses pasted with only a space or a line end between them each get the message: the first was taken for the second's name.
- A folder a server names `R&D-Team` without encoding it shows its whole name, and one it lists with letters outside ASCII is opened by that name, not another.
- A folder with more than about 130,000 messages syncs: the account signed in again and again instead.
- A change to another message, told by the server while new mail is fetched, no longer adds an empty message to the list.
- An answer from an IMAP server that MailCraft's reader can't take (`INTERNALDATE NIL`) no longer stops the account until MailCraft starts again.
- On a server without UIDPLUS, moving or deleting a message no longer removes for good the messages another program had marked as deleted.
- New mail is noticed at once after a message is deleted for good, and after a folder that's gone from the server was opened.
- Sending waits up to ten minutes for a server that checks a message before it answers, instead of sending it a second time after one.
- In the Outbox, the selected message stays selected when one above it is sent: Delete could take the next one, which was then never sent.
- After a delete or a move, the message opened is the next one down as the list shows it, with pinned messages and conversations too.
- A message closed while it's being sent opens again if the server then refuses it: it was in neither Drafts nor the Outbox.
- A rule, a Quick Step and a snoozed message follow a folder that's renamed or moved: the rule's mail stayed in the Inbox, and the snoozed message never came back. Snoozing a message that's already snoozed changes when it comes back.
- Mail you put in the Inbox of an account on a server (dragged there, a delete undone, **Not junk**) isn't taken for mail arriving: a rule could file it straight back, and it was announced as new.
- `larger:2 MB`, with a space before the unit, searches by that size.
- Mail from the first days of this month is under **Earlier this month**, not **Last month**.
- In a small window the message read or written keeps a width it can be read in: the folders and the list give way. At the smallest size it was a strip one word wide.
- A reply written in a narrow reading pane: the ribbon's rows are one under the other (wrapped, they were far apart), and **Save draft** isn't drawn under **Discard**.
- A message in a window of its own: the toolbar's last buttons go without their labels when the window is too narrow, as the ribbon's do. They were cut off at its edge.
- A very long subject is four lines over its message, all of it under the pointer: it filled the reading pane.
- A conversation's messages listed in the reading pane: the first words stop before the time, and a long name before the words.
- Calendar in a narrow window: the days shown are no longer drawn under the **Day**, **Work week**, **Week** and **Month** buttons (those are on the ribbon), and **All day** fits beside the days in every language.
- Tasks and People: a title, a status or an address too long for its column ends in "…". It was cut mid-letter.
- **File › Info** in a narrow window: the page stays inside the window. In French and Russian the names at its side are whole.
- In a narrow message list the sender's name keeps its room: the folder's name and **External** before the time give way.
- An HTML message with text parts after it (a mailing list's footer) or before it shows them too: only the HTML was shown.
- A message with two **From** or two **Subject** lines shows the first of each, as other mail programs do, and is never marked as from a checked sender.
- A date written the old way, `04:07 EST`, is read in that time zone: it was taken for UTC, hours off.
- A message the server took but never answered about (it stopped responding, or the connection dropped) is no longer sent again by itself, which is how mail arrived twice: it comes back saying it may have gone.
- Calendar: events side by side in a narrow day show the start of their title on one line. It was a letter a line.
- People: in a short window the A–Z index shows the letters that have names, all within reach. Its end was under the window's edge.
- A message opened by a command, or from its conversation in the reading pane, is brought into view in the list.
- In Arabic and Hebrew a notice appears in the left corner, clear of what opens from the status bar.
- Mail that arrives while MailCraft is minimized is announced, and counted in the window's title, when it arrives: it waited until the window was opened again (#68).
- **Print** and **Save as PDF** keep a message's formatting: headings, bold, italic, sizes, colours, highlights, underlined links and struck-through text. It printed the plain text (#72). Tables print as rows of text, and pictures as their descriptions.
- In a PDF, the lines a table is drawn with in text are printed with `-`, `|` and `+`: they were empty boxes.
- A logo or a title a message centres (`<center>`, `align`, `text-align`) is centred when the rest of the message flows. Only mail laid out in tables kept what it centred (#74).
- A message forwarded or answered shows the pictures from the web it quotes, when you were shown them reading it: they were empty boxes (#74). Nothing is fetched for it.
- The window's title bar, where the system draws it, is light or dark as MailCraft is set to be (File › Options › Theme): on Linux it followed the system alone (#74).
- An Outlook data file import that was paused or cut off isn't continued on a file that has changed since, even when the change left the file's size and date as they were: MailCraft goes by what the file holds.
- In Arabic and Hebrew what you type in the search box starts at its right edge, and in a narrow window the box is laid out from the right as the rest of the window is.
- In Arabic and Hebrew the People pane is mirrored: pictures and names on the right, the A–Z index on the left, and the list view's columns from the right.
- A new account's calendar and contacts are read as soon as its Inbox is in. They waited for every mail folder, so a big mailbox's calendar stayed empty for as long as its first sync took (#80).
- An Exchange calendar or contacts folder the server won't give says so, with why, and appointments the server fails on are counted: nothing was said, and the calendar just stayed empty (#80).

## 0.36.0 - 2026-11-11

*MailCraft as a project of its own: its own publisher, app id and releases.*

### Changed

- MailCraft is a project of its own: File › About, the installers and the Linux packages name Zinadin Zidan as its publisher, and their help links go to MailCraft's GitHub issues.
- MailCraft's id is `io.github.zidan44.MailCraft` (it was `ai.storyteller.mailcraft`). Updating tidies what the old one left: its notification name and settings on Windows, its preferences and saved window state on macOS, a default mail app chosen on Linux, and its files in an unpacked `.tar.gz`.
- The Flatpak is a new app under the new id. It brings your accounts and settings across from the old one, then says how to remove that (`flatpak uninstall --delete-data ai.storyteller.mailcraft`).
- Releases are published as **MailCraft vX.Y.Z**; test builds are still published for trying out what's next.
- A mail file you open (File › Open & Export, or Open with) shows in the same window as a message double-clicked in a folder, which you can move and resize. It still loads nothing from the web, isn't added to a folder and leaves no trace once closed, and a link whose words name another address says where it really goes (#56).

### Fixed

- The Flatpak can add an account on a desktop with a Documents folder: it couldn't write the data files it keeps in Documents › MailCraft Files.
- MailCraft's window shows and works while the system asks whether MailCraft may read its passwords (macOS does after an update), instead of staying blank until that's answered.

## 0.35.0 - 2026-11-10

*Mail built from templates reads as it was designed, a mail file opens from inside MailCraft, calendars and contacts sync with no email account, and each account checks for mail as often as you choose.*

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
- A calendar or address book you may not read no longer keeps the others from syncing, nor asks for your password again: it's kept as it is, and named (#61).
- On FreeBSD, a new account isn't started from a data file another account has open, as elsewhere: it was copied while it was being written.

### Added

- File › Options › Safety can show web pictures as a message opens, unless it looks suspicious: in Junk, from a blocked sender, or from a sender your mail server couldn't confirm. Off at first; a picture's server can see your IP address and when you opened the message.
- File › Open & Export opens a saved `.eml` or Outlook `.msg` file to look at as plain text, without adding it to a folder (#56). Opening one shows who else it went to, what's attached and where a link really goes.
- File › Options sets how often each account looks for new mail: automatically, as before, or every 5, 10, 15, 30 or 60 minutes (#58).
- Add account can add calendars and contacts only, with no email, from any CalDAV and CardDAV server (Nextcloud, Baïkal, SOGo, Radicale and others): give its address, your user name and an app password, and they sync both ways. A server that schedules meetings sends their invitations (#61).

## 0.34.0 - 2026-11-08

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
