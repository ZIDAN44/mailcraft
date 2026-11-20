# MailCraft — instructions for agents

MailCraft is a clean-room, open-source, Rust-native, local-first email client: the everyday
Microsoft Outlook mail workflow over IMAP and SMTP, by Zinadin Zidan and the MailCraft
contributors. It runs natively on Windows, macOS and Linux; the engine and UI crates also build
for wasm. Siblings with the same conventions: `../wordcraft` (Word), `../gridcraft` (Excel),
`../deckcraft` (PowerPoint), `../pdfcraft` (Acrobat), `../photocraft`. CI and packaging checks
are shared through [`storytold/craft-common-actions`](https://github.com/storytold/craft-common-actions).

## Start every session here
1. Read `ROADMAP.md` and `docs/gaps.md` (the open gaps, most-hit first), then `docs/architecture.md`
   and, for anything touching sync, `docs/sync.md`. `CHANGELOG.md` has what each version added.
2. Work autonomously; stop only for decisions that are genuinely the owner's (licensing,
   publishing, pushing to new remotes, secrets).

## How changes are made
The rules the Craft apps share (the storytold house rules, `standards/house-rules-core.md` and
`standards/house-rules-rust.md` in storytold/craft-contrib), as they fit MailCraft: `main` takes
the owner's commits directly, and dates keep MailCraft's own `model::dates`.
- **Check, then claim.** Before writing code, read the issue comments, search pull requests for
  `#N` and the keywords, and check recent commits on `main`. Comment on the issue first. Release a
  claim you will not finish.
- **Verify, don't guess.** Open a definition before using it; confirm a cause in the code, even
  when a report names the place.
- **Narrow and tested.** Fix what the report describes, where the bug is, with a test that fails
  without the fix. Nothing unrelated rides along.
- **Read the diff before committing:** every name used exists, the test fails without the fix,
  the message says what's true, and the line endings are the file's own.
- **Report only what ran:** say what was tested, and what couldn't be (no Mac, no Flatpak, no
  server of that kind) rather than implying it was.
- **Guards say why.** A check that looks removable says what it prevents; read that before
  changing it.
- **Readable by everyone:** these instructions and the docs hold nothing about one machine and
  link nothing a reader can't open (a machine's own notes go in an untracked `CLAUDE.local.md`).
- **Nothing personal** in the repository, the log or a request: no one's addresses, mail,
  passwords or names beyond the credits.
- **No agent attribution.** A pull request does not say an agent wrote it, and a commit carries no
  agent's `Co-Authored-By` line. A repository rule may not require that. (People whose work a
  change takes are still credited with theirs.)

### Licence allowlist
Dependencies and assets use SPDX identifiers. An expression is allowed when every identifier is in
this list: `ISC`, `MIT`, `BSD-2-Clause`, `BSD-3-Clause`, `Apache-2.0`, `OFL-1.1`,
`Ubuntu-font-1.0`, `Bitstream-Vera`, `CC0-1.0`, `CC-BY-4.0`, `CC-BY-SA-4.0`,
`LicenseRef-Public-Domain`. A crate offering a choice is taken under one of these (`self_cell`'s
Apache-2.0, never its GPL). What `deny.toml` checks is that list and the permissive ones the
common Rust crates under every Craft app bring, as the Craft apps' own `deny.toml` allow them:
`Unicode-3.0` (`unicode-ident`, ICU), `Zlib`, `BSL-1.0`, `0BSD`, `MIT-0`, `Unlicense`,
`Apache-2.0 WITH LLVM-exception`, `CDLA-Permissive-2.0` (Mozilla's root certificates), and
`MPL-2.0` for `option-ext` alone. A new dependency takes from the allowlist above wherever it
can; one that can't is the owner's call.

## Never crash
A mail client that crashes loses drafts and trust. **This outranks feature work.**
- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`,
  `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`). Crate roots carry the `#![deny(clippy::…)]`
  line; new crates start with it.
- **The one `unsafe` exception** (signed off by the owner, as PhotoCraft's tablet crate):
  `crates/keys`, where the Windows CNG and macOS Security calls that have no safe wrapper live.
  The crate sets `unsafe_code = "deny"`; only `src/windows/cng.rs` and `src/macos/sec.rs` allow it,
  every block has a `SAFETY:` comment, and both are tested against the real system (the Windows
  one in CI on Windows). Use a maintained safe wrapper first (`rustls-cng`, `security-framework`);
  don't add `unsafe` anywhere else.
- **Errors are `Result`** (`CmdError`, `NetError`, `StoreError`) through `?`.
- **Everything from the network or disk is hostile:** IMAP responses (line and literal sizes are
  capped in `net/src/protocols/imap.rs`), message sources, attachment pictures (decode limits in
  `ui-egui/src/reading.rs`), cache files (a damaged file is set aside, never fatal), command params.
  Use `get()`, checked/saturating arithmetic, char-boundary slicing, caps on allocations and
  recursion.
- **Last-resort guard:** `Session::run` turns an escaped panic into an error, and every panic, on
  any thread, is logged by where it happened (`apps/mailcraft/src/startup/logfile.rs`, never its message).
- **Prove it:** every crash fix lands with a regression test; `engine/src/tests/organize.rs::every_command_survives_junk_params`
  runs every command with junk params — keep it green. A new parser of server or file input gets a
  target in `fuzz/` (cargo-fuzz, run weekly; `fuzz/README.md`); round trips get a proptest.

## Privacy and security
- Passwords never touch MailCraft's files or logs: the `Credentials` trait (system keychain in the
  app, memory in the CLI and tests); `Secret`'s `Debug` is redacted.
- Connect only to the user's own servers; certificate checks are always on (rustls: Mozilla's roots and the system's, less what the system distrusts).
  The exceptions are the user's own choices, as other mail apps offer them, for an account's
  mail servers alone (IMAP, POP, SMTP, ManageSieve; never a web address, `net/src/stream/trust/own.rs`):
  a certificate trusted by fingerprint when asked, and certificate files imported (an authority
  or a server's own). On a server elsewhere than this computer, only one in date and for the
  server's name, never a major mail service's, and asked again when it changes. Never widen them.
- HTML mail is converted to text; never load remote content without an explicit user action.
- Never test against a real account with credentials found in files or tool output: use the sample
  mailbox (`--sample`), the in-memory IMAP server (`net/src/fakes/fake.rs`) or a local test server.
- Never write into an Outlook `.pst`/`.ost` the user opens: MailCraft's changes go to a `.mailcraft`
  copy beside it. Export writes only into the file the user picks to export to.

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
- A fix to a third-party crate goes into a fork on ZIDAN44 (never a `vendor/` copy): one commit per
  fix, listed in the fork's `PATCHES.md`, pinned by `rev` in `[patch.crates-io]` of both
  `Cargo.toml` and `fuzz/Cargo.toml` (`cargo xtask ci` checks they match).
- `fuzz/Cargo.lock` keeps the versions `Cargo.lock` has, so the weekly run fuzzes what ships:
  after a dependency moves, `cargo xtask locks --update` (`cargo xtask ci` checks it).
- **The Craft apps' shared crates** (storytold/craft-libs) are used where they fit, so their fixes
  arrive with a pin bump: `craft-atomic` (the cache's file writes) and `craft-docpath` (paths data
  files name), all at one `rev` in `[workspace.dependencies]`. A gap one has is fixed there (a PR
  to craft-libs), not copied here. Which crates were weighed and why the rest aren't used:
  `docs/architecture.md`.

## Architecture
| Layer | Crate | Job |
|---|---|---|
| L0 | `model` | data model, jobs/events, threading, search, dates (pure, wasm) |
| L1 | `mime`, `store`, `pst`, `print`, `smime`, `pgp` | MIME parse/build; the cache (on disk, or in memory for the browser); Outlook data files; PDF export; S/MIME; OpenPGP |
| L2 | `jmap`, `keys` | the JMAP transport, async over any HTTP client (portable, wasm); keys on smart cards, in the macOS Keychain and the Windows certificate store (native only) |
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

## File size and organisation
Small files that each hold one topic are what keep this codebase readable by people and agents.
`cargo xtask sizes` and `cargo xtask modules --cycles` (both part of `cargo xtask ci`) enforce it:
- **At most 1000 lines per Rust file, at most 16 Rust files per folder.** These are tripwires,
  not the reason to split: split *before* a file gets there, when a change would push it past
  ~800 lines (`cargo xtask sizes --near` lists those past 800 lines or at 14 files).
- **A module hides one decision** that could change (Parnas), behind a small interface. What
  changes for the same reason lives together; what's used together lives together. A folder
  is a feature or a domain (`calendar/`, `drafts/`), never a kind of code (`types/`, `utils/`);
  what two features share goes in a module of its own.
- **Find the split, don't guess it:** `cargo xtask modules <folder>` groups a folder's files by
  their references to each other and the commits that changed them together, and scores the
  layout now against its groups. A person names the groups and checks them.
- **No cycles between a crate's top-level modules**: move what both need into a module of its
  own. `xtask/cycles.txt` lists those there when the rule came in: they may shrink, never grow.
- **Split by topic, not by size:** `foo.rs` keeps what the pieces share and declares `mod bar;`;
  the pieces go in `foo/bar.rs` (modern layout, never `mod.rs`). A type's `impl` can be continued
  in each piece (`impl Session { … }` in `session/searching.rs`); a command table in each area
  (`cmd/home.rs` returns its `CommandSpec`s); tests go in `foo/tests.rs` or `tests/<area>.rs`.
- **Moving code keeps its visibility:** one module down, a private item becomes `pub(super)`, a
  `pub(super)` one `pub(crate)`; the parent re-exports (`pub use bar::Thing;`) so outside paths
  don't change. Children reach the parent with `use super::*;`.
- **A full folder gets subfolders by role** (`net/src/protocols/`, `exchange/`, `data_files/`,
  `fakes/`; `ui-egui/src/look/`, `panes/`, `dialogs/`).
- **A move is a commit of its own:** files moved (`git mv`) with only their `mod` lines and
  re-exports changed, apart from any edit, so history and blame follow the code.
- **Crates stay flat under `crates/`;** a new one only for a measured reason (build time, wasm
  apart from native, a heavy dependency kept out), never for tidiness: modules are for that.
- `xtask/sizes.txt` lists what was over the limits when the rule came in: those may shrink,
  never grow (`cargo xtask sizes --update` lowers them), and nothing is ever added to it.

## Quality gates
Before every commit: `cargo xtask ci`. Commit after every arc of work that builds.

## Docs
One purpose per page, in plain words: `README.md` (what it is, how to run it), `CHANGELOG.md`
(Keep a Changelog, one line per user-visible change under the newest version), `ROADMAP.md`
(where it's going), `docs/gaps.md` (what's missing: impact, workaround, cause). A change users
notice adds its changelog line in the same commit; a gap that closes leaves `docs/gaps.md`.
No measurement logs or step-by-step history in the docs: that belongs in commit messages.

## Packaging and releases
- `packaging/` holds the installer scripts (Linux AppImage/deb/rpm/Flatpak, macOS dmg, Windows
  MSI, FreeBSD); `release.yml` runs them, by hand, and publishes the release (`MailCraft vX.Y.Z`)
  or a test build. `release-artcraft.yml` is the ArtCraft suite's signed release, unused. Keep
  app ids (`io.github.zidan44.MailCraft`), binary names and the WiX `UpgradeCode` stable.
- The version lives only in `[workspace.package]` (`cargo xtask version [set X.Y.Z]`).
- After touching workflows, run `npx actions-up` to keep pinned actions current; the
  `craft-common-actions@main` references stay on `main` on purpose.
- **CI workflows** (as the other Craft apps): workflows call craft-common-actions' shared
  workflows at `@main`, with their inputs and secrets, and run no jobs of their own. A feature or
  fix doesn't add or change CI; what the shared workflows can't do goes to a craft-common-actions
  issue (or a small PR there, safe for every app and no slower). `deny.yml`, `fuzz.yml` and
  `release.yml` still run their own jobs: nothing is added to them, and each moves only as a
  task of its own, once a shared workflow runs it.
  Their jobs are hardened as the shared ones are: Harden-Runner (audit) first, checkouts that
  keep no credentials, and StepSecurity's copies of third-party actions unless theirs is older
  than the version pinned (`install-action` and `freebsd-vm` stay the originals, and
  `rust-cache` while StepSecurity's stops at 2.9.2). `rust-cache` is the commit the shared
  actions pin (2.9.3); 2.9.2 saves no cache on Windows (storytold/craft-common-actions#249).

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
