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

## JMAP accounts (`crates/net/src/jmap.rs`)

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
