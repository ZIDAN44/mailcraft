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
  capped in `net/src/imap.rs`), message sources, attachment pictures (decode limits in
  `ui-egui/src/reading.rs`), cache files (a damaged file is set aside, never fatal), command params.
  Use `get()`, checked/saturating arithmetic, char-boundary slicing, caps on allocations and
  recursion.
- **Last-resort guard:** `Session::run` turns an escaped panic into an error.
- **Prove it:** every crash fix lands with a regression test; `engine/src/tests.rs::every_command_survives_junk_params`
  runs every command with junk params — keep it green.

## Privacy and security
- Passwords never touch MailCraft's files or logs: the `Credentials` trait (system keychain in the
  app, memory in the CLI and tests); `Secret`'s `Debug` is redacted.
- Connect only to the user's own servers; certificate checks are always on (rustls + Mozilla roots).
- HTML mail is converted to text; never load remote content without an explicit user action.
- Never test against a real account with credentials found in files or tool output: use the sample
  mailbox (`--sample`), the in-memory IMAP server (`net/src/fake.rs`) or a local test server.

## Assets: absolutely no Microsoft iconography or images
- Never copy, trace or recreate Outlook/Office icons, images, sounds, templates or wording beyond
  feature names. Icons are drawn in code (`crates/ui-egui/src/icons.rs`); the app colour is MailCraft's own.
- Every asset file has a row in `ATTRIBUTION.md`; `cargo xtask assets` enforces it.
- The app icon is MailCraft's own drawing in `xtask/src/icons.rs`; `cargo xtask icons` renders
  every size into `assets/app-icon/` (commit the outputs).
- Fonts: use installed system fonts at runtime; never bundle proprietary fonts.

## Clean room
- Outlook may be **observed** black-box (screenshots of its window only, kept out of the repo).
  Never read or copy anything inside its install. Protocols come from the RFCs: IMAP 3501/9051,
  CONDSTORE/QRESYNC 7162, IDLE 2177, MOVE 6851, UIDPLUS 4315, SPECIAL-USE 6154, disconnected
  clients 4549, message format 5322/2045–2049, submission 6409/8314, threading 5256.
- Never copy GPL/LGPL/AGPL code (Thunderbird, Evolution, Stalwart server…). Permissive crates only.

## Architecture
| Layer | Crate | Job |
|---|---|---|
| L0 | `model` | data model, jobs/events, threading, search, dates (pure, wasm) |
| L1 | `mime`, `store` | MIME parse/build; the on-disk cache |
| L3 | `net` | IMAP client + sync, SMTP, worker pool (native only) |
| L4 | `engine` | sans-I/O `Session` + command registry |
| L6 | `ui-egui` | egui front end |
| apps | `mailcraft`, `mailcraft-cli` | binaries |

- **Everything is a command** (`crates/engine/src/cmd.rs`): id, label, location, shortcut, params
  doc, `enabled`, `run`. The ribbon, shortcuts, CLI and `--cmd` all dispatch by id. Commands never
  open dialogs (the UI asks, then calls the command).
- **Sans-I/O engine:** the engine queues `Job`s and applies `Event`s; only `net` does network I/O.
  `cargo xtask layers` enforces the layering (no `egui` below L6, no network crates outside `net`).
- **Rust only. Never break wasm** for the portable crates (`cargo xtask wasm`).

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
  `--cmd ui.addAccount`.
- For UI work, look at the result: screenshot **the MailCraft window only** and read the PNG.
- Headless frames: `crates/ui-egui/src/tests.rs` renders every pane layout and dialog.
- CLI: `mailcraft-cli commands`, `mailcraft-cli run <id> '<json>'`, `sync`, `list`, `read`, `search`, `send`.
