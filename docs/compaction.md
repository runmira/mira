# Conversation compaction

Mira measures each provider request, clears old tool results first, and retains
synchronous compaction and two bounded overflow retries as its recovery path.
External agents own their histories; Mira does not feed their transcripts into
its own provider history.

## Summary checkpoints

Portable compaction retains complete recent user turns within an estimated
15,000-token budget and summarizes the older prefix. Tool calls and results never
straddle the boundary. Small histories and a single oversized newest turn are
summarized in full; an unanswered user prompt is always retained. The tight
overflow retry can summarize the retained tail too.

The last checkpoint is passed in full, without the old 4,000-character clipping,
followed by newer history. The prompt requires exact identifiers, newer evidence
taking precedence, only unfinished requests, explicit Git/PR state, and validation
limitations. Truncated or failed summaries leave history intact. Shrinking-input
retries retain the checkpoint while reducing the newer transcript. Foreground
portable compaction can restore five recent files; speculative jobs omit file
snapshots because files may change while a job runs.

## Native providers and fallback

The provider contract offers optional native compaction. Mira prefers it on the
session model, with a 30-second deadline, then falls back to the configured
summarizer and finally the session model. `/compact <focus>` uses the portable
prompt to respect the explicit focus.

- Official OpenAI API: `/responses/compact`. Its output is stored verbatim and
  replayed through Responses. Subsequent native output items, including encrypted
  reasoning required beside function calls, are retained for stateless replay.
  Compatibility endpoints retain Chat Completions and portable summaries.
- Official Anthropic API: on-demand `compaction: {type: "summarize"}` with the
  `compact-2026-09-04` beta. Only a completed, signed block is adopted. It is
  replayed verbatim first in Messages, with the beta header. Native requests
  carry the session's tools and thinking options. Unsupported models fall back.
- Other adapters use portable summarization unless they implement the optional
  contract. Bedrock's existing Converse adapter is unchanged.

Native state is persisted with endpoint, credential fingerprint, and model ownership. Resuming with an
incompatible provider/model or switching models restores portable history from
the archive, retaining assistant text and calls while removing opaque payloads
and obsolete checkpoint markers. Portable fallback over a native checkpoint asks
through its owning provider instead of treating ciphertext as transcript text.

Sources: [OpenAI compaction](https://developers.openai.com/api/docs/guides/compaction),
[Anthropic on-demand compaction](https://platform.claude.com/docs/en/build-with-claude/compaction-on-demand),
[Codex background lifecycle](https://github.com/openai/codex/blob/e89b7c442c0433ad5167e72a5a25bb6d0712766a/codex-rs/core/src/session/async_compaction.rs),
[OpenCode checkpoints](https://github.com/anomalyco/opencode/blob/v2.0.22/packages/core/src/session/compaction.ts).

## Background and end-of-turn lifecycle

At 70% of the window, Mira may start one speculative job per send while work
continues. At a successful turn's end, 80% triggers preparation for the next turn,
including when the normal compaction threshold was crossed. Preparation does not
delay Done. The session owns the job; it has a 60-second deadline and is aborted
on Stop, manual compaction, rewind, model change, or disposal. Identical snapshot
attempts are deduplicated. No live history lock is held during the network call.

A ready candidate is adopted at a model-request boundary only after comparing
configuration, tools, and every captured message. Appends survive; edits, rewinds,
changed tools, or rewritten results invalidate it. Adoption updates the archive,
resets clearing indices, emits the usual compaction event, and checkpoints before
the next request. A crash before adoption loses only an optimization. Failures
leave foreground recovery available. PreCompact-hook sessions retain foreground
compaction so speculative attempts never run user hooks or announce discarded work.

## Claude resume choice

Before the first ordinary input to a resumed native Claude process, Mira queries
its context figures. At 90% of the reported auto-compact threshold, the existing
question card offers **Compact first**, **Continue**, and **Continue and never ask
in this chat**. Opt-out is persisted per chat and reset for newly created chats.
Unknown/unsupported usage skips the question; slash commands bypass it.

Compaction waits for Claude's separate `/compact` turn before sending the pending
input. That maintenance turn does not end the pending foreground turn. Stop
cancels preflight; failures leave the message unsent. Dropping the preflight
cleans up its prompt registration and card.

## Recall evaluation

Tests cover tail identity, tool/image pairing, native fallback and replay, stale
snapshot rejection, cancellation, and Claude maintenance ordering. They do not
prove live provider availability or packaged UX.

```sh
cargo run -p mira-harness --example compaction_recall -- \
  /path/to/sanitized-chat.json /absolute/path/to/claude haiku
```

The runner compares before/after recall through the actual portable compactor,
with tools disabled, persistence off, settings excluded, a dedicated evaluation
system prompt, and a working directory outside the checkout. Obtain consent
before sending private transcripts externally; never commit transcripts/results.

A consented sanitized Mira desktop-flavors chat was evaluated on 2026-10-15:
131 messages, approximately 81,000 content characters; 47 messages summarized and
84 retained. Recall preserved the Alpha/Beta objective, Cargo/Tauri/release paths,
failed build command and missing workspace dependencies, and the unshipped Git
state. The after answer was imperfect about which user-request wording was
latest. This is one qualitative sample, not a recall score, a live native API
test, or packaged desktop verification.
