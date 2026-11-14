# Fuzzing

Every parser that reads network or disk input has a fuzz target here, because MailCraft must never
crash on hostile input (see "Never crash" in [CLAUDE.md](../CLAUDE.md)). The targets live in a
workspace of their own, so the main build never needs a nightly toolchain.
`.github/workflows/fuzz.yml` runs every target once a week.

That workspace has a `Cargo.lock` of its own. It keeps the versions the main one has, so the
fuzzers test what MailCraft ships: after a dependency moves, run `cargo xtask locks --update`
(`cargo xtask ci` fails while the two differ).

## Run a target

You need Linux, macOS or WSL, a nightly toolchain, and cargo-fuzz (`cargo install cargo-fuzz`).

1. Go to this folder: `cd fuzz`.
2. Run the target with its seeds and dictionary. This runs `imap` for five minutes:

   ```bash
   cargo +nightly fuzz run -O imap corpus/imap seeds/imap -- -dict=dicts/imap.dict -max_total_time=300
   ```

The fuzzer keeps what it finds in `corpus/` and `artifacts/`. Git ignores both folders.

## Add a target

1. List the target in `fuzz/Cargo.toml` and write its source in `fuzz_targets/<target>.rs`. When
   that folder is full, put the source in a subfolder named for its role (for example
   `fuzz_targets/store/`).
2. Add at least one seed file in `seeds/<target>/`.
3. Add a dictionary in `dicts/<target>.dict`. libFuzzer stops when a dictionary is missing.
4. Run `cargo xtask fuzz` (part of `cargo xtask ci`). It checks that every target has all three.

## Fix a crash

1. Find the input that crashed in `artifacts/<target>/`. The weekly run also keeps it as a
   downloadable artifact.
2. Replay it: `cargo +nightly fuzz run -O <target> <file>`.
3. Fix the code.
4. Add a regression test next to the code you fixed.

## What counts as a crash

- Every panic counts, even one that `catch_unwind` would catch.
- `pst` treats every checksum as correct, so damage reaches the code behind the checks.
- `pst_append` does so on half its inputs, and there only a panic counts: such a file can hold
  anything. On the other half a damaged file must give an error, and adding mail must leave the
  earlier messages unchanged.
- Some `pst` and `pst_append` seeds are written by tests run with `MC_PST_SEED` or
  `MC_PST_ANSI_SEED` set. The tests are `pst/tests/pim_written.rs` and `pst_export/tests/ansi.rs`,
  both under `crates/net/src/data_files/`.

## Targets

| Target | What it reads |
|---|---|
| `mime_parse` | A message source: parse, summary, search text, HTML, attachments |
| `msg` | An Outlook message file (`.msg`): properties, recipients, files, attached messages, named properties |
| `index_text` | A message source read for the search index |
| `html` | An HTML body: text, layout, CSS, links, pictures, trackers, read as Outlook reads it (conditional comments, VML) |
| `ical` | iCalendar, written and read back; WebDAV `multistatus` replies |
| `vcard` | vCard, written and read back |
| `backup` | A backup file of what the browser keeps: read, written and read back |
| `http` | HTTP answers, whole and streamed; `Content-Range`; release data and `SHA256SUMS.txt` |
| `imap` | A whole IMAP session, including a folder sync |
| `jmap` | JMAP sign-in, sync, push events and URLs |
| `sign_in` | How a server asks to be signed in to (`WWW-Authenticate`), and a JMAP server's refusal |
| `search` | The query language, dates, the **Advanced** form, JMAP filters, modified UTF-7 |
| `data_file` | A damaged account data file |
| `data_folder` | A damaged data folder's own files |
| `search_index` | A damaged search index |
| `photo` | A JPEG or PNG with its location removed |
| `pdf` | A message printed: a picture read from any bytes and embedded, tables laid out on pages |
| `smime` | BER, signatures, encrypted mail, certificates, `.p12` files |
| `pgp` | OpenPGP keys, encrypted and signed messages, detached and cleartext signatures; PGP/MIME, inline PGP, Autocrypt headers and attached keys |
| `pst` | A `.pst` file (Outlook data file), patched or whole |
| `pst_append` | Mail added to a `.pst` file and removed; other messages stay unchanged |
| `import_checkpoint` | An import checkpoint, read and written back |
| `pst_item` | A `.pst` mail item built from hostile property values |
| `pst_props` | HTML bodies and 8-bit strings in their code pages |
| `pst_recurrence` | A recurrence pattern, read and written |
| `pst_pim_text` | Contact and task text, exported and read back |
| `signature` | A signature as HTML code or a saved web page |
| `tnef` | `winmail.dat` (TNEF): files, body, attached messages, compressed RTF |
