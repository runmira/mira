# Mira in Slack

Mention Mira in a channel, or send it a direct message, and it works on
your project: reads code, runs commands, edits files. Each Slack thread
is one Mira session, so replies in the thread continue the conversation.

The bot runs on your machine (or any machine with the repo) with
`mira slack`. It connects to Slack over Socket Mode, so there's no
public URL, webhook or server to set up.

## Set it up (about 3 minutes)

1. **Create the Slack app.** Go to <https://api.slack.com/apps>, click
   **Create New App → From a manifest**, pick your workspace, and paste
   the manifest below.
2. **Install it.** On the app's **Install App** page, click **Install to
   Workspace**. Copy the **Bot User OAuth Token** (`xoxb-…`).
3. **Make an app token.** On **Basic Information → App-Level Tokens**,
   click **Generate Token and Scopes**, add the `connections:write` scope,
   and copy the token (`xapp-…`).
4. **Give both tokens to Mira.** In the web app, open **Settings → Search &
   keys** and paste them into **Slack bot token** and **Slack app token**.
   (Or set `SLACK_BOT_TOKEN` and `SLACK_APP_TOKEN` in your shell.)
5. **Start the bot** in your project folder:

   ```bash
   mira slack
   ```

6. Invite the bot to a channel (`/invite @Mira`) and mention it:
   `@Mira why does the login test fail?`

### App manifest

```yaml
display_information:
  name: Mira
  description: Open-source coding agent
features:
  bot_user:
    display_name: Mira
    always_online: true
  app_home:
    messages_tab_enabled: true
    messages_tab_read_only_enabled: false
oauth_config:
  scopes:
    bot:
      - app_mentions:read
      - chat:write
      - channels:history
      - groups:history
      - im:history
      - im:read
      - im:write
settings:
  event_subscriptions:
    bot_events:
      - app_mention
      - message.channels
      - message.groups
      - message.im
  interactivity:
    is_enabled: true
  socket_mode_enabled: true
```

## Using it

- **Start:** mention the bot in a channel, or DM it.
- **Continue:** reply in the thread. No need to mention it again.
- **Stop:** reply `stop` in the thread to cancel what it's doing.
- **Approvals:** in `manual` mode (the default), Mira asks before
  editing files or running commands. It posts **Allow** / **Deny**
  buttons in the thread; an unanswered request is denied after 10
  minutes. Start with `mira --mode auto slack` (or set `default_mode` in
  Settings) to be asked less.

The answer is one message that updates as Mira works, with its recent
steps listed on top.

## Options

| Flag | What it does |
| --- | --- |
| `--cwd <dir>` | Work in this folder instead of the current one. |
| `--allow-users U123,U456` | Only these Slack users can use the bot or click its buttons. Find a user's ID in their Slack profile → ⋯ → **Copy member ID**. |

Global flags work too: `mira --model <id> --mode auto slack`.

Mira uses the provider, model, MCP servers, plugins and permission rules
you already set up. Keep the process running (a terminal, `tmux`, or a
service) for the bot to stay online.
