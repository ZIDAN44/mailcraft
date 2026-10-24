# Fuzzing

Fuzz targets for the parsers that read network and disk input (CLAUDE.md, "Never crash"). This
is its own Cargo workspace, outside the main one, so the main build never needs nightly or
libFuzzer. `.github/workflows/fuzz.yml` runs every target weekly on Linux.

| Target | What it reads |
|---|---|
| `mime_parse` | a message source: parse, list summary, search text, HTML body, attachments |
| `html` | an HTML body: text, formatted lines, columns, CSS, links and pictures; MailCraft's markup |
| `ical` | an iCalendar file, written back and read again; a WebDAV `multistatus` answer |
| `vcard` | a vCard file, each card written back and read again |
| `imap` | everything an IMAP server sends through a session (greeting to MOVE, with a folder sync) |
| `jmap` | a JMAP server's answers (split at form feeds) through sign-in, folder list and folder sync; push events; URLs |
| `search` | the search box: the query language, the Advanced form, JMAP filters, modified UTF-7 |
| `photo` | a JPEG or PNG file: where it was taken left out, and the result checked for anything left |
| `pst` | an Outlook data file: a good exported file with the input as byte patches, or a whole file |

## Running

Linux or macOS (or WSL), with a nightly toolchain and `cargo install cargo-fuzz`:

```bash
cd fuzz
cargo +nightly fuzz run -O imap corpus/imap seeds/imap -- -dict=dicts/imap.dict -max_total_time=300
```

`seeds/` holds a hand-written valid input per target and `dicts/` the protocol's keywords, both
to get the fuzzer past the first line quickly; `corpus/` and `artifacts/` stay out of git. A
crash leaves its input in `artifacts/<target>/`: replay it with
`cargo +nightly fuzz run -O <target> <file>`, fix the code, and add the input to a regression test
next to the code (never only to the corpus).

Every panic counts as a crash, even one MailCraft's own `catch_unwind` fences would catch (the
HTML converter's, for instance): there the message would lose its formatting, so it is worth a
fix. The exception is `pst`, where `pst::import` fences the outlook-pst reader and reports the
file as damaged, the right answer for a damaged file: that target aborts only on a panic in
MailCraft's own code.
