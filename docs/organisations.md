# MailCraft in an organisation

An IT department can give everyone the same MailCraft: its look, the settings it wants, the
servers of its mail domains, a signature and a notice under every message, and its policies.
Each setting is a **default** (people may change it) or **locked** (they can't). This is the
**organisation profile**, read as MailCraft starts. Without one, MailCraft is as it always is.

People see what it sets in **File › Info** (where it came from, what it locks, its rules, and
what of it couldn't be taken), locked settings greyed out with **Set by your organisation**, and
the organisation's name and logo in **File › About**, beside MailCraft's own. Through
`mailcraft-cli` or an AI agent, a locked setting's command still says what it is; a change is
refused.

## Where MailCraft reads it

In this order, a later one over an earlier one:

1. `policies.json` beside a portable copy (the folder `MailCraftData` is in): defaults only, since
   the person carrying it can change it.
2. `policies.json` only an administrator can write:

   | System | Place | Taken when |
   |---|---|---|
   | Windows | `%ProgramData%\MailCraft\policies.json` | Administrators, SYSTEM or TrustedInstaller own it and its folder, and no one else may change, replace or delete it. Every user may make files in `%ProgramData%`, so one they made is refused. |
   | macOS | `/Library/Application Support/MailCraft/policies.json` | root owns it and its folder, and neither is writable by anyone else (`sudo chown root:wheel`, mode 644). |
   | Linux, the BSDs | `/etc/mailcraft/policies.json` (FreeBSD: `/usr/local/etc/mailcraft/` too) | As on macOS. |

   A **Flatpak** sees the computer's `/etc` only once its administrator allows it, once:
   `flatpak override --system --filesystem=host-etc:ro io.github.zidan44.MailCraft`. Inside
   it, root and every other user show as one, so keep `/etc/mailcraft` root's alone.
   On Windows, the file's owner is read through PowerShell: where PowerShell is held to its
   constrained language (AppLocker, WDAC), the file can't be checked and is refused, so use
   Group Policy there.
3. **Windows:** Group Policy and Intune, `Software\Policies\MailCraft`, the user's then the
   computer's. `packaging/windows/policy/` has MailCraft's ADMX template and its English
   strings: copy `MailCraft.admx` into `PolicyDefinitions` and `en-US\MailCraft.adml` into its
   `en-US` (or the domain's Central Store). **macOS:** a configuration profile for
   `io.github.zidan44.MailCraft`, installed by an MDM (`packaging/macos/MailCraft.mobileconfig`
   is a sample), the user's then the computer's.

A policy in the user's own `Software\Policies` (HKCU) holds against everyone who can't change
their own registry; on a computer where they can, locks are best set for the computer (HKLM).

## The profile

JSON. Every part is optional; what can't be read is said in File › Info and left out, never a
reason to refuse the rest.

```json
{
  "organisation": "Contoso",
  "logo": "C:\\ProgramData\\MailCraft\\contoso.png",
  "contact": "IT help desk, extension 4000",
  "theme": {"value": "system", "locked": false},
  "language": "de",
  "updates": {"value": "off", "locked": true},
  "settings": {
    "accent": "#0f6cbd",
    "plainTopBar": {"value": true, "locked": true},
    "autoPictures": {"value": false, "locked": true},
    "answeredInvites": "delete",
    "agentsRun": {"value": false, "locked": true}
  },
  "requireEncryption": true,
  "certificateExceptions": false,
  "accounts": {
    "onlyOrganisationDomains": true,
    "domains": ["contoso.example"],
    "servers": {
      "contoso.example": {
        "imap": "mail.contoso.example:993:tls",
        "smtp": "mail.contoso.example:587:starttls",
        "caldav": "https://mail.contoso.example/SOGo/dav/",
        "carddav": "https://mail.contoso.example/SOGo/dav/"
      }
    },
    "autoconfig": "https://config.contoso.example/mail/config-v1.1.xml",
    "lookups": {"thunderbird": false, "mx": false, "dns": true, "domain": true},
    "microsoft": {"clientId": "00000000-0000-0000-0000-000000000000", "tenant": "contoso.example"}
  },
  "signature": {
    "name": "Contoso",
    "text": "{name}\n{title}, {department}\nPhone {phone}",
    "html": "<p><b>{name}</b><br>{title}, {department}</p>",
    "locked": true,
    "newMessages": true,
    "replies": true
  },
  "disclaimer": {"text": "This message may be confidential.", "html": "<p><small>This message may be confidential.</small></p>"},
  "profiles": [
    {"domains": ["fabrikam.example"], "organisation": "Fabrikam", "signature": {"text": "{name}, Fabrikam"}}
  ]
}
```

| Key | What it sets |
|---|---|
| `organisation`, `logo`, `contact` | The name and logo (PNG, JPEG, GIF, WebP or BMP, 2 MB at most) in File › About, and who to ask in File › Info. |
| `theme` | `light`, `dark` or `system`. |
| `language` | The interface language's code (`de`, `pt-br`…), or `auto`. |
| `updates` | `on` (as the copy was installed), `stable` (releases only) or `off`. Locked off, File › About doesn't check either. |
| `settings` | Any of MailCraft's settings, by its key in `ui.json`'s `prefs` (`accent`, `plainTopBar`, `compact`, `readingPane`, `autoPictures`, `answeredInvites`, `undoSendSecs`, `agents`, `agentsRun`…): its value, a default, or `{"value": …, "locked": true}`. What a person keeps for themselves (folders open, panes folded, searches lately) isn't set. |
| `requireEncryption` | `true`: no connection without encryption (IMAP, POP, SMTP, Exchange, JMAP, calendars and contacts). An account that has one is held back and says why; none can be set up or changed to one. |
| `certificateExceptions` | `false`: no certificate trusted by hand (by its fingerprint, or from a file), and those trusted so before aren't used. An organisation's own authority goes into the computer's trust store, which MailCraft uses. |
| `accounts.servers` | Each mail domain's servers: `imap`, `pop`, `smtp` (`host:port:tls|starttls|none`, or `{"host", "port", "security"}`; port and security may be left out: security then follows the port, STARTTLS on 587, 143 and 110, TLS on any other), `caldav`, `carddav`, `ews`, `jmap` (https addresses), `graph` (`true`: Exchange Online through Microsoft Graph). An address of the domain and a password are then all an account needs. Server rules (ManageSieve) are on the mail server itself, as always. |
| `accounts.onlyOrganisationDomains` | `true`: accounts of the organisation's domains only (`accounts.domains`, the profiles' `domains`, and those `servers` names). |
| `accounts.autoconfig` | An autoconfig file asked before anything else, for any address. |
| `accounts.lookups` | Each way an address's servers are looked for, on unless `false`: `domain` (its own autoconfig files), `thunderbird` (Thunderbird's provider database), `mx` (its mail exchanger's provider), `dns` (service records). |
| `accounts.microsoft` | The organisation's own Microsoft 365 app registration: `clientId`, and `tenant` (its directory's id or domain; without it, any work or school account). It's for the accounts of the organisation's domains (a domain's profile's for its own; the others: `accounts.domains`, those it names servers for, and `tenant` when it's a domain); any other account, a personal one among them, signs in as it always did. An account of those domains signed in before signs in once more. Registering it, and granting its permissions for everyone: [An organisation's own app](oauth.md#an-organisations-own-app). |
| `signature` | A template: `{name}`, `{title}`, `{department}`, `{phone}`, `{mobile}`, `{office}`, `{company}`, `{email}`. People give their details in File › Info (Exchange and Microsoft 365 fill in name, title and department from the directory, those not given, once the account is online; on Microsoft 365, once the person has allowed searching the directory, which the Address Book offers); in text, a line whose details are missing goes. `text` and `html` (either), `name`, `locked` (it can't be changed or removed, nor another made the default), `newMessages` and `replies` (it's the default for them; `true` unless said). Unlocked, it's put in once, and again only when the profile's changes. |
| `disclaimer` | A notice under every message sent from the domain's accounts, meeting invitations and answers included (`text`, and `html` for formatted mail), added as it's sent: before it's signed or encrypted, and never in the message being written, where it could be taken out. A reaction, which is its emoji alone, goes without it. |
| `profiles` | More institutions in one file, each with `domains` (and one `default`), and any key above but `profiles`. |

### Several institutions

An account follows its domain's profile (else the top of the file): its servers, signature and
disclaimer. What is the window's alone follows the top, then the profile marked `default`. A
setting locked in the profile of any of the person's domains stays locked; of the policies
(`updates`, `requireEncryption`, `certificateExceptions`, `onlyOrganisationDomains`, `lookups`)
the strictest wins.

## The named values

Group Policy, and a macOS configuration profile, can set the profile whole (`Profile`, its JSON;
or `ProfileFile`, a file's full path) and these, each over it:

| Name | Type | |
|---|---|---|
| `Profile` | text (lines) | The profile's JSON. |
| `ProfileFile` | text | The full path of a profile file (only administrators should be able to change it). |
| `Organisation`, `Contact` | text | The organisation's name, and who to ask. |
| `Updates` | text | `off`, `stable` or `on`, locked. |
| `RequireEncryption` | number (1) | As `requireEncryption`. |
| `CertificateExceptions` | number (0) | As `certificateExceptions: false`. |
| `WebPictures` | number (0) | Pictures from the web never load by themselves, for no sender: locked. |
| `AIAgents` | text | `off`, `read` (read only), `ask` (each command asked for, none let run without asking): locked. |

A macOS configuration profile may also carry the profile's own keys (`settings`, `accounts`,
`signature`…) as keys of its own, as the sample does.

## What it can't do

- Locks hold in MailCraft on this computer. Someone with an administrator's rights, a portable
  copy, or another mail program isn't bound by them; what must hold everywhere (a disclaimer, no
  mail leaving unencrypted) belongs on the mail server too.
- A signature in the message being written can be edited there, as any text; the disclaimer,
  added as the message is sent, can't.
- No password, and no certificate, is ever set by a profile.
