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
