# How MailCraft syncs

MailCraft is local-first: the UI only ever reads the on-disk cache (`crates/store`), and per-account
worker threads (`crates/net/src/worker.rs`) keep that cache in step with the server. This page
describes the algorithm and the standards it follows.

## Per-folder state

For every folder the cache keeps (`FolderState`):

| Field | Meaning | Source |
|---|---|---|
| `validity` | UIDVALIDITY when the cache was filled | RFC 3501 §2.3.1.1 |
| `uid_next` | UIDNEXT at the last sync; UIDs at or above it are new | RFC 3501 §2.3.1.1 |
| `highest_modseq` | HIGHESTMODSEQ at the last sync (0 without CONDSTORE) | RFC 7162 §3.1 |
| `messages` | summaries (headers, flags, size, preview), newest first | |

Message sources live next to it as `<validity>/<uid>.eml`. A message is identified everywhere by
`(account, folder, UIDVALIDITY, UID)` (`MessageKey`), which RFC 3501 guarantees is never reused.

## A folder sync (`crates/net/src/sync.rs`)

1. **SELECT** the folder (with `(CONDSTORE)` when the server offers it). With QRESYNC enabled and a
   cached state, the SELECT carries `(QRESYNC (<uidvalidity> <modseq> <cached range>))` and the
   server answers steps 3 and 4 itself: `VANISHED (EARLIER)` lists the expunged UIDs and FETCH
   responses carry the changed flags (RFC 7162 §3.2.5), so only new mail is left to ask for.
2. **UIDVALIDITY check.** If it differs from the cache, every cached UID is meaningless: drop the
   folder's cache, including pending changes for it, and start over (RFC 4549 §4.1).
3. **Flag changes** for cached messages:
   - with CONDSTORE: `UID FETCH <cached range> (UID FLAGS) (CHANGEDSINCE <modseq>)` returns only what
     changed, and the fetch is skipped entirely when HIGHESTMODSEQ hasn't moved;
   - without: `UID FETCH <cached range> (UID FLAGS)` and compare.
4. **Deletions:** `UID SEARCH UID <cached range>` lists the survivors; everything else (and its cached
   source) is removed.
5. **New mail:** `UID SEARCH UID <UIDNEXT>:*` (filtered, since `n:*` always returns the last message)
   then headers for those UIDs in chunks: `UID FETCH … (UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER])`.
   The first sync of a folder fetches only the newest `window` messages (1000 by default).
6. **Save** the new state atomically (temporary file + rename).
7. **Prefetch** the newest sources (40 by default, up to 2 MB each) with `BODY.PEEK[]` so they open
   instantly, can be searched, and fill in previews. Bigger or older messages download when opened.
8. **Counts:** `UID SEARCH UNSEEN` for the folder's unread count.

`PEEK` is used everywhere so syncing never marks mail as read.

## The worker loop

- On start: connect (TLS, or STARTTLS with a fresh capability list), authenticate (`AUTHENTICATE PLAIN`
  with SASL-IR when offered, else `LOGIN`; non-ASCII credentials go as literals), `ENABLE QRESYNC`
  (or `ENABLE CONDSTORE` without it).
- Replay pending offline changes, then **sync all**: `LIST` (special-use roles from RFC 6154
  attributes, or the usual names), then every selectable folder, Inbox first. Opened messages jump
  the queue between folders, so the UI stays responsive during a long first sync.
- Then **wait for mail**: `IDLE` on the Inbox (RFC 2177) in half-second slices, so new jobs interrupt
  it at once; re-issued every 10 minutes (servers drop IDLE after 30). Without IDLE, poll every 2
  minutes. A full sync runs every 15 minutes.
- **Failures:** a broken connection is retried with exponential back-off (5 s → 5 min). A refused
  password stops retrying and asks the user (`NeedsPassword`). A refused command is reported and
  the cache is resynced.

## Offline changes (RFC 4549 "pending actions")

Flag changes, moves and deletes are applied to the cache immediately (the UI updates before the
server answers) and recorded in `<account>/ops.json`. When connected, the worker replays them
oldest first:

- flags: `UID STORE <set> +FLAGS.SILENT (…)` / `-FLAGS.SILENT`;
- move: `UID MOVE` (RFC 6851), else `UID COPY` + `\Deleted` + `UID EXPUNGE` (RFC 4315; plain
  `EXPUNGE` only as a last resort);
- delete permanently: `\Deleted` + `UID EXPUNGE`.

Keys whose UIDVALIDITY no longer matches are skipped rather than applied to whatever now has that
UID. If the server refuses a change, it is dropped, the folder's sync state is reset (forcing a full
flag comparison) and the user is told.

Folder changes (create, rename, move, delete) are pending actions too: `CREATE` + `SUBSCRIBE`,
`RENAME` (subfolders follow; the cache directories move with them), `DELETE`. Deleting a folder
first moves it into Deleted Items, like mail.

## Sending

Messages are built with `mail-builder` (Bcc never written into the message), submitted over SMTP
(implicit TLS on 465 preferred per RFC 8314, or STARTTLS on 587), then filed in Sent Items with
`APPEND (\Seen)` — except on Gmail, which files sent mail itself. Replies set `In-Reply-To` and
`References` (trimmed per RFC 5322 §3.6.4) and the original gets `\Answered` (or `$Forwarded`).

**Outbox.** A message that can't go out now (no network, refused connection, TLS failure, a
timeout, or an SMTP 4xx "try later") is written to `<account>/outbox/<id>.eml` with its envelope in
`<id>.json`, and the compose window closes as if sent. The worker tries again after connecting,
after every full sync and on *Send now*, oldest first, stopping at the first failure that may
clear up. A permanent refusal (5xx) keeps the message in the Outbox with the error until the user
edits or deletes it.

## JMAP accounts (`crates/jmap`)

An account with a JMAP session address (`AccountConfig::jmap`, RFC 8620/8621) syncs over HTTPS
instead of IMAP and SMTP, behind the same jobs and events. Each request is one POST of a batch of
method calls, signed in with Basic authentication or an OAuth bearer token.

- **Keys.** JMAP message ids are strings that stay the same across mailboxes. The worker numbers
  them as it first sees them (`<account>/jmap.json`, numbers never reused) and uses the number as
  the UID, with a constant UIDVALIDITY of 1, so the cache, the engine and offline changes are the
  same as for IMAP. Numbers no cached message or pending change uses are pruned after a full sync.
- **Folders** are named by their path of mailbox names joined by `/` (the Inbox is `INBOX`); the
  mailbox id behind each path is kept with the numbers. Roles come from the mailbox `role`.
- **A folder sync** is two requests: `Email/query` (newest first, up to the window) with an
  `Email/get` of the keywords by back-reference and the mailbox's counts, then the `headers` of
  the messages not cached yet. Mail that leaves the window leaves the cache. The newest sources
  are downloaded as blobs.
- **Changes** are `Email/set` patches (`keywords/$seen`, `mailboxIds/<id>`), destroys for
  permanent deletion, and `Mailbox/set` for folders.
- **Sending** uploads the message, imports it into Sent (`$seen`) and submits it with
  `EmailSubmission/set` from the identity for the sender; a refused submission removes the copy.
  Offline sends use the same Outbox. Drafts are imported with `$draft`.
- **Push.** Between jobs the worker holds the session's event stream open (RFC 8620 §7.3,
  `text/event-stream`), reading it in half-second slices so jobs never wait, the way IMAP holds
  IDLE; a `state` event syncs the Inbox. Servers without an event stream are polled every two
  minutes.

The transport is written once (`mailcraft-jmap`, async over an `Http` trait) and shared: the
desktop worker runs it over its own HTTP client, the browser app over `fetch`.

## Exchange Online through Microsoft Graph (`crates/net/src/graph.rs`)

An account with a Graph address (`AccountConfig::graph`) syncs with Exchange Online through the
Microsoft Graph REST API, signed in with an OAuth token that asks for Graph's mail permissions
(`Mail.ReadWrite`, `Mail.Send`, `MailboxSettings.ReadWrite`). It works where an organisation has
switched IMAP off.

- **Ids** are asked for in their immutable form (`Prefer: IdType="ImmutableId"`), so a message
  keeps its id when it moves; they are numbered like JMAP's, with the same folder paths.
- **Folders** come from `/me/mailFolders` and each one's `childFolders`, following
  `@odata.nextLink` pages on the same server only. The well-known folders (`inbox`, `sentitems`,
  `deleteditems`…) are looked up in one JSON batch for their roles; the Outbox and Exchange's own
  bookkeeping folders are left out.
- **A folder sync** lists the newest messages' ids with `isRead`, `flag` and `isDraft`, then the
  details of those not cached yet (their internet headers when Exchange has them) in batches of
  20. Sources come from `/messages/{id}/$value`; mail with attachments downloads when opened.
- **Changes** are `PATCH`es (`isRead`, `flag`), `move`, `DELETE` and folder calls, batched.
- **Sending** posts the MIME source to `sendMail`, which files the copy in Sent Items; recipients
  only on the envelope go in a `Bcc:` header, which Exchange strips. Drafts are made from MIME and
  moved where they belong.
- **Automatic replies** are the mailbox settings' `automaticRepliesSetting` (scheduled when the
  dates are set). Graph doesn't tell a mailbox's size to its owner, so there is no quota.

## On-premises Exchange through EWS (`crates/net/src/ews.rs`)

An account with an EWS address (`AccountConfig::ews`, `https://<server>/EWS/Exchange.asmx`) syncs
with an organisation's own Exchange server (2010 SP2 and later) through Exchange Web Services, for
servers that don't offer IMAP. Every call is SOAP over HTTPS.

- **Signing in** follows what the server's first `401` offers: NTLMv2 (`crates/net/src/ntlm.rs`,
  as `NTLM` or `Negotiate`) on a connection kept open, since NTLM signs in the connection rather
  than each request; or Basic. The user name is `DOMAIN\user` or an address; the password stays
  in the keychain. Plain `http://` is refused except for local test servers.
- **Folders** come from one deep `FindFolder` under the message folder root, in pages; the
  distinguished folders (`inbox`, `sentitems`, `deleteditems`…) are looked up in one `GetFolder`
  for their roles, and the Outbox, Exchange's bookkeeping folders and calendar, contact and task
  folders are left out.
- **A folder sync** finds the newest items with their read, flag (`PidTagFlagStatus`) and draft
  (`PidTagMessageFlags`) state, then the details of those not cached yet with `GetItem`, fifty at a
  time; sources are `GetItem` with the MIME content. Item ids are numbered like JMAP's; Exchange
  gives a moved item a new id, so it shows up as a new message in its new folder.
- **Changes** are `UpdateItem`, `MoveItem`, `DeleteItem` and the folder calls, fifty items a call.
- **Sending** is `CreateItem` with the MIME source and `SendAndSaveCopy` (the copy goes to Sent
  Items); envelope-only recipients go in a `Bcc:` header. Drafts are `CreateItem` with `SaveOnly`,
  marked unsent.
- **Automatic replies** are `Get`/`SetUserOofSettings`. Calendars, contacts and the quota aren't
  synced over EWS yet.

## Calendars and address books (`crates/net/src/dav.rs`)

Password accounts sync calendars over CalDAV (RFC 4791) and address books over CardDAV
(RFC 6352). The homes are found as RFC 6764 describes: an SRV record, then the `/.well-known/`
addresses on the mail domain, its `caldav.`/`carddav.` hosts and the IMAP host, through the
user's principal; a server with none is asked again a day later. A sync first sends what changed
here (deletions with `If-Match`, edits guarded by the item's etag so a newer copy on the server is
never overwritten, new items with `If-None-Match: *`), then reads each collection whose `getctag`
moved: the etags by `PROPFIND`, and the new or changed items by `calendar-multiget` /
`addressbook-multiget`. The engine merges the result: what changed here meanwhile stays to be sent.

**Invitations** travel by mail (iTIP over iMIP, RFC 5546/6047): a `text/calendar` part with
`METHOD:REQUEST` shows as an invitation card (Accept, Tentative, Decline), which updates the
calendar and mails a `REPLY` to the organizer; replies and `CANCEL`s update the event; events with
guests send them requests when saved and cancellations when deleted.

## Automatic replies on IMAP servers (`crates/net/src/sieve.rs`)

On IMAP hosts, automatic replies are a Sieve script (RFC 5228) with a `vacation` action
(RFC 5230), limited to dates with `currentdate` (RFC 5260), kept over ManageSieve (RFC 5804). A
script the user already runs stays in charge of their other rules: ours `include`s it (RFC 6609).

## Outlook data files (`crates/net/src/pst.rs`)

*Open Outlook data file* creates an account with no server named after the file and reads the
`.pst` (MS-PST, Unicode or ANSI, with Microsoft's `outlook-pst` crate) on a thread of its own:
the mail folders under the top of the mailbox become folders, and each mail item is written
again as a standard message from its properties (addresses, dates, text and HTML bodies,
attachments, pictures shown by `cid:`, embedded messages, read and follow-up flags), keeping its
original internet headers when it came in by mail. The file is hostile input: the reader runs
behind `catch_unwind`, unreadable items are skipped and counted, and sizes are capped.

## In the browser (`apps/mailcraft-web`)

The web build keeps the cache in memory, mirrored to IndexedDB, and runs each account's jobs in
order over JMAP and `fetch`. New mail comes by push: the session's event stream is read through
`fetch` (an `EventSource` can't send the `Authorization` header), a `state` event syncs the Inbox,
and the Inbox syncs once whenever the stream opens. Without push the Inbox is polled every two
minutes; everything syncs every eight checks either way. Passwords stay in the tab; after a
reload the account asks for its password again.
