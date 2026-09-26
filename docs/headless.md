# Running Mira from scripts and CI

`mira -p` runs one task without a UI and exits. Use it from shell
scripts, CI jobs, git hooks, or another program.

```bash
mira -p "explain what src/main.rs does"
git diff | mira -p "write a commit message for this diff"
mira -p "fix the failing test" --allow "Bash(cargo test:*)" --allow "Edit(src/**)"
```

- **The prompt** is the text after `-p`. Piped stdin is added below it as
  context. With `-p` and no text, stdin is the whole prompt:
  `cat task.md | mira -p`.
- **Permissions** follow your usual rules (`permissions` in `mira.yaml`).
  Anything that would stop to ask you is denied instead, and listed in
  the result. Allow more with `--allow RULE` (repeatable, same syntax as
  `permissions.allow`) or a mode: `--mode edit` lets it edit files,
  `--mode yolo` allows everything.
- **`--max-turns N`** stops after N model rounds.
- **`--resume [ID]`** continues an earlier session, so a script can ask
  a follow-up.
- Logs and warnings go to stderr; stdout carries only the output below.

## Output

`--output-format text` (the default) prints only the final answer.

`--output-format json` prints one object when the task ends:

```json
{
  "type": "result",
  "subtype": "success",
  "is_error": false,
  "error": null,
  "result": "The failing test expected 3 items; it now…",
  "session_id": "sess_…",
  "num_turns": 4,
  "duration_ms": 18234,
  "usage": { "input_tokens": 51200, "output_tokens": 1830, "cached_input_tokens": 40960 },
  "total_cost_usd": 0.0412,
  "permission_denials": [{ "tool": "bash", "input": { "command": "rm -rf target" } }],
  "rate_limit": {
    "requests": { "limit": 500, "remaining": 40, "reset_secs": 90 },
    "tokens": { "limit": 30000, "remaining": 29000 }
  }
}
```

`total_cost_usd` is `null` for models Mira has no price for.
`rate_limit` is the provider's last reading from its response headers
(`requests`, `tokens`, `input_tokens`, `output_tokens`, each with
`limit`, `remaining` and `reset_secs`), or `null` if it sends none. A
script can use it to slow down before hitting a 429.

`--output-format stream-json` prints one JSON object per line as the task
runs, then the same result object:

| `type` | Fields |
| --- | --- |
| `system` (`subtype: init`) | `session_id`, `model`, `cwd`, `tools` |
| `text` | `text`: a piece of the model's reply |
| `tool_use` | `id`, `name`, `input` |
| `tool_result` | `id`, `is_error`, `content` |
| `warning` | `message` |
| `compacted` | `messages_removed` |
| `rate_limit` | `rate_limit` (as in the result), `summary`: e.g. `8% of requests left · resets in 2m` |
| `result` | as above |

```bash
mira -p "list the TODOs" --output-format stream-json \
  | jq -r 'select(.type == "tool_use") | .name'
```

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The task finished. |
| 1 | The model or provider failed (bad key, network, a stream that broke). The error is in `error` and on stderr. |
| 2 | Bad usage, e.g. `-p` with no task. |

A task that finished with some tools denied still exits 0; check
`permission_denials`.
