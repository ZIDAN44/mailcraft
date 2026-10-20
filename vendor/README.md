# Vendored crates

Permissively licensed crates MailCraft carries patches for, each with its upstream licence files
unchanged. Never copyleft code. Remove a vendored copy once upstream releases the fixes.

| Crate | Version | Licence | Why |
|---|---|---|---|
| winit | 0.30.13 | Apache-2.0 | Upstream fixes no 0.30 release has, as PdfCraft carries them (copied from storytold/pdfcraft's `vendor/winit`; each patch is marked `PdfCraft patch:` in the source). |

winit's patches, each a port of an upstream pull request or PdfCraft's own fix:

1. **Windows 11, a window dragged to a monitor with another scale** took the rect Windows
   suggested only on winit master ([winit#4341](https://github.com/rust-windowing/winit/pull/4341)):
   0.30.13 nudged it back onto the monitor being left, so it grew and stayed at the wrong scale.
2. **Wayland, IME after a device change:** a seat's text input was destroyed whenever the seat
   lost any capability (a tablet unplugged, a remote session ending), so input methods stopped
   until restart ([winit#4747](https://github.com/rust-windowing/winit/pull/4747)).
3. **Windows, the Korean IME's caret:** the composition cursor read as 0, so no caret was drawn
   while composing ([winit#4746](https://github.com/rust-windowing/winit/pull/4746)).
4. **Windows, a window created maximized** showed at once, though eframe keeps it hidden until
   its first frame: a window saved maximized appeared, vanished and appeared again (measured in
   MailCraft: 6 starts in 6; with this patch shown once, maximized, 5 in 5)
   ([winit#4587](https://github.com/rust-windowing/winit/pull/4587)).
5. **X11, the input method and the focused text field:** egui allows IME as a field gains focus,
   which replaced the input context without focusing it ([winit#4727](https://github.com/rust-windowing/winit/pull/4727)).
6. **macOS, input methods:** text committed after the marked text is cleared, Korean keys typed
   after a syllable, Space with no syllable, the first key in a fresh window, an unfinished
   composition coming back in the next field, emoji from the picker
   ([winit#4650](https://github.com/rust-windowing/winit/pull/4650),
   [#4478](https://github.com/rust-windowing/winit/pull/4478),
   [#4744](https://github.com/rust-windowing/winit/pull/4744),
   [#4745](https://github.com/rust-windowing/winit/pull/4745),
   [#4749](https://github.com/rust-windowing/winit/pull/4749)).
7. **Linux, scale factor:** the X server's DPI is preferred over a panel's physical DPI.
8. **Wayland, wheel scrolling:** a pixel-only wheel no longer starts a finger gesture that never
   ends, which left Ctrl held for zooming.
9. **macOS:** a window's `movable_by_window_background` setting is honoured.

`cargo xtask ci` builds MailCraft against this copy (`[patch.crates-io]` in the root
`Cargo.toml`); `Cargo.lock` names it by path.
