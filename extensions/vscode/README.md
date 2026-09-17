# Mira Coding Agent for VS Code

**Chat with [Mira](https://github.com/runmira/mira) — an open-source coding
agent that runs on your machine — right from the VS Code Chat sidebar.**

Mira reads your code, edits files, runs commands, reviews diffs, and
delegates subtasks to specialised subagents. It talks to any
OpenAI-compatible provider (OpenRouter, OpenAI, Anthropic, Groq, DeepSeek,
Ollama, LM Studio). Your key, your model, your machine.

> **Preview.** Building in the open. `@mira` chat is stable; approvals
> in `manual` mode and inline diffs are on the near roadmap.

## What you get

- **`@mira` chat participant** in the built-in VS Code Chat sidebar —
  same surface as Copilot Chat, Continue, Cursor's assistant.
- **Streaming responses** — tokens land as they're generated, tool calls
  render inline as fenced code blocks with a one-line arg summary so
  you can see *what* Mira actually did (read this file, ran that
  command, edited that region).
- **Any OpenAI-compatible provider** — Anthropic, OpenAI, OpenRouter,
  Groq, DeepSeek, Ollama, LM Studio. Configured once in `mira.yaml`,
  swappable per-turn via `/model`.
- **Permission modes** — `plan` / `manual` / `auto` / `edit` / `yolo`
  decide how freely the agent edits and runs commands. Switchable via
  `/mode`.
- **File and selection references** — `#file:foo.ts`, `#selection`, or
  drag-and-drop; the extension flattens VS Code's native chat
  references into the prompt.
- **Follow-active-folder** — Mira's working directory tracks the
  VS Code workspace folder, so a fresh chat lands in the repo you're
  looking at.

## Requirements

Mira runs as a local backend the extension talks to over WebSocket.
**Install and start the backend once**, then this extension takes care
of the rest.

Install Mira (macOS / Linux):

```bash
brew install runmira/tap/mira
# or, no Homebrew:
curl -fsSL https://raw.githubusercontent.com/runmira/mira/main/install.sh | bash
```

Configure a provider (either via env vars or by editing `~/.mira/mira.yaml`):

```bash
export MIRA_API_KEY=sk-or-v1-...                 # e.g. an OpenRouter key
export MIRA_BASE_URL=https://openrouter.ai/api/v1
export MIRA_MODEL=google/gemini-2.5-flash
```

Start it:

```bash
mira serve --port 8787
```

Open VS Code's Chat sidebar (⌃⌘I on macOS, Ctrl+Alt+I on Windows/Linux),
type `@mira`, and go.

## Slash commands

| Command | What it does |
| --- | --- |
| `/reset` | Start a fresh session (drops in-memory history). |
| `/mode plan\|manual\|auto\|edit\|yolo` | Set the permission mode. |
| `/model <name>` | Hot-swap the model for the next turn. |

## Configuration

| Setting | Default | What it does |
| --- | --- | --- |
| `mira.baseUrl` | `http://127.0.0.1:8787` | Where `mira serve` is listening. Loopback by default. |
| `mira.showToolCalls` | `true` | Render each tool call the model makes (name + args + result). Turn off for prose-only. |
| `mira.followActiveFolder` | `true` | Sync the Mira session's cwd to the open workspace folder on each chat send. |

Change any of these in VS Code's Settings under **Mira**.

## What this extension doesn't do (yet)

- **Doesn't auto-spawn `mira serve`.** Run it yourself; a follow-up
  release will start it on demand when the binary is on PATH.
- **Doesn't show approval modals for `manual` mode.** If the model
  requests permission, open the web UI to approve, or set `/mode auto`
  to skip. Inline `showQuickPick` approvals are on the near roadmap.
- **Doesn't render the subagent / review / pull-request / plugins
  panels.** Those live in the browser UI — run the *Mira: Open Web UI
  in Browser* command from the palette to reach them.

## Related

- **[Mira on GitHub](https://github.com/runmira/mira)** — the backend,
  CLI, and web UI live here. Apache-2.0.
- **[Report an issue](https://github.com/runmira/mira/issues)** — file
  extension bugs against the main repo, tagged `vscode`.

## License

Apache-2.0. See [LICENSE](./LICENSE).
