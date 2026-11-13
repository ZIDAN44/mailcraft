# Host MailCraft on the web

The web build of MailCraft is a static site with no server-side code, so any static host works.
In the browser, MailCraft connects to JMAP accounts only.

## What's in the zip

Each release has a `mailcraft-web-<version>.zip`. Everything sits in one folder,
`mailcraft-web-<version>/`.

| File | What it is |
|---|---|
| `index.html` | The page |
| `mailcraft-web-<hash>.js` | The wasm-bindgen glue that loads the app |
| `mailcraft-web-<hash>_bg.wasm` | The app itself |
| `_headers` | Sample server settings for Netlify and Cloudflare Pages |
| `.htaccess` | Sample server settings for Apache |
| `fonts/` | The Simplified Chinese font, in release builds; fetched only when needed |
| `HOSTING.md` | This page |
| `README.md`, licences, `ATTRIBUTION.md` | The project's readme, licences and credits |

All URLs in the page are relative (`public_url = "./"` in `apps/mailcraft-web/Trunk.toml`). The
site works from any path, inside an `<iframe>` or from a CDN bucket.

## Put it online

1. Get the zip from the GitHub release, or build it yourself with `packaging/web/package.sh`.
   Building needs `trunk` (`cargo install trunk --locked`) and the `wasm32-unknown-unknown` target.
2. Upload the contents of the `mailcraft-web-<version>/` folder to your static host.
3. Set the [server settings](#server-settings) below. The sample `_headers` and `.htaccess` cover
   the MIME types, caching and compression.
4. Make the JMAP server accept requests from your page. It must send CORS headers for the page's
   origin: `Access-Control-Allow-Origin`, and `Authorization` in `Access-Control-Allow-Headers`.
   Or serve MailCraft from the JMAP server's own domain, which needs no CORS.

## Server settings

| Setting | Value |
|---|---|
| MIME types | `.wasm` as `application/wasm`, `.js` as `text/javascript` |
| Compression | gzip or Brotli for `.wasm`, `.js` and `.html` |
| Caching, hashed files | `Cache-Control: public, max-age=31536000, immutable` |
| Caching, `index.html` | `Cache-Control: no-cache` |
| HTTPS | Required for WebGPU, the clipboard and JMAP passwords |
| CSP | `connect-src` includes the JMAP origin; `script-src` includes `'wasm-unsafe-eval'` |

Without HTTPS, MailCraft draws with WebGL2 instead of WebGPU.

## What works in the browser

- **Accounts:** JMAP only (RFC 8620 and 8621), over `fetch`. The rest of the window works as on
  the desktop.
- **Passwords** stay in the tab's memory. After a reload, MailCraft asks for them again.
- **Storage:** mail and accounts are kept in the browser's IndexedDB. If IndexedDB isn't
  available, accounts go to local storage and mail isn't kept. Settings always go to local
  storage.
- **S/MIME `.p12` files** are stored as they came, still encrypted with their password.
  **Unlock…** opens them.
- **Saving and printing** download a file. **Print** downloads a PDF.
- **Backup:** **File › Open & Export › Back up…** downloads everything the browser keeps for the
  site (accounts and their settings, rules, signatures, calendars, contacts; the downloaded
  mail too, when asked) as one `.mailcraft-backup` file, locked with a password if one is
  given. **Restore…** brings such a file in, in this browser or another, in place of what's
  kept there. Nothing goes through a server, and no password is ever in the file. Clearing
  the browser's data for the site removes everything MailCraft keeps: tell your users to make
  a backup first.
- **Desktop only:** IMAP and SMTP, Google and Microsoft sign-in, Exchange, `.pst` files,
  CalDAV and CardDAV, notifications, smart cards, certificate revocation checks, and dragging
  attachments out.

Two URL flags help with testing:

- `?sample` opens the sample mailbox. Nothing is kept.
- `?webgl` forces WebGL2 even where WebGPU is available.

## Test it locally

In the unzipped folder, run:

```sh
python3 -m http.server 8774
```

Then open http://localhost:8774/?sample.
