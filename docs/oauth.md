# Browser sign-in (OAuth)

Microsoft (Outlook.com, Hotmail, Live and Microsoft 365) and Google no longer accept passwords
from mail apps over IMAP, POP and SMTP: they want their own sign-in in the browser (RFC 8252 with
PKCE). Providers only give tokens to a registered app, and the registration belongs to whoever
ships MailCraft, so a copy of MailCraft offers *Sign in with Microsoft* / *Sign in with Google*
only when it was given a registration:

| Setting | What it is |
|---|---|
| `MAILCRAFT_MICROSOFT_CLIENT_ID` | the Application (client) ID of a Microsoft Entra app registration |
| `MAILCRAFT_GOOGLE_CLIENT_ID`, `MAILCRAFT_GOOGLE_CLIENT_SECRET` | a Google "Desktop app" OAuth client (its secret isn't secret for installed apps) |

They're read when MailCraft runs, or built in when they're set while building. The test release
workflow (`release-wip.yml`) passes the repository secrets of those names to the build; without
them the build offers passwords only, and an Outlook.com address can't be added (the Add account
dialog says why). The signed release (`release.yml`) runs the shared
`storytold/craft-common-actions` workflow, which accepts only the secrets it declares: to give it
sign-in, that workflow needs a pass-through for these settings, added by whoever ships it with
their own registration.

## Registering MailCraft with Microsoft

1. In the Microsoft Entra admin centre (any Microsoft account will do), **App registrations ›
   New registration**: a name, and *Accounts in any organizational directory and personal
   Microsoft accounts*.
2. **Authentication › Add a platform › Mobile and desktop applications**, redirect URI
   `http://localhost`; also add `http://127.0.0.1` and `http://127.0.0.1/` (MailCraft listens
   on `http://127.0.0.1:<port>/`: Microsoft lets loopback redirects use any port, but the path
   has to match). Turn on **Allow public client flows**. No client secret.
3. **API permissions:** nothing has to be added. MailCraft asks for what it needs when someone
   signs in (IMAP and SMTP access at `outlook.office.com` with `offline_access` for an IMAP
   account; Graph's `Mail.ReadWrite`, `Mail.Send`, `MailboxSettings.ReadWrite`,
   `Calendars.ReadWrite`, `Calendars.Read.Shared` and `Contacts.ReadWrite` for Exchange Online
   through Graph), and the person approves it on Microsoft's page. Listing them here as
   delegated permissions only shows them in advance.
4. Copy the **Application (client) ID** into the repository secret
   `MAILCRAFT_MICROSOFT_CLIENT_ID` (Settings › Secrets and variables › Actions), or set it in
   the environment before starting MailCraft to try it.

Personal accounts (Outlook.com, Hotmail, Live) can approve an unverified app; work and school
accounts often can't until the publisher is verified (a Microsoft AI Cloud Partner Program ID
under **Branding & properties**), or their administrator approves MailCraft for the organisation.

A personal Microsoft account made with another provider's address (a Gmail address, say) has a
hidden mailbox address (`outlook_…@outlook.com`): mail sent through Graph goes out from it, and
SMTP refuses the sign-in (535 5.7.3) even though it covers sending. An Outlook.com address added
to the account and made its **primary alias** (account.live.com › Manage how you sign in) fixes
both; then the account is added once, with that address.

Sign in once with a test account: if Microsoft says the redirect URI doesn't match
(`AADSTS50011`), check the two loopback addresses in step 2.

## When an Outlook.com or Hotmail account doesn't connect

- **"Microsoft doesn't accept passwords from mail apps":** the account was added with a password
  (an older MailCraft, or a password typed in). Click **Sign in with Microsoft** in the status bar
  (or on the account's right-click menu): the browser opens, and the account keeps its mail.
  MailCraft no longer offers a password for these addresses when it can sign in with Microsoft.
- **"Microsoft signed you in, but its mail server won't open this mailbox over IMAP"** (the
  server's words: *User is authenticated but not connected*): the token is good, but IMAP won't
  open the mailbox. Either IMAP is switched off for it (Outlook.com › Settings › Mail ›
  Forwarding and IMAP › *Let devices and apps use IMAP*, off by default on new accounts; for a
  work account, the organisation's administrator), or the browser signed in as **another
  account** than the one added (a browser already signed in to Microsoft picks that one). MailCraft
  asks Microsoft who signed in, tries that address once, and otherwise names it in the message.
  Turn IMAP on, or sign in again with the right account, then choose Sync; or **Use Microsoft
  Graph**, which reaches the mailbox without IMAP (and brings its calendar and contacts): the
  account starts again through Graph.

