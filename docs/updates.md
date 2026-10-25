# Updates

How a MailCraft test build keeps itself up to date (v0.25). The code: `apps/mailcraft/src/self_update.rs` (the updater, its helper scripts in
`self_update/install.rs`), `crates/net/src/updates.rs` and `updates/download.rs` (GitHub's list
and the resumable download), `crates/ui-egui/src/app/updates.rs` and `dialogs/closing.rs` (what
the window says).

Only builds from the unsigned test-build workflow update themselves (`MAILCRAFT_UPDATE_CHANNEL=test`
at compile time); signed releases and developers' builds never change themselves, and
`MAILCRAFT_NO_AUTO_UPDATE=1` turns it off for a run.

## What happens

1. **Checks.** A minute after start, then hourly (±10 minutes) for as long as MailCraft stays open,
   the way browsers and code editors check. GitHub is asked with the last `ETag`, so an unchanged
   list costs nothing against its anonymous quota of 60 an hour. A check that fails is tried again
   after 5 minutes, then 10, 20, 40, and hourly after that.
2. **Download.** The installer of the kind this copy came from (`.msi`, the portable
   `.zip`, `.dmg`, `.AppImage`) goes to `<data>/updates`, written to
   `<file>.<size>.part` as it comes. A download cut off, by the connection or by MailCraft closing,
   carries on from the partial file with an HTTP range request (`Range: bytes=N-`), in the same run
   or the next one; a run that finds a partial file checks after ten seconds rather than a minute.
   GitHub's release files redirect to its asset host, which answers ranges with `206` and a
   `Content-Range`, and a range past the end with `416`. The address asked is the release's own
   every time, so the asset host's signed address (valid for about an hour) is never reused.
3. **Checked.** The finished file is kept only if its SHA-256 is what GitHub records for that file
   (`digest`), or else what the release's `SHA256SUMS.txt` says, and its size is the release's.
   A resumed file that doesn't match is fetched once more from the start; a second mismatch, a
   size other than the release's, or a file gone from the release ends it without an installer.
4. **Said.** Once checked, the window shows a bar: *MailCraft 0.25.0 is ready. Restart now to
   install it, or it installs when you close MailCraft.* with **Restart now** and **Later**. If
   MailCraft isn't in front, the system's notification says it too, once per version. The status
   bar shows the download's progress, then *Restart to update*.
5. **Closing asks.** With a version ready, closing asks *Install MailCraft 0.25.0 as it closes?*:
   **Update and close**, **Close without updating** (it's asked again the next time), or Cancel.
   With a download still going, it says it pauses and carries on next time: **Close** or **Keep
   MailCraft open**. If the download finishes while the question is open, the question becomes the
   one about installing, so closing never installs anything unannounced. These come after the
   questions about messages being written and online-only accounts.
6. **Installed.** A detached helper waits for MailCraft to end, puts the new version in place the
   way this copy was installed, and with Restart now opens it. A marker keeps the version just
   installed from being offered again for a day.

| Installed from | The helper |
|---|---|
| `.msi` (Program Files, for everyone) | `msiexec /i … /passive`; Windows asks for an administrator, which the bar and the close question say |
| portable `.zip` | unzipped over the app's folder; `MailCraftData` stays |
| `.dmg` | `MailCraft.app` copied out in place of this one, where this user may write: `/Applications` on an administrator's account, `~/Applications` |
| `.AppImage` | the new file renamed over this one, where this user may write (the running one keeps its mounted, unlinked file) |

Tested for real (2026-10-25) with a test build pretending to be 0.23.0: the portable copy found
`v0.24.0-test.21`, downloaded its 54 MB zip, showed the bar, asked on closing and installed it;
a partial file of 20 MB left from a killed run was carried on with a range request to the same
checksum.

## Windows: one install, for everyone

The MSI installs into Program Files for everyone on the PC, so each update has Windows ask for an
administrator's permission; the bar and the close question say so beforehand. A second package
installing for one user only (`%LOCALAPPDATA%\Programs`, no administrator) was built and tried,
and set aside by the owner: two packages to ship and explain, and every install for everyone to
move over. MSIX would need a code-signing certificate the PC trusts; Velopack would replace the MSI
and the updater both.

## macOS and Linux

There is no installing for everyone or for one user there: the app is a file the user places.

- **macOS (`.dmg`):** `MailCraft.app` is dragged into `/Applications`. On an administrator's
  account (most Macs' first account) that folder is writable without a password, so the update
  replaces the bundle quietly. On a standard account `/Applications`
  isn't writable: put MailCraft in `~/Applications` and it updates itself there; left in
  `/Applications` it keeps File › About's Download link (replacing it would need an administrator's
  password, as Sparkle asks; not done). A copy run straight from the disk image or Downloads is
  translocated by macOS to a read-only place and doesn't update until it's moved to Applications.
  Craft apps in `~/Applications` are found as in `/Applications`.
- **Linux (`.AppImage`):** the file lives wherever the user put it (`~/Applications`, `~/bin`,
  Downloads) and the update is renamed over it, no root needed; the running copy keeps its mounted
  file until it closes. An AppImage in a folder the user can't write (`/opt`, put there with sudo)
  isn't touched and keeps the Download link.
- **`.deb`, `.rpm`, Flatpak, the tarballs, FreeBSD:** their package managers update them (`apt`,
  `dnf`, `flatpak update`); MailCraft doesn't replace them and File › About links the release.

## Not done, and why

- **A signature on the update.** The SHA-256 comes from the same GitHub release as the file, so it
  proves the file arrived whole, not who made it: anyone able to change the release could change
  both. Tauri and Sparkle sign updates with an Ed25519 key whose public half is compiled into the
  app. Doing so needs a signing key kept as a secret of the release workflows: the owner's to make.
- **Staged rollout.** Every tester gets a test build within the hour. A rollout like Sparkle's
  (a random group kept on each PC, one more group let in each day from the release's date) needs
  no server and could be added when there are more than testers.
- **Smaller downloads.** The whole installer is fetched; AppImage's zsync and Velopack's deltas
  fetch only what changed.
