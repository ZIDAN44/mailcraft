# MailCraft's MCP server

`mailcraft-cli mcp` lets an AI assistant on this computer (Claude Desktop, Claude Code, any
[Model Context Protocol](https://modelcontextprotocol.io) client) read your mail, calendar, tasks
and people, and, with MailCraft open, run its commands once you allow each in MailCraft's window.
It speaks newline-delimited JSON-RPC 2.0 on stdin and stdout (protocol `2025-06-18`; `2025-03-26`
and `2024-11-05` clients are answered in theirs); its own messages go to stderr.
It comes with MailCraft 0.42.0: an older `mailcraft-cli` has no `mcp`.

## Turning it on

Nothing reaches MailCraft until you let agents in: **File › Options › AI agents › Let AI agents use
MailCraft**. While it's on, MailCraft listens on a loopback port of its own (never the network),
behind a token made afresh each time in a file only you can read, and says where in its data
folder (`agent-connection.json`: the port, its process id and the token file; no secret). Turned
off, or as MailCraft closes, it stops listening and those files go.

Agents then read. To let them ask to run commands too, turn on **Let AI agents ask to run
commands** beside it. With several accounts, **Accounts AI agents read** has a switch for each:
an account turned off there is left out of everything agents read (its mail, folders, events,
tasks, the people only it knows, its part of everything with someone), `inspect` leaves out what
the window shows of it, and no `screenshot` is taken while one is kept (the window could show
it). No command of theirs runs that names it, or while the window shows its mail (its folder, All
Inboxes, a message of it open), and while one is kept they aren't given what a command returns
(a command can read too), only that it ran.

## Connecting an assistant

Claude Code:

```sh
claude mcp add mailcraft -- mailcraft-cli mcp
```

Claude Desktop and other clients (`mcpServers` in their settings):

```json
{
  "mcpServers": {
    "mailcraft": { "command": "mailcraft-cli", "args": ["mcp"] }
  }
}
```

The client's settings hold no port and no token: before each tool call, `mailcraft-cli mcp`
looks for the MailCraft that has the same data folder open and lets agents in. For another data
folder (`--data` when MailCraft was started with one): `"args": ["--data", "D:\\Mail", "mcp"]`.
A portable copy's `mailcraft-cli` (beside `portable.txt`) takes the `MailCraftData` folder beside
it, as the MailCraft beside it does: give its full path as the command.

## Where a call goes

| MailCraft | What the tools do |
|---|---|
| Open, agents let in | Everything below, on the mail as the window has it. A command waits for your answer in the window. |
| Open, agents not let in | Nothing: the data folder is MailCraft's, and the tools say to let agents in. |
| Closed, agents let in | The reading tools read the data folder here, with your settings (the accounts kept from agents), each call on its own: the folder is let go of between calls, so MailCraft opens as usual and takes the next one. Nothing runs: MailCraft isn't started for an agent. |
| Closed, agents not let in | Nothing. |

## Tools

| Tool | Arguments | Does |
|---|---|---|
| `list_accounts` | | The accounts and their folders: paths, names, roles, how many messages and unread ones. |
| `list_messages` | `account?`, `folder?`, `limit?`, `offset?` | A folder's messages, newest first (the account's Inbox by default): key, date, sender, recipients, subject, the start of the text, unread, flagged, files. |
| `search_mail` | `query`, `account?`, `folder?`, `limit?` | As the search box finds them (`from:`, `subject:`, `has:attachment`, `is:unread`, `received:`, `before:`/`after:`, `category:`, `-word`, `"a phrase"`, words in the text of the messages downloaded). |
| `read_message` | `key` | One message: sender, recipients, date, subject, its text (HTML as text), its files. |
| `list_events` | `from?`, `to?` (`YYYY-MM-DD`) | The calendars' events, each occurrence on its own: times, place, the organizer, the guests and their answers, your answer. Today and the six days after by default; a year at most. |
| `list_tasks` | `query?`, `done?` | The tasks: title, due day, list, notes. |
| `find_people` | `query` | People from the address books, and from the mail and meetings with them. |
| `everything_with` | `email`, `query?` | Everything with one person: the conversations, the files and links in them, the meetings, the others on that mail. |
| `list_commands` | `search?` | MailCraft's commands: id, label, where they are, their params, whether they can run now (and why not), whether agents run them without asking, and whether they take `keys`. |
| `execute_command` | `command`, `params?` | Runs one of them in the window, once you allow it (with agents let ask to run commands). A command that acts on the selected messages (`mail.delete`, `mail.move`, `mail.flag`…) takes `"keys"`, the messages' keys in the list shown: they're selected first, as clicks would. One that asks a server (`rules.server`, `account.vacation`) answers with what the server says, a minute at most. |
| `inspect` | | What the window shows: the module, folder, search, selection, the message open or being written, a dialog, the commands waiting for you. |
| `screenshot` | | A picture of the window (PNG). |

The reading tools change nothing: a message read isn't marked read, the window's folder, search
and reading pane stay as they are, and nothing is asked of a server (an organisation's directory
included: `find_people` gives what it answered before). `list_commands`,
`execute_command`, `inspect` and `screenshot` need MailCraft open with agents let in.

## You decide

Each command an agent wants to run is shown in a small window of its own in MailCraft, above the
rest (a dialog you're filling in stays), MailCraft brought forward and a notification shown as it
comes up: what it is and what it names in words (the event and its day, the message, whom to,
the account), its params as sent under **Details**, then **Allow**, **Always allow** (that
command, from then on), or **Deny**. The call waits for your answer, ten minutes at
most (then nothing runs), and the agent's other calls wait with it; Deny is the agent's answer.
A command that may no longer run by the time you answer (agents not let ask any more, an account
kept meanwhile) isn't run, nor one the agent took back. **File › Options › AI agents** lists the commands agents run
without asking, each with **Forget**; agents can't change these settings themselves.
**File › Info › What AI agents did** lists what they read and ran lately (pictures of the window
and `inspect` too) and what came of it.

## The control channel

What `mailcraft-cli mcp` sends the window, for another client: the Craft apps' control protocol
(`craft-control`), JSON lines on the port in `agent-connection.json`: each request
`{"id": 1, "method": "…", "params": {…}}`, the first on a connection with `"token"` beside them,
the token from the file it names. Each gets one reply, `{"ok": true, "result": …}` or
`{"ok": false, "error": "…"}`:

| Method | Params | |
|---|---|---|
| `agents.read` | `{tool, args}` | A reading tool above. |
| `engine.commands` | `{search?}` | As `list_commands`. |
| `engine.execute` | `{command, params}` | As `execute_command`: answered once you decide. |
| `ui.inspect` | | As `inspect`. |
| `ui.screenshot` | | `{png, width, height}`, the picture base64. |
