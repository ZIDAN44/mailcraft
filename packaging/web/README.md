# Hosting MailCraft for the web

`mailcraft-web-<version>.zip` (from the GitHub release, or `packaging/web/package.sh`) holds a
static site in `mailcraft-web-<version>/`:

| File | What it is |
|---|---|
| `index.html` | The page. It loads everything through relative URLs. |
| `mailcraft-web-<hash>.js` | wasm-bindgen glue (generated, ES module) |
| `mailcraft-web-<hash>_bg.wasm` | The app |
| `_headers`, `.htaccess` | Sample header rules for Netlify/Cloudflare Pages and Apache |

There is no server-side code. Upload the folder's contents anywhere that serves static files.

## What works in the browser

A web page can't open IMAP or SMTP connections, so the browser version syncs **JMAP** accounts
(RFC 8620/8621: Fastmail, Stalwart, Cyrus, Apache James…) over `fetch`, with the same code as the
desktop app. Everything else in the window works as on the desktop: the list, conversations,
reading, writing with attachments, folders, rules, categories, search, the calendar and people
kept with the account, signed and encrypted mail (S/MIME), and every language.

- **The mail server must allow the page.** The browser only lets the page talk to a JMAP server
  that answers with CORS headers for the page's origin (`Access-Control-Allow-Origin`, and
  `Authorization` in `Access-Control-Allow-Headers`). Without them, connecting fails with
  "the server may not allow web pages to reach it". Serving MailCraft from the mail server's own
  domain avoids the question.
- **Passwords are never kept.** They stay in the tab's memory; after a reload MailCraft asks
  again. The account list, folder lists and settings are kept in the page's `localStorage`
  (no mail, no passwords). Mail is cached in memory and synced again on each visit.
- **Certificates stay locked with their passwords.** An S/MIME certificate's `.p12` file is kept
  in the page's storage as it came, encrypted with its password, which isn't kept; after a reload
  Unlock… opens every certificate that password fits. A renewed certificate keeps the old one, to
  read mail sent to it. Smart cards, the system's certificate store and revocation checks are the
  desktop app's.
- **Desktop only:** IMAP/SMTP, Google and Microsoft sign-in, Exchange (Microsoft Graph), opening
  Outlook data files, CalDAV/CardDAV sync, desktop notifications, dragging attachments out.
- **Files are downloads.** Save attachment, Save as and Save as PDF download the file; Print
  downloads the PDF to print from the browser's viewer.
- `?sample` opens the sample mailbox (nothing is kept); `?webgl` forces the WebGL2 renderer.

## Any path works

All URLs in `index.html` are relative (`public_url = "./"` in `apps/mailcraft-web/Trunk.toml`),
so the site works at a domain root, under a prefix (`https://example.com/mail/`) and from a CDN
bucket. The asset names carry a content hash, so they can be cached forever; only `index.html`
needs revalidation.

## Required server settings

- **MIME type:** serve `.wasm` as `application/wasm` (browsers refuse to stream-compile it
  otherwise) and `.js` as `text/javascript`.
- **Compression:** turn on gzip or Brotli for `.wasm`, `.js` and `.html`.
- **Caching:** `Cache-Control: public, max-age=31536000, immutable` on the hashed `.wasm` and
  `.js` files, and `no-cache` on `index.html`.
- **HTTPS:** WebGPU and the clipboard need a secure context (`https://` or
  `http://localhost`); elsewhere the app falls back to WebGL2. A JMAP password should only ever
  travel over HTTPS anyway.
- **Content Security Policy:** if you send one, `connect-src` must include your JMAP server's
  origin, and `script-src` needs `'wasm-unsafe-eval'`.

Local test: `python3 -m http.server 8774` inside the folder, then open http://localhost:8774/
(or http://localhost:8774/?sample).
