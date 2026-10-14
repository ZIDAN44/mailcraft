# Gaps

What MailCraft lacks compared with the everyday Outlook mail workflow, ranked by how often people
hit it. v1 scope follows [storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177):
calendar, contacts, Exchange (EWS / Graph / ActiveSync) and PST import are deliberately later.

_Status: v0.1, 2026-10-15._

| # | Gap | Notes |
|---|---|---|
| 1 | **Rich-text compose** | Messages are plain text. Next: bold/italic/lists/links with a `text/html` alternative, and pasting pictures inline. |
| 2 | **HTML rendering** | HTML mail is shown converted to text (safe, private). Next: a sanitised HTML renderer with "show pictures" per sender. |
| 3 | **Outbox** | Sending needs a connection; offline sends fail with a message. Queue them and send on reconnect. |
| 4 | **Folder management** | Create, rename, delete and move folders; favorites chosen by the user. |
| 5 | **OAuth2 sign-in** | Gmail and Microsoft 365 increasingly require OAuth (XOAUTH2 / OAUTHBEARER) instead of passwords; app passwords work today. |
| 6 | **Autoconfig** | Server settings are guessed from the domain. Add Mozilla ISPDB / `autoconfig.` lookup and SRV records (RFC 6186). |
| 7 | **QRESYNC** | Expunges are found with a UID search; QRESYNC's `VANISHED` would make large folders cheaper (docs/sync.md). |
| 8 | **Notifications** | Desktop notifications and a taskbar/dock badge for new mail. |
| 9 | **Rules and categories** | Inbox rules, colour categories, pinning, snooze. Focused/Other sorting. |
| 10 | **Attachments, more previews** | PDF and video previews ("Open in PdfCraft" / FilmCraft), drag attachments out to Craft apps. |
| 11 | **Contacts autocomplete** | Suggest addresses from mail already seen. |
| 12 | **Print and export** | Print a message; save as `.eml`; export to PDF. |
| 13 | **Localisation** | The interface is English only; the Craft apps' i18n system should be adopted. |
| 14 | **JMAP** | Second transport (RFC 8620/8621) behind the same Job/Event interface. |
| 15 | **Calendar, contacts, Exchange, PST** | Deferred per #2177 (CalDAV/CardDAV first). |

## Done in v0.1

IMAP/SMTP accounts with keychain passwords; unified inbox; local cache with incremental sync
(CONDSTORE, UIDVALIDITY, IDLE, offline change replay); date-grouped list with conversations,
filters, multi-select and drag to folder; reading pane right/bottom/off with conversation strip and
picture previews; reply / reply all / forward with quoting and signatures; drafts; delete / archive /
junk / move / flag / read; offline search with field operators; dark mode; CLI; sample mailbox.
