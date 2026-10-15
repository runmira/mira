# External-agent acceptance (#202)

`scripts/external-agent-acceptance.mjs` is an opt-in run against a live Mira
server: the packaged desktop app's server, or `mira serve`. It uses the real,
signed-in Claude Code, Codex and OpenCode CLIs and spends real turns, so it
refuses to start without `MIRA_ACCEPTANCE=1`.

```bash
MIRA_ACCEPTANCE=1 MIRA_URL=http://127.0.0.1:8787 node scripts/external-agent-acceptance.mjs
```

| Variable | Meaning |
| --- | --- |
| `MIRA_URL` | Server to drive. For the packaged app, use the port its `mira serve` sidecar listens on. |
| `MIRA_ACCEPTANCE_AGENTS` | Engine instances to run (default `claude-code,codex,opencode`). Use separate instances (for example `codex-acceptance` with its own `home_path`) to keep test accounts apart. |
| `MIRA_ACCEPTANCE_MODEL_<INSTANCE>` | Model to start on, for example `MIRA_ACCEPTANCE_MODEL_OPENCODE=opencode/big-pickle`. |
| `MIRA_ACCEPTANCE_SWITCH_MODEL_<INSTANCE>` | Model for the model-change scenario (default: another model from the same provider). |
| `MIRA_ACCEPTANCE_OUT` | Report directory (default `acceptance-report`). |

Each agent gets a throwaway git project. The report records the Mira version, the
CLI version, the version the agent reported, the capability source, and the
negotiated capabilities. Each scenario ends in one of these states:

- `pass`
- `fail`: includes reproduction steps and the last frames, with credentials and
  long values removed
- `unavailable`: the installed agent does not offer the capability, so it is not a
  failure
- `blocked`: a precondition is missing, such as provider sign-in for that model
- `manual`: needs a person in the UI; see below

The exit status is non-zero only when something failed.

## Automated scenarios

Readiness, start, a prompt round trip, a model change on the same conversation
(the run then switches back to its starting model), cancellation of a streaming turn, native
steering of a queued input, delivery of input queued behind a running turn, reconnecting to the
same chat without duplicated input, and a native fork into a separate chat, with
both chats answering afterwards.

## Manual checklist (packaged app)

Run in the packaged app with the same isolated projects, and record the app and
CLI versions:

1. **Sign-in:** Codex browser and device sign-in, and OpenCode terminal sign-in,
   each from Settings. Readiness and models refresh afterwards.
2. **Approvals and questions:** an edit and a command each ask for approval;
   structured questions show as a card and the answer is delivered once.
3. **Images:** an image prompt on an image-capable model, and the explanation
   on a text-only model.
4. **Subagents:** subagent cards appear early; child streams stay scoped to their
   card; background activity continues after the parent's result; child cards
   reach a terminal state.
5. **Transcript:** scrolling up keeps position while output streams; returning to
   the bottom resumes following.
6. **Settings and sidebar:** settings finish loading; the sidebar's elapsed time
   and spinner advance during a turn.
7. **Recovery:** quit the app mid-turn and reopen it. The chat recovers, and an
   input whose delivery was not confirmed is shown as such, never resent.
8. **Forks and rewind:** Fork chat (and Fork from here on Codex); Restore files on
   a Claude chat previews Claude's checkpoint and can be undone.

## Recorded runs

| Date | Server | Agent | Result |
| --- | --- | --- | --- |
| 2026-10-15 | `mira serve` from this branch (isolated `HOME`; OpenCode used its existing login through `XDG_DATA_HOME`) | OpenCode v2.0.22, `opencode/big-pickle` | Pass: readiness, start, prompt, cancellation, steering, fork (whole session, both chats answered afterwards). Blocked: model change, because the other models need provider sign-in. Queue delivery passed once and then failed after steering; the cause was a real queue race, fixed in this branch and not yet re-run live. Reconnect's check read the wrong Ready field; that harness bug is fixed but not yet re-run. |
| 2026-10-15 | Same, with an isolated OpenCode home | OpenCode v2.0.22 | Blocked: the first turn needed provider authentication, although readiness reported `ready`. The harness records that readiness gap as a failure. |

Claude Code and Codex were not run. Codex is not installed on the recording
machine, and a Claude run would spend the account owner's quota. None of these runs
is packaged desktop acceptance; packaged runs for all three agents remain
outstanding.
