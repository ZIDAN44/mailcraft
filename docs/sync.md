# How MailCraft syncs

MailCraft shows only what is in its local cache, so it works offline. One worker thread per
account (`crates/net/src/worker.rs`) keeps that cache in step with the server.

## The local cache

The local cache lives on disk and is managed by the `store` crate (`crates/store`).

- Each folder keeps a `FolderState` with the folder's message summaries, newest first, and three
  counters from the server:
  - `validity`, the folder's UIDVALIDITY. The server changes it when the folder's message numbers
    (UIDs) are no longer valid.
  - `uid_next`, the folder's UIDNEXT: the UID the next new message will get.
  - `highest_modseq`, the server's change counter. It comes from CONDSTORE (RFC 7162), which lets
    MailCraft ask only for flags that changed. It is 0 on servers without CONDSTORE.
- Each message's source is saved as `<validity>/<uid>.eml`.
- A message is identified by its account, folder, UIDVALIDITY and UID (`MessageKey`). A key is
  never reused for another message.

## Syncing an IMAP folder

`crates/net/src/sync.rs` syncs one folder in these steps:

1. **Open the folder** with `SELECT … (CONDSTORE)`. When the folder is already cached, MailCraft
   uses `(QRESYNC …)` instead. QRESYNC (RFC 7162) makes the server report deleted messages and
   changed flags itself.
2. **Check UIDVALIDITY.** If it changed, MailCraft drops the folder's cache and its pending
   changes (RFC 4549).
3. **Update flags.** With CONDSTORE, MailCraft fetches only flags `CHANGEDSINCE <modseq>`, and
   skips this step when nothing changed. Without it, MailCraft fetches every flag.
4. **Remove deleted mail.** A `UID SEARCH` over the cached range finds which UIDs remain. Cached
   messages that are missing are removed.
5. **Fetch new mail.** `UID SEARCH UID <UIDNEXT>:*` finds new messages, and their headers arrive
   in chunks. A first sync takes the newest 1,000 by default.
6. **Save** the folder atomically: MailCraft writes a temporary file, then renames it.
7. **Download the newest 40 messages** of up to 2 MB each. Other messages download when opened.
8. **Count unread mail** with `UID SEARCH UNSEEN`.

Every fetch uses `PEEK`, so syncing never marks mail as read.

## The worker loop

Each account's worker repeats these stages:

- **Connect.** The worker connects over TLS or STARTTLS and signs in with `AUTHENTICATE PLAIN` or
  `LOGIN`. It then turns on QRESYNC with `ENABLE QRESYNC`, or CONDSTORE where QRESYNC is missing.
- **Sync everything.** The worker:
  1. replays pending offline changes;
  2. lists the folders with `LIST`, and finds each folder's role (Sent, Drafts…) from SPECIAL-USE
     (RFC 6154) or from the usual folder names;
  3. syncs every folder, Inbox first, and downloads messages the user opens between folders;
  4. syncs calendars and contacts (an account whose calendars were never read has them read
     right after its Inbox, so a big mailbox's first sync doesn't leave the calendar empty).
- **Wait for new mail.** IDLE (RFC 2177) keeps a connection open so the server can announce new
  mail at once. The worker IDLEs on the Inbox and renews it every 10 minutes. On servers without
  IDLE it checks every 2 minutes. Every 15 minutes it runs a full sync.
- **Handle failures.**
  - A lost connection is retried after a growing pause, from 5 seconds up to 5 minutes.
  - If the server refuses the password, MailCraft asks the user for it (`NeedsPassword`).
  - If the server refuses a command, MailCraft reports it and syncs the folder again.
- **Compact the data file.** While `worker/compact.rs` compacts the account's data file, the
  account's sync pauses.

## Offline changes

MailCraft follows RFC 4549, which describes how a client works while disconnected.

- Flags, moves, deletes and folder changes show in the local cache at once. `worker/edits.rs`
  also queues them, in order, in `<account>/ops.json`.
- Once connected, the worker replays the queue, oldest first. It removes each entry when the
  server answers.
- Each change becomes one IMAP command:
  - **Flags:** `UID STORE ±FLAGS.SILENT`.
  - **Move:** `UID MOVE`. On servers without MOVE: `UID COPY`, then `\Deleted`, then
    `UID EXPUNGE`.
- If a queued change carries an outdated UIDVALIDITY, the worker skips it.
- If the server refuses a change, the worker drops it, syncs the folder again from scratch and
  tells the user.
- Deleting a folder moves it to **Deleted Items** first.

## Sending

Mail that can't go right away waits in the **Outbox** (`<account>/outbox/`) and goes later.

- MailCraft builds each message with `mail-builder`. The message has no Bcc header.
- It sends over SMTP on port 465 (TLS from the start) or 587 (STARTTLS).
- It files a copy in **Sent Items** with `APPEND (\Seen)`. Gmail files its own copy, so MailCraft
  doesn't.
- A reply sets `In-Reply-To` and `References`. The original message gets the `\Answered` or
  `$Forwarded` flag.
- A message waits in the Outbox when MailCraft is offline, the server times out or SMTP answers
  with a temporary error (4xx). MailCraft retries it after connecting, after each sync and on
  **Send now**.
- If SMTP refuses a message for good (5xx), it stays in the Outbox with the error. Edit or delete
  it to clear it.

## Other account types

Every account type shares the same jobs, events, local cache and Outbox. Servers that name
messages with strings get stable numbers in their place, used as UIDs under UIDVALIDITY 1.

- **JMAP** (`crates/jmap`) bundles calls into a few HTTPS requests: `Email/query`, `Email/get`,
  `Email/set` and `EmailSubmission/set`. The server pushes changes over an event stream; without
  one, MailCraft checks every 2 minutes. The client is async over an `Http` trait, so the
  browser uses it too.
- **Microsoft Graph** (`crates/net/src/exchange/graph.rs`) serves Exchange Online accounts
  without IMAP.
  - It signs in with OAuth and asks for ids that don't change when a message moves.
  - It fetches headers in `$batch` requests and waits as told by `Retry-After` when the server
    answers 429 (too many requests).
  - It sends the message's MIME source with `sendMail`.
- **EWS** (`crates/net/src/exchange/ews.rs`), Exchange Web Services, serves on-premises Exchange
  2010 SP2 and later.
  - It signs in with NTLMv2 (Windows sign-in) or Basic, and finds the server with Autodiscover.
  - Calendars and contacts are in `ews_pim.rs`.
  - A moved message shows as a new one.
- **POP** (`crates/net/src/protocols/pop.rs`) gives an Inbox plus local folders. MailCraft
  downloads each message whose `UIDL` it hasn't seen. With `popDelete` set, it then deletes them
  from the server.

## Calendars and contacts

`crates/net/src/protocols/dav.rs` syncs calendars over CalDAV and contacts over CardDAV.

- MailCraft finds the servers as RFC 6764 describes, or uses the ones set by hand in
  `account.dav`.
- A sync sends local changes first, each guarded by its etag so it never overwrites a newer
  server copy. It then reads each collection whose `getctag` changed.
- Meeting invitations travel by mail as iMIP (RFC 6047). **Accept**, **Tentative** or
  **Decline** updates the calendar and sends the reply.

## Automatic replies on IMAP

`crates/net/src/protocols/sieve.rs` sets automatic replies through ManageSieve. It writes a Sieve
`vacation` script that `include`s the user's existing script, so the user's own rules keep
working.

## .pst files

`crates/net/src/data_files/pst.rs` opens a `.pst` file (Outlook data file) as an account with no
server.

- It reads the file with MailCraft's own `pst` crate and rebuilds each item as a standard message.
- It treats the file as hostile input. Bad items are skipped and counted, and sizes are capped.
- It refuses `.ost` files.

## In the browser

In the web build (`apps/mailcraft-web`), the local cache lives in memory and is copied to
IndexedDB. Jobs run over JMAP through `fetch`. New mail comes by push, and MailCraft checks every
2 minutes when the push stream is closed. Passwords stay in the browser tab only. See
[hosting the web build](../packaging/web/README.md).
