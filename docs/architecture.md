# Architecture

This page explains how MailCraft's crates fit together and why they are split this way. It is for
contributors. The rules every change follows are in [`CLAUDE.md`](../CLAUDE.md); what each version
added is in the [changelog](../CHANGELOG.md).

## How a click becomes network work

On the desktop (`apps/mailcraft`, built on eframe), one user action travels through the layers
like this:

1. `ui-egui` (L6) turns the input into a command.
2. `engine::Session` (L4) runs the command and queues `Job`s.
3. `net::Pool` (L3) takes the jobs and talks to the server over IMAP, SMTP, JMAP, Microsoft Graph,
   EWS or CalDAV/CardDAV.
4. The pool writes the results into the local cache (`store`, L1) and sends `Event`s back to the
   session.
5. The UI draws from the session's state.

The app itself supplies the keychain, file dialogs, the system font and the clock.

In the browser (`apps/mailcraft-web`), the same `ui-egui` and engine run over JMAP. See
[The browser](#the-browser-appsmailcraft-web).

## Layers

| Layer | Crate | Depends on | Job |
|---|---|---|---|
| L0 | `mailcraft-model` | — | Pure data and algorithms. `Job` and `Event` live here, so `net` and `engine` agree without depending on each other |
| L1 | `mailcraft-mime` | model | Parse and build MIME (`mail-parser`, `mail-builder`); turn HTML into text (`html2text`, `html5ever`) |
| L1 | `mailcraft-store` | model | The local cache: one redb data file per account, or plain files plus JSON; atomic writes; the search index |
| L1 | `mailcraft-print` | — | Messages as PDF pages, laid out as they're read (tables, backgrounds, pictures, links), with an embedded TrueType font |
| L1 | `mailcraft-pst` | — | Outlook data files (MS-PST): read them (wasm-safe), add to them and write new ones |
| L1 | `mailcraft-smime` | — | S/MIME (RFC 8551) on RustCrypto, with keys behind a `PrivateKey` trait; wasm-safe |
| L1 | `mailcraft-pgp` | — | OpenPGP (RFC 9580) on rPGP: keys and their self-signatures, signing, encrypting, decrypting, verifying; Autocrypt headers; inline PGP blocks; wasm-safe |
| L2 | `mailcraft-keys` | smime | Keys on smart cards (PKCS #11), in the macOS Keychain and in the Windows certificate store; native only |
| L2 | `mailcraft-jmap` | model, mime, store | JMAP, async over a two-method `Http` trait; wasm-safe |
| L3 | `mailcraft-net` | model, mime, store, pst, jmap | All network and `.pst` I/O; native only; TLS through `rustls` |
| L4 | `mailcraft-engine` | model, mime, store | The sans-I/O `Session` and the command registry; wasm-safe |
| L6 | `mailcraft-ui-egui` | engine | The egui interface; platform services behind traits; interface languages |

- `cargo xtask layers` enforces this table: `egui` appears only in L6, and network crates only in
  `net`.
- `cargo xtask wasm` builds every crate except `net` and `keys`, plus the web app, for `wasm32`.
- `jmap`'s `Http` trait is served by `net`'s HTTP client on the desktop and by `fetch` in the
  browser.
- `net` covers IMAP, POP3, SMTP, JMAP push, Microsoft Graph, EWS with NTLM (Windows sign-in over
  HTTP), CalDAV/CardDAV, ManageSieve, HTTP, DNS and OAuth.
- `keys` is the one crate with `unsafe`. The crate denies it, and only `windows/cng.rs` and
  `macos/sec.rs` allow it.
- `.pst` files (Outlook data files) go through MailCraft's own crate, `pst`, written from
  Microsoft's published [MS-PST](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/)
  specification. It reads both formats (Unicode and the older ANSI), adds to files and makes new
  ones, and holds only what MailCraft uses.
- **Portable crates start no threads.** Slow window work, such as decoding pictures and video
  frames or parsing the open message, goes through `Services::background` to a *lane*: a named
  queue of background jobs. The desktop runs one thread per lane
  (`apps/mailcraft/src/desktop/lanes.rs`) and runs each job under `catch_unwind`. The browser and
  the tests run a lane one step per frame.

## Source layout

Code is split by topic. `foo.rs` sits beside a `foo/` folder and declares its pieces; there is no
`mod.rs`. A big type keeps one `impl`, continued across files. The size limits are in `CLAUDE.md`,
and `cargo xtask sizes` enforces them.

| Where | What |
|---|---|
| `engine/src/session.rs` + `session/` | `Session`'s state, with its `impl` continued per area (`opening`, `changing`, `writing`, `searching`, `accounts`, `organize/`…) |
| `engine/src/cmd.rs` + `cmd/` | The command registry; each area returns its own commands |
| `ui-egui/src/app.rs` + `app/` | `MailApp`, the window |
| `ui-egui/src/panes/`, `dialogs/`, `reading/`, `rich_edit/`, `look/` | Panes; dialogs; the reading pane and compose; the editor; theme and icons |
| `net/src/protocols/`, `exchange/`, `data_files/`, `fakes/`, `worker/` | Protocols; Graph, EWS and NTLM; `.pst` files; test servers; the worker pool |

## Transports

- Mail syncs over IMAP with SMTP, POP, JMAP, Microsoft Graph or EWS. All of them use the same
  `Job`s and `Event`s.
- JMAP, Graph and EWS name messages with strings. `jmap::State` maps those to IMAP-style numbers,
  so the engine and the local cache only ever see IMAP-style keys.
- Calendars and contacts sync over CalDAV and CardDAV. Automatic replies use ManageSieve.
- Some accounts have no server, such as the sample mailbox or an opened `.pst` file. These are
  *local* accounts: the pool applies their changes straight to the local cache.

## The session

`Session` (`crates/engine`) holds everything the window shows:

- the accounts;
- the view: one folder, or one role across accounts, such as All Inboxes;
- the filter and the search;
- the message list and the selection;
- the open message (`Reading`) and the draft (`Compose`);
- preferences and notices.

How it works:

- **Everything is a command.** `run(id, params)` looks the command up in the registry (`cmd.rs`),
  checks that it is `enabled`, and turns an escaped panic into an error. Commands never open
  dialogs; the UI asks first, then runs the command.
- **The session does no I/O.** `take_jobs()` hands work to the network layer, and `apply(event)`
  folds the results in. The session has no clock, no network and no threads.
- **Changes show at once.** `apply_op` updates the list in memory. The worker then applies the
  change to the local cache and the server and reports `FolderChanged`. That reloads the list from
  the local cache, which is the single source of truth. A change's *echo* (the `FolderChanged`
  that reports the session's own change) doesn't reload (`session/mailboxes/echo.rs`).
- **The message list** is indexed as it is built (`ListNav`). Flag and category changes are
  patched into it in place (`patch_list`). The proptest `patched_list_equals_refresh` checks that
  a patched list always equals a full rebuild. Summaries are shared as `SharedSummary`
  (`Arc<Summary>`).
- **The open message** can be parsed on the reading lane (`session/opening/prepare.rs`). A command
  that acts on it finishes parsing the body first. The CLI and the tests always parse straight
  away.
- **Tasks** live in each account's `pim.json`, on this computer only.
- **Sending:** `Session::send_block` says why an account can't send. `Session::deliver`, which
  every way of sending goes through, refuses to send from such an account.

## Storage

`crates/store` keeps the local cache.

- **Data files.** Each account's local cache is one crash-safe redb file, its *data file*, named
  `<address>.mailcraft`, in **Documents\MailCraft Files** (`packed.rs`). The account list and app
  state stay in the app folder (`%APPDATA%\MailCraft` on Windows). The sample mailbox uses plain
  files, and the browser keeps everything in memory.
- **An opened `.pst` file is never written.** It gets its own `.mailcraft` data file beside it,
  listed in `datafiles.json` (`beside.rs`). MailCraft writes `.pst` files only when it exports one.
- **Old folder layouts** are brought into data files when the store opens. This needs the store to
  itself, so it holds `<root>/.locks/folders.lock`.
- **Moving the data folder** (`location.rs`): a requested move runs at the next start, before
  anything opens the files.
- **Commits** (`packed/commit.rs`):
  - Writes from the window reach the disk at once.
  - Background writers, such as imports, compress off the file and commit in batches without
    waiting for the disk.
  - A data file is *sealed* (marked consistent on disk) after two quiet seconds, at the end of an
    import and on exit. After a crash, a sealed file reopens at once; an unsealed one gets one full
    check first.
- **Opening** one data file locks no others. The desktop opens them on a `data-open` thread while
  the window shows `BootCard`.
- **Health and compaction** (`packed/health.rs`, `compact.rs`, `shrink.rs`):
  - Health is measured without reading any mail.
  - Compaction runs in place when it fits the time allowed. Otherwise it copies the file, if the
    drive has room (`mailcraft_model::job::plan`).
  - The pool compacts on a `mail-compact` thread, never while an import runs.
  - The engine measures health after ten idle minutes. While a data file is being compacted,
    commands on its account are refused.
  - `dataFile.compact` compacts by hand. Compacting on close is off by default
    (`prefs.compactOnClose`).
- **Work kept off the window thread:**
  - On local accounts, read marks, flags and categories go to the `mail-marks` thread
    (`Store::mark_later`). Loading a folder waits for its pending marks (`Session::load_cached`
    defers it).
  - `ui.json` and `mailcraft.log` are written on threads of their own.

## Importing .pst files

- The session tracks imports in `Imports` and `imports.json` (`session/accounts/importing.rs`,
  `cmd/data_files.rs`). `account.inspectDataFile` looks inside a file before `account.importPst`
  imports it.
- **Resuming.** Each import keeps a note in the account's data file (`import.json`). The note is
  committed with each batch, at least every 10 seconds. An interrupted import resumes at the next
  start. If the source file changed, went missing or is locked, the import waits for the user.
- **While an import runs,** the account's folders can't be reorganised, and new Inbox mail isn't
  treated as new. The import pauses when free disk space drops below 1 GiB plus the data file's
  next growth (`Blocked::DiskFull`).
- **The importer** (`pst/` in `net`):
  1. A planner hands out numbered jobs.
  2. Lanes build items, within a byte budget.
  3. One writer applies the items in order.

  A panic in a lane costs only the item it was building. The `.pst` file is opened read-only.
  Sent Items is read last, so replies can find the messages they answer by Message-ID. Mail is
  indexed for search while it imports.
- **Stopping.** `pst::Stop` steps from pause to cancel to discard. Closing the app pauses an
  import.

## The browser (`apps/mailcraft-web`)

- The local cache is `Store::memory()`. It is loaded from IndexedDB at start, or from local
  storage if IndexedDB is missing. Each frame writes the changes (`Store::take_changes`) back.
- Settings go in local storage. Passwords never do.
- `WebBackend` runs each account's jobs in order over `fetch`. It keeps the JMAP push stream open
  and polls when push isn't available.
- Browser accounts are JMAP only (`Session::jmap_only`).

## Search

### Queries

- `model::search` parses the query language (`search/parse.rs`). Dates are resolved with a clock
  the caller passes in, because portable code can't read one.
- Words are folded on both sides before they are compared (`search/fold.rs`): lower case, accents
  removed, ß becomes ss. `cargo xtask fold` generates the folding table.
- `Query::eval` answers yes, no or unknown for a message.

### Running a search

- The session searches in small units (`session/searching/run.rs`: `start_search`,
  `search_step`).
- A message's summary answers questions about flags, dates, senders and subjects. The index
  answers questions about text.
- In the window, a search runs for 6 ms each frame. Results join the list by date as they are
  found.
- A search that waits for its answer (the CLI, the tests) reads mail that isn't indexed yet from
  its source. In the window, such mail may be missing from the results, and the list says so.

### The index

The index lives with the account's mail, under `<account>/search/v1/`
(`store/src/search.rs` + `search/`).

- It is a series of *generations*. Each holds immutable word segments plus each message's kept
  text.
- `meta.bin` names the current generation. It is written last, in one sealed transaction, so a
  crash leaves the previous generation in place.
- Each account has one index writer, even across processes. The writer holds a lease in
  `<root>/.locks/`.
- Every index file is decoded with checks, and the `search_index` fuzz target tests that decoding.
  A damaged index is set aside and rebuilt.
- **Keeping the index current:** `search::plan` compares each folder with the index and adds,
  removes or moves entries. Nothing hooks into each change.
- Building the index is `Job::Index`. It starts after 1.5 seconds of quiet. On the desktop it runs
  on a `mail-index` thread, never during an import; in the browser it runs in small steps.

## Conversations

`model::thread` groups messages into conversations with JWZ threading, the same method as IMAP
`THREAD=REFERENCES` (RFC 5256):

- A message's parent comes from its `References` and `In-Reply-To` headers, with loops removed.
- A message whose parent is missing is placed by its conversation index (`Thread-Index`).
- Replies that lost their references are grouped by base subject (the subject without "Re:" and
  similar prefixes).

Every message is in exactly one conversation. Everything sorts by `Summary::when`.

## Interface languages

- Interface text is written in English in the code and translated through `tl!` and `tf!` from
  `crates/ui-egui/src/i18n/*.tsv`. Command ids, the CLI and mail stay in English.
- The interface follows the system language unless the user picks one in Settings.
- Release builds can embed fallback fonts from
  [storytold/craft-fonts](https://github.com/storytold/craft-fonts) through `CRAFT_FONTS_DIR`. The
  fonts are never a Cargo dependency.
- egui and winit are the Craft forks (storytold/egui, storytold/winit), pinned in the root
  `Cargo.toml` under `[patch.crates-io]`. That egui lays out right-to-left text itself.

## The Craft apps' shared libraries and services

MailCraft uses what the other Craft apps share where it fits, so the suite's fixes reach it.

| Shared piece | Used | Why, or why not |
|---|---|---|
| `craft-atomic` (craft-libs) | yes: `crates/store/src/atomic.rs` | The cache's file writes: a new scratch file never written through one at its name, a plain flush where a Mac's network share refuses a full one, renames tried again while Windows holds a file |
| `craft-docpath` (craft-libs) | yes: `.pst` linked files | Whether a path a data file names is a local file: device names, Windows' file streams and `\\?\` forms refused |
| craft-fonts, craft-common-actions | yes | The fonts in releases; CI, packaging lint and the FreeBSD and Windows ARM64 builds |
| Dependabot, `SECURITY.md` | yes | Weekly grouped dependency updates; private vulnerability reports |
| `craft-update` (craft-libs) | no | Built for signed suite releases: it refuses unsigned builds on Windows and macOS (an Apple team of the suite's), skips pre-releases, and wants an asset's version to equal its tag's. MailCraft's own updater (`docs/updates.md`) takes its ways of telling installs apart instead |
| `craft-quit` (craft-libs) | not yet | `apple_events::quit` and `MailApp::quit_instead_of_close` are current copies of it; the swap is macOS code, to be built and tried on a Mac |
| `craft-dialogs` (craft-libs) | not yet | File dialogs that don't block the window (a macOS crash with a blocking one); every file picker would become ask-then-answer, and no Craft app uses it yet |
| `craft-i18n` | no | Its catalogs are storytold's repository, for its apps; MailCraft's would move there by a PR to it. Its rule that a missing translation never fails is the opposite of MailCraft's |
| Flatpak OCI publishing (`flatpak-oci.sh`) | no | Publishes to storytold's registry from the shared release, which MailCraft's releases don't use; a Flatpak updates through the software installer |
| SonarCloud | no | Needs a SonarCloud project and token for ZIDAN44, and its coverage builds run on every push |
| Contributor credits (`contributors/`) | no | The generator is shared, but the About page credits are each app's own code, for one maintainer |
| Other craft-libs crates | no | Nothing to replace (audio, PDF, raster, fonts for documents), or a larger rewrite than they'd save (`craft-ui`, `craft-menu`, `craft-print`, `craft-proof`) |

## Crash safety

MailCraft must never crash; the rules are in `CLAUDE.md`. `Session::run`, lanes and the
importer's lanes catch escaped panics. A damaged cache file or index is set aside, never fatal.

## Tests

- `model`, `mime`, `store`: unit tests, including hostile input.
- `net`: in-memory IMAP, JMAP, Graph, EWS, CalDAV/CardDAV and ManageSieve servers (`fakes/`)
  drive accounts through the real pool. Local stubs stand in for SMTP, HTTP, DNS and OAuth.
  `serve_for_the_browser` (ignored by default) runs a JMAP server with CORS for the web app.
- `.pst` export and import round trips, including calendars, contacts, notes and tasks.
- Fuzz targets for every parser of network and disk input; see [Fuzzing](../fuzz/README.md).
- `engine`: commands against the sample mailbox and scripted events, plus every command with junk
  params.
- `ui-egui`: headless frames of every layout and dialog, and a check that every language catalog
  covers every interface string.
