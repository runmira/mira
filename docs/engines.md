# Engines

An **engine** is what a chat runs on. There are two kinds:

- **Native**: Mira's own agent loop over a model provider (Anthropic,
  OpenAI, OpenRouter, Groq, Bedrock, a local server…). Mira owns the
  history, tools and approvals.
- **External**: another coding agent running as its own process
  (Claude Code, Codex, OpenCode, Grok, Cursor, Antigravity). The agent owns
  its history and tools; Mira drives it and gives it Mira's tools.

Each configured engine is an **instance** with an id. Every provider in
`providers:` is a native instance, every agent Mira knows is an external
instance named after its driver (`claude-code`, `codex`, …), and `engines:`
adds more or changes them.

## Configuring engines

Engines live in the global config, `~/.mira/mira.yaml`. The `engines:` block
is read only from that file, never from a project's `.mira/mira.yaml`: an
engine says which binary runs with which credentials, and a cloned
repository must not be able to change that.

```yaml
default_provider: openrouter
providers:
  openrouter:
    api_key_env: OPENROUTER_API_KEY
  groq:
    api_key_env: GROQ_API_KEY

engines:
  # A second Anthropic account, side by side with the first.
  anthropic-work:
    driver: native
    display_name: Anthropic (work)
    config:
      provider: anthropic        # preset + the providers: entry to inherit
      api_key_env: ANTHROPIC_WORK_KEY

  # Codex with its own home, so it signs in as a different account.
  codex-work:
    driver: codex
    display_name: Codex (work)
    model: gpt-5-codex
    config:
      home_path: ~/.codex-work

  # Overriding a built-in instance: Claude Code from a specific binary.
  claude-code:
    config:
      binary_path: /opt/claude/bin/claude
      effort: high
```

Fields of an `engines:` entry:

| Field | Meaning |
| --- | --- |
| `driver` | `native`, or an agent driver (`claude-code`, `codex`, `opencode`, `grok`, `cursor`, `antigravity`). Required for a new id; overrides of a known instance can leave it out. |
| `display_name` | The name pickers show. |
| `enabled` | Whether pickers offer it (default true). |
| `model` | The model it starts on when picked without one. |
| `config` | Driver settings, below. |

**Native `config`** layers over the `providers:` entry it inherits from:
`provider` (the preset: base URL, key variable, protocol; defaults to the
instance id), `api_key` or `api_key_env`, `base_url`, `extra_headers`,
`prompt_caching`. An `api_key_env` replaces an inherited literal key, so a
second account never falls back to the first one's key. Errors name the
layer to fix: `engine anthropic-work (engines.anthropic-work.config) has no
api_key`.

**External `config`**: `binary_path`, `launch_args`, `env`, `api_key` or
`api_key_env` (a variable in Mira's environment to read the key from),
`home_path` (the agent's own config home: a separate account), `effort`,
`setting_sources`, `auto_compact_after`. Agents launched by Mira don't
inherit API keys from your shell; a key reaches an agent only through its
own settings.

Settings → Agents edits this same block. Keys live only in `mira.yaml`,
which Mira writes readable by you alone (`0600`): the app shows a masked
copy of a key, and of any `env` value whose name contains `key`, `token`,
`secret` or `password`, and never sends the real value to the browser.
Older builds kept agent keys in the browser's storage; the first launch of
this one moves them into `mira.yaml` and deletes them there.

Changes to `mira.yaml` apply **without a restart**: Mira watches the file,
rebuilds its engines, picks up new instances and keys, and tells open
pickers to refresh. A chat on an engine you removed moves to the default
and says so. A file that doesn't parse is ignored until it does.

## Background models

Compaction summaries, chat titles, memory extraction and goal checks run on
a cheaper model. By default it's on the same provider as the chat:

```yaml
small_model: claude-haiku-4-5
```

Name an instance to run them elsewhere, whatever the chat is on:

```yaml
small_model: groq:llama-3.1-8b-instant          # instance:model
compactor_model: { instance: openrouter, model: google/gemini-2.5-flash }
memory:
  extractor_model: groq:llama-3.1-8b-instant
```

A prefix only counts as an instance when one with that id is configured,
so model ids that contain a colon (`llama3:8b`) still work. If the named
instance can't serve (no key), the job runs on the chat's provider and
logs a warning instead of failing.

## Prompt caching

Each native instance reports its caching mode in `/api/engines`:

| Mode | Who | What Mira does |
| --- | --- | --- |
| `markers` | Anthropic, and Anthropic's API behind any URL | Marks the system prompt and tools for the cache. |
| `automatic` | OpenAI and similar | Nothing; the provider caches long prompts itself. |
| `off` | Local servers, most gateways | Never sends cache markers. |

`prompt_caching: true` or `false` on the provider (or the engine's
`config`) always wins. Cache reads and cache writes are both counted and
priced: writes at the model's cache-write price, or 1.25× input.

## Picking and switching

The composer's engine picker lists native instances with their models and
external agents with their health. Picking one sends:

```json
{ "type": "set_model", "instance": "groq", "model": "llama-3.3-70b-versatile",
  "options": { "reasoning_effort": "high" } }
```

The model is resolved as: the one sent, else the instance's `model`, else
the chat's current model. `options` are the model options remembered for
that instance and model (two instances offering the same model keep their
own). Picking an external agent starts it with that instance's `engines:`
settings.

In the terminal UI, `/engine` lists instances and `/engine <id>` switches
between native ones (the TUI runs Mira's own engines, not external agents);
the choice is saved to `~/.mira/state.yaml` like the app's.

### What carries over

- **Between providers**: history is provider-neutral, so it re-serializes
  for whichever provider the next turn goes to. Reasoning doesn't always
  survive: Anthropic replays its own signed thinking, other providers'
  reasoning is display-only, and OpenAI-style providers replay none. Mira
  warns when a switch drops reasoning from earlier replies; `/compact`
  first gives a clean boundary.
- **Between Mira and an agent**: whichever side takes over gets the turns it
  missed, up to 12,000 characters, newest first.
- **Between agents** (say Claude Code → Codex): the new agent gets what the
  previous one did, and the chat shows a "Context handoff" divider with
  both engines.

## `GET /api/engines`

Every engine with its health, for pickers and settings.

```json
{
  "engines": [
    { "instance": "anthropic-work", "driver": "native", "flavor": "native",
      "display_name": "Anthropic (work)", "enabled": true,
      "state": { "state": "ready" }, "models": [ … ],
      "default_model": "claude-sonnet-4-5", "prompt_caching": "markers" },
    { "instance": "codex", "driver": "codex", "flavor": "external",
      "display_name": "Codex", "enabled": true,
      "state": { "state": "ready" }, "models": [ … ],
      "auth": "ChatGPT Pro", "agent": { … } }
  ],
  "active_instance": "anthropic-work",
  "active_model": "claude-sonnet-4-5",
  "fresh": true
}
```

`state` is one of:

| State | Meaning |
| --- | --- |
| `ready` | Usable now. |
| `not_configured` | Missing a key or base URL; `reason` says which and where. |
| `not_found` | The agent's binary isn't installed; `looked_for` lists what was tried. |
| `failed` | It's installed but the check failed (not signed in, crashed); `reason`. |
| `unavailable` | This build doesn't know the instance's driver. |

External agents are probed in the background (probing starts a process),
and results are cached. Right after Mira starts, external rows are
placeholders and `fresh` is `false` until the first probe lands.
`?refresh=1` forces a new probe. External rows carry the agent's model list
when its CLI can list one without starting a session (Codex, OpenCode) or
has stable aliases (Claude Code: `opus`, `sonnet`, `haiku`, `fable`), and
`agent`, its full status (versions, transport, sign-in methods).

## Multiple accounts

- **Agents**: one instance per account, each with its own `home_path` (and
  `env` if needed). Two enabled instances of one driver that would sign in
  as the same account get a warning in the picker.
- **Providers**: one native instance per account, each with its own key, as
  in `anthropic-work` above.

## `GET/PUT /api/engines/:instance/settings`

An external agent's `engines:` entry, for Settings → Agents. Native
instances answer `400`: their keys are provider settings.

```json
{ "instance": "codex", "driver": "codex", "enabled": true,
  "home_path": "~/.codex-work", "launch_args": [],
  "env": [ { "key": "CODEX_PROFILE", "value": "work" },
           { "key": "GITHUB_TOKEN", "masked": "ghp_…abcd" } ],
  "has_api_key": true, "api_key_masked": "sk-p…wxyz",
  "api_key_vars": ["CODEX_API_KEY", "OPENAI_API_KEY"] }
```

`PUT` takes any subset of `display_name`, `enabled`, `binary_path`,
`home_path`, `launch_args`, `env`, `effort`, `setting_sources`, `api_key`,
`api_key_env`. An empty string clears a field. `env` replaces the whole
set, and a `null` value keeps that variable's stored value (how a masked
secret survives an edit of the others). The change applies to the next
agent start at once.
