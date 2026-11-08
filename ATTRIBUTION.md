# Third-party assets

Every non-code asset in this repository has a row here (path, author, source, licence).
`cargo xtask assets` (part of `cargo xtask ci`) fails otherwise.

None of this repository's assets are third-party: the interface icons are drawn in code
(`crates/ui-egui/src/look/icons.rs`), the sample picture is generated in code, and the UI uses an
installed system font when one is found. The app does carry egui's built-in fonts, which come in
with the `epaint_default_fonts` crate (egui's fallbacks for text, symbols and emoji, under their
own licences, in the crate beside each font): Ubuntu Light (Ubuntu Font Licence 1.0), Hack (MIT
and the Bitstream Vera licence), Noto Emoji (SIL Open Font License 1.1), and John Slegers'
emoji-icon-font with egui's subset of it, egui-icons (MIT). Builds made with `CRAFT_FONTS_DIR` (the releases) embed
the fonts listed in [storytold/craft-fonts](https://github.com/storytold/craft-fonts)' `ATTRIBUTION.md`
(SIL Open Font License 1.1); none are in this repository, and each one's licence is shown on the
About page. The app icon files in `assets/app-icon/`
are rendered by `cargo xtask icons` from MailCraft's own drawing in `xtask/src/icons.rs`.

`crates/pst` was written from Microsoft's published MS-PST specification. Microsoft's
[outlook-pst](https://github.com/microsoft/outlook-pst-rs) reader and Dong Shin's
[outlook-pst-rw](https://github.com/dongs0104/outlook-pst-rs) writer (both MIT) were read to check
how real files behave, and its results were compared with theirs.

| Path | Author | Source | Licence |
|---|---|---|---|
| `docs/images/reading.png` | MailCraft contributors | Screenshot of MailCraft's own sample mailbox | MIT OR Apache-2.0 |
| `docs/images/search-dark.png` | MailCraft contributors | Screenshot of MailCraft's own sample mailbox | MIT OR Apache-2.0 |
| `docs/images/compose.png` | MailCraft contributors | Screenshot of MailCraft's own sample mailbox | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/128x128/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/16x16/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/24x24/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/256x256/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/32x32/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/48x48/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/512x512/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/64x64/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/scalable/apps/ai.storyteller.mailcraft.svg` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/mailcraft-1024.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `fuzz/seeds/photo/geotagged.jpg` | MailCraft contributors | Written byte by byte as a fuzzing seed (a few header bytes, no picture) | MIT OR Apache-2.0 |
| `fuzz/seeds/photo/geotagged.png` | MailCraft contributors | Written byte by byte as a fuzzing seed (a few header bytes, no picture) | MIT OR Apache-2.0 |
| `crates/smime/testdata/encrypted-cbc.eml` | MailCraft contributors | Made with `openssl cms` from MailCraft's own test certificates (S/MIME tests) | MIT OR Apache-2.0 |
| `crates/smime/testdata/encrypted-gcm.eml` | MailCraft contributors | Made with `openssl cms` from MailCraft's own test certificates (S/MIME tests) | MIT OR Apache-2.0 |
| `crates/smime/testdata/signed-clear.eml` | MailCraft contributors | Made with `openssl cms` from MailCraft's own test certificates (S/MIME tests) | MIT OR Apache-2.0 |
| `crates/smime/testdata/signed-ec.eml` | MailCraft contributors | Made with `openssl cms` from MailCraft's own test certificates (S/MIME tests) | MIT OR Apache-2.0 |
| `crates/smime/testdata/signed-encrypted.eml` | MailCraft contributors | Made with `openssl cms` from MailCraft's own test certificates (S/MIME tests) | MIT OR Apache-2.0 |
| `crates/smime/testdata/signed-opaque.eml` | MailCraft contributors | Made with `openssl cms` from MailCraft's own test certificates (S/MIME tests) | MIT OR Apache-2.0 |
| `crates/smime/testdata/signed-tampered.eml` | MailCraft contributors | Made with `openssl cms` from MailCraft's own test certificates (S/MIME tests) | MIT OR Apache-2.0 |
| `assets/app-icon/mailcraft.icns` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/mailcraft.ico` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
