# How test builds update themselves

Only unsigned test builds, compiled with `MAILCRAFT_UPDATE_CHANNEL=test`, update themselves.
Releases and developer builds never change themselves. To skip updates for one run, start
MailCraft with `MAILCRAFT_NO_AUTO_UPDATE=1`.

## How an update installs

1. **Check.** MailCraft asks GitHub for new test builds a minute after it starts, then every hour.
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

## Per platform

- **Windows:** The MSI installs for all users, so approve each update as an administrator.
- **macOS:** Move MailCraft to **Applications** before it can update itself. On a standard
  account, use `~/Applications` instead. A copy run from the disk image or Downloads doesn't update.
- **Linux:** Keep the `.AppImage` in a folder you can write to, and it updates without root. In
  other folders (for example `/opt`), MailCraft shows a Download link instead.
- **`.deb`, `.rpm`, Flatpak, tarballs and FreeBSD:** Update through your package manager.
  **File › About** links to the release.

## Code

- `apps/mailcraft/src/self_update.rs` and `self_update/install.rs`: checking, scheduling and the
  helper.
- `crates/net/src/updates.rs` and `updates/download.rs`: reading the release list and downloading.
- `crates/ui-egui/src/app/updates.rs` and `dialogs/closing.rs`: the bar and the close dialog.

Signed updates, staged rollouts and delta downloads are still missing. See
[known gaps](gaps.md#4-signed-updates-and-releases).
