# External agent runtime and execution capacity

Mira isolates provider selection per chat. The native provider pool is forked
for each slot, including the provider used by that slot's subagents. Selecting
an engine in another chat changes future defaults without redirecting an
existing slot. SessionConfig persists `engine_instance` for reload. Legacy
records without that field inherit the configured default because they have
no historical instance identity to recover.

## Asynchronous questions

Codex async agent-message questions become provider-neutral RuntimeRequests.
They have native thread/turn/item identifiers, an engine-instance owner, and a
message response capability. Blocking questions retain their live RPC path.

Requests and answer receipts live in an atomic `*.runtime` sidecar next to
the agent transcript. Mira commits the resolution and answer message together
before contacting the provider. Duplicate notifications and submissions return
the existing record. Required questions and option labels are validated. Ready
frames restore question cards independently of the recent transcript window.
Unreadable persistence fails closed; it is not replaced with an empty ledger.

The answer outbox checks the owning engine instance and native thread before
sending. It starts a new turn or steers the current Codex turn using its expected
native turn ID. Queued answers wait for lifecycle notifications rather than a
per-session polling timer. Capacity failures park in one shared, bounded retry
index and wake when a runtime lease is released. Queued answers also recover
when the persisted chat is loaded or attached after a server restart.

`dispatching` means a provider call might have been accepted. A crash or timeout
in that state does not automatically resend: native turn APIs do not provide
Mira with an exactly-once delivery guarantee. The question card keeps the saved
answer and says delivery has not been confirmed. `delivered` means the provider
accepted the call, not that it finished responding.

The ledger retains at most 4,096 receipts per chat, with a 64 KiB limit per request
or answer. The in-memory capacity retry index holds at most 4,096 parked chats.
If it fills, answers remain on disk and reopening the chat retries admission.

## Native work ownership

The adapter boundary emits RuntimeTurn and RuntimeWork data in addition to
text/tool events. Work distinguishes goals, tasks and subagents and preserves
native thread identities. Terminal work updates remove active ownership rather
than accumulating completed work indefinitely.

Codex goal updates keep ownership across gaps between native turns. Paused,
blocked, completed and budget/usage-limited goals release active goal ownership.
Codex subagent activity and collaboration states are tracked separately from
the root; a child's text or completion cannot overwrite the root's native
thread or terminate its turn. Claude task-start/progress/notification events
keep background task ownership after a parent result.

The reaper checks active native work, active harness/agent turns and pending
blocking approvals/questions. Dispatch and start locks prevent it from stopping
a newly admitted prompt. Generation fencing ignores events from replaced
runtimes. Explicit shutdown releases native ownership and its capacity lease.
Spend booking has a bounded queue and backpressure.

Capabilities describe lifecycle features implemented by the active adapter;
they are not evidence that an older installed CLI implements every native API.
ACP adapters that do not advertise native background activity retain conservative
capability defaults. Their foreground turn remains protected from reaping.

## Local limits versus one million simultaneous sessions

Defaults are 64 resident agent runtimes globally and 16 per provider driver.
Configure them at server startup with:

- `MIRA_MAX_AGENT_RUNTIMES`
- `MIRA_MAX_AGENT_RUNTIMES_PER_PROVIDER`

A lease covers the lifetime of a resident CLI process, including background work.
Admission is nonblocking. Limits must fit the machine's RAM, process/file
descriptor limits and provider account quotas. Increasing these variables to one
million does not make the machine capable of running one million agents.

This is a bounded single-server implementation, not a million-session distributed
execution service. Session slots still hold in-memory state, native runtimes
remain owned by one server, file persistence is local, and retries after a full
server restart are activated when a chat is loaded. Native background work is
observed while its runtime is alive; it is not automatically reconstructed or
resumed as a fleet job after the server process dies.

For a million actually running sessions, the remaining deployment work is:

1. A partitioned transactional session/request/message store and durable outbox.
2. A bounded worker fleet with instance/account-scoped quotas, renewable leases,
   worker epochs and fenced provider ownership; no shared account mutable state.
3. Partitioned scheduling and event streams with resumable subscriber cursors,
   bounded delivery queues and cold-session eviction.
4. Recovery/reconciliation for uncertain native acceptance and orphaned workers,
   plus provider-specific authentication placement and native background resume.
5. Multi-host load and crash tests with realistic provider costs, limits and
   runtime memory, proving the intended concurrent-active-session target.

The local admission burst test checks rejection and bounded waiter behavior
under a million attempts. It is not a test of a million running sessions.

## Shared transcript presentation

Provider and native chats use the same virtual transcript, compact thought rows,
worked-time divider, activity disclosures, quote selection, and per-turn workspace
change summaries. Initial replay sends the latest 20 whole turns; `history` requests
load older pages using a session-scoped prefix fingerprint. Rewritten history
invalidates its cursors. The frontend falls back to full replay for older servers.
Deploy the new frontend and server together: older frontends do not understand the
paged Ready payload.

Scroll anchors use stable turn keys and an offset within the visible row. They are
remembered per chat for the browser session. Loading earlier pages preserves the
reader's position; minimap jumps can target turns that are not mounted. Provider
turn metadata retains its original index across mixed native/provider histories.
Codex message phases and Claude successful-result metadata identify final answers;
legacy records without phase metadata retain their existing fallback.

Turn diffs compare before/after Git workspace snapshots without modifying the
user's index. They include all workspace changes in that interval, including user
or concurrent-chat edits, and are not proof of agent authorship. Older turns without
an after snapshot have no historical turn summary. Snapshots may expire under Git
object cleanup. Paging bounds network replay and mounted DOM; it still reads the
combined stored history on the server, and a single very large turn remains whole.
This work does not establish million-active-session fleet capacity.

## Queued message controls

The composer can expand the full queue and move pending messages by dragging
their handles or using Move up / Move down in the row menu. Reordering changes
one item relative to another rather than overwriting a stale client queue.
The worker rechecks the first item under the same lock before claiming delivery.

Queued edits keep their original position, file payloads and images until an
atomic save succeeds. The server rejects outdated content fingerprints and
edits or moves of messages already being delivered. The editor keeps the user's
text after rejection or connection loss. Conditional queue mutations bypass
the reconnect buffer and wait for a session-scoped acknowledgement; an
unconfirmed save must be checked against the queue before retrying.

These controls share the queue implementation across providers and external
agents. Deploy the updated server and frontend together.


## Recovery, drafts, and composer notices

A quota failure offers a saved continuation in the affected chat. Resume at reset
is offered only when the runtime reported a future reset; otherwise retry is
manual. Cancellation and snoozing are distinct. Scheduled continuations run while
Mira's server is running, survive restart, and verify the engine/model before
delivery. A newer input supersedes them. Interrupted delivery receipts stay
visible for inspection and never replay automatically.

Draft text, file contents, and images are saved per chat in IndexedDB, with a
small text fallback. Payloads expire after 14 days and retention is bounded to
40 drafts / 32 MiB; individual drafts are limited to 12 MiB. Storage failures
keep the unsaved draft in memory and show a warning. Only the selected chat's
payload is read on restore. Sending fences delayed hydration and pending writes.
The composer lists all questions, plans, approvals, connection notices, and
agent updates. Switching notices preserves unsent answers and edited plans.

## Codex's Mira tool bridge

Codex receives an authenticated, chat-scoped MCP server through the app-server's
`mcp_servers.mira` thread override. Only that server is overridden; the user's
other MCP servers are preserved. Mira exposes plan/question cards, workspace
read/write/edit/search/commands, browser verbs, delegation, and persistent
background processes. Workspace and background calls evaluate explicit policy
rules and the chat's current approval mode on every invocation. Tool contexts
follow the chat's active compute environment.

Codex is instructed to prefer those Mira tools. Its own native computer-use
permissions remain separate, and native terminal IDs are not Mira process IDs.
Mira's `run_background` / `read_output` handles remain valid across interrupted
turns. Mira tool-server startup failures become visible stream activities.
Workspace tool calls have one stream owner and are persisted for replay.

New messages expose their timestamp in the hover actions. Native-agent replay
uses persisted frame times; legacy provider messages without a stored timestamp
do not receive an invented time. Deploy frontend and server together, and
reconnect existing Codex agents to pick up the revised tool configuration.


## External subagents in Mira's agent UI

Native child identities and explicit delegation tool calls use the same `agent`
entries, AgentCard/AgentGroup and SubagentPanel as Mira's own helpers. This follows
T3's separation of provider lifecycle projection from each child's stream,
adapted to Mira's transcript and session store.

Codex collaboration states and child-thread lifecycle frames produce stable child
IDs, names/prompts when supplied, completion status and reported results. Child
assistant text, snapshots, reasoning and tool updates travel in a scoped
`external_subagent_frame` envelope. Child spend retains the existing accounting
path. Child narration cannot append to the parent reply or finish its turn.

ACP `Task`, `Agent`, `task`, `agent` and `subagent` calls with a prompt (including
OpenCode), or a prompt plus an explicit subagent type, translate into Mira agent
cards. Providers can report tool progress and results without exposing a full
child transcript; the panel displays what is available. Ordinary task-management
and shell calls keep their regular tool UI. Claude background lifecycle updates
retain their launch call ID so a tool's result cannot prematurely finish a
still-running child or create a second card.

Late snapshots do not reopen a completed child. A new child turn explicitly
reopens it; Ready's authoritative activity snapshot restores resumed work.
Sidecars replay lifecycle cards and child streams separately. Provider child IDs
never go to Mira's session-history endpoint.

Reference inspected: T3's CodexAdapterV2 subagent projection and child contexts,
and the OpenCode2AdapterV2 native child/session background lifecycle fixtures.
Verification covers parent/child isolation, parallel identities, replay, late
snapshots, reopening, delegation translation and launch/lifecycle convergence.
Live authenticated provider acceptance remains separate; this does not claim
that every provider exposes child tokens or tools.


### Child stream completion follow-up

OpenCode ACP child updates use its documented `_meta["opencode/child-session"]`
identity to route assistant text, reasoning and tools to child panels. Child
configuration updates cannot change the parent's configuration. Claude stream
updates carrying `parent_tool_use_id` use independent child normalizers, preserving
parallel children even when their local message or tool IDs coincide. Child
terminal frames cannot finish the parent turn. Explicit foreground child tool
results complete that child; parent completion pauses remaining exposed activity.
Providers that do not emit child content continue to show supplied progress/results.

Lifecycle and child content frames are persisted in the transcript sidecar and
replayed separately from parent narration. Late launch IDs become aliases of the
first displayed child identity, preserving open tabs and avoiding duplicate cards.
An approved pending tool becomes running on its tool-start event; settled tools
remain settled.

Protocol reference: [OpenCode ACP child sessions](https://dev.opencode.ai/v2/docs/cli/acp/).


### Early launch visibility and OpenCode sign-in

Claude tool-use block starts are emitted immediately, before completed argument
JSON or the assistant snapshot. Explicit delegation names can create a Mira agent
card with a pending prompt; later sparse updates merge launch metadata into that
same card and child panel. Ordinary shell and task-management calls retain their
existing tool presentation. The ACP client explicitly advertises
`_meta["opencode/child-session-updates"]`, which OpenCode requires before sending
child updates.

OpenCode's ACP `authenticate(opencode-login)` returns success without collecting
credentials. Its own CLI login supplies saved provider credentials. Mira keeps
its restricted child environment; it does not silently forward ambient provider
or GitHub tokens. OpenCode settings show a copyable login command using the
configured binary, and ACP provider-auth failures direct the user to that flow.
After saving the selected provider login, retry the message explicitly.

Sources inspected: [OpenCode ACP service](https://github.com/anomalyco/opencode/blob/dev/packages/cli/src/acp/service.ts)
and [child capability negotiation](https://github.com/anomalyco/opencode/blob/dev/packages/cli/src/acp/capabilities.ts).

### Desktop and CLI credential compatibility

A working OpenCode desktop login does not establish that Mira’s configured CLI
has the same credentials. Diagnose the exact CLI binary and version first, then
check `auth list --standalone` on OpenCode v2 with the same environment Mira
passes. Do not forward ambient shell provider tokens as a workaround.

OpenCode v1 stores provider credentials in its XDG data directory’s `auth.json`;
v2 stores credentials in its database. The v2 `auth import --standalone` command
accepts exported `Credential.Entry` records, not the legacy JSON object. Migration
must preserve existing destination integrations and translate the legacy schema.
Pipe the translated records directly to the import process; keep secret values
out of logs, temporary files, browser responses, and issue or PR descriptions.
Verify the stored provider appears without shell tokens, then separately verify
the selected model and a live prompt. Importing a credential alone does not prove
that the provider accepts it or has credit.

Sources: [OpenCode credential schema](https://github.com/anomalyco/opencode/blob/v2.0.22/packages/schema/src/credential.ts),
[legacy credential migration](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/database/migration/20260805200742_import_legacy_credentials.ts),
and [auth import](https://github.com/anomalyco/opencode/blob/dev/packages/cli/src/commands/handlers/auth/import.ts).


## OpenCode native steering

OpenCode v2's ACP process owns a private server. Mira installs a temporary local
plugin package using OpenCode's promise-plugin API and admits queued input through
that server's `session.prompt` with `delivery: "steer"`. The existing ACP prompt
and conversation remain active; steering does not cancel or start a second ACP
prompt. Legacy OpenCode versions that advertise HTTP flags retain their original
control path. The prepared launch configuration reaches the actual ACP process.

The plugin exposes a loopback-only endpoint authenticated by a per-runtime token
stored in a private file (0600 on Unix). It rejects browser origins, accepts text
and inline images, deduplicates admission IDs, and closes when its plugin owners
stop. Mira advertises steering only after the bridge passes its authenticated
health check. Ambiguous delivery failures are never automatically retried.

Validation includes the installed OpenCode v2.0.22 ACP startup and a live free-model
turn that receives a steering instruction and finishes without cancellation.
These opt-in tests use `MIRA_TEST_OPENCODE_BIN` and remain ignored in normal CI.
Plugin authentication, admission deduplication and cleanup also run in the normal
frontend test suite.


## ACP compaction

This build enables the ACP SDK's preview session-compaction schema and advertises
`clientCapabilities.session.compaction`. Native IDs are scoped by native session;
start, terminal patches and summary chunks update one retained transcript card.
Replacement flags preserve omitted versus explicitly cleared summary/error fields
through sidecar serialization. Live traffic and replay use the same reducer.
Cancellation has its own state. An unfinished native compaction cannot be marked
successful merely because its surrounding prompt ended.

OpenCode's compact action uses only the live session's advertised `compact`
command. It shares normal input admission, foreground reservation, Stop epochs
and turn completion with ordinary prompts; a busy session rejects the request.
The captured engine instance is rechecked under the dispatch lock. Compaction
keeps `/compact` at the beginning of the prompt and preserves pending context
handoff for the next ordinary input. The UI creates no optimistic busy state for
OpenCode; the server admits the command before creating its fallback card. Older
agents without native compaction updates retain that turn-based fallback.
Deploy the frontend and server together for the new transcript frame types.


OpenCode v2.0.22 uses `session_info_update` metadata under
`opencode/compaction` for its lifecycle receipts. Mira normalizes that older
format into the same ID-addressed events. That release supplies no retained
summary through this ACP metadata; Mira does not invent one. The preview schema
path preserves supplied summaries/chunks from agents that implement it.
Compaction presentation follows T3's small transcript divider with a live label;
retained context and failures expand inline in a bounded panel. The context
meter disables compaction while the turn is active or OpenCode has not advertised
the command, with an explanation beside the action.


Codex model picks acknowledge the refreshed catalog/current value through the
same config-option path as ACP agents. Mira publishes the acknowledged model
into the session engine immediately and saves the standing choice. Codex applies
that choice on the next `turn/start` in the existing native thread; an already
running turn retains its original model. No new Mira session or native thread
is required, and stale context-window readings are cleared on the switch.

## Codex model capabilities and per-chat choices

Codex model catalogs retain supported/default reasoning efforts, service tiers
and defaults, input modalities, and legacy speed metadata. Live config options
and engine resources carry this metadata. The composer shows only advertised
reasoning and speed choices for the selected model. A null service-tier default
keeps a Default choice available alongside optional Fast tiers. Older catalogs
without these fields expose no invented controls and defer to Codex's defaults.

Confirmed choices are persisted with the chat, keyed by engine instance, and
reapplied after a restart against the current catalog. Changing models retains
compatible choices and otherwise uses the new model's advertised defaults.
Each subsequent native turn receives the selected effort (including collaboration
mode settings) and service tier on the same thread; an active turn keeps its
existing request. Explicit text-only models reject image prompts and steering,
and the composer explains the restriction before sending attached images.

Reference: [Codex model schema](https://github.com/openai/codex/blob/f87ba04ec3cd386a92e775405b252a310b40bb0d/codex-rs/app-server-protocol/schema/typescript/v2/Model.ts).

## Codex sign-in (#196)

The Codex instance's settings contain a sign-in row backed by an account-only
app-server connection. It reads saved CLI credentials without opening a thread,
supports browser and device-code sign-in, matches completion by the native login
ID, and rereads account/requirements before confirming success. Native tokens
never enter the UI or transcript; pending handoffs stay in memory and HTTP
responses disable caching. Older CLI methods fall back explicitly to another
flow or that instance's CLI login.

The connection and pending attempt survive UI reconnects. Cancellation checks
both connection and login IDs under the protocol gate and closes the native
callback listener even when older cancellation methods fail. Connections expire
after ten minutes, with a fresh ten-minute window when a sign-in starts. Changes to the instance's home, binary, launch environment or
arguments revoke old handoffs; stale completions cannot update the new owner's
readiness/model cache.

Account changes refresh authenticated readiness/catalogs and affected live Codex
threads' account requirements/model options. The selected native thread is kept,
and failed prompts are never resent automatically. Mira's own account and native
provider OAuth settings remain separate.

References: [official authentication protocol](https://developers.openai.com/codex/app-server#authentication),
[pinned account protocol](https://github.com/openai/codex/blob/f87ba04ec3cd386a92e775405b252a310b40bb0d/codex-rs/app-server-protocol/src/protocol/v2/account.rs).

## Native forks as separate chats (#194, #197, #201)

A native fork creates a new Mira chat. It never replaces the runtime in the current
chat. The new chat is nested under the original in the sidebar. It keeps the project,
engine instance, model and confirmed launch options. It also receives the transcript
sidecar through the fork point and a resume cursor for the new native session. The
original chat, its running agent and its cursor are left unchanged. Nothing is saved
unless the native fork succeeds; the sidecar and cursor are removed if the chat
record cannot be saved.

| Agent | Native operation | Scope offered |
| --- | --- | --- |
| Codex (app-server) | `thread/fork` with `lastTurnId`, run in a short-lived app-server so the live thread never sees the new thread's notifications | Fork from a completed turn, or the whole chat |
| OpenCode (ACP) | `session/fork` with the source session's exact working directory, only when `sessionCapabilities.fork` is advertised | Whole chat |
| Claude Code | `--resume <id> --fork-session` on the new chat's first start; the cursor is marked pending until Claude reports the new session ID | Whole chat |

Codex fork boundaries come from completed native turn IDs, which are now recorded
in the sidecar. Subagent threads are excluded. Chats recorded before this change can
fork only from the latest turn. A boundary that is still running is rejected with
instructions to wait or stop it. A message the provider or another agent answered
cannot be a native boundary. Agents that fork only whole sessions do not show
"Fork from here" on individual messages. Their action is labelled **Fork chat**.
An installed version without the operation reports that it cannot fork natively.
Mira never falls back to replaying the transcript into a fresh conversation.

OpenCode refuses to fork a session that has no messages. The opt-in
`installed_opencode_forks_a_session_without_touching_the_source` test uses a
free model to verify the fork with OpenCode v2.0.22, then resumes both sessions.

References: [pinned `thread/fork` schema](https://github.com/openai/codex/blob/f87ba04ec3cd386a92e775405b252a310b40bb0d/codex-rs/app-server-protocol/schema/typescript/v2/ThreadForkParams.ts),
[OpenCode ACP sessions](https://dev.opencode.ai/v2/docs/cli/acp/#sessions).

## Installed-version capability negotiation (#203)

Each runtime start records, per engine instance, the resolved binary, its reported
version, the transport, and the capabilities that runtime negotiated. Versions come
from ACP `agentInfo` or the Codex `userAgent`. Claude Code reports no version on the
wire, so Mira asks its `--version` after start. The registry is
`~/.mira/agent-capabilities.json`; an unreadable file is treated as unknown.

- **Live**: the running agent's negotiated capabilities, plus Mira's own gates
  (OpenCode's steering bridge, Codex model input modalities).
- **Recorded**: the agent is not running and the launch is unchanged (same resolved
  program and modification time, arguments and home or config locations). The last
  runtime's capabilities are used, so a reloaded chat can offer what its install
  supports.
- **Unknown**: a new or changed launch. Conservative defaults apply, and actions such
  as forking stay hidden until the agent starts.

Every start takes an epoch first. A commit from an older epoch never replaces a newer
runtime's record. Native calls answered as unknown methods (`-32601`, "method not
found", "unknown variant") permanently downgrade fork, rollback or steering for that
launch and version. Mira reports the operation as unavailable; it never substitutes
a fresh conversation or a cancellation. An upgraded CLI starts with a new record and
is asked again.

Goals and background activity remain event-driven for Codex: an older app-server that
never emits goal or subagent updates shows none. ACP fork, resume, load and image
support come from `initialize`. Malformed capability fields count as unadvertised and
do not fail startup.

## OpenCode terminal sign-in (#198)

OpenCode's settings include **Sign in in terminal**. Mira initializes that instance's
configured agent with terminal sign-in advertised: the spec `auth.terminal`
capability, plus OpenCode's older `_meta["terminal-auth"]` flag. Mira reads the
terminal method OpenCode returns; v2.0.22 returns `opencode auth login`. A method
that names the agent's own binary resolves to the instance's configured program,
never to whatever `PATH` finds first. It runs with the instance's environment and
deny prefixes in a Mira terminal shown inside the settings panel, so the login
updates the same credential store the instance's prompts use. Terminal methods are
never sent to `authenticate`.

Exit status 0 re-probes readiness and the model catalog. It then reconnects idle
runtimes on that instance, which re-initialize with the new credentials on their next
prompt and resume the same session. A running turn is left alone, and nothing is
resent. Any other exit is reported as a failure. Cancel kills the login first and
marks the attempt cancelled, so its exit is not reported as a failure. A newer attempt
or a changed binary, home or arguments makes an older attempt's completion stale.
After a successful login, the panel reports readiness and the number of available
models. It shows a warning when the agent is signed in but has no models (no provider
connected yet). Agents without a terminal method keep the copyable login command as
the fallback. Terminals stay disabled when the server listens beyond localhost.

## Claude Code native file rewind (#200)

Mira starts Claude Code with `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=true` and
`--replay-user-messages`. Each accepted prompt's UUID is Claude's file checkpoint.
It is recorded in the sidecar with the Claude session it belongs to. On a Claude
chat, a message's **Restore files** action uses these checkpoints: a `rewind_files`
dry run lists the files first, and the user confirms. The dialog states the
coverage. Only Write, Edit and NotebookEdit changes are restored. Bash commands,
subagent edits and edits made outside Claude are not. The conversation is not
rewound; that remains a separate revert.

Before a real rewind, Mira snapshots tracked files, so **Undo** can put them back.
Claude replaces each listed file with its checkpoint copy, so later edits to those
files are replaced too; the dialog says so. Mira reports success only when Claude
reports `canRewind`, and it surfaces Claude's error otherwise (for example, "No file
checkpoint found"). Messages sent before checkpoints were enabled, messages another
engine answered, and checkpoints from a different Claude session (such as the
original chat of a fork) are reported as unavailable. A busy turn must finish or be
stopped first. An idle chat resumes its Claude session before rewinding.
