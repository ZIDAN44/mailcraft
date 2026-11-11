# How MailCraft updates itself

Builds made by MailCraft's release workflow (`release.yml`) update themselves, from
MailCraft's releases on GitHub (ZIDAN44/mailcraft):

- **A test build** (`v0.33.0-test.31`, a pre-release; built with `MAILCRAFT_UPDATE_CHANNEL=test`)
  updates to the next test build or release, whichever is newer.
- **A release** (`v0.33.0`; built with `MAILCRAFT_UPDATE_CHANNEL=stable`) updates to the next
  release only, never to a test build.

A version's release comes after its test builds: a test build of 0.33.0 updates to the release of
0.33.0. Developer builds never change themselves. To skip updates for one run, start MailCraft with
`MAILCRAFT_NO_AUTO_UPDATE=1`. The workflow's `kind` input publishes a release (the default) or a
test build.

## How an update installs

1. **Check.** MailCraft asks GitHub for its releases a minute after it starts, then every hour.
   It sends the last `ETag` (GitHub's tag for the list it saw), so an unchanged list costs nothing.
   A failed check tries again after 5, then 10, 20 and 40 minutes.
2. **Download.** The installer for this platform goes into `<data>/updates` as
   `<file>.<size>.part`. If the download stops, it carries on from where it was.
3. **Verify.** The file's size and SHA-256 checksum must match the release's own record: GitHub's
   `digest` for the file, or else the release's `SHA256SUMS.txt`. A file that doesn't match is
   deleted.
4. **Announce.** A bar offers **Restart now** or **Later**. The system shows one notification per
   version, and the status bar shows the download's progress.
5. **Close.** Closing MailCraft offers **Update and close** or **Close without updating**. If the
   download isn't finished, it carries on the next time MailCraft runs.
6. **Install.** A small helper waits for MailCraft to exit and installs the update. After
   **Restart now**, it opens MailCraft again.

## What the helper does

| Installed from | The helper |
|---|---|
| `.msi` | Runs `msiexec /i … /passive`; needs an administrator |
| portable `.zip` | Unzips over the app folder; `MailCraftData` is kept |
| `.dmg` | Copies `MailCraft.app` over this one, if the folder is writable |
| `.AppImage` | Renames the new file over this one, if the folder is writable |
| `.deb`, `.rpm` | Installs the new package with `apt-get`, or `dnf`, `zypper` or `rpm`, after the system asks for your password (`pkexec`); without `pkexec`, opens it in the software installer |
| `.tar.gz` (Linux, FreeBSD) | Unpacks it over the folder it was unpacked into, if that folder is writable |
| Flatpak | Opens the new bundle in the software installer as MailCraft closes; Flatpak installs it (MailCraft can't from inside its sandbox, and can't reopen itself) |

## Per platform

- **Windows:** The MSI installs for all users, so approve each update as an administrator.
- **macOS:** Move MailCraft to **Applications** before it can update itself. On a standard
  account, use `~/Applications` instead. A copy run from the disk image or Downloads doesn't update.
- **Linux:** Keep the `.AppImage` or the unpacked `.tar.gz` in a folder you can write to, and it
  updates without root. In other folders (for example `/opt`), MailCraft shows a Download link
  instead. The `.deb` and `.rpm` update through the system's package tool, which asks for your
  password. The Flatpak bundle updates through your software installer (GNOME Software, KDE
  Discover), which opens with the new bundle.
- **FreeBSD:** The unpacked `.tar.gz` updates in place, in a folder you can write to.

## MailCraft's new id (0.36)

MailCraft's id was `ai.storyteller.mailcraft` until 0.36 and is `io.github.zidan44.MailCraft`
now. An update installs the new id's files, and the `.msi`, `.deb` and `.rpm` remove their old
ones. At each start MailCraft tidies whatever else the old id left:

- **Windows:** its notification name, and the notification settings Windows kept for it.
- **macOS:** its preferences, saved window state, caches and web storage in `~/Library`.
- **Linux, FreeBSD:** a default mail app chosen in `mimeapps.list` now names the new desktop entry,
  and an unpacked `.tar.gz` loses the old desktop entry, AppStream file and icons.
- **Flatpak:** the new id is a new app. On its first start it copies the old one's app folder
  (accounts and settings; the data files are in Documents › MailCraft Files, shared by both), then
  says to remove the old one with `flatpak uninstall --delete-data ai.storyteller.mailcraft`, which
  MailCraft can't do from inside its sandbox.

## Code

- `apps/mailcraft/src/self_update.rs`, `self_update/detect.rs` and `self_update/install.rs`:
  checking, scheduling, how this copy was installed, and the helper; `self_update/renamed.rs`:
  tidying what the old id left.
- `crates/engine/src/session.rs` (`newer_release`): which release is newer, by channel.
- `crates/net/src/updates.rs` and `updates/download.rs`: reading the release list and downloading.
- `crates/ui-egui/src/app/updates.rs` and `dialogs/closing.rs`: the bar and the close dialog.

Signed updates, staged rollouts and delta downloads are still missing. See
[known gaps](gaps.md#4-signed-updates-and-releases).
