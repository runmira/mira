# Issue #187: frontend state migration

PR #190 implements the frontend migration on one branch. This document supersedes
its earlier phase inventories. The architecture proposal in `frontend-state.md`
remains separate. Backend contracts, replay algorithms and routing are retained.

## Current ownership

- The backend/account scoped runtime owns session projections: transcript,
  turn/work status, model/mode/environment, usage, history metadata, requests,
  subagents, tasks, goals and recovery. Session activity is an atomic projection.
- TanStack Query owns supported HTTP snapshots: settings, engines, skills,
  commands, sessions, context, git status, session diff and branch PR. Cache keys
  include backend/account and applicable session, cwd, engine/model and environment.
- Preferences and shared workspace UI use scoped stores. Versioned persistence
  explicitly allowlists preferences and panel visibility. Server defaults remain
  server owned; requests, secrets and transcripts are not persisted in these stores.
- Composer drafts use account/backend/session scoped rich-draft persistence.
  Isolated form state, quotes, DOM interactions and unsaved settings stay local.
- React Router owns navigation. Socket, timers, observers, request bookkeeping,
  buffers and scroll resources live in disposable services, outside render state.

The runtime is created in provider effect setup and disposed during cleanup.
Strict Mode recreates resources. Logout/account replacement cancels queries and
transport work, clears projections, and rejects stale auth validation callbacks.
Capabilities are cached in the runtime by driver rather than unscoped storage.

## Runtime and recovery

Named commands update projections; typed field reducers preserve structural
sharing. Transitions commit buffered text and structural facts atomically before
notifications or other effects. Stream buffers capture session and generation,
flush once per animation frame, and have a bounded timer fallback when hidden.
Tool, request and terminal events flush text first. Stop preserves final buffered
text and late terminal events cannot restart a stopped turn.

Session selection exposes retained cached projections while awaiting authoritative
Ready. Retention limits idle sessions to 20, protecting running work, pending
requests and queued inputs. Ready/history keep the existing reconstruction and
pagination helpers. History responses require matching session, request and
attachment generation. Native subagent review state survives navigation.

Decisions remain actionable until server resolution. Submitting disables duplicate
clicks; failure, timeout or disconnect restores retryable UI. Resolution tombstones
and input IDs prevent replayed requests or queue snapshots reopening actions.
Diagnostics are bounded, development-only metadata and omit content and credentials.

Normal sends use the existing durable server QueueInput outbox. The editor retains
its draft until authoritative acceptance; acceptance cannot erase newer typing.
Unconfirmed rows are retained by stable ID, never deduplicated by text. Side effects
are not automatically replayed after uncertain delivery or reconnect.

Query reads classify transient retries, reject obsolete attachment responses,
coalesce refreshes and cancel superseded snapshots. Settings writes are serialized
and reconcile authoritative responses; unsaved forms detect remote conflicts.
Contextual requests are disabled until attachment is ready.

## Rendering and validation

Application reads transcript outlines and stable metadata; live text subscribes
inside transcript/right-panel boundaries. Composer and context-panel boundaries
exclude streamed text. Elapsed time ticks only where visible timing is rendered.
The render probe sends 100 separate streaming updates: transcript renders 100
additional times, shell and composer render zero additional times. Switching to a
same-shaped transcript still updates the selected session correctly.

Consolidated frontend suite: 76 tests passing. Related native-stream, tool/turn,
queue and rich-draft helper suites: 26 tests passing. TypeScript, production build,
module limits and whitespace checks pass. Tests cover scopes and Strict Mode,
transport attachment/reconnect, stale history/query/auth responses, atomic Stop,
request acknowledgement, duplicate submission, draft retention, persistence,
settings serialization, scheduler cleanup and render isolation.

A fresh comparable baseline from ecf39b9 measures 401.2 kB initial gzip. The
migration measures 423.6 kB, versus the unchanged 400 kB budget.
The size gate fails; its limit has not been raised. Exact final output is recorded
in the PR validation notes.

## Remaining acceptance limits

Desktop UI verification is deferred because authentication configuration is server
side. Live multi-session navigation, provider restart, reconnect and permission
flows still require testing in the configured application. Automated tests do not
establish native desktop performance or live authentication behavior.

The socket watches one attached session. Legacy token/native terminal frames lack
explicit session/stream cursors; client generation fences cannot identify a server
frame emitted with incorrect identity after Ready. No exactly-once/background
transcript streaming guarantee is added. Repository HTTP endpoints still target
the server's active pointer: client attachment fences prevent stale client writes,
but cross-window server-pointer isolation requires a backend contract change.

The PR stays draft and issue #187 stays open pending the bundle gate and live
acceptance checks. Implementation is consolidated; these limitations must remain
visible when evaluating completion.

## Transcript scroll stabilization

The follow-up fixes use one transcript scroll owner. Browser anchoring is disabled
for the virtual pane; measured height changes and older-history insertion retain
a stable row ID and offset from the actual scroll position. Explicit scroll input
suspends following immediately; reaching the actual end or Jump to latest can
resume it. Disclosure toggles pin their visible header, or keep the end visible
when following there. Observer subscriptions depend on mounted row identity, not
streamed content, and remounted tool rows do not replay entrance fades.

Compared with the local T3 implementation in `MessagesTimeline.tsx` and
`work-log/scrollAnchor.ts`, the same separation of end following, reading anchors
and disclosure settling is used without replacing Mira's virtual list implementation.
Regression coverage includes fast scroll before resize, height changes above the
viewport, history prepend, disclosure geometry, observer stability and disposal.
The frontend suite now passes 82 tests. Native scrolling and streaming remain
manual acceptance checks; simulated DOM geometry does not establish native paint
or momentum-scroll behavior.

## Visible notices and stream alignment

Lifecycle notices (including manual turn-end notices), environment/context changes,
goals and render cards remain outside the collapsed work disclosure. Notices are
rendered underneath the reply in server order; status messages use the independent
horizontal divider pattern from T3's TimelineSystemDivider. Fatal errors retain
their existing visible error/retry controls.

Iconless assistant messages and duration rows begin at the same left edge as the
existing tool/thinking icons. No new agent-message or duration icons are added.
Inline reasoning owns its own gutter, preventing message padding from doubling
its inset. The alignment preview now includes inline reasoning and an actual
collapsed completed turn. Browser measurements confirm equal left coordinates
for prose, duration text and the existing icon rail. The suite passes 86 tests,
including notice visibility/order and inline reasoning padding regressions.

Only the final assistant reply renders its timestamp/actions. Intermediate replies
have no secondary row. Compact thinking/tool headers use a 28px row height with
14px icon boxes; thinking has no extra outer margins. Iconless prose keeps its
original left edge, and segment spacing is 8px. Successful tool status marks are
quiet outlines rather than filled color dots. Browser measurements cover the
standalone and embedded reasoning paths as well as grouped and individual tools.

## Bounded thinking and live previews

Thinking content is capped at 240px in both streaming and completed states, with
contained vertical scrolling. Live content follows the bottom until the reader
scrolls away. Overflow masks reflect the current top/bottom position instead of
hiding short content. Expanded reasoning renders plain text with preserved line
breaks, avoiding a full Markdown reparse on every token.

Collapsed live thinking shows the newest nonempty line, updating tokens in place,
scrolling horizontally to reveal the tail, and using a 300ms entrance on newline
changes. The preview stays one row high. Opening thinking hides the preview;
completion leaves the standard Thought/duration header. Tool and thinking labels
share ZCode's four-second gradient sweep, mapped to Mira's theme and reduced-motion
settings. Source attribution is in THIRD-PARTY-NOTICES.md.

Validation: 89 frontend tests pass, plus TypeScript/build/module checks. Browser
measurements confirm a 240px body with overflow content, continued bounded height
after completion, and a four-second activity sweep. Tests cover latest-line identity,
preview scrolling/observer disposal, manual reading retention and completed caps.
Initial gzip remains 423.4kB against the unchanged 400kB limit.
