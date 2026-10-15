//! MailCraft in the browser.
//!
//! Runs the same [`mailcraft_ui_egui::MailApp`] as the desktop app through eframe's web runner
//! (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release` from this
//! directory (output in `dist/web`).
//!
//! A web page can't open IMAP or SMTP connections, so accounts sync over JMAP (RFC 8620/8621),
//! the same transport as the desktop app's (`mailcraft-jmap`) running over `fetch`. The JMAP
//! server must allow the page's origin (CORS), and new mail arrives by push (the session's
//! event stream, read through `fetch` so it can carry the credentials). The cache is kept in
//! IndexedDB between visits and settings in local storage; passwords never: they are asked for
//! again on each visit. Files are downloads, and attachments come from the browser's file picker.
//! URL flags: `?sample` opens the sample mailbox (nothing is kept), `?webgl` forces WebGL2.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_arch = "wasm32")]
mod backend;
#[cfg(target_arch = "wasm32")]
mod fetch;
#[cfg(target_arch = "wasm32")]
mod idb;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("mailcraft-web only runs in the browser: build it with `trunk build --release` in apps/mailcraft-web");
}
