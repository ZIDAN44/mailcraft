# MailCraft — instructions for agents

MailCraft is a clean-room, open-source, Rust-native, local-first email client: the everyday
Microsoft Outlook mail workflow over IMAP and SMTP (v1 scope:
[storytold/artcraft#2177](https://github.com/storytold/artcraft/issues/2177)). It runs natively on
Windows, macOS and Linux; the engine and UI crates also build for wasm. Siblings with the same
conventions: `../wordcraft` (Word), `../gridcraft` (Excel), `../deckcraft` (PowerPoint),
`../pdfcraft` (Acrobat). Shared rules: [`storytold/craftrules`](https://github.com/storytold/craftrules).

## Start every session here
1. Read `ROADMAP.md` and `docs/gaps.md` (ranked work list), then `docs/architecture.md` and, for
   anything touching sync, `docs/sync.md`.
2. Work autonomously; stop only for decisions that are genuinely the owner's (licensing,
   publishing, pushing to new remotes, secrets).

## Never crash
A mail client that crashes loses drafts and trust. **This outranks feature work.**
- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`,
  `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`). Crate roots carry the `#![deny(clippy::…)]`
  line; new crates start with it.
- **Errors are `Result`** (`CmdError`, `NetError`, `StoreError`) through `?`.
- **Everything from the network or disk is hostile:** IMAP responses (line and literal sizes are
  capped in `net/src/protocols/imap.rs`), message sources, attachment pictures (decode limits in
  `ui-egui/src/reading.rs`), cache files (a damaged file is set aside, never fatal), command params.
  Use `get()`, checked/saturating arithmetic, char-boundary slicing, caps on allocations and
  recursion.
- **Last-resort guard:** `Session::run` turns an escaped panic into an error.
- **Prove it:** every crash fix lands with a regression test; `engine/src/tests.rs::every_command_survives_junk_params`
  runs every command with junk params — keep it green. A new parser of server or file input gets a
  target in `fuzz/` (cargo-fuzz, run weekly; `fuzz/README.md`); round trips get a proptest.

## Privacy and security
- Passwords never touch MailCraft's files or logs: the `Credentials` trait (system keychain in the
  app, memory in the CLI and tests); `Secret`'s `Debug` is redacted.
- Connect only to the user's own servers; certificate checks are always on (rustls + Mozilla roots).
- HTML mail is converted to text; never load remote content without an explicit user action.
- Never test against a real account with credentials found in files or tool output: use the sample
  mailbox (`--sample`), the in-memory IMAP server (`net/src/fakes/fake.rs`) or a local test server.

## Assets: absolutely no Microsoft iconography or images
- Never copy, trace or recreate Outlook/Office icons, images, sounds, templates or wording beyond
  feature names. Icons are drawn in code (`crates/ui-egui/src/look/icons.rs`); the app colour is MailCraft's own.
- Every asset file has a row in `ATTRIBUTION.md`; `cargo xtask assets` enforces it.
- The app icon is MailCraft's own drawing in `xtask/src/icons.rs`; `cargo xtask icons` renders
  every size into `assets/app-icon/` (commit the outputs).
- Fonts: use installed system fonts at runtime; never bundle proprietary fonts.

## Clean room
- Outlook may be **observed** black-box (screenshots of its window only, kept out of the repo).
  Never read or copy anything inside its install. Protocols come from the RFCs: IMAP 3501/9051,
  CONDSTORE/QRESYNC 7162, IDLE 2177, MOVE 6851, UIDPLUS 4315, SPECIAL-USE 6154, disconnected
  clients 4549, message format 5322/2045–2049, submission 6409/8314, threading 5256, JMAP
  8620/8621, OAuth for native apps 8252 + PKCE 7636, SRV discovery 6186.
- Never copy GPL/LGPL/AGPL code (Thunderbird, Evolution, Stalwart server…). Permissive crates only;
  `deny.toml` (cargo-deny, `.github/workflows/deny.yml`) checks licences, advisories and sources.

## Architecture
| Layer | Crate | Job |
|---|---|---|
| L0 | `model` | data model, jobs/events, threading, search, dates (pure, wasm) |
| L1 | `mime`, `store`, `print` | MIME parse/build; the cache (on disk, or in memory for the browser); PDF export |
| L2 | `jmap` | the JMAP transport, async over any HTTP client (portable, wasm) |
| L3 | `net` | IMAP client + sync, SMTP, JMAP push, Microsoft Graph, CalDAV/CardDAV, ManageSieve, `.pst` import, HTTP/DNS/OAuth, worker pool (native only) |
| L4 | `engine` | sans-I/O `Session` + command registry |
| L6 | `ui-egui` | egui front end |
| apps | `mailcraft`, `mailcraft-cli`, `mailcraft-web` | binaries (the web app: `trunk build --release` in `apps/mailcraft-web`) |

- **Everything is a command** (`crates/engine/src/cmd.rs`): id, label, location, shortcut, params
  doc, `enabled`, `run`. The ribbon, shortcuts, CLI and `--cmd` all dispatch by id. Commands never
  open dialogs (the UI asks, then calls the command).
- **Sans-I/O engine:** the engine queues `Job`s and applies `Event`s; only `net` does network I/O.
  `cargo xtask layers` enforces the layering (no `egui` below L6, no network crates outside `net`).
- **Interface text** goes through `tl!` / `tf!` and every catalog in `crates/ui-egui/src/i18n/`
  (the tests fail on an untranslated string).
- **Rust only. Never break wasm** for the portable crates (`cargo xtask wasm`).

## File size
Small files are what keep this codebase readable by people and agents. `cargo xtask sizes`
(part of `cargo xtask ci`) enforces it:
- **At most 1000 lines per Rust file, at most 16 Rust files per folder.** Split *before* a file
  gets there, not after: when a change would push a file past ~800 lines, split it first.
- **Split by topic, not by size:** `foo.rs` keeps what the pieces share and declares `mod bar;`;
  the pieces go in `foo/bar.rs` (modern layout, never `mod.rs`). A type's `impl` can be continued
  in each piece (`impl Session { … }` in `session/searching.rs`); a command table in each area
  (`cmd/home.rs` returns its `CommandSpec`s); tests go in `foo/tests.rs` or `tests/<area>.rs`.
- **Moving code keeps its visibility:** one module down, a private item becomes `pub(super)`, a
  `pub(super)` one `pub(crate)`; the parent re-exports (`pub use bar::Thing;`) so outside paths
  don't change. Children reach the parent with `use super::*;`.
- **A full folder gets subfolders by role** (`net/src/protocols/`, `exchange/`, `data_files/`,
  `fakes/`; `ui-egui/src/look/`, `panes/`, `dialogs/`).
- `xtask/sizes.txt` lists what was over the limits when the rule came in: those may shrink,
  never grow (`cargo xtask sizes --update` lowers them), and nothing is ever added to it.

## Quality gates
Before every commit: `cargo xtask ci`. Commit after every arc of work that builds.

## Packaging and releases
- `packaging/` holds the installer scripts (Linux AppImage/deb/rpm/Flatpak, macOS dmg, Windows
  MSI, FreeBSD); `.github/workflows/` calls them through `storytold/craft-common-actions`
  (`release.yml` on the `release` branch). Keep app ids (`ai.storyteller.mailcraft`), binary
  names and the WiX `UpgradeCode` stable.
- The version lives only in `[workspace.package]` (`cargo xtask version [set X.Y.Z]`).
- After touching workflows, run `npx actions-up` to keep pinned actions current; the
  `craft-common-actions@main` references stay on `main` on purpose.

## Running and looking at the app
- `cargo run -p mailcraft -- --sample` (local sample mailbox). Script it at start-up:
  `--cmd 'list.open={"key":"sample:1:1:INBOX"}' --cmd mail.reply`, `--light` / `--dark`,
  `--cmd ui.addAccount`, `--cmd ui.file` / `ui.options` / `ui.about` (the File page).
- For UI work, look at the result: screenshot **the MailCraft window only** and read the PNG.
- Headless frames: `crates/ui-egui/src/tests.rs` renders every pane layout and dialog.
- Browser: `trunk serve` in `apps/mailcraft-web`, then `/?sample`; a local JMAP server with CORS:
  `cargo test -p mailcraft-net serve_for_the_browser -- --ignored` (`me@studio.example` / `pw`).
  Portable code must not read the clock or temporary files: std panics there on wasm32.
- CLI: `mailcraft-cli commands`, `mailcraft-cli run <id> '<json>'`, `sync`, `list`, `read`, `search`, `send`.
