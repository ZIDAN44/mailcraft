# Architecture

```
            ┌──────────────── apps/mailcraft (eframe) ────────────────┐
            │  keychain · file dialogs · system font · clock           │
            │                                                          │
   input ──▶│  ui-egui (L6) ──commands──▶ engine::Session (L4)         │
            │      ▲  reads state              │  queues Jobs           │
            │      └──────────────── Events ◀──┼── net::Pool (L3) ◀── IMAP·SMTP·JMAP·Graph·EWS·DAV
            │                                  ▼        │               │
            │                          store (L1) ◀─────┘ writes cache  │
            └──────────────────────────────────────────────────────────┘

            ┌──────────── apps/mailcraft-web (eframe web runner) ─────────┐
            │  the same ui-egui and engine; store in memory ⇄ IndexedDB    │
            │  WebBackend: jobs run per account ── jmap (L2) over fetch ──▶ JMAP
            └──────────────────────────────────────────────────────────────┘
```

## Layers

| Layer | Crate | Depends on | Notes |
|---|---|---|---|
| L0 | `mailcraft-model` | — | Pure data + algorithms; `Job`/`Event` are defined here so `net` and `engine` agree without depending on each other |
| L1 | `mailcraft-mime` | model | `mail-parser`, `mail-builder`, `html2text` (formatted text), `html5ever` (the tokenizer, for table layouts) |
| L1 | `mailcraft-store` | model | Plain files + JSON; atomic writes; no database engine |
| L1 | `mailcraft-print` | — | Messages laid out as PDF pages with an embedded TrueType font (`ttf-parser`, `miniz_oxide`) |
| L1 | `mailcraft-smime` | — | S/MIME (RFC 8551) on RustCrypto: `.p12` files, signatures, encryption (RSA and ECDH), revocation answers; keys behind a `PrivateKey` trait. Portable, wasm-safe |
| L2 | `mailcraft-keys` | smime | Keys that stay where they are: smart cards through their PKCS #11 module (`cryptoki`), the macOS Keychain (`security-framework`), the Windows certificate store (`rustls-cng`). Native only; the one crate whose `unsafe` is denied rather than forbidden (`windows/cng.rs`, `macos/sec.rs`) |
| L2 | `mailcraft-jmap` | model, mime, store | The JMAP transport, written once as async code over a two-method `Http` trait: the desktop runs it over `net`'s client (answers are ready at once, so a no-op `block_on` drives it), the browser over `fetch`. Portable, wasm-safe |
| L3 | `mailcraft-net` | model, mime, store, jmap | Native only. IMAP (`imap-proto`), POP3, SMTP (`lettre`), JMAP push, Microsoft Graph, Exchange Web Services with NTLM, CalDAV/CardDAV (`quick-xml`), ManageSieve, a small HTTP/1.1 client, DNS for autoconfiguration, OAuth, Outlook data files (`outlook-pst` to read and `outlook-pst-rw` to write, both built from MailCraft's fork with its fixes, branches `craft/1.2.0` and `craft/rw-1.2.14`); TLS through `rustls` |
| L4 | `mailcraft-engine` | model, mime, store | Sans-I/O, wasm-safe |
| L6 | `mailcraft-ui-egui` | engine | egui only; platform services behind traits; interface languages (`i18n/*.tsv`) |

`cargo xtask layers` enforces this, including that `egui` stays in L6 and network crates stay in `net`.
`cargo xtask wasm` checks that every crate but `net`, and the web app, build for `wasm32`.

Threads stay out of the portable crates. Slow work in the window (decoding a message's pictures,
reading and parsing the open message) goes through `Services::background` to a lane: the desktop app runs one thread per lane
(`apps/mailcraft/src/desktop/lanes.rs`), each piece of work under `catch_unwind`, and gives a
lane caught on cancelled work for 30 seconds a new thread. Where the work is given back (the
browser, the tests) the UI does it a step a frame.

## Source layout

Modules use the 2018+ layout (`foo.rs` beside `foo/`, no `mod.rs`), split by topic, with every
file at most 1000 lines and every folder at most 16 Rust files (`cargo xtask sizes`, rules in
`CLAUDE.md` › File size). The big types are one `impl` continued across files:

| Where | What |
|---|---|
| `engine/src/session.rs` + `session/` | `Session`'s state; its `impl` continued per area (`opening`, `changing`, `writing`, `outbox`, `searching`, `accounts`, `events`, `prefs`…; `organize/` for junk, Quick Steps, Search Folders) |
| `engine/src/cmd.rs` + `cmd/` | the registry; each area (`home`, `navigation`, `compose`, `view`, `pim`, `accounts`) returns its commands |
| `ui-egui/src/app.rs` + `app/` | `MailApp`, the window: `frame`, `ribbon`, `bars`, `search_box`, `keys`, `actions`, `windows` |
| `ui-egui/src/panes/`, `dialogs/`, `reading/`, `rich_edit/`, `look/` | the panes, the dialogs by group, the reading pane and compose window, the editor, and theme/icons/motion/widgets/progress |
| `net/src/protocols/`, `exchange/`, `data_files/`, `fakes/`, `worker/` | IMAP/SMTP/POP/JMAP/DAV/Sieve; Graph/EWS/NTLM; `.pst`; the in-memory test servers; the worker pool |

## Transports

An account syncs over one of four transports behind the same `Job`s and `Event`s, chosen by its
configuration: IMAP + SMTP (`imap`, `smtp`), JMAP (`jmap`, a session address), Microsoft Graph
(`graph`, Exchange Online) or Exchange Web Services (`ews`, on-premises Exchange). JMAP, Graph and
EWS number their string message ids the same way
(`jmap::State`), so the engine and cache only ever see IMAP-style keys. Calendars and address
books sync over CalDAV / CardDAV for password accounts (or at addresses given by hand, with a
password of their own), and automatic replies over ManageSieve on
IMAP hosts. An account with none of these (the sample mailbox, an opened Outlook data file) lives
on this computer only: the pool applies its changes to the cache directly. A POP account
(`pop`) is the same after its mail is downloaded: only the Inbox talks to the server.

Each account is one data file, `<address>.mailcraft`, in Documents\MailCraft Files, as Outlook
keeps an account in `<address> - <name>.pst` in Documents\Outlook Files
(`crates/store/src/packed.rs`): a crash-safe single-file database (redb) holding the account's
cache, keyed by the paths the store always used. The account list and the app's own state stay
in the per-user app folder (`%APPDATA%\MailCraft`). `.pst` is only read (an Outlook data file
opened gets a `.mailcraft` file of its own; Outlook's file isn't touched) and written (export).
Account folders from before are brought into data files when the store opens, and removed once
read back; not while a store on the folder layout has them open, here or in another copy of
MailCraft: every such store holds `<root>/.locks/folders.lock` for its life (shared by
`Store::open`'s, alone by the app's, `Store::open_alone`, so a second window on its folder
layout runs without the cache instead of writing the folder states and `ops.json` over the
first's), and the bring-in takes it alone or brings nothing in ("open in another copy of
MailCraft", which the CLI answers with "close MailCraft first" and the window shows as the move
that didn't happen). The data files' folder can be moved (`crates/store/src/location.rs`): `location.json`
in the app folder points at it, and a move asked for (`move-pending.json`) is carried out at the
next start, before anything opens the files, every file checked before the old ones go. The
sample mailbox and the browser keep plain files and memory. On Windows a data file's database
reads through ten handles of its own, each thread through its own while it's free
(`crates/store/src/packed/handles.rs`): through one handle every read waited for the reads,
writes and flushes before it, from every thread. Writes, flushes and the lock that keeps
another copy of MailCraft out stay on the database's own handle, the lock from its lock bytes
(far past any data) rather than from the file's start, where it held the other handles' reads
back. A file kept compressed in pieces is uncompressed as its pieces are read
(`packed/kept.rs`, `read_file`), so a big message's source is held once, never compressed and
uncompressed at the same time.

How a data file commits (`crates/store/src/packed/commit.rs`): writes from the window are durable
at once. Writers in the background (a `.pst` import, a MailCraft data file brought in) prepare
what they write away from the file (`Store::prepare` compresses it on their own thread) and
commit batches of at most `BACKGROUND_COMMIT` (2 MiB, a quarter of the write buffer) that don't
wait for the disk (`Store::write_prepared`); the folder lists naming them are written durably
after them, and a batch that would leave more than 8 MiB unwritten is durable itself. What's
waiting is counted with the file's writer held, so the count follows the order of the commits. A
write from the window writes out what's waiting with its own: in tests 8 MiB added 10 to 30 ms to
it, and against an import going flat out its p99 reached 45 to 230 ms on a busy machine (13 ms
with nothing else writing), over the 60 ms aimed at; those timings are measurements run on their
own (`--ignored`), as they swing too far for the test suite. A data file's cache is 16 MiB
(`CACHE`), half of it the write buffer a transaction holds its changes in; while an import writes,
its account's file has 32 MiB (`Store::bulk_begin` / `bulk_end`, `packed/open.rs`): the file is
closed and opened again with the other cache when nobody holds it, at once or the next time it's
asked for, and never under someone using it. A commit isn't sealed
(redb's quick-repair, measured at three times a small commit's cost on a 37 GB file): a data file
is sealed by an empty commit once its writes have been quiet for two seconds, at an import's end
or when it gives up (`Store::seal`), and as the app or the CLI exits (`Store::seal_all`, which also
seals every commit after it: the threads holding the store are let go, not waited for, so the files
are seldom closed). A crash after a seal reopens at once; one between a write and its seal costs a
full check of the file at the next open, and a file checked is sealed as it opens, so a check is
never repeated however the process then ends. While an account takes a bulk write, every commit of
its file that waits for the disk is sealed and batches wait for it 256 MiB at a time
(`BULK_UNDURABLE_MAX`), so an import killed at any point reopens at its last durable commit at
once (12–126 ms on a 37 GB file, where a check took up to 55 s). A long local write takes the
same (`Store::long_write`, counted, with no other cache): a move of more than a batch of sources
(1,000 or 16 MiB) writes its batches without waiting for the disk, its last batch as a sealed
commit refused if the file lost what didn't wait meanwhile (`Store::write_batch` with
`data_file_reopened`), and the lists and counts after it sealed too; a file written since its last
seal is sealed as the long write begins. Removing an account waits for a seal under way on its
file, so nothing still holds the file open. Opening a data file holds no lock over the others: the
account is marked as opening, and only a reader of that account waits for it (a full check takes
minutes on a big file). `Store::packed` opens nothing; `Store::open_data_files` brings folders from
before in, then opens every data file in turn, saying how it goes in `OpenProgress` (which file,
whether it's being checked and how far, and the ones that couldn't be opened). The database says
how far its check is only as each pass over the file begins (0, 60 and 90%), so `OpenProgress`
also counts the bytes the data files read (`DataFile::counted`) and moves on from what it last
said by the share of the file read since, short of the next, never going back. The desktop app
calls it on a `data-open` thread and makes the session there (`apps/mailcraft/src/desktop/boot.rs`):
its window draws ui-egui's `BootCard` until then, which shows nothing for the first 300 ms, so a
start with nothing to check has no flash of it.

How a data file is doing (`crates/store/src/packed/health.rs`, `Store::data_file_health`) is
measured without reading any mail: one read of its list of files, its pages counted as the
database counts them (whole values and the notes of files in pieces) and each file in pieces as
the page runs its pieces fill, comes within 2% of the pages the database has in use (0.4% in the
tests against redb's own walk of every page, on a file of mixed sizes and on one of small messages
only; that walk needs a write transaction and takes minutes on a cold 37 GB file, where this one
took 0.32 s cold, opening the file included, 6 ms warm and 34 MB at most for its 34,741 files),
and the file's size over that is what compacting would give
back, less what the file had over the count just after its last compaction
(`Store::data_file_compacted`; 2% until then). One file in 128 is sampled: one kept raw has up to
64 KiB of its first piece compressed for real, so attachments a build before 0.32 kept raw are
found and pictures that don't compress aren't counted. A `meta` table, which old files lack and old
builds never open, says how the file was written, each value read on its own and one that can't be
read ignored: a file this build makes says, sealed as it's made, that everything in it is kept
compressed as it keeps it (so it isn't sampled, even after a crash or a full drive before anything
else reached the disk), and a recompress keeps the last key it did there. What a sample finds is
scaled to the files it's drawn from, not the search index, which a recompress passes over.

A recompress (`crates/store/src/packed/compact.rs`, `Store::recompress_data_file`) compresses
again what such a file keeps raw that compresses, the first half of compacting it. It goes through
the list of files in key order, a batch at a time: read in a read transaction of its own (at most
`BACKGROUND_COMMIT` as kept, a bigger file alone up to 64 MiB), compressed away from the file up
to eight ways at once, then written in a short transaction not waited for to the disk, with the
last key it got to in `meta` (`recompress.cursor`), so a stopped or killed run goes on from there
and a write from the window waits for one batch at most. A file is written back only if it's what
was read (its length and a hash of every piece), so one written again meanwhile is left as
written; that's read again with the writer held only when something was written since the batch
was read (the data files count their writes as they begin and end). A move that puts a file kept
raw behind the cursor, while a run is under way or stopped, pulls it back (`recompress.back`), so
nothing moved is left behind. The search index's files, those compressed already and small ones
are passed over; those tried that stayed as they were (pictures) are counted in `meta` with what
the others saved, and so are those kept raw it passed over (damaged, or over 64 MiB:
`recompress.passed`). Each batch is written only while its account is kept (`Store::while_kept`)
and never makes a data file, so removing the account stops the run without bringing its file
back. Its last commit is sealed: once stopped, to put the cursor on the disk; at the end, to say
every file was looked at (`deflate`: what's left raw doesn't compress, or was passed over). The
`data_file` fuzz target builds a hostile file in memory from records and checks every file reads
back the same after it.

redb's own compaction (`Database::compact`) is the second half: it needs the database to itself
and can't be stopped or followed. Measured cold on copies of a 45 GB file compressed again
(`packed/tests/compaction.rs`), it reads the whole file at least once whatever it moves (63 s for
35 GiB with nothing to give back) and about 5.3 s more for each GiB it moves (125 s to move
8.7 GiB of 44 GiB); a copy of the files into a new data file took as long (127 s) and needs the
room for it. An earlier run took half as long again (other work on the machine, likely), so a
prediction needs a margin.

Both run in the store (`crates/store/src/packed/shrink.rs`). In place
(`Store::compact_data_file`), the account's file is taken out of the open ones with the account
marked as opening, so whoever asks for it meanwhile (a sync, a read, a removal) waits, but for a
read from the window's thread (`store::no_waiting`, set as the window opens), which fails at once
("being compacted") rather than freeze it, and taken up once nothing holds it (five seconds at
most, else refused); the compaction's commit after is sealed,
and how long it took against the measured cost (about 1.8 s per GiB of the file plus 5.3 s per GiB
moved) is kept as the file's pace (`compact.pace` in `meta`) for the next estimate. A compaction
that fails or panics closes the file rather than give it back, and it's opened (checked if need
be) when next asked for. By a copy (`Store::copy_data_file`), every file goes into a new database
beside it (`<file>.compacting`) in key order, in background batches, compressed again on the way
when kept raw, while the account goes on as ever: each key written meanwhile is noted as it's
committed, and once the copy has everything the file is had to itself just long enough to copy
those again (compressed as the batches are), seal the copy with what recompressing had counted,
and rename it over the file (tried again while another program holds it a moment; the folder
flushed after, on Unix). A file with a file in pieces that don't add up isn't copied (the copy
would be without it). Stopped, it's thrown away; a copy a crash left is removed as the data files
next open.

What's done is the plan's (`mailcraft_model::job::plan`, from the file's health, how it was asked,
and the room on its drive): in place when the part that has the file to itself is estimated to fit
how it was asked (15 s when MailCraft is idle, 30 s on closing, any length when asked for by hand),
compressing again first what an earlier build kept raw; a file kept raw that's too big for that is
copied when its drive has room (never on closing), and importing it again is suggested when it
hasn't. The pool runs it on a `mail-compact` thread (`crates/net/src/worker/compact.rs`), holding
the account's jobs while its file is had to itself (in place, and at a copy's end), and begins
no import beside a compaction (nor a compaction beside an import); the engine keeps each account's health and
compaction (`Session::compaction`), measures data files when the window says MailCraft has been
left alone for ten minutes (`dataFile.idle`, at most once a day each, sooner when a recompress was
stopped part way), compacts those that need it, refuses commands on an account whose file is had to
itself (and imports into one being compacted), holds its events until it's given back, and reads
nothing of it from the window meanwhile: a search that reads it waits, and a folder's list not in
memory isn't read. `dataFile.compact` asks for it by hand
(`mailcraft-cli run dataFile.compact` runs it there), or as MailCraft closes (`"mode": "close"`).
The window (`ui-egui/src/app/compacting.rs`) calls `dataFile.idle` every 30 s with the time since
the last input (a message's or its source's own window counts too), stops an idle compaction on any
input (`dataFile.cancelCompact`), keeps the part that has a file to itself on screen in a dialog
that can't be put away, and on closing, when `prefs.compactOnClose` is on (off by default),
compacts the data files `Session::close_compactions` finds quick enough from their last health and
waits for each of them (`CompactUi::waiting`, the dialog going on to the next as one ends), then
offers a longer one by hand; the File page's row is `panes/backstage/data_file.rs`. The idle tick
runs in `ui`, which the desktop app doesn't call while the window is minimized: no idle
compaction starts then.

A local account's read marks, flags and categories don't commit in the window's frame (each was a
durable commit of the data file, 24 ms by median on a 38 GB file, up to 1.4 s when the disk was
slow). The window changes its lists in memory and hands the flags its messages have now, with the
folders' counts, to the store's `mail-marks` thread (`Store::mark_later`, `store/src/marking.rs`),
which writes each in one durable commit, in the order they were made: a list kept in pages takes
them in `state-marks.json` beside its pages (512 at most; more go into the pages, and any save of
the whole list empties it), a small list is written whole. A local account's folders counted
again (after a move, a draft filed) go the same way (`Store::count_later`), after the marks before
them, and so does a folder list the window writes whole (`Store::save_folders_later`). Whatever
reads a folder's list from disk (`folder_load`) or writes one whole first waits for the marks handed
over before in that folder, whatever reads or writes the folder list for the account's, and
`seal_all` for all of them, so a move files a message with its mark and closing seals it; nothing
waits for another folder's or another account's, and the thread's own store waits on nothing. The
window doesn't read a folder whose marks are still to be written (`Store::marks_waiting_for`): a
search reads the other folders meanwhile and that one after, and `Session::load_cached` leaves it
unread for now (`WouldBlock`, as while a compaction has the file to itself), remembered in
`marks_deferred` and read by `tick` once they're written, the list built again if it's on screen.
One said changed on disk meanwhile (`Session::folder_changed`) keeps its list in memory, and the
message read and the selection with it, until then; a command acting on all of a folder's mail
(Delete All, rules run now, Clean Up) waits for its marks instead (`Session::folders_ready`). A wait
for the marks, or for the lock every change of a list takes, over 50 ms is logged with the thread
and the caller that waited. A mark that couldn't be written comes back through
`Store::take_marks_failed`, and the session's `tick` says so and reads the folders again. The
window's own settings (`ui.json`) are written on a thread of their own too
(`apps/mailcraft/src/desktop/state_writer.rs`), as each write is flushed to the disk, and so are
the lines of `mailcraft.log` (`apps/mailcraft/src/logfile.rs`): a line logged on the window's
thread is handed over, never written there.

## The browser (`apps/mailcraft-web`)

The same `MailApp` runs in eframe's web runner. The store is `Store::memory()`, loaded from
IndexedDB at start; it notes the files that change (`Store::take_changes`) and each frame's
changes are written back, so mail stays between visits (settings stay in local storage; never
passwords). The `WebBackend` runs each account's jobs in order on the browser's event loop with
`mailcraft-jmap` over `fetch`, holds the push stream open (read through `fetch`, parsed by
`mailcraft_jmap::sse`), polls as the fallback, and answers what a browser can't do (IMAP,
OAuth sign-in, data files) with an explanation. `Services` tells the UI files are downloads and
that the file picker answers on a later frame. `Session::jmap_only` keeps the account form to
JMAP.

## The session (`crates/engine`)

`Session` holds the accounts (config, folders, connection status), the view (a folder, or a role
across all accounts such as *All Inboxes*), the filter and search, the message list, selection and
focus, the open message (`Reading`, parsed on demand from the cached source), the message being
written (`Compose`), preferences and status-bar notices.

- The open message's body can be got ready away from the window's thread
  (`session/opening/prepare.rs`): once the app turns it on (`defer_bodies`, after the start-up
  `--cmd`s, where there is a reading lane), showing a message queues a `BodyWork` (read from the
  cache, or as it arrived, then parsed) that the window runs on the reading lane, latest only,
  and hands back to `finish_body`; a result for a message the pane moved on from is let go.
  Signed or encrypted mail is still unwrapped on the session's thread (it needs its keys).
  `Session::run` finishes the waiting body before any command but moving about the list, so a
  command acts on the message as shown; the CLI and the tests never defer.

- The list is indexed as it's built (`refresh_list` → `ListNav` in `session/mailboxes/nav.rs`):
  `index_of`, `summary`, `selected_iter` and `first_selected` are lookups, not passes over the
  list, and a message's conversation is worked out with the list. What's drawn each frame uses
  them; `selected()` copies and is for commands. The list's rows (headings, conversations,
  pinned mail) are built with it too, and again by `tick()` when the day turns: `rows()` lends
  them, Up, Down and Shift+click look their row up (a step copies one key, a Shift range as
  many as it selects), and the interface lays the rows out again only when `nav_structure()`
  changes. A flag or category changed on mail in the list is taken into the list as it stands
  (`patch_list` in `session/mailboxes/patch.rs`) rather than by building it again, where that
  gives the same list: a read mark counts its row's unread again, mail the filter now leaves out
  goes, a list arranged by flag is arranged again and a pin lays the rows out again; with a
  search, the changed messages alone are weighed against it again (built again only when what
  it finds changed); the Outbox, deleted and junk marks, and mail the filter lets in are built
  again. The proptest `patched_list_equals_refresh` holds it to `refresh_list`.
  The list, a folder's list in the cache (`FolderState.messages`), the list kept while a search
  shows and what a search found all hold their summaries shared (`SharedSummary`, an
  `Arc<Summary>`, kept on disk as the summary itself): showing a folder or a search copies
  pointers, not summaries. A summary changed in place (`Arc::make_mut`) is copied only while
  another list shares it, so the cache and the list part as they did when each held its own; a
  patched list then takes the folder's summary again (`patch_list`, `filled`).

- `run(id, params)` dispatches a command from the registry (`cmd.rs`), checking `enabled` first and
  catching panics.
- `take_jobs()` hands queued work to the network layer; `apply(event)` folds results back in.
- Changes are optimistic: `apply_op` updates the in-memory list immediately, then the worker applies
  it to the cache and server and reports `FolderChanged`, which reloads from the cache (the single
  source of truth). A flag or category change's own echo doesn't (`session/mailboxes/echo.rs`): the
  changes sent are counted per folder, and a `FolderChanged` while one waits keeps the folder in
  memory (patching in the change if the folder was read again before it reached the cache); one
  more than were sent, a notice from the account, or a minute's wait reads the folder again. A
  message downloaded comes back as its summary filled from its source (`Event::Filled`, made by
  `mailcraft_jmap::keep_source` for every protocol), which the folder in memory and the list's
  entry take (`patch.rs`), built again only where what changed decides what the list shows; the
  text searches keep for that summary is made again (`FolderTexts::changed`).
- Local-only accounts (the sample mailbox) apply changes to the cache directly, including moves
  (new UIDs in the destination) and, for the sample mailbox alone, sending (filed in Sent).
- Mail is sent only from an account that can send it (`session/writing/sending.rs`):
  `Session::send_block` says why not (no account, a data file, a mailbox kept only here, no
  outgoing server), `compose.send` is off with that reason, and `Session::deliver`, which every
  way of sending reaches, refuses it, as Send now in the Outbox and meeting answers do.
- Data files imported into accounts (`session/accounts/importing.rs`, commands in
  `cmd/data_files.rs`) are followed in `Imports`: each import running or waiting on the user, by
  account, with its last progress and time left (`model::job::Eta`, on the session's clock), and
  `imports.json` (beside `rules.json`), where each account's mail came from and how far its
  import got (`DataSource`: written as an import is asked for, as it ends, stops or is kept, and
  as its account is removed). An account removed while its file is read stays, its import told
  to throw it away (`Job::StopImport{Discard}`, its id not given again), and goes with what came
  in once the import says so; when that never comes it leaves the list after `REMOVE_BACKSTOP` events, its data
  file and record left to the import (which removes the file as it ends) or to the next start,
  and a removal not finished when MailCraft closed is finished as it opens. An import notes where it got in
  the account's own data file (`import.json`, `pst/resume.rs`): the folders done by node, the
  one part-way through with its next row, next number and list tail (`ListWriter::tail`,
  `Store::resume_list_writer`), the rows the reader failed on, the counts and the file's size,
  time, record key and fingerprint (each folder's node, items and bytes), the Sent Items folder
  made in its place and not read yet (`deferred`), and the replies read for it (`sent`). It goes in with the
  batch it names (`Store::write_batch`: one durable, sealed commit) at least every 10 s, before
  finished folders are published, and as the import stops or ends (`Running`, `Cancelled`,
  `Done`). `ImportOptions::resume` (`Job::ImportPst{resume}`) goes on from it: done folders are
  skipped, the part-way one reopened (sources past its number dropped, `Store::drop_bodies_from`),
  and a file not the same (`Blocked::SourceChanged`, `SourceMissing`, `Locked`) waits on the
  user with nothing written; a `Done` note answers without reading the file. Kept as it is
  (`Job::KeepImport`, `pst::keep_as_is`), a stopped import's folders kept out of the list (the one
  part-way through, the Sent Items folder put off) are finished from their tails, sources past
  them dropped, and listed, and the note says `Done`. The room left where
  the data file is written (`net::space`, `fs4` on the nearest folder there) is asked as the
  import begins, with every checkpoint and every 64 MiB written (`pst/out/room.rs`): under
  `LOW_SPACE` (1 GiB) plus what the data file can grow by in one step (`store::data_file_growth`:
  redb grows it by doubling, then a 4 GiB region at a time) it stops itself as a pause does and
  waits as `Blocked::DiskFull`, nothing counted as left out. A write that fails at the disk makes
  the store let go of the data file (redb refuses everything after one until it's opened again,
  back at its last durable commit) and keeps the drive's own error, so a full drive stays
  `StorageFull`: the import then writes nothing more and stops at its last checkpoint. Whoever's
  write failed (the window's, the seal's), the store counts the file let go of
  (`Store::data_file_reopened`): an import that sees the count change stops the same way, and a
  checkpoint's write made past it is refused, so no checkpoint names batches that went. Only the
  failed handle is let go of, and opening the file again waits for whoever still holds it. A
  `.mailcraft` copied in is read from a copy made beside the data files, so it asks for room for
  both before it begins, and the same as an import every 250 ms; short of room, it waits with
  nothing kept.
  The note wins over
  `imports.json` as MailCraft opens (`accounts/importing/note.rs`): a `Cancelled` one isn't gone
  on with by itself, and a data file's account with a `Running` or `Cancelled` note and no record
  gets one back. An import interrupted
  (still `Running` in `imports.json`) goes on as MailCraft opens; one stopped or blocked waits
  for `account.resumeImport`, `account.keepImported` or `account.restartImport` (a POP account's
  connection waits with either); until then, as while it runs, the account's folders aren't
  made, renamed, moved, deleted, emptied or cleaned up and its mail isn't moved, deleted for good
  or snoozed, nor a move undone (`import_holds`, a `CmdError::Failed` the window shows), and what comes into its Inbox isn't new mail (no rules, nothing announced). A
  data file's account left with no folders and no record (from before this was kept, and with no
  note of its own import inside its file) is found at start as one to import again or remove.
  `account.inspectDataFile` keeps the last four files looked at (looked at on a thread of its
  own, a panic said as a file that couldn't be looked at; the copy's expected size, the room
  asked for and the room free in the data files' folder with them); a file already open as another
  account (the same path however written, or the same record key) is imported again only with
  `again`. Progress isn't said in notices: the window shows it (`account.importStatus` for the
  CLI and tests). In the window (`ui-egui/src/app/importing.rs`, its questions and summary in
  `dialogs/importing.rs`) an import is the status bar's background segment, ahead of the search
  index's build: its bar from `ImportProgress::permille` (items and bytes weighed by
  `ITEM_WEIGHT`), the time left from `Session::import_eta_s`, its details beside it; the folder
  pane has a line under the account, and its menu stops, continues or keeps it; under the
  account's folders, those its plan names that aren't in yet are listed faint, indented by
  depth (each run's plan comes as `Event::ImportPlanned`, the path each folder will get worked out by the writer as it
  will make it, `pst/out.rs`). A bar whose total
  isn't known slides a segment at most ten times a second (`look/progress.rs`). A file chosen is
  looked at (`account.inspectDataFile`) before `account.importPst`; closing while one is read
  says it goes on next time and pauses it (`account.stopImport {"how":"pause"}`); an import that
  finishes is handed to the window once (`Session::take_finished_import`) for its summary.

## Languages

Interface text is English in the code and looked up in a catalog per language
(`crates/ui-egui/src/i18n/*.tsv`: context, English, translation) through `tl!` and `tf!`; anything
untranslated shows in English. Commands, the CLI and mail stay English. The language follows the
system unless one is chosen in Settings; Chinese and Japanese load an installed CJK font.
Release builds also compile in fonts from [storytold/craft-fonts](https://github.com/storytold/craft-fonts)
(Arabic, Hebrew, Japanese, Chinese, Korean, Thai and more; in the browser the Japanese and Korean
interface fonts and the small Arabic, Hebrew and Thai ones) as the last fallbacks, with
their licences on the About page. It is an optional build input, never a Cargo dependency:
`CRAFT_FONTS_DIR=<checkout>` makes `crates/ui-egui/build.rs` embed what its manifest lists;
unset, the table is empty and nothing changes.

egui and winit are the Craft apps' forks (storytold/egui, storytold/winit), pinned by revision
in the root `Cargo.toml`'s `[patch.crates-io]` to the same revisions the other apps use; each fix
they carry is in the fork's PATCHES.md. That egui draws right-to-left text itself: the Unicode
Bidirectional Algorithm orders each line, Arabic is joined by the shaper, right-to-left paragraphs
start on the right, and a text box's caret and selection follow. MailCraft hands it text as
written, translations and mail alike; `ui-egui/src/look/bidi.rs` only says whether a paragraph
reads right to left (where a line painted by hand belongs) and whether the interface font has the
scripts (the main font's coverage, read with `skrifa`).

## Search

`model::search` parses the query language into clauses (`search/parse.rs`; dates in
`search/dates.rs`, placed in the local zone by `Query::resolve` from the clock the caller passes).
Words are read folded (`search/fold.rs`: lower-case, accents dropped, ß → ss and the like) on both
sides, from a table `cargo xtask fold` generates from Unicode's decompositions (`fold_table.rs`;
`fold --check` in `xtask ci`), so no normalizer is carried at run time. A query is checked with
`Query::eval` (`search/matching.rs`), which answers yes, no or unknown (Kleene logic): the summary's
clauses come from a folded `SummaryText` (a word is looked for in all of it at once, a quick look
most summaries fail, before in each part; made into one string, each word folded into it as it's
read, an ASCII text split into words a byte at a time by UAX #29's ASCII rules (`fold/ascii.rs`,
held to unicode-segmentation by a proptest), an ASCII word only lower-cased and split at its join
characters, the addresses folded a name and an address at a time: about 4x as fast on plain
summaries as a string for each part, word and piece, and byte for byte the same, which a proptest
holds against the old way), the text's from a `TextSource` (the index, or the text itself), which
answers unknown for a word it lacks when the text was cut short (a message read whole with no text
has no word: no). A word UAX #29 keeps joined by `.`, `:`, an apostrophe or `_` ("site.word") has
its parts of two letters or more indexed and read as words beside it (`fold::index_tokens`, `parts`;
`TOKENIZER_VERSION` 2).

The session runs a search a bounded unit of work at a time (`session/searching/run.rs`:
`start_search`, then `search_step(units)`), no clock and no network in it: a unit is a page of a
folder's list read (`Store::folder_load`), an account's index opened (in the window, ticks open each
account's ahead of any search, one a tick, so a search just after the window opens finds it open:
`open_indexes_ahead`, and kept open, and in memory, whether or not anyone searches; the tick takes
2-4 ms over the made-up 50,000 messages), up to two of its pages looked through for a query word and
the docs of about two pages' worth of its words set (`Reader::scan_prefix`: a one-letter word's
thousands of words, their docs in the lexicon page itself, are spread over units too, the lexicon
page looked through last kept decoded so a page is decoded once however many units read it; a word
looked up lately is taken as the same generation of the index gave it, `searching/look_up.rs`, 32
words an account), 512 summaries checked (128 whose folded text is made first, in a session just
opened), a message's kept text read or 256 KiB of it checked (a phrase across two slices is in the
second, which starts 4 KiB back), or putting the list together at the end (`searching/finish.rs`: a
search that streamed into the list adds what's left to it, arranged otherwise or put into
conversations when the list shows them so; one that waited gathers its finds from the folders, a
step each for gathering, arranging and showing). A phrase's candidates are checked in the index's
doc order, so each page of texts is read once. The folders a search reads stay in the cache while it
lasts, and their summaries' folded text is kept between searches (`searching/folder_text.rs`, 48
MiB, forgotten with the folder's list); a phrase's candidates take theirs from there, and the texts
they're checked against are kept folded, by message, for the next search (`searching/folded.rs`, 32
MiB: what the running search uses stays, what earlier ones used makes room, and past that a text
isn't kept), so a phrase typed on or searched again reads and folds nothing. Each is kept with the
doc it was read from, in the index's epoch: a key that comes back as another message (a local folder
made again numbers its mail from 1 again) is another doc, and its text is read afresh; a folder's
texts go when it leaves the listing. The index reader keeps the text pages it reads in a cache apart
from its lexicon and postings pages, so checking texts doesn't push out the pages words are looked
up in. The summary settles flags, dates, senders and subjects; the index's words settle the text
(`searching/matching.rs`: `has:attachment` takes the files the index counted, pictures the text
shows left out); a phrase, or an address the index can't look up, is checked against the text the
index keeps. A word the index doesn't find in a text cut short isn't: its words are in the index as
they are in that text, so the message is counted apart (`cut_short`), not as missing. What a
message's text said is kept for the search (flags are never text), so a flag changed on a result is
weighed again for that message alone (`search_patched`). A message the index doesn't have yet may be
missing (`missing`); a search that waits (the CLI, scripts, tests: the default) reads it from its
source instead (`mime::index_doc`), so it finds what the index would. The window turns waiting off
(`set_background_search`) and steps the search for 6 ms of each frame (`search_step_timed`, which
asks the window's clock after each unit and tells it what each part of the step was, `SearchWork`,
`searching/work.rs`: the window logs a part over 50 ms by its kind; the engine reads no clock). A
folder's list isn't read while read marks wait to be written (the read would wait for them): the
step goes on and reads it once they're on disk. What's found is merged into the list by date
(`searching/publish.rs`): at the step's end while the list holds at most 4,096 messages, else at the
start of a later step (where the window's clock counts it) once a step has gone by for each 2,048
messages it holds, so a long list isn't built again every step; the list's index keeps its keys'
hashes for that (`ListNav::build_hashed`), what's found carries its key's hash, made from its
folder's (`key_hash_in`: no key's strings hashed again), and a run's sets hold a message as its
folder's place, validity and number (`searching/found.rs`, `RunKey`), copying no strings; the
list a run shows at its end is put together as a step's first unit, never after others late in a
frame; and the list a search replaces is let go of 4,096
summaries a step or tick (what its candidates' texts said, 1,024). Conversations and other
arrangements come at the end. `sample::big` makes a mailbox of 50,000 messages in 40 folders, what
each measured search should find counted as it's made, for `tests/mail/search_big.rs`, the window's
frame test and `bench.rs` (which also counts what the summaries held take, `Summary::heap_bytes`,
each shared one once, through a 500-Down walk and searches). A step that panics ends the search, and the indexes it was
reading are left unread until their writer writes them again. The list says "Searching…" while it
runs, that results may be missing while mail isn't indexed yet (only then; mail too long to index
whole gets a quieter note: how far the passes under way have come, `indexing_work`, which the status
bar shows too through `ui-egui/src/look/progress.rs`), and what couldn't be read of the query
(`search_problems`). Mail coming into or leaving a folder has that folder scanned again (what it
found stays shown until the scan says otherwise); a newer generation of an index (`Event::Indexed`,
read again at most every 2 s, at once when the pass is over) has its account's words looked up again
and its folders scanned again; folders appearing or going start the search again.

The index is kept with each account's mail (`store/src/search.rs` + `search/`,
under `<account>/search/v1/`, so in the account's data file, on disk, or in memory like the rest of
it; a data file from before has none and opens unchanged). A meta (`meta.bin`) names what the index
is: a doc table of 32-byte records in pages of 4,096, copied on write under the generation that
changed them; the texts and files' names kept for each message (versioned records in text pages
of up to 256 KiB, a longer text alone in one, what phrases are checked against and words made
again from when the tokenizer changes, with no message read; one pool of pages for the whole
index rather than a set per segment: a removed message leaves its page in its own commit, so a
merge has only words to drop); and immutable segments of words (field letter + folded word: `b`
text, `n` names, `a` whole addresses and domains), each a head, front-coded lexicon pages and
varint postings pages. `search::Writer` adds, removes and moves docs in memory and flushes a new
generation in two steps: `prepare` encodes and compresses its files, `commit` writes them in
short transactions under names the current meta doesn't use, then the meta with the files it no
longer uses removed, in one sealed transaction (the data file's last commit, so a crash after it
costs no repair); a crash or a
failed batch before the meta leaves the generation before, and the writer removes what it wrote
when it next opens (only names the meta lets it work out). A message removed leaves its text page
in that same commit; its words go when its segment is merged. The writer never writes what its
reader would refuse: a folder name over 4 KiB is refused (that message isn't indexed), a text is
cut at 1.875 MiB (marked truncated; a million characters of plain text fit), so a page alone
still goes in one background transaction, and at 4,096 segments the words are made again from the texts.
One writer per account at a time, across processes too: a writer of the folder layout's or a
data file's index also holds a lease, the system's lock on `<root>/.locks/search-<account>.lock`
(`store/src/leases.rs`; the file is never deleted, and the lock goes however its process ends).
A second copy of MailCraft or the CLI on the same folder is told `Busy`, so its writer's clean-up
never removes the holder's uncommitted files: its indexer tries again every 5 s (said once in the
log), its searches read the index as the holder's commits leave it, and an index it finds damaged
is left to the holder to set aside. An index in memory takes no lease. The folder layout's own
folders lease (above) keeps a CLI's data files from bringing in an index the app is writing,
and a second app window from the index of the first's folder layout. `search::Reader` holds the doc table
and the segments' heads, reads lexicon, postings and text pages as needed (a page cache keyed by
the index's random epoch), looks words up a few pages at a time, and follows the writer by reading
only what a newer generation changed. Every file is decoded with checks (`search/format.rs`,
fuzzed by `search_index`); an index that doesn't read back is set aside (`meta.bin.bad`) and made
again past its generation and segment numbers. A page missing counts as damage only when the same
meta still names it (a newer commit removing it is read again), and while a writer holds the index
the reader returns the damage instead of setting it aside under the writer.

The index keeps up with the mail by reconciliation, not by hooks in each place mail changes:
`search::plan` compares a folder's list and kept sources with the docs the index has for it. A
message with a source and no doc is to add (newest first); one with no source kept is recorded as
such (`add_no_source`: a search settles it from its summary, and it isn't counted as waiting); a
doc whose message is gone, or whose Message-ID hash and size no longer match, is removed; a doc
going and a message coming with the same hash and size, each the only one so, is a move (the doc
goes along, nothing read). The plan reads no message, and passes over a folder whose revision (a
hash of its list's head and its sources' names) is the one the index was marked with when that
folder was last brought up; the marks are in the meta. `Writer::merge_step` keeps the segments
few, one step at a time: it gives the ids again (the index made anew from its kept texts, a new
epoch) once there are more than twice the live docs and 4,096; merges the segments holding words
of a doc removed a day ago or more (the meta keeps since when); merges eight segments of one size
(powers of 4 docs); and past 24 segments, the smallest eight. A merge streams each segment a
lexicon page at a time and writes the merged pages as they fill, in transactions nothing names
yet, then one meta switches to it: it holds a lexicon and a postings page of each segment, the
pages being written and the longest list of docs, never the segments. Which segments hold a
removed doc's words is known from each segment's span of doc ids only, so the large segment a
merge or new ids leave is merged again, whole, a day after any removal in its span (a removed doc
with no source never had words and needs none); and giving the ids again is one step that can't
be stopped partway (it reads every kept text). A plan's doc ids are those of the epoch it was made
in: `Plan::apply` refuses a plan made before the ids were given again, and makes its moves
together, so two messages swapping uids under a new validity keep their docs.

The index is built by `Job::Index { account, folders }`, which the session asks for as mail
changes (`want_index`: a folder changed, a body arrived, the folder list, an import's end; asks
waiting to be taken are one) once the window turns it on (`Session::set_indexing`; the CLI
doesn't). Both sides run the same `search::Pass`: it holds the writer, plans, makes the changes
that read nothing, is handed each message to add, writes a generation whenever it holds enough,
marks the folders only when it read all it planned, and merges. On the desktop
(`net/src/worker/indexing.rs`) one "mail-index" thread runs a pass at a time, once an account's
asks have stopped for 1.5 s; the messages are read and taken apart (`mime::index_doc`) on all but
two of the machine's threads (1 to 8), each under `catch_unwind` (one that panics is kept with no
words and counted), through a channel of 64 to the pass thread, the sources being read and the
texts waiting held to 48 MiB by their sizes. Each write of the index (a batch, a meta, a removal)
and each folder's list put right first lets the read marks handed over before it be written
(`Store::give_way_to_marks`, a second at most), so a mark from the window waits behind one of the
pass's writes at most. No pass runs for an account a
data file is being imported into, and `Job::StopIndex` (or the account's removal) stops one
between two messages. A `.pst` import indexes as it goes instead (`pst/out/indexing.rs`): it
holds the account's writer for the whole import (waiting up to 5 s for a pass asked to stop to
let go of it, and importing without it if it doesn't), the lanes make each message's words from
the message as rebuilt (`mime::index_doc`, then `search::Ready::of`, so the writer thread only
files them with `Writer::add_ready`), each message goes in once its source's batch is written,
handed with the batch to a thread of its own that holds the writer (at most 32 jobs queued, so
the import's writer thread doesn't wait on the index; let go of because writing stopped, it does
nothing more, not even the jobs queued), and each folder is marked as brought up as it is finished, so the pass at the import's end reads
nothing and passes every folder over. A write of the index that fails lets it go for the rest
of the import and costs the import nothing: the next pass adds what it lacks. Indexing as it
goes cost the import about a fifth of its time on a 41 GB file (against a 157 s pass after it)
while the writer thread filed and wrote the words itself; on a thread of its own, the import
takes no longer than one without the index, within the runs' spread.
Its progress, `Event::Indexed`, isn't sent on the pool's channel: only the
newest for each account is kept until `Pool::poll` takes it. A pass also compares what a message's
summary says about files with what its doc counted (pictures the text shows aren't files): the
plan for the docs it already has (a folder unchanged since it was marked is passed over, so the
marks carry a kind, bumped once to have every folder looked at again), each add as it comes.
What disagrees is put right in the folder's list by `Store::set_has_files` (the pages holding
one rewritten, under the lock every save of a list takes; at a generation written at most every
5 s, and at the pass's end) and sent as
`Event::AttachmentsKnown`, which the session takes into the summaries it holds
(`mailboxes/patch.rs`), so the paperclip, Has files and `has:attachment` agree; a doc that couldn't
be read, or whose parts were cut with no file found (`IndexDoc::parts_cut`: a file may be nested
past the walk's limits), leaves the summary to say (`DocInfo::files_known`). The lock keeps the
putting right from writing over a save, not a save that loaded the list before it from writing
the old flags back (a sync, a flag from the window), so a folder put right, or whose list
couldn't be written, isn't marked (`Pass::unmark`): the next pass looks at it again, from the
docs with nothing read, and puts it right once more. In the browser
(`apps/mailcraft-web/src/backend/indexing.rs`) the pass gives the page back every 16 messages
or 1 MiB read, and between merge steps.

## Threading

`model::thread` implements JWZ threading (as used by IMAP `THREAD=REFERENCES`): containers per
Message-ID, parent links from `References`/`In-Reply-To` without creating loops, empty containers
promoted away. A message the ids left without a parent, or under ids no listed message carries,
is then placed by Outlook's conversation index (`Thread-Index`, kept in the summary as hashes
of the whole index, of the index it answers
and of the conversation's 22-byte header): under the message it answers, or under the
conversation's first message when the base subjects match. Then replies that lost their
references are grouped by base subject. Every message appears in exactly one conversation;
conversations and the list go by `Summary::when` (when the message was received, where that is
known, else its date).

## Testing

- `model`, `mime`, `store`: unit tests, including hostile inputs.
- `net`: a scripted stream for framing edge cases, an in-memory IMAP server (`fake.rs`) for the sync
  algorithm and server-side changes, a local SMTP stub, an end-to-end test of the threaded worker
  pool over real TCP sockets, and in-memory JMAP (`fake_jmap.rs`), Microsoft Graph
  (`fake_graph.rs`), Exchange Web Services with NTLM (`fake_ews.rs`; NTLM also against the
  MS-NLMP test vectors), CalDAV/CardDAV (`fake_dav.rs`) and ManageSieve servers driving accounts
  through the same pool. Outlook data files are exported with `outlook-pst-rw` (MailCraft's fork, which adds to
  files Outlook made, permute- or cyclic-encoded, trusts their allocation map only when it agrees
  with the B-trees, writes a message's class with any tagged or named property, adding names to
  the file's name-to-ID map, and records special folders, and is fuzzed adding to a damaged one,
  `pst_append`, which checks that every message that read before still reads) and read back by
  the importer: `pst_export/item.rs` turns a message's source and what the store keeps of it
  (flags, categories, the follow-up day the job carries) into those properties, and
  `pst_export/place.rs` puts the account's Sent, Trash, Drafts and Junk folders in the file's own; contact and appointment mapping, recurrence patterns with their changed occurrences (`pst_pim/recur.rs`, fuzzed by `pst_recurrence`) and the files cards and events keep (`pst_pim/files.rs`, `pst/out/pim.rs`) are tested on their own, and on a file the writer makes (`pst/tests/pim_written.rs`: an `IPF.Appointment` folder with a repeating appointment whose pattern and times are named properties, its changed occurrence a hidden attached message, and an `IPF.Contact` folder with a contact whose picture is a flagged attachment, imported through `pst::import`). An import counts where its time went (`pst::ImportStats`: opening the file and its folders, reading, rebuilding, compressing, writing and committing, and the bytes it held and wrote), logs it at the end, and `pst_bench::import` (ignored) runs one on a big file as `pst::import_with` lets it be tuned, printing those numbers (`MC_VERIFY=1` then reads every message back and counts those present, parsed and their attachments, with digests to tell two imports apart); it also pauses an import at random, or first at a place (`MC_PAUSE_AT`), and times each stop (`MC_PAUSES`), goes on with one cut off (`MC_RESUME`, timing the data file's opening), and fits what an item and a byte took over the import's progress and over its folders, judging how far off the time left would have been, and how far off even the run's own average rate would have been (`MC_SAMPLES` keeps the progress for `pst_bench::eta::replay`). The bytes held (`peak_budget_bytes`) are sources as rebuilt, before compression, the size an import budget charges, so later figures compare with it. An import asked to stop (`ImportOptions::stop`, a `pst::Stop` raised from any thread and never lowered: pause, then cancel, then discard) says how and in which folder (`Imported::stopped`, `stopped_in`): cancelled, it keeps what it read; paused, the same, less the folder part-way through, which isn't listed until the import goes on or is kept as it is (`pst::keep_as_is`, through `Job::KeepImport`: the folder is listed with what its checkpoint names, and `Event::ImportKept` answers, the engine holding a POP account's connection until then); discarded, it writes nothing more. The pool runs one import per account (`worker/imports.rs`): `Job::StopImport` raises its stop, `Job::Remove` discards it (the data file it made goes, by the path it noted when it made it, and only while the account still has that very file: the engine lets go of a removed account's file before the pool hears of it, and may give its id to a new account at once, whose file stays; one still held is tried again when its thread is joined), and either is answered at once, as discarded, when nothing is importing. A discarded import says nothing more of the account but its end: an account removed while it imports goes when that end comes (the engine asks with `Job::StopImport{Discard}`, and its `Job::Remove` follows), and an end of a discarded import for any other account is passed over (its id may be another's by then). An import asked for while the account's last is being discarded runs on the same thread once that one is over; one queued so and discarded in its turn is answered at once, and the next asked for takes its place. `Pool::shutdown` (MailCraft closing, through `Backend::shutdown`) pauses every import and waits 10 s at most, while dropping the pool pauses them without waiting. The window is sent each folder as it finishes (the list at most twice a second) and the progress with the folders done of those planned; folders not yet finished aren't sent as folders: each run's plan is sent first as `Event::ImportPlanned`, and the window lists the folders not in yet as a faint outline, not open to changes. `Job::InspectDataFile` plans an Outlook file (`pst::inspect`: its messages, mail folders, other items and bytes) and the pool keeps the last plan, which the file's import is handed (`ImportOptions::planned`) while the file's size and time are as they were. A MailCraft data file brought in (`Store::import_data_file_until`) asks whether to stop as it's copied to be read (8 MiB at a time) and before each batch, and its copy goes either way; stopped part-way, none of it is kept (the pool removes it: a part of a data file, in the order it keeps its files, is folder lists naming messages not there, all of them the old account's). An import runs on several threads (`pst/plan.rs`, `lanes.rs`, `budget.rs`, `order.rs`): a planner opens the file, plans it (folders in order, what each is, its items and their sizes; a folder seen twice read once, nesting, folders and items capped) and hands out a numbered sequence, folder starts and ends to the writer and one job per item to the lanes; each lane reads items through a handle of its own and makes the message, summary and compressed source, within a byte budget (`ImportOptions::budget_bytes`) charged before an item is read and given back once it's written, the room given in the items' order (a lane waits while one with an earlier item waits for room, unless that one is charged more than the whole budget and so only waits for its turn), the item the writer needs next always let through; the writer, the calling thread, applies everything in sequence order, numbering messages as one lane would, and adds each finished folder to the account's list (`Store::upsert_folder`), telling the `pst::ImportSink`. The Sent Items folder (the store's `PidTagIpmSentMailEntryId`) is begun in its place but read after every other (`pst/out/sent_ids.rs`): the writer notes each message's id and, for a reply, its conversation index's parent (a hash) with the id it answers (In-Reply-To), up to 16 MB, then at a `Step::Freeze` barrier hands the planner a frozen copy for the lanes reading Sent Items, which give an item without an id or an original header the one its replies answer; the writer takes back, in order, one an earlier message has (rewriting that source with its made-up id), and `ImportStats` counts the ids given and taken back. The folder is read in its own `catch_unwind`, after a walk that failed too. Once it's read, the ids its items have and are given are kept past the cap (its rows bound them), so a checkpoint taken while it's read reads back whole; one without the map (written before) gives no ids. Every job is answered once, a lane's panic costing its item (32 in a row stop the import), and a fault in the planner or writer stops every thread; the tests compare one lane with four byte for byte and put a fault in each thread (`ImportOptions::fault`, tests only). The `.pst` is opened to read only, and on Windows no other program may open it to write while it's read; it is read through a cache of the ranges read (`pst/chunked.rs`: scattered pages read exactly, a block read within 4 MiB of where the last block read ended reads the 64 KiB chunk around it, as a big attachment's blocks lie close together in no order, reads that follow on read ahead 256 KiB, 4 MiB held), shared by the handles outlook-pst reads through, so the one that reads items can be let go of and reopened (`pst/open.rs`) once the B-tree pages it keeps are thought to hold 4 MiB, without reading the file again; `ImportStats` counts the bytes and reads and the reopenings. The reader is tested against the file read plainly over ten thousand random seeks and reads, and an import reopening its reader every item against one that doesn't. HTTP, DNS, autoconfig and OAuth run against local stubs. `serve_for_the_browser` (ignored)
  keeps a JMAP server with CORS running for trying the web app by hand.
- `jmap`: the session, URLs and paths against canned answers.
- `engine`: commands against the sample mailbox and scripted network events; every command with junk params.
- `ui-egui`: headless egui frames in every layout and dialog, shortcuts as input events, and a 2,000-message folder;
  every catalog covers every interface string, and a tour in a pseudo-language finds text that skips the catalogs.
