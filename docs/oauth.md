# Signing in through the browser

A build of MailCraft offers **Sign in with Microsoft** and **Sign in with Google** only when it
carries a registered app. This page is for whoever builds or ships MailCraft, for users adding a
Gmail or Google Workspace account, for users whose Outlook.com account won't connect, and for
the administrator of a single sign-on service that a JMAP mail server signs its users in with.

Microsoft, Google and single sign-on services ask mail apps to sign in through the browser.
MailCraft does this the way RFC 8252 describes for native apps, with PKCE (RFC 7636): a one-time
code that stops another program from taking over the sign-in.

## What a build needs

| Variable | What it holds |
|---|---|
| `MAILCRAFT_MICROSOFT_CLIENT_ID` | The Application (client) ID of a Microsoft Entra app |
| `MAILCRAFT_GOOGLE_CLIENT_ID` | The client ID of a Google "Desktop app" client |
| `MAILCRAFT_GOOGLE_CLIENT_SECRET` | That client's secret, which isn't secret for installed apps |
| `MAILCRAFT_OIDC_CLIENTS` | Client IDs for single sign-on services, by service ([below](#a-jmap-servers-single-sign-on)) |

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

## A JMAP server's single sign-on

Some JMAP servers take no passwords: their users sign in through the browser, with the
organisation's single sign-on service (OpenID Connect). Such a server answers a request without
a sign-in with `401` and `WWW-Authenticate: Bearer`.

- **Adding the account.** MailCraft asks the server how it signs you in, then finds its service
  from the server's `/.well-known/openid-configuration`, following redirects. The service must be
  on https and must name itself as the address it was found at. MailCraft then offers **Sign in
  through the browser**. If the server couldn't be asked (it wasn't reached, or it failed),
  MailCraft says why, and **Try again** asks again.
- **An account that has a password** moves to the single sign-on under **Server settings ›
  Sign-in**, and back the same way.
- **An account moved to another server** (its **JMAP session address**) signs in through the
  service that server names: in the browser again if it's another service.
- **What MailCraft keeps.** The browser opens the service's sign-in page. MailCraft keeps no
  password, only the service's refresh token, in the system keychain. It sends that token only
  to the service the account names, and refuses a description there that names another.
- **In the desktop app only.** The browser app can't sign in this way
  ([gap 8](gaps.md#8-single-sign-on-in-the-browser)).

### Register MailCraft with the service

The service's administrator registers MailCraft once, as one client for all its users:

- **A public client**, with no secret. A secret inside a program anyone can download protects
  nothing.
- **PKCE with S256**, required. MailCraft sends nothing else, and refuses a service whose
  description lists PKCE methods without `S256`.
- **The redirect URI `http://127.0.0.1`, on any port.** For each sign-in, MailCraft listens on a
  port the system gives it, and the browser is sent back to `http://127.0.0.1:<port>/`
  (RFC 8252 §7.3). A service that compares redirect URIs exactly must allow any port for this
  one.
- **The scopes `openid profile email offline_access`.** MailCraft asks for `openid profile
  email`, or for the scopes the mail server names in its `WWW-Authenticate` challenge, and always
  adds `offline_access` (OIDC Core §11). Some services give a refresh token only when it's asked
  for, even though their `scopes_supported` doesn't list it. Allow it for this client.
- **Refresh tokens allowed.** MailCraft keeps the sign-in with the refresh token. If the service
  gives none, signing in fails: MailCraft says the service "gave no lasting sign-in".
- **Tokens the mail server takes** for this client: the server checks the access token on every
  request.

A test service on this computer, at a loopback address such as `http://127.0.0.1:8080`, can use
plain http. Every other service must use https.

### Give MailCraft the client ID

Users type it in **Client ID**, which MailCraft shows when it has none for the service: in **Add
account**, or in the **Server settings** of an account that needs one.

Or the copy of MailCraft your users install can carry it, in `MAILCRAFT_OIDC_CLIENTS`:

```sh
MAILCRAFT_OIDC_CLIENTS="https://sso.example.com=mailcraft;https://kc.example.org/realms/mail=mc-desktop"
```

- Each pair is the service's issuer, then `=`, then the client ID. Pairs are separated by `;`,
  and spaces around them don't count.
- The issuer is the one the service's description names, written the same way except for a
  trailing `/`.
- MailCraft reads the variable when it starts. If it's set when MailCraft is built, the build
  keeps it. For the same issuer, a value set at start comes before the built one.
- An account doesn't keep the client ID MailCraft was given: it's looked up each time the account
  signs in, so a new one given with MailCraft is the one it signs in under. A client ID typed for
  an account is that account's own.
