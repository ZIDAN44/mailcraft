# Microsoft and Google sign-in

A build of MailCraft offers **Sign in with Microsoft** and **Sign in with Google** only when it
carries a registered app. This page is for whoever builds or ships MailCraft, and for users whose
Outlook.com account won't connect.

Microsoft and Google ask mail apps to sign in through the browser. MailCraft does this the way
RFC 8252 describes for native apps, with PKCE (RFC 7636): a one-time code that stops another
program from taking over the sign-in.

## What a build needs

| Variable | What it holds |
|---|---|
| `MAILCRAFT_MICROSOFT_CLIENT_ID` | The Application (client) ID of a Microsoft Entra app |
| `MAILCRAFT_GOOGLE_CLIENT_ID` | The client ID of a Google "Desktop app" client |
| `MAILCRAFT_GOOGLE_CLIENT_SECRET` | That client's secret, which isn't secret for installed apps |

- MailCraft reads these variables when it starts. If they are set when you build it, the build
  keeps them.
- `release-wip.yml` passes the repository secrets with these names to the build.
- `release.yml` uses the shared `storytold/craft-common-actions` workflow, which needs a step
  added to pass them through.
- Without them, only passwords work, so an Outlook.com account can't be added.

## Register the app with Microsoft

1. In **App registrations › New registration**, choose accounts in any organizational directory
   and personal Microsoft accounts.
2. In **Authentication › Mobile and desktop applications**:
   - Add the redirect URIs `http://localhost`, `http://127.0.0.1` and `http://127.0.0.1/`.
   - Turn on **Allow public client flows**.
   - Don't create a secret.

   If sign-in shows `AADSTS50011`, these redirect URIs are wrong.
3. Leave **API permissions** empty. The user approves the scopes when signing in:
   - IMAP and SMTP at `outlook.office.com`, with `offline_access`.
   - For Microsoft Graph: `Mail.ReadWrite`, `Mail.Send`, `MailboxSettings.ReadWrite`,
     `Calendars.ReadWrite`, `Calendars.Read.Shared` and `Contacts.ReadWrite`.
4. Put the client ID in `MAILCRAFT_MICROSOFT_CLIENT_ID`.

Two kinds of account need more:

- **Work and school accounts** often need a verified publisher, or an administrator's approval.
- **Personal accounts on another provider's address** (for example Gmail) have a hidden
  `outlook_…@outlook.com` mailbox, and SMTP refuses it with `535 5.7.3`. Make an Outlook.com
  address the account's **primary alias** at account.live.com, then add the account with that
  address.

## Google says MailCraft isn't verified

When you **Sign in with Google**, Google may first warn that it hasn't verified MailCraft. This
is expected. MailCraft asks for full access to Gmail, which IMAP and SMTP need, and
Google shows this warning for every app with that access that hasn't been through its review.
That review includes a security assessment by an outside lab, repeated every year, and MailCraft
hasn't been through it.

- **To go on:** choose **Advanced**, then **Go to MailCraft**, and approve access to your mail.
- **Google Workspace, if your company blocks unverified apps:** an administrator can allow
  MailCraft under **Security › Access and data control › API controls › Manage Third-Party App
  Access**: **Configure new app**, find MailCraft by name or client ID, and set it to **Trusted**.
- **Without Google sign-in:** create an [app password](https://support.google.com/accounts/answer/185833)
  (it needs 2-Step Verification, and on Workspace your administrator must allow it), then add the
  account with that password. MailCraft connects over IMAP and SMTP, and Google shows no warning.

## Fix an Outlook.com account that won't connect

### "Microsoft doesn't accept passwords from mail apps"

Click **Sign in with Microsoft** in the status bar or the account menu. Your mail stays where it is.

### "…won't open this mailbox over IMAP"

IMAP is turned off for the account, or the browser signed in as a different account.

1. Turn IMAP on in Outlook.com under **Settings › Mail › Forwarding and IMAP**. New accounts start
   with it off.
2. If the browser used another account, sign in again with the right one.
3. Sync again.

Or choose **Use Microsoft Graph** instead. It needs no IMAP, and it adds your calendar and
contacts.
