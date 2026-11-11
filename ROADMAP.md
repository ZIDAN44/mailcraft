# Roadmap

MailCraft is heading toward the everyday Outlook mail workflow, over IMAP and SMTP,
as an open-source, local-first app for Windows, macOS, Linux and the browser.

## Where it stands

- **Scope:** v1 is the everyday Outlook mail workflow. What it still lacks is in
  [known gaps](docs/gaps.md), most-hit first.

## Headline numbers

These numbers are as of v0.34.

| | |
|---|---|
| Crates | 12 libraries, 3 apps and `xtask` |
| Commands | 283 (list them with `mailcraft-cli commands`) |
| Tests | 2,006, over in-memory servers, data files, TCP and the headless UI |
| Languages | 15, including Arabic and Hebrew |
| Platforms | Windows, macOS and Linux; the browser, for JMAP accounts |

## Next

The work left is listed in [known gaps](docs/gaps.md), most-hit first:

- Right-to-left text in dialogs, menus and the editor.
- A data folder that moves keeps its claim on its `.pst` copies.
- Dragging attachments out of MailCraft on a Wayland-only desktop.
- Signed releases and signed updates.

See [what each version added](CHANGELOG.md) and [what's still missing](docs/gaps.md).
