# Architecture

```
            ┌──────────────── apps/mailcraft (eframe) ────────────────┐
            │  keychain · file dialogs · system font · clock           │
            │                                                          │
   input ──▶│  ui-egui (L6) ──commands──▶ engine::Session (L4)         │
            │      ▲  reads state              │  queues Jobs           │
            │      └──────────────── Events ◀──┼── net::Pool (L3) ◀── IMAP·SMTP·JMAP
            │                                  ▼        │               │
            │                          store (L1) ◀─────┘ writes cache  │
            └──────────────────────────────────────────────────────────┘
```

## Layers

| Layer | Crate | Depends on | Notes |
|---|---|---|---|
| L0 | `mailcraft-model` | — | Pure data + algorithms; `Job`/`Event` are defined here so `net` and `engine` agree without depending on each other |
| L1 | `mailcraft-mime` | model | `mail-parser`, `mail-builder`, `html2text` |
| L1 | `mailcraft-store` | model | Plain files + JSON; atomic writes; no database engine |
| L1 | `mailcraft-print` | — | Messages laid out as PDF pages with an embedded TrueType font (`ttf-parser`, `miniz_oxide`) |
| L3 | `mailcraft-net` | model, mime, store | Native only. IMAP (`imap-proto`), SMTP (`lettre`), JMAP and a small HTTP/1.1 client, DNS for autoconfiguration, OAuth; TLS through `rustls` |
| L4 | `mailcraft-engine` | model, mime, store | Sans-I/O, wasm-safe |
| L6 | `mailcraft-ui-egui` | engine | egui only; platform services behind traits; interface languages (`i18n/*.tsv`) |

`cargo xtask layers` enforces this, including that `egui` stays in L6 and network crates stay in `net`.

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
  pool over real TCP sockets, and an in-memory JMAP server (`fake_jmap.rs`) driving a JMAP account
  through the same pool. HTTP, DNS, autoconfig and OAuth run against local stubs.
- `engine`: commands against the sample mailbox and scripted network events; every command with junk params.
- `ui-egui`: headless egui frames in every layout and dialog, shortcuts as input events, and a 2,000-message folder;
  every catalog covers every interface string, and a tour in a pseudo-language finds text that skips the catalogs.
