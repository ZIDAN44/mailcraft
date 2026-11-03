# Fuzzing

Every parser that reads network or disk input has a fuzz target here, because MailCraft must never
crash on hostile input (see "Never crash" in [CLAUDE.md](../CLAUDE.md)). The targets live in a
workspace of their own, so the main build never needs a nightly toolchain.
`.github/workflows/fuzz.yml` runs every target once a week.

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
- `pst` is the one exception. `pst::import` runs the `.pst` reader (from the fork
  [ZIDAN44/outlook-pst-rs](https://github.com/ZIDAN44/outlook-pst-rs), `craft/rw-1.2.14`) behind
  `catch_unwind`, so only a panic in MailCraft's own code counts.
- `pst_append` has no exception. A damaged file must give an error, and adding mail must leave the
  earlier messages unchanged.
- Some `pst` and `pst_append` seeds are written by tests run with `MC_PST_SEED` or
  `MC_PST_ANSI_SEED` set. The tests are `pst/tests/pim_written.rs` and `pst_export/tests/ansi.rs`,
  both under `crates/net/src/data_files/`.

## Targets

| Target | What it reads |
|---|---|
| `mime_parse` | A message source: parse, summary, search text, HTML, attachments |
| `index_text` | A message source read for the search index |
| `html` | An HTML body: text, layout, CSS, links, pictures, trackers |
| `ical` | iCalendar, written and read back; WebDAV `multistatus` replies |
| `vcard` | vCard, written and read back |
| `http` | HTTP answers, whole and streamed; `Content-Range`; release data and `SHA256SUMS.txt` |
| `imap` | A whole IMAP session, including a folder sync |
| `jmap` | JMAP sign-in, sync, push events and URLs |
| `search` | The query language, dates, the **Advanced** form, JMAP filters, modified UTF-7 |
| `data_file` | A damaged account data file |
| `data_folder` | A damaged data folder's own files |
| `search_index` | A damaged search index |
| `photo` | A JPEG or PNG with its location removed |
| `smime` | BER, signatures, encrypted mail, certificates, `.p12` files |
| `pst` | A `.pst` file (Outlook data file), patched or whole |
| `pst_append` | Mail added to a `.pst` file and removed; other messages stay unchanged |
| `import_checkpoint` | An import checkpoint, read and written back |
| `pst_item` | A `.pst` mail item built from hostile property values |
| `pst_props` | HTML bodies and 8-bit strings in their code pages |
| `pst_recurrence` | A recurrence pattern, read and written |
| `pst_pim_text` | Contact and task text, exported and read back |
| `signature` | A signature as HTML code or a saved web page |
| `tnef` | `winmail.dat` (TNEF): files, body, attached messages, compressed RTF |
