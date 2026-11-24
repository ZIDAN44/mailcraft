# Microsoft and Google sign-in

A build of MailCraft offers **Sign in with Microsoft** and **Sign in with Google** only when it
carries a registered app. This page is for whoever builds or ships MailCraft, for users adding a
Gmail or Google Workspace account, for users whose Outlook.com account won't connect, and for
organisations whose work or school accounts need MailCraft approved.

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
3. Leave **API permissions** empty. The user approves the permissions as they sign in, each
   sign-in asking for its own:
   - An account over IMAP: `IMAP.AccessAsUser.All` and `SMTP.Send` at `outlook.office.com`.
   - An account through Microsoft Graph: `Mail.ReadWrite`, `Mail.Send`,
     `MailboxSettings.ReadWrite`, `Calendars.ReadWrite`, `Calendars.ReadWrite.Shared` and
     `Contacts.ReadWrite`.
   - Searching a work or school directory, asked the first time someone allows it:
     `User.ReadBasic.All` and `People.Read`.

   Each also asks for `offline_access`, so the sign-in lasts. A sign-in in the browser also
   asks for `openid` and `email`, so MailCraft can tell which account signed in.
4. Put the client ID in `MAILCRAFT_MICROSOFT_CLIENT_ID`.

Two kinds of account need more:

- **Work and school accounts:** many organisations let people approve only apps from a
  verified publisher. Their people see **Need admin approval** instead of the permissions. The
  app registered above has no verified publisher until its owner verifies one through
  Microsoft's partner programme. Until then, see
  [Your organisation needs to approve MailCraft](#your-organisation-needs-to-approve-mailcraft).
- **Personal accounts on another provider's address** (for example Gmail) have a hidden
  `outlook_…@outlook.com` mailbox, and SMTP refuses it with `535 5.7.3`. Make an Outlook.com
  address the account's **primary alias** at account.live.com, then add the account with that
  address.

### An organisation's own app

An organisation can register MailCraft in its own directory and give that app to its people
through its [profile](organisations.md) (`accounts.microsoft`). Its accounts then sign in through
that app, whichever build of MailCraft they use. Once an administrator has granted its
permissions for everyone, no one is asked to approve it:

1. Register the app as above, but choose accounts in this organizational directory only.
2. Under **API permissions**, add the Microsoft Graph and Office 365 Exchange Online permissions
   listed above that your accounts need, then choose **Grant admin consent**.
3. In the profile, give the app's client ID as `clientId`, and the directory's ID or one of its
   domains as `tenant`.

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
open **Server settings**: `imap.gmail.com`, 993, SSL/TLS, and `smtp.gmail.com`, 465, SSL/TLS, with
your full address as the user name.

If Google shows no App passwords page, 2-Step Verification is off, the account is in Advanced
Protection, or (Google Workspace) your administrator hasn't allowed app passwords. A Workspace
administrator can also have IMAP turned off for the organisation.

### Why the released builds have no Google sign-in

MailCraft asks Google for full access to Gmail (`https://mail.google.com/`), which IMAP and SMTP
need, and for Google Calendar (`https://www.googleapis.com/auth/calendar`) and Google Contacts
(`https://www.googleapis.com/auth/carddav`), which CalDAV and CardDAV need. Google treats Gmail's
as a restricted scope and the other two as sensitive ones:

- An app Google hasn't verified shows a warning before sign-in and is limited to 100 users over
  the life of its Google project. After that, Google blocks new sign-ins.
- Verification for the restricted scope includes a security assessment by an outside lab,
  repeated every year. MailCraft hasn't been through it.

### Sign in with Google through a client of your own

A build, or a running MailCraft, that is given a Google client offers **Sign in with Google**.
This suits a Google Workspace organisation: a client its administrator creates as **Internal**
has no warning and no user limit, and works for that organisation's accounts only.

1. In Google Cloud Console, create a project, and under **Google Auth Platform › Audience** choose
   **Internal**.
2. Under **Clients**, create an OAuth client of the type **Desktop app**.
3. Set `MAILCRAFT_GOOGLE_CLIENT_ID` and `MAILCRAFT_GOOGLE_CLIENT_SECRET` to its ID and secret
   before MailCraft starts.
4. To sync calendars and contacts, enable the **CalDAV API** (`caldav.googleapis.com`) and
   **CardDAV API** (`carddav.googleapis.com`) for the project. Without them Google answers HTTP
   403 `accessNotConfigured`, and MailCraft shows Google's own message.

Accounts signed in with Google sync their calendars and contacts over CalDAV and CardDAV with the
OAuth token, which goes to Google's servers alone. An account signed in before MailCraft asked for
these permissions, or that granted only some of them, is told so, and signing in again grants
them; its calendars and contacts are then looked for at once.

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

## Your organisation needs to approve MailCraft

A work or school account may show **Need admin approval** at Microsoft's sign-in: the
organisation lets people approve only apps it trusts. MailCraft can't get past this. Ask your IT
department for one of these:

- **Approve MailCraft for everyone.** An administrator chooses **Have an admin account? Sign in
  with that account** on that page, and approves for the whole organisation. That approves what
  this sign-in asked for. Searching the directory has a sign-in of its own, the first time
  someone chooses **Allow searching the directory…**, and is approved the same way, once.
- **Use the organisation's own app**, given to everyone in its profile: see
  [An organisation's own app](#an-organisations-own-app).

Exchange Online can also have IMAP or authenticated SMTP turned off for the organisation. An
account through Microsoft Graph needs neither: `graph` among the domain's servers in the
profile, or **Use Microsoft Graph** on the account.
