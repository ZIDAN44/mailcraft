# Third-party assets

Every non-code asset in this repository has a row here (path, author, source, licence).
`cargo xtask assets` (part of `cargo xtask ci`) fails otherwise.

MailCraft currently ships **no asset files**: icons are drawn in code (`crates/ui-egui/src/icons.rs`),
the window icon and the sample picture are generated in code, and the UI uses an installed system
font when one is found (never bundled).

| Path | Author | Source | Licence |
|---|---|---|---|
| `docs/images/reading.png` | MailCraft contributors | Screenshot of MailCraft's own sample mailbox | MIT OR Apache-2.0 |
| `docs/images/search-dark.png` | MailCraft contributors | Screenshot of MailCraft's own sample mailbox | MIT OR Apache-2.0 |
| `docs/images/compose.png` | MailCraft contributors | Screenshot of MailCraft's own sample mailbox | MIT OR Apache-2.0 |
