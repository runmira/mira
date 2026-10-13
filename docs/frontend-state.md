# Frontend state architecture

Status: proposed implementation plan. No runtime migration is included in this document.

## Goal

Make Mira's shared frontend state explicit, predictable, and cheap to subscribe to. A streaming chat must not rerender the entire workspace, switching chats must not lose pending work, and reconnecting must reconcile with the server without duplicating messages or decisions.

Use TanStack Store for shared client/runtime state and TanStack Query for fetched backend resources. Keep React Router as the navigation owner. Keep component-local state in React where no other component needs it. This is a state migration, not a router, backend, or transcript-format rewrite.

Success means observable improvements in ownership, correctness, and rendering. Replacing `useState` calls by itself is not success.

## Existing implementation

The main implementation lives under `crates/mira-server/frontend/src/`:

- `app/useWorkspaceState.ts` collects workspace state and mutable refs.
- `app/useWorkspaceEffects.ts` owns startup, subscriptions, and resource effects.
- `app/useWorkspaceView.ts` connects runtime state to rendered components.
- `app/useAgentEvents.ts`, `useSessionEvents.ts`, `useTranscriptEvents.ts`, and `useApprovalEvents.ts` apply server messages through setters and refs.
- `app/useAppNavigation.ts` already states that the URL owns navigation.
- `transcript/entries.ts` and `transcript/replay.ts` implement transcript updates and history reconstruction.

The four workspace/agent hooks above total approximately 2,620 lines at proposal time. State includes both durable server facts and ephemeral resources. Existing code also handles meaningful races: proposals arriving before tool starts, prompts resolved from another window, late completion events after Stop, buffered tokens, and session history responses. Preserve these behaviors explicitly rather than flattening them into a generic store.

## Ownership rules

Every field has one authoritative frontend owner. Components may derive views, but must not maintain a second writable copy.

1. **Runtime store:** live session projections, transcript entries, turn status, pending requests, queued-message projections, usage, and session-specific composer drafts.
2. **Query cache:** resource snapshots such as session lists, engine catalogs, settings, skills, commands, and repository resources fetched through the API. Existing WebSocket-only resources remain in their current owner until a supported snapshot/reconciliation path is identified; do not invent an HTTP API just to use Query.
3. **Preferences store:** shared UI preferences and optimistic views of backend-persisted provider/model defaults. Existing backend persistence remains authoritative for saved engine/model selection.
4. **Workspace UI store:** shared panel visibility, selected panel tabs, and other cross-component interaction state.
5. **Router:** route and settings section. Do not mirror `mainView` into a writable store.
6. **Local React state:** hover, temporary menus, isolated form edits, and component-only disclosure state.
7. **Runtime service:** sockets, abort controllers, timers, animation-frame handles, token buffers, DOM references, and transport bookkeeping. These are resources, not renderable store data.

The server remains authoritative for permissions, queue acceptance, running work, persisted history, and resolved decisions. Changing a client store must never grant a permission or imply that a command reached the server.

## Store structure and lifetime

Create one runtime through a factory at the application root, scoped to the current backend/account connection. Provide it through a stable React context. Do not use an unscoped module singleton: tests, logout, backend switching, and multiple roots must get independent runtimes.

Suggested layout:

```text
src/state/
  createAppRuntime.ts       # constructs stores, Query client, and service
  RuntimeProvider.tsx      # exposes a stable runtime instance
  runtimeTypes.ts          # domain state, keys, events, commands
  runtimeReducer.ts        # pure event -> next state
  runtimeStore.ts          # store creation and event application
  runtimeSelectors.ts      # narrow, stable selectors
  workspaceUiStore.ts
  preferencesStore.ts
  resourceKeys.ts          # Query key factories
  resources/              # resource-specific query/mutation options
  persistence.ts          # allowlisted, versioned UI persistence
  transport/              # connection lifecycle and message adaptation
```

Start with one runtime store containing session-keyed state. This permits atomic updates across related transcript, turn, and approval fields. Separate low-frequency UI/preferences stores so token updates cannot wake those subscribers. Split the runtime further only when measurements justify it and cross-store consistency is addressed.

Conceptual shape, not a new wire protocol:

```ts
type RuntimeState = {
  connection: ConnectionState;
  activeSessionKey: SessionKey | null;
  sessions: Record<SessionKey, SessionProjection>;
};

type SessionProjection = {
  transcript: TranscriptProjection;
  turn: TurnProjection;
  requests: PendingRequestProjection;
  queue: QueueProjection;
  composer: ComposerDraft;
  history: HistoryLoadState;
};
```

Use existing domain types wherever possible. A session key must include the backend/account scope where IDs can overlap. Selecting a session changes the active key; it does not overwrite a global transcript.

Bound retention. Keep active sessions, sessions with ongoing work or unanswered requests, and a measured recent-session cache. Evict only idle projections that can be reconstructed from authoritative history. Do not let an ever-growing session map retain every transcript for the lifetime of the desktop app.

## Events and commands

Components subscribe through selectors and invoke named commands such as `selectSession`, `sendMessage`, `stopTurn`, or `resolveApproval`. Do not export arbitrary setters as the public application API.

Use the flow:

```text
component command -> runtime service -> existing transport
server message -> transport adapter -> domain event -> pure reducer -> store
resource response -> Query cache -> subscribing component
```

The reducer performs no network access, persistence, notifications, clock reads, or DOM work. Supply timestamps as event inputs where required. Side effects run in an explicit service path after the state transition, once per applicable event. Reuse existing transcript helpers; extract any ref mutations from updater callbacks first.

Represent connection state separately from turn state. A disconnected socket does not prove that a server-side turn stopped. Model sending/acknowledgement/recovery independently from running/thinking/awaiting-input/completed states, using existing protocol facts rather than speculative statuses.

When one message affects several fields, commit one coherent state transition. Subscribers must never see a completed turn with an undrained final token buffer, or a resolved approval still presented as actionable.

## Streaming and event correctness

- Buffer token deltas in the runtime service, keyed by session and turn/stream identity available from the protocol. Flush at most once per animation frame while visible; define a bounded fallback for hidden windows where animation frames pause.
- Flush before structural boundaries such as tool starts, turn completion, Stop reconciliation, and history replacement. Cancel scheduled work on disposal. A callback scheduled for chat A must never append into chat B.
- Preserve ordering within a stream. Do not coalesce approval, tool, error, or terminal events as if they were text.
- Associate events with the session identity actually supplied by the transport. Inspect current `ws.ts` and `ServerMsg` routing before implementing. If identity is connection-scoped, bind the connection to an immutable session/generation context rather than reading the currently selected session at callback time.
- Use stable call/request IDs for upserts and reconciliation. Do not deduplicate raw text deltas by content: repeated words may be legitimate.
- Do not claim exactly-once replay without protocol support. Document available cursors/IDs and how reconnect snapshots reconcile. Any required new server sequence/acknowledgement contract is a separate backend change.
- Retain the existing proposal-before-tool-start staging behavior, scoped to its session. Replayed requests and resolutions must converge on the same visible state.
- Keep a resolved-request tombstone or equivalent authoritative reconciliation rule where needed so a late duplicate request cannot reopen a decision. Bound its retention using known replay boundaries.

## Session switching and history

Give every history load a session key, connection generation, and request identity. Abort superseded requests when supported and reject stale results even if cancellation loses the race.

History paging must preserve stable entry identities, scroll anchors, tool attachments, usage, turn timing, and the existing `interleaveReplay` behavior. Specify whether a response replaces a snapshot or prepends a page; never blindly append replayed history to live entries. Reuse the current replay functions before considering normalization.

Switching away from a running session leaves its projection alive. Switching back displays its current state. Per-session drafts, attachments, queued messages, and unanswered requests must survive navigation. Backend/account switching disposes the old transport and clears scoped caches before showing the new account's data.

Do not introduce a new frontend replay algorithm and a new storage layout in the same migration phase. Keep the existing entry array initially. Normalize turns/tools or subscribe at turn granularity only if profiling establishes a need and stable identity is preserved.

## Approvals, questions, and queued messages

An approval click enters a submitting state; resolution comes from the server response/event. Disable duplicate submissions while in flight, expose a retryable failure, and reconcile decisions made in another window. Closing a dialog does not resolve the underlying request.

Preserve explicit permission rules and provider-specific approval semantics. The frontend store never evaluates itself into an allow decision. Secrets remain transient and must not enter persistence or diagnostics.

Queued messages retain current durable backend semantics. Use existing `QueueMutations` behavior during migration. Client command IDs are not a substitute for server-supported idempotency: uncertain delivery requires reconciliation, not automatic resend. Stop and retry must not silently clear drafts or resend previously accepted messages.

## Query integration

Define keys centrally with every identity that changes the result: backend/account, workspace/repository, session, engine, and relevant parameters. Do not use a generic `['settings']` or `['git']` key across multiple scopes.

Each resource declares stale time, retention, retry, cancellation, and refresh triggers. Audit Query defaults explicitly. Reads may retry bounded transient failures; side-effecting commands must not automatically retry without verified idempotency. Authentication, permission, and invalid-input errors should not be retried as transient network failures.

WebSocket resource changes either patch the authoritative Query entry with a complete supported update or invalidate the affected key. Choose one strategy per event/resource. Coalesce repeated invalidations so a tool burst does not produce a refetch burst. Prevent an older in-flight snapshot from overwriting a newer event update using cancellation plus supported revision/reconciliation rules.

Query owns fetched data and request status; stores hold selected IDs and interaction state. Do not copy complete query results into a store. Settings forms may keep an explicit unsaved draft, with save/reset/conflict behavior distinct from the saved resource.

Optimistic updates are suitable for reversible UI choices. Permission decisions, running-state transitions, and command delivery must retain authoritative acknowledgement semantics. Scope rollback to the operation so a failed old mutation cannot overwrite a newer successful change.

## Rendering contract

Use narrow selectors for exactly what a component renders. The composer should not subscribe to transcript entries. The sidebar should subscribe to activity/title summaries, not token buffers. An approval card should select its request, not the entire session map.

Selectors must preserve reference identity when their inputs do not change. Avoid returning a fresh object/array on every store notification. Use the equality/memoization mechanisms supported by the pinned library version, or primitive/reference selections with memoized derivation. Do not assume APIs from an older TanStack Store tutorial.

Use immutable updates with structural sharing; never mutate a Map, array, or nested object that a subscriber already holds. Derived totals and display labels should be selectors rather than independent state that can drift.

Keep React Compiler enabled and validate compatibility. A store does not replace thoughtful component boundaries, token batching, or transcript virtualization.

## Persistence and diagnostics

Persist only an explicit allowlist: appropriate UI preferences and, if existing product behavior requires it, scoped drafts. Use versioned schemas, validation, migrations, and safe fallback for malformed data. Debounce draft writes; never write per token.

Do not persist transcripts, approvals, secrets, credentials, transport handles, or Query caches wholesale. Backend-owned provider/model defaults continue through existing APIs. Hydration must not overwrite a newer server value or briefly select a different provider that then starts a thread.

Development diagnostics may record event type, scope, timestamps, and transition counts. Redact content, tool arguments, prompts, secrets, and tokens by default. Prefer bounded metadata logs to a full event journal. Dispose subscriptions/timers cleanly under React Strict Mode, hot reload, logout, and window close.

## Migration sequence

Each phase should be independently reviewable and leave a working app. No phase should maintain two writable owners for the same field.

1. **Baseline and ownership audit.** Inventory fields from `useWorkspaceState`, classify owner/scope/persistence, trace transport identity and replay contracts, and capture representative render/interaction measurements. Record existing behavior before changing it.
2. **Runtime foundation.** Add pinned TanStack dependencies, runtime factory/provider, explicit disposal, narrow selector conventions, and isolated tests. Prove selector isolation and library compatibility before adopting it throughout the app.
3. **Shared UI and preferences.** Move a small cross-component slice first. Preserve provider/model persistence and startup behavior. Retain local form/disclosure state where appropriate.
4. **One session runtime slice.** Move turn/activity state and its event handlers together. Temporary adapters may expose old hook signatures, but delegate to the sole store owner and must have a removal plan.
5. **Transcript and requests.** Move transcript updates, token buffers, history/replay, approvals, and queue integration in coherent slices. Preserve existing helpers and protocol. Validate switching/recovery after each slice.
6. **Fetched resources.** Introduce Query resource by resource with scoped keys, mutations, invalidation, and explicit defaults. Remove the replaced fetch/loading/refresh state in the same change.
7. **Finish and measure.** Remove compatibility adapters, obsolete mirrored refs, broad workspace subscriptions, and unused effects. Compare baseline and migrated behavior and bundle size. Update the ownership inventory to the final implementation.

Keep rollback available through small commits/PRs. Avoid a long-running dual-write mode. If a temporary rollout flag is necessary, choose one complete owner at runtime, never both, and schedule its removal.

## Required verification

Test invariants and user outcomes, not every setter wrapper:

- Reducer fixtures cover tool-start/result ordering, proposal-before-tool-start, duplicate request/resolution, late completion after Stop, and external-agent/native-provider events.
- Fake scheduler/transport tests verify token ordering, final-buffer flush, hidden-window fallback, connection-generation rejection, and cleanup with no duplicate subscription.
- History tests cover stale responses, paging during live streaming, reconnect reconciliation, stable call identities, and retained scroll anchors.
- Command tests cover uncertain delivery, double clicks, failed approval submission, resolution from another window, and queue reconciliation without duplicate sends.
- Query tests cover scope isolation, invalidation bursts, old fetch/new event races, logout cache clearing, and mutation rollback ordering.
- Persistence tests cover invalid/old schemas, account scope, server hydration precedence, and absence of sensitive data.
- Render tests/profiling verify that token updates do not rerender the composer, unrelated session, settings form, or sidebar when their selected values are unchanged.
- Real UI checks cover switching away from/back to a running chat, two simultaneous sessions, offline/reconnect, approvals, draft restoration, and provider selection after desktop restart. Check native and external providers, including Codex.

Run the frontend's existing `npm test`, `npm run build`, `npm run check:modules`, and bundle-size check. Compare the same build configuration before/after; do not hide an existing budget failure or raise the budget to make the migration pass. A passing reducer test or build does not establish desktop recovery behavior.

Acceptance requires no lost/repeated transcript content, no cross-session leakage, no phantom approval resolution, preserved provider/draft behavior, bounded retention, and demonstrable subscription isolation. Record actual measurements and remaining limitations rather than promising that a library automatically makes the app faster.

## Implementation references and open decisions

- [TanStack Store React quick start](https://tanstack.com/store/latest/docs/framework/react/quick-start): current documentation exposes `createStore` and `useSelector`; verify the installed release's exports and types before implementation. The docs currently label Store v0, so pin the tested version and isolate its integration.
- [TanStack Query overview](https://tanstack.com/query/latest/docs/framework/react/overview): Query is the server-resource cache, not the live runtime's event log.

Resolve these during phase 1: exact transport session identity; replay cursor/duplicate guarantees; which repository resources support snapshots; backend switching/account lifecycle; existing draft persistence; and representative performance baselines. These are implementation discovery tasks, not reasons to replace established server behavior.

The first implementation PR should deliver the ownership inventory, tested runtime foundation, and one small shared UI slice. It should not migrate streaming until the scope and reconciliation contracts are written down.
