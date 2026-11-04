# Security policy

## Reporting a vulnerability

Please report security problems privately through GitHub:
**[Report a vulnerability](https://github.com/ZIDAN44/mailcraft/security/advisories/new)**
(Security tab › Advisories › Report a vulnerability).

Don't open a public issue or pull request for a vulnerability until a fix is released.

Useful to include:
- the MailCraft version (File › About) and the platform (Windows, macOS, Linux, FreeBSD, web);
- what an attacker can do, and what they need (a message sent to you, a mail server you connect
  to, a data file you open, a local account…);
- steps or a sample message or file that reproduce it.

We'll acknowledge the report, keep you updated while we work on a fix, and credit you in the
advisory unless you'd rather not be named.

## Supported versions

Fixes go into `main` and ship in the next test build and release, which installed copies update
to by themselves. Older versions don't get backported fixes.

## Scope

In scope: anything that lets a message, an attachment, a mail or calendar server's answer, a data
file (`.pst`, `.ost`, `.msg`, `.eml`, `.ics`, `.vcf`), a link or a command:
- read or write files it shouldn't, or reach a computer you didn't choose (remote pictures,
  links in data files);
- run code, or show a message, signature or encryption as something it isn't;
- leak mail, passwords, keys or what's being written;
- crash MailCraft or lose what you were writing.

Crashes from hostile input count, because MailCraft treats every panic as a bug (see
[Never crash](CLAUDE.md)).
