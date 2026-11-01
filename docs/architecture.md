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
| L3 | `mailcraft-net` | model, mime, store, jmap | Native only. IMAP (`imap-proto`), POP3, SMTP (`lettre`), JMAP push, Microsoft Graph, Exchange Web Services with NTLM, CalDAV/CardDAV (`quick-xml`), ManageSieve, a small HTTP/1.1 client, DNS for autoconfiguration, OAuth, Outlook data files (`outlook-pst` to read, built from MailCraft's fork with its fixes, `outlook-pst-rw` to write); TLS through `rustls` |
| L4 | `mailcraft-engine` | model, mime, store | Sans-I/O, wasm-safe |
| L6 | `mailcraft-ui-egui` | engine | egui only; platform services behind traits; interface languages (`i18n/*.tsv`) |

`cargo xtask layers` enforces this, including that `egui` stays in L6 and network crates stay in `net`.
`cargo xtask wasm` checks that every crate but `net`, and the web app, build for `wasm32`.

Threads stay out of the portable crates. Slow work in the window (decoding a message's pictures)
goes through `Services::background` to a lane: the desktop app runs one thread per lane
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
| `ui-egui/src/panes/`, `dialogs/`, `reading/`, `rich_edit/`, `look/` | the panes, the dialogs by group, the reading pane and compose window, the editor, and theme/icons/motion/widgets |
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
read back. The data files' folder can be moved (`crates/store/src/location.rs`): `location.json`
in the app folder points at it, and a move asked for (`move-pending.json`) is carried out at the
next start, before anything opens the files, every file checked before the old ones go. The
sample mailbox and the browser keep plain files and memory.

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
own (`--ignored`), as they swing too far for the test suite. A commit isn't sealed
(redb's quick-repair, measured at three times a small commit's cost on a 37 GB file): a data file
is sealed by an empty commit once its writes have been quiet for two seconds, at an import's end
or when it gives up (`Store::seal`), and as the app or the CLI exits (`Store::seal_all`, which also
seals every commit after it: the threads holding the store are let go, not waited for, so the files
are seldom closed). A crash after a seal reopens at once; one between a write and its seal costs a
full check of the file at the next open, and a file checked is sealed as it opens, so a check is
never repeated however the process then ends. Removing an account waits for a seal under way on its
file, so nothing still holds the file open. Opening a data file holds no lock over the others: the
account is marked as opening, and only a reader of that account waits for it (a full check takes
minutes on a big file). `Store::packed` opens nothing; `Store::open_data_files` brings folders from
before in, then opens every data file in turn, saying how it goes in `OpenProgress` (which file,
whether it's being checked and how far, and the ones that couldn't be opened). The desktop app
calls it on a `data-open` thread and makes the session there (`apps/mailcraft/src/desktop/boot.rs`):
its window draws ui-egui's `BootCard` until then, which shows nothing for the first 300 ms, so a
start with nothing to check has no flash of it.

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
  goes, a list arranged by flag is arranged again and a pin lays the rows out again; a search,
  the Outbox, deleted and junk marks, and mail the filter lets in are built again. The proptest
  `patched_list_equals_refresh` holds it to `refresh_list`.

- `run(id, params)` dispatches a command from the registry (`cmd.rs`), checking `enabled` first and
  catching panics.
- `take_jobs()` hands queued work to the network layer; `apply(event)` folds results back in.
- Changes are optimistic: `apply_op` updates the in-memory list immediately, then the worker applies
  it to the cache and server and reports `FolderChanged`, which reloads from the cache (the single
  source of truth).
- Local-only accounts (the sample mailbox) apply changes to the cache directly, including moves
  (new UIDs in the destination) and sending (filed in Sent).

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
clauses come from a folded `SummaryText`, the text's from a `TextSource` (the index, or the text
itself), which answers unknown for a word it lacks when the text was cut short or had none. The
session builds an inverted index (`search/index.rs`: `BTreeMap<word, docs>`, so a word prefix is
one range scan) over summaries, cached bodies and attachment names for the chosen scope;
candidates are verified against the full query, so the index may over-approximate but never
misses. The index is rebuilt lazily after changes.

The index that replaces it is kept with each account's mail (`store/src/search.rs` + `search/`,
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
One writer per account at a time within a process; across processes (the CLI and the app on one
folder layout) nothing stops a second yet: the `search/v1/lock` lease comes with the CLI's
`--index`, when the CLI first writes an index (open in [gaps.md](gaps.md)). `search::Reader` holds the doc table
and the segments' heads, reads lexicon, postings and text pages as needed (a page cache keyed by
the index's random epoch), looks words up a few pages at a time, and follows the writer by reading
only what a newer generation changed. Every file is decoded with checks (`search/format.rs`,
fuzzed by `search_index`); an index that doesn't read back is set aside (`meta.bin.bad`) and made
again past its generation and segment numbers. A page missing counts as damage only when the same
meta still names it (a newer commit removing it is read again), and while a writer holds the index
the reader returns the damage instead of setting it aside under the writer.

## Threading

`model::thread` implements JWZ threading (as used by IMAP `THREAD=REFERENCES`): containers per
Message-ID, parent links from `References`/`In-Reply-To` without creating loops, empty containers
promoted away, then replies that lost their references grouped by base subject. Every message
appears in exactly one conversation.

## Testing

- `model`, `mime`, `store`: unit tests, including hostile inputs.
- `net`: a scripted stream for framing edge cases, an in-memory IMAP server (`fake.rs`) for the sync
  algorithm and server-side changes, a local SMTP stub, an end-to-end test of the threaded worker
  pool over real TCP sockets, and in-memory JMAP (`fake_jmap.rs`), Microsoft Graph
  (`fake_graph.rs`), Exchange Web Services with NTLM (`fake_ews.rs`; NTLM also against the
  MS-NLMP test vectors), CalDAV/CardDAV (`fake_dav.rs`) and ManageSieve servers driving accounts
  through the same pool. Outlook data files are exported with `outlook-pst-rw` and read back
  by the importer; contact and appointment mapping and recurrence patterns are tested on their own. An import counts where its time went (`pst::ImportStats`: opening the file and its folders, reading, rebuilding, compressing, writing and committing, and the bytes it held and wrote), logs it at the end, and `pst_bench::import` (ignored) runs one on a big file as `pst::import_with` lets it be tuned, printing those numbers (`MC_VERIFY=1` then reads every message back and counts those present, parsed and their attachments). The bytes held (`peak_budget_bytes`) are sources as rebuilt, before compression, the size an import budget charges, so later figures compare with it. An import asked to stop (`ImportOptions::cancel`) keeps what it read and says so (`Imported::cancelled`). HTTP, DNS, autoconfig and OAuth run against local stubs. `serve_for_the_browser` (ignored)
  keeps a JMAP server with CORS running for trying the web app by hand.
- `jmap`: the session, URLs and paths against canned answers.
- `engine`: commands against the sample mailbox and scripted network events; every command with junk params.
- `ui-egui`: headless egui frames in every layout and dialog, shortcuts as input events, and a 2,000-message folder;
  every catalog covers every interface string, and a tour in a pseudo-language finds text that skips the catalogs.
