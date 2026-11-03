# Fuzzing

Fuzz targets for the parsers that read network and disk input (CLAUDE.md, "Never crash"). This
is its own Cargo workspace, outside the main one, so the main build never needs nightly or
libFuzzer. `.github/workflows/fuzz.yml` runs every target weekly on Linux.

| Target | What it reads |
|---|---|
| `mime_parse` | a message source: parse, list summary, search text, HTML body, attachments |
| `index_text` | a message source read for search: boundaries found, text parts decoded, files counted and named but never decoded, pictures the HTML shows told from files |
| `html` | an HTML body: text, formatted lines, columns, CSS, links and pictures, trackers and cleaned links, hidden text; MailCraft's markup |
| `ical` | an iCalendar file (events and to-dos), written back and read again; a WebDAV `multistatus` answer |
| `vcard` | a vCard file, each card written back and read again |
| `http` | an HTTP answer read whole and streamed to a file (lengths, chunks, to the end), a download's `Content-Range`; GitHub's release list, `SHA256SUMS.txt`, its quota header and tags |
| `imap` | everything an IMAP server sends through a session (greeting to MOVE, with a folder sync) |
| `jmap` | a JMAP server's answers (split at form feeds) through sign-in, folder list and folder sync; push events; URLs |
| `search` | the search box: the query language, its dates on any clock, a query checked against a message and its text, folding, the Advanced form, JMAP filters, modified UTF-7; lookalike senders, known names on other addresses, External |
| `data_file` | an account's data file as a build before 0.32 may have left it, damaged any way (files whole and in pieces, notes of any length, pieces missing, compressed or raw, the search index's, its `meta` table): its health measured, then compressed again, stopped after a batch and gone on with, and every file that read back before reads back the same |
| `search_index` | an account's search index as its files read back (meta, doc table, text pages, segments), each damaged any way: opened, words looked up whole and a page at a time, texts read, a commit followed, and written to |
| `photo` | a JPEG or PNG file: where it was taken left out, and the result checked for anything left |
| `smime` | S/MIME: BER, signatures (detached and opaque), encrypted mail opened with a test key, certificates, `.p12` files; a message told signed or encrypted from its header alone, never wrongly told it isn't |
| `pst` | an Outlook data file: a good exported file with the input as byte patches, or a whole file |
| `pst_append` | mail added to an Outlook data file already there, as an export adds to one: a file the writer made with the input as byte patches, or a whole file (a seed is one encoded as Outlook encodes its files), the special folders it records looked up (entry IDs, the folders they name, the hierarchy down to them), then a message with a file appended (unread, flagged, in a category: tagged properties, and named ones looked up in the file's name-to-ID map and added to it) and every message that read before read again, as it was, beside the one added |
| `import_checkpoint` | an import's checkpoint read back from an account's data file: checked (counts, folders named once and where they're used, the part-way folder's list tail), and one that would be gone on from written again and read back the same |
| `pst_item` | a `.pst` mail item rebuilt as a message from hostile property values: the headers made from them (Thread-Index, Thread-Topic, Sensitivity) stay on clean lines of their own; recipients' one-off entry ids; categories as keywords; an original header's `multipart/report` type |
| `pst_props` | text in a data file's properties: an HTML body or an 8-bit string in its code page (a declared charset, byte-order marks), and the bytes in a code page of each kind MailCraft reads (the input names any other) |
| `pst_recurrence` | a `.pst` appointment's recurrence pattern: its rule, deleted occurrences and changed ones (their times, 8-bit and UTF-16 subjects and rooms), each changed one an event replacing one occurrence; the calendar they make written and read again |
| `pst_pim_text` | what an export to a data file keeps in a contact's and a task's text (a card's or a to-do's lines after it) read back into the card and the task, held to what they keep; a task's rule as the recurrence pattern the export writes, which the import reads |
| `signature` | a signature as HTML code or a saved web page (Outlook's `.htm`): made harmless, its pictures out and back, its text, as sent |
| `tnef` | Outlook's `winmail.dat`: its files, body and attached messages, compressed RTF and the HTML or text in it, and as a message's attachment |

## Running

Linux or macOS (or WSL), with a nightly toolchain and `cargo install cargo-fuzz`:

```bash
cd fuzz
cargo +nightly fuzz run -O imap corpus/imap seeds/imap -- -dict=dicts/imap.dict -max_total_time=300
```

`seeds/` holds a hand-written valid input per target and `dicts/` the protocol's keywords, both
to get the fuzzer past the first line quickly; a new target needs both (`cargo xtask fuzz`, part
of `cargo xtask ci`, checks). `corpus/` and `artifacts/` stay out of git. A
crash leaves its input in `artifacts/<target>/`: replay it with
`cargo +nightly fuzz run -O <target> <file>`, fix the code, and add the input to a regression test
next to the code (never only to the corpus).

Every panic counts as a crash, even one MailCraft's own `catch_unwind` fences would catch (the
HTML converter's, for instance): there the message would lose its formatting, so it is worth a
fix. The exception is `pst`, where `pst::import` fences the outlook-pst reader and reports the
file as damaged, the right answer for a damaged file: that target aborts only on a panic in
MailCraft's own code, and a panic in the reader (built from MailCraft's fork) is not one.
Its seeds are the patches' starting point and a whole file whose first attachment holds a
message and whose second is a zero-byte file (as `pst/tests/reader.rs` makes it), so the reader's
paths for those are fuzzed too, and one with an RSS Feeds folder and a feed's posts, a recall, a
sharing invitation and a contact in a mail folder, and a note with a file (`import-rest`, written
by `pst/tests/pim_written.rs` with `MC_PST_SEED` set). `pst_append` has no such exception: the writer, built from
MailCraft's fork (`craft/rw-1.2.14`), reads the file it adds to as the reader does and turns a
damaged one into an error, so a panic anywhere in it is a crash to fix there; so is an append
that succeeds while a message that read before it no longer reads as it did.
