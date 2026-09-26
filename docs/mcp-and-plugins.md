# MCP servers and plugins

Mira connects to [Model Context Protocol](https://modelcontextprotocol.io)
servers and installs plugins in Claude Code's format. The two work
together: a plugin can ship MCP servers, and every MCP server's tools go
through Mira's permission rules like the built-in ones.

You can do everything below from the **Plugins** page in the web UI
(`mira serve`) or from the terminal.

## MCP servers

### Add a server

```bash
# A local program, talking over stdio
mira mcp add github -- npx -y @modelcontextprotocol/server-github

# A remote server, over HTTP (use --transport sse for older servers)
mira mcp add --transport http linear https://mcp.linear.app/mcp

# Environment variables and headers
mira mcp add -e API_KEY=... myserver -- ./server
mira mcp add -H "Authorization: Bearer ..." api https://example.com/mcp

# Paste a definition in `.mcp.json` shape
mira mcp add-json docs '{"type":"http","url":"https://mcp.context7.com/mcp"}'
```

`mira mcp list` shows every server and whether it's connected;
`mira mcp get <name>` shows its tools, prompts and resources.

### Where servers live (scopes)

| Scope | Flag | Stored in | Who sees it |
| --- | --- | --- | --- |
| Local | `-s local` (default) | `~/.mira/mcp/state.json`, per project | You, in this project |
| Project | `-s project` | `.mcp.json` in the repo | Everyone who clones the repo |
| User | `-s user` | `mcp_servers:` in `~/.mira/mira.yaml` | You, in every project |
| Plugin | — | Inside an installed plugin | Wherever the plugin is on |

When two servers share a name, local beats project, and project beats user.

**Project servers need your approval first.** A cloned repo could ask for
any command, so Mira won't start servers from `.mcp.json` or
`.mira/config.yaml` until you run `mira mcp approve <name>` (or click
Approve on the Plugins page). `mira mcp reject <name>` refuses one.

### Sign in (OAuth)

Remote servers that use OAuth show **Sign in** on the Plugins page, or:

```bash
mira mcp login linear      # opens your browser
mira mcp logout linear
```

Mira registers itself with the server, runs the PKCE flow and keeps the
tokens in `~/.mira/mcp/credentials.json` (mode 0600), refreshing them
when they expire.

Some servers only accept sign-ins from apps they have approved in
advance. Figma's hosted server is one: sign-in fails with a 403 there.
See [Figma](#figma) below for what works instead.

### Tokens (`${VAR}` values)

Server definitions can use `${VAR}` for secrets, e.g. the GitHub
plugin's `Authorization: Bearer ${GITHUB_PERSONAL_ACCESS_TOKEN}`. A
server that needs a value you haven't set shows **needs setup** and an
**Add token** button. From the terminal:

```bash
mira mcp set-var GITHUB_PERSONAL_ACCESS_TOKEN    # prompts, nothing echoed
mira mcp set-var GITHUB_PERSONAL_ACCESS_TOKEN --unset
```

Values are saved in `~/.mira/mcp/variables.json` (mode 0600). A saved
value wins; a variable you haven't saved falls back to your shell
environment. `${VAR:-default}` works too.

### Turn servers and tools on or off

```bash
mira mcp disable github         # keep it configured, don't connect
mira mcp enable github
mira mcp tool disable mcp__github__delete_repository
mira mcp tool enable  mcp__github__delete_repository
```

MCP tools are named `mcp__<server>__<tool>`, and permission rules can
target a whole server, one tool, or a pattern:

```yaml
permissions:
  allow: ["mcp__context7"]                 # every context7 tool
  ask:   ["mcp__github__create_issue"]
  deny:  ["Mcp(github:delete_*)"]
```

MCP prompts appear in the slash menu as `/mcp__<server>__<prompt>`.

### Lots of tools: load them on demand

Every tool's description costs context on every turn. With many servers
connected, let the model find tools when it needs them:

```bash
mira mcp tool-loading on-demand   # or: all, auto
```

| Mode | What the model sees |
| --- | --- |
| `all` | Every enabled MCP tool, up front. |
| `on-demand` | Two tools: `search_mcp_tools` (find tools by keyword, with their schemas) and `call_mcp_tool` (call one by name). |
| `auto` (default) | `all` up to 30 MCP tools, `on-demand` above. |

Permission rules still apply to the real tool behind `call_mcp_tool`.
`MIRA_MCP_TOOL_LOADING` overrides the setting for one run.

## Plugins

Mira reads Claude Code plugins as they are: commands, agents, skills,
hooks and MCP servers.

### Marketplaces

A marketplace is a catalog of plugins, usually a GitHub repo with a
`.claude-plugin/marketplace.json`:

```bash
mira plugin marketplace add anthropics/claude-plugins-official
mira plugin marketplace add anthropics/claude-code
mira plugin marketplace add https://example.com/marketplace.json
mira plugin marketplace add ./my-marketplace       # a local folder
mira plugin marketplace list
mira plugin marketplace update                     # refresh all
```

### Install and manage

```bash
mira plugin browse                        # what your marketplaces offer
mira plugin install commit-commands       # or name@marketplace
mira plugin info commit-commands
mira plugin disable commit-commands
mira plugin update commit-commands
mira plugin uninstall commit-commands
```

Plugins install under `~/.mira/plugins/`. What each part becomes:

| In the plugin | In Mira |
| --- | --- |
| `commands/*.md` | Slash commands: `/<command>`, or `/<plugin>:<command>` |
| `agents/*.md` | Subagent types for the `agent` tool |
| `skills/*/SKILL.md` | Skills |
| `.mcp.json` | MCP servers (plugin scope) |
| `hooks/hooks.json` | Lifecycle hooks (below) |

## Hooks

Hooks run at points in a session: shell commands, or prompts a model
judges. They use Claude Code's format, so a plugin's `hooks/hooks.json`
works unchanged.

The easiest way to add your own is **Settings → Hooks** in the web app:
pick when (e.g. "Mira needs your approval", "Before Mira runs a shell
command") and what to do (show a notification, ask the AI to check, or
run a command). It also lists the hooks your plugins add. It writes the
same `hooks:` section you can edit by hand in `~/.mira/mira.yaml`:

```yaml
hooks:
  PreToolUse:
    - matcher: "Bash"
      hooks:
        - type: command
          command: "~/.mira/hooks/check-bash.sh"
          timeout: 30
  Stop:
    - hooks:
        - type: command
          command: "~/.mira/hooks/tests-pass.sh"
```

Hooks in a repo's `.mira/config.yaml` are ignored on purpose, so
cloning a repo can't run commands on your machine.

| Event | When | What a hook can do |
| --- | --- | --- |
| `SessionStart` | First message of a session | Add context |
| `UserPromptSubmit` | Each message you send | Add context, or block the message |
| `PreToolUse` | Before a tool runs | Allow, deny or ask; rewrite the input |
| `PostToolUse` | After a tool runs | Add feedback for the model |
| `Stop` | When the agent wants to stop | Make it keep going, with a reason |
| `SubagentStop` | When a subagent wants to stop | Make it keep going, with a reason |
| `Notification` | A tool call is waiting for your approval (matcher: `permission_prompt`) | Alert you (desktop notification, chat ping) |
| `PreCompact` | Before older history is summarized to free context (matcher: `auto`) | Save or log the transcript; it can't stop compaction |
| `SessionEnd` | The CLI is exiting (matcher and `reason`: `prompt_input_exit`) | Clean up or log; it can't block |

Subagents run your hooks too: `PreToolUse` and `PostToolUse` fire for
their tool calls, so a guard on `Bash` also guards what a subagent runs.
Their stop fires as `SubagentStop`; `SessionStart`, `UserPromptSubmit`
and `SessionEnd` only fire for your own session.

A hook gets a JSON object on stdin (`session_id`, `cwd`,
`permission_mode`, `hook_event_name`, plus `tool_name` / `tool_input` for
tool events) and answers with its exit code and output:

- **Exit 0**: fine. Stdout that's JSON is read as a response (below).
  For `SessionStart` and `UserPromptSubmit`, other stdout is added as
  context.
- **Exit 2**: block. Stderr is the reason the model sees.
- **Anything else**: a warning in the UI, then carry on.

A JSON response can include
`hookSpecificOutput.permissionDecision` (`allow`, `deny`, `ask`),
`permissionDecisionReason`, `updatedInput`, `additionalContext`, and
`decision: "block"` with a `reason`.

### Prompt hooks

A `prompt` hook asks a model instead of running a command. It's for
checks that are easier to describe than to script:

```yaml
hooks:
  Stop:
    - hooks:
        - type: prompt
          prompt: "Did the agent run the tests and see them pass? $ARGUMENTS"
          timeout: 30
```

`$ARGUMENTS` is replaced with the event's JSON input (without it, the
input is added after the prompt). The model answers `{"ok": true}` to
let things go ahead, or `{"ok": false, "reason": "…"}` to block: the
agent keeps working (`Stop`, `SubagentStop`), the message is refused
(`UserPromptSubmit`), or the tool call is denied (`PreToolUse`). Those
are the four events prompt hooks work on. They use your `small_model`
(or the main model). If the model call fails or its answer is unclear,
you get a warning and nothing is blocked.

Tool names use Claude Code's spelling for matchers and input: `Bash`,
`Read`, `Write`, `Edit`, `MultiEdit`, `Grep`, `Glob`, `WebFetch`,
`WebSearch`, and `mcp__<server>__<tool>`. `tool_input` has `file_path`
next to Mira's `path`. A hook's `allow` skips the approval prompt but
never overrides a deny rule. Hooks run with `CLAUDE_PLUGIN_ROOT` and
`CLAUDE_PROJECT_DIR` set.

## Common setups

### GitHub

The GitHub plugin talks to GitHub's hosted MCP server with a personal
access token rather than OAuth:

1. Create a token at <https://github.com/settings/personal-access-tokens>
   with access to the repos you want Mira to use.
2. Click **Add token** on the GitHub server, or run
   `mira mcp set-var GITHUB_PERSONAL_ACCESS_TOKEN`.

### Figma

Figma's hosted MCP server only accepts sign-ins from clients Figma has
approved, so **Sign in** fails with a 403. Two things work today:

- **Figma's desktop app.** Turn on the Dev Mode MCP server in Figma's
  preferences, then add it:
  `mira mcp add --transport http figma-desktop http://127.0.0.1:3845/mcp`
- **A personal access token** with the community server:
  `mira mcp add figma -e FIGMA_API_KEY='${FIGMA_API_KEY}' -- npx -y figma-developer-mcp --stdio`,
  then `mira mcp set-var FIGMA_API_KEY`.

## Files

| Path | What's in it |
| --- | --- |
| `~/.mira/mira.yaml` | `mcp_servers:` (user scope) and `hooks:` |
| `~/.mira/mcp/state.json` | Local servers, approvals, on/off switches, tool loading |
| `~/.mira/mcp/credentials.json` | OAuth tokens (0600) |
| `~/.mira/mcp/variables.json` | Saved `${VAR}` values (0600) |
| `~/.mira/plugins/` | Marketplaces, installed plugins |
| `<repo>/.mcp.json` | Project servers (need approval) |
