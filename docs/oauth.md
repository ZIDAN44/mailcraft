# Microsoft and Google sign-in

A build of MailCraft offers **Sign in with Microsoft** and **Sign in with Google** only when it
carries a registered app. This page is for whoever builds or ships MailCraft, for users adding a
Gmail or Google Workspace account, and for users whose Outlook.com account won't connect.

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
- `release.yml` passes the repository secrets with these names to the build.
- `release-artcraft.yml` (the ArtCraft suite's shared release, unused) would need a step added
  to pass them through.
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

## Gmail and Google Workspace

The MailCraft released from this repository has no **Sign in with Google**: its builds carry no
Google client, so they never open Google's sign-in page. A Gmail or Google Workspace account is
added with an app password.

### Add the account with an app password

1. In your Google account, turn on **2-Step Verification** (myaccount.google.com › Security).
2. Open [App passwords](https://myaccount.google.com/apppasswords), name one "MailCraft" and
   create it. Google shows its 16 letters once.
3. In MailCraft, add the account with your address and that app password, not your usual one.

MailCraft fills in Gmail's servers for a `gmail.com` address. For an address on your own domain,
tick **Server settings**: `imap.gmail.com`, 993, SSL/TLS, and `smtp.gmail.com`, 465, SSL/TLS, with
your full address as the user name.

If Google shows no App passwords page, 2-Step Verification is off, the account is in Advanced
Protection, or (Google Workspace) your administrator hasn't allowed app passwords. A Workspace
administrator can also have IMAP turned off for the organisation.

### Why the released builds have no Google sign-in

MailCraft asks Google for full access to Gmail (`https://mail.google.com/`), which IMAP and SMTP
need. Google treats that as a restricted scope:

- An app Google hasn't verified shows a warning before sign-in and is limited to 100 users over
  the life of its Google project. After that, Google blocks new sign-ins.
- Verification for this scope includes a security assessment by an outside lab, repeated every
  year. MailCraft hasn't been through it.

### Sign in with Google through a client of your own

A build, or a running MailCraft, that is given a Google client offers **Sign in with Google**.
This suits a Google Workspace organisation: a client its administrator creates as **Internal**
has no warning and no user limit, and works for that organisation's accounts only.

1. In Google Cloud Console, create a project, and under **Google Auth Platform › Audience** choose
   **Internal**.
2. Under **Clients**, create an OAuth client of the type **Desktop app**.
3. Set `MAILCRAFT_GOOGLE_CLIENT_ID` and `MAILCRAFT_GOOGLE_CLIENT_SECRET` to its ID and secret
   before MailCraft starts.

A client that is **External** and not verified works too, with Google's warning (choose
**Advanced**, then **Go to MailCraft**) and its limit of 100 users.

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
