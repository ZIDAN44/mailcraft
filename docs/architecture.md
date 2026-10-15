# Architecture

```
            ┌──────────────── apps/mailcraft (eframe) ────────────────┐
            │  keychain · file dialogs · system font · clock           │
            │                                                          │
   input ──▶│  ui-egui (L6) ──commands──▶ engine::Session (L4)         │
            │      ▲  reads state              │  queues Jobs           │
            │      └──────────────── Events ◀──┼── net::Pool (L3) ◀── IMAP·SMTP·JMAP·Graph·DAV
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
| L1 | `mailcraft-mime` | model | `mail-parser`, `mail-builder`, `html2text` |
| L1 | `mailcraft-store` | model | Plain files + JSON; atomic writes; no database engine |
| L1 | `mailcraft-print` | — | Messages laid out as PDF pages with an embedded TrueType font (`ttf-parser`, `miniz_oxide`) |
| L2 | `mailcraft-jmap` | model, mime, store | The JMAP transport, written once as async code over a two-method `Http` trait: the desktop runs it over `net`'s client (answers are ready at once, so a no-op `block_on` drives it), the browser over `fetch`. Portable, wasm-safe |
| L3 | `mailcraft-net` | model, mime, store, jmap | Native only. IMAP (`imap-proto`), SMTP (`lettre`), JMAP push, Microsoft Graph, CalDAV/CardDAV (`quick-xml`), ManageSieve, a small HTTP/1.1 client, DNS for autoconfiguration, OAuth, Outlook data files (`outlook-pst` to read, `outlook-pst-rw` to write); TLS through `rustls` |
| L4 | `mailcraft-engine` | model, mime, store | Sans-I/O, wasm-safe |
| L6 | `mailcraft-ui-egui` | engine | egui only; platform services behind traits; interface languages (`i18n/*.tsv`) |

`cargo xtask layers` enforces this, including that `egui` stays in L6 and network crates stay in `net`.
`cargo xtask wasm` checks that every crate but `net`, and the web app, build for `wasm32`.

## Transports

An account syncs over one of three transports behind the same `Job`s and `Event`s, chosen by its
configuration: IMAP + SMTP (`imap`, `smtp`), JMAP (`jmap`, a session address) or Microsoft Graph
(`graph`, Exchange Online). JMAP and Graph number their string message ids the same way
(`jmap::State`), so the engine and cache only ever see IMAP-style keys. Calendars and address
books sync over CalDAV / CardDAV for password accounts, and automatic replies over ManageSieve on
IMAP hosts. An account with none of these (the sample mailbox, an opened Outlook data file) lives
on this computer only: the pool applies its changes to the cache directly.

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

## Search

`model::search` parses the query language into clauses. The session builds an inverted index
(`BTreeMap<word, docs>`, so a word prefix is one range scan) over summaries, cached bodies and
attachment names for the chosen scope; candidates are verified against the full query, so the index
may over-approximate but never misses. The index is rebuilt lazily after changes.

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
  (`fake_graph.rs`), CalDAV/CardDAV (`fake_dav.rs`) and ManageSieve servers driving accounts
  through the same pool. Outlook data files are exported with `outlook-pst-rw` and read back
  by the importer; contact and appointment mapping and recurrence patterns are tested on their own. HTTP, DNS, autoconfig and OAuth run against local stubs. `serve_for_the_browser` (ignored)
  keeps a JMAP server with CORS running for trying the web app by hand.
- `jmap`: the session, URLs and paths against canned answers.
- `engine`: commands against the sample mailbox and scripted network events; every command with junk params.
- `ui-egui`: headless egui frames in every layout and dialog, shortcuts as input events, and a 2,000-message folder;
  every catalog covers every interface string, and a tour in a pseudo-language finds text that skips the catalogs.
