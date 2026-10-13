# Issue #187: first migration phase

This inventory describes the current source, not completed session migration.
The separate `frontend-state.md` proposal remains the overarching plan.

## Ownership inventory

Each name below was a field from the baseline `app/useWorkspaceState.ts`.
`ctxOpen` now belongs to the UI store; `sessionActivity` and `completedSessions`
now belong to the activity projection. Other fields retain their hook owner. Runtime session fields are
currently mostly attached-session state; the target is scoped session projections.
`status` and `sessionActivity` are connection/backend scoped; `completedSessions`,
`queuedBySession`, and `recoveryBySession` already aggregate sessions.

### Runtime projections — session unless noted

`status`, `sessionActivity`, `completedSessions`, `sessionId`, `queuedBySession`, `sessionTitle`, `model`, `mode`, `cwd`, `entries`, `historyCursor`, `historyLoading`, `historyError`, `turnDiffs`, `acpModes`, `acpConfig`, `acpCommands`, `acpUsage`, `acpLimits`, `providerContext`, `engine`, `acpError`, `turnTimings`, `busy`, `thinking`, `subagentState`, `usage`, `cacheMiss`, `rateLimit`, `tasks`, `goal`, `environment`, `environments`, `envSwitching`, `recoveryBySession`, `turnUsage`, `turnModels`, `secretRequests`.

### Fetched resources — Query candidates, retained in hooks in this phase

`engines`, `acpAgents`, `acpStatusPending`, `configured`, `providerName`, `gitStatus`, `sessionDiff`, `branchPr`, `skills`, `commands`.

### Workspace UI — context visibility migrated; others retained in hooks

`pickerOpen`, `paletteOpen`, `inspectOpen`, `importOpen`, `reportOpen`, `panelFilePickerOpen`, `sidebarOpen`, `drawerOpen`, `agentTabs`, `fileTabs`, `toolTabs`, `activeAgentTab`, `rightPanelWidth`, `reviewPanelOpen`, `ctxOpen`, `reviewOpen`, `reviewFocus`, `lightbox`, `terminalOpen`, `selectedProcess`.

### Interaction or unsaved workflow state — retain React owner until coherent slice migrates

`ruleEditorCallId`, `approvalRules`, `infoNotices`, `pendingAcpMode`, `expandedTurns`, `githubReturn`, `reviewState`, `sessionCommitted`, `chatColWidth`, `restoreAsk`, `restoreNote`, `secondOpinionSeen`, `showJump`.

### Refresh/event counters — migrate with the resource or service they drive

`sidebarRefresh`, `extensionsVersion`, `skillsVersion`, `browserPing`.

### Runtime service resources and bookkeeping

These refs remain in hooks until their corresponding commands/events migrate.
DOM refs, timers, buffers and sockets must never enter renderable/persisted state.
Projection mirrors must be removed with their store migration, rather than dual-written.

`pingPrimedRef`, `activityTitlesRef`, `sessionIdRef`, `queuedBySessionRef`, `pendingSteerRef`, `approvalStarted`, `historyRequestRef`, `historyPreviewsRef`, `historyOffsetRef`, `providerTurnCursorRef`, `acpStopRef`, `capsDriverRef`, `engineRef`, `acpAgentsRef`, `completedUpdatesRef`, `turnStartRef`, `nativeWorkRef`, `nativeTurnsRef`, `busyRef`, `thinkingIdleTimerRef`, `drawerTouchX`, `drawerRef`, `chatShortcutRequest`, `shortcutEntriesRef`, `chatColObserver`, `repoSeqRef`, `wsRef`, `queueMutationsRef`, `paneRef`, `pendingProposalsRef`, `pendingAskUserRef`, `followRef`, `readingAnchorRef`, `lastScrollInputRef`, `chatTitleRef`, `tokenBufRef`, `tokenRafRef`, `usageRef`, `turnBaseRef`, `nativeFramesRef`, `nativeRafRef`, `visibleApprovalRef`, `browserShownFor`, `browserPingSeen`.

`mainView`, `settingsSection`, and their navigation commands belong to React
Router (`useAppNavigation`). `phone`, `ctxFits`, and `keybindings` are existing
external hooks, not additional store-owned fields. The unnamed `setNowTick`
state is a rendering clock and should move to a narrow timing subscriber later.

## Persistence and scope

The first slice is `ctxOpen`: one primitive subscription and one named command.
It keeps the existing browser-wide `mira.context.open` boolean encoding (`1`
means open; other/missing values mean closed). Storage denial falls back safely.
No new persistence schema is introduced; migrating other preferences requires
an allowlisted versioned schema and explicit legacy migration.

Sidebar and terminal restore preferences stay in their current owners. Provider
and model defaults remain backend owned; this phase does not change startup
selection. `lib/composerDrafts.ts` persists text with a 400 ms debounce, 14-day
expiry and 40-entry cap; rich attachments are cached by composer logic.
Existing draft keys use session IDs, not backend/account identity. Scope that
cache before adopting it into a multi-account runtime.

AuthGate supplies the validated user ID and backend origin. Each provider
creates a fresh runtime and Query client. Account changes clear the old cache;
sign-out unmounts the provider. Pairing is above AuthGate; the transport URL
currently uses `location.host`. In-place backend switching is not an exposed
workspace path. Any future host switch must supply a different provider scope.
No module singleton or credentials enter the new runtime.

## Transport and reconciliation contract

`ws.ts` connects to same-origin `/ws`. `attach` changes the server connection's
attached slot and process-wide active pointer. `ready.session_id` identifies
its snapshot. Token, reasoning, tool and legacy approval messages generally
lack session identity. Do not route these by whichever session is selected
when a deferred callback fires. A later migration must bind them to an
immutable attached-session/generation context and define attach boundaries.

Reconnect sends attach (or sync), waits for the expected Ready, and suppresses
other session messages except activity snapshots. Ordinary attach does not
currently use that reconnect gate. The transport buffers ordinary disconnected
sends; `sendImmediate` bypasses replay for conditional mutations. Do not add
automatic mutation retries. Runtime work/request records carry provider/native
identities, but there is no global token cursor or exactly-once replay guarantee.
Activity snapshots use epoch/revision; queue and history messages carry session
IDs and request IDs. These guarantees are different and cannot be generalized.

`useSessionEvents` rejects history pages with mismatched session/request IDs,
prepends older pages, and reconstructs Ready snapshots with existing replay
helpers including `interleaveReplay`. Proposal/ask-user staging uses call IDs.
Preserve these helpers and Stop/late-completion behavior in subsequent phases.
The current token buffer is global to the workspace and drained before most
structural events in `Application`; it is intentionally untouched here.

## Resource discovery and Query boundary

Existing API snapshots include sessions, engines, skills, commands, settings,
git status, session diff and branch PR. Several repository HTTP helpers target
the server's active session implicitly. Adding scoped Query keys alone would
not make those requests race safe: establish explicit request identity and
snapshot reconciliation first. Existing `repoSeqRef` guards stale refreshes.
WebSocket-only usage, approvals, live work and environment projections stay out
of Query until a supported snapshot path is established.

The new Query client retries neither reads nor mutations by default. Resource
options must explicitly classify retryable errors. Defaults: 30-second stale
time, five-minute retention, no window-focus refresh. No resources have migrated
in this phase, so this is a tested lifecycle foundation, not a new fetch owner.
Keys include backend/account plus resource and caller-supplied identity.

## Selector and lifecycle contract

Pinned integration: react-store 0.11.2 and react-query 5.104.2. Installed Store
exports `createStore`, `get`, `subscribe` (returning an unsubscribe object),
`setState`, and `useSelector`. Select primitives or stable references; use the
installed hook's `compare` option when necessary. Public UI access is read-only
with named commands; no arbitrary application setter is exported.

Provider resources are created in effect setup and disposed in cleanup so
Strict Mode creates a fresh instance on replay. Disposal is idempotent, cancels
queries, clears caches and blocks stale UI commands. Transport remains owned
by existing workspace effects; there is no second socket.

The temporary `setCtxOpen` hook property delegates directly to the sole store
command. Remove it when panel consumers subscribe directly. Application still
subscribes to visibility, and streaming still drives its existing broad render
path. No whole-workspace performance improvement is claimed from this slice.

## Baseline and verification

Before changes: 44 frontend tests passed; module check passed. Existing built
assets reported 400.7 kB initial gzip versus the unchanged 400 kB budget.
A fresh baseline build passed TypeScript but failed emitting assets with ENOSPC.
The incomplete generated dist was removed to recover space before installing
dependencies. This means the old size measurement is an existing-artifact
baseline, not a successful comparable fresh build.

New isolated React probe: 100 unrelated store updates cause zero extra renders
for a primitive selector; changing the selected value causes one. Tests cover
independent runtimes/accounts, cache keys, cancellation, disposal, malformed or
unavailable storage, Strict Mode replay, scope replacement and unsubscription.
These synthetic measurements prove the installed integration contract, not
native desktop performance. Live streaming, reconnect, simultaneous sessions,
provider restart and full workspace profiling remain later-phase verification.

Final local checks: 48 tests passed, TypeScript passed, module limits passed,
and diff whitespace checks passed. Production asset emission again failed with
ENOSPC. The size check could not run against the incomplete output because its
manifest was not emitted. The budget remains 400 kB. No desktop build/install or
live UI verification was performed. The full issue remains open.

## Activity slice on the same PR branch

`state/sessionActivityStore.ts` reuses `applySessionActivity` in a pure reducer.
Running sessions and completion revisions commit atomically. Duplicate snapshots
retain references; revision-only changes preserve the running-set reference.
Completed revisions retain the existing 500-entry cap. A server epoch change
clears previous-epoch completion markers rather than comparing incompatible
revision namespaces. Account runtimes isolate the projection; disposal blocks
late updates. No activity data is persisted.

The old activity state, setter, mirrored ref and completion setter are removed
from `useWorkspaceState`. `useSessionCommands` delegates its existing event
command to the store and issues completion notifications only for the returned
new transitions. Notifications remain outside the reducer. `WorkspaceSidebar`
subscribes directly to running/completed projections, so activity notifications
no longer rerender Application and its command/event hooks.

`busy`, `thinking`, native turn/work refs, token buffers and transcript state
remain in their existing owners. Moving them requires the attached-session and
connection-generation contract described above. This slice begins the activity
phase; it does not complete the turn or streaming migration.

Updated validation: 53 frontend tests pass. TypeScript, production build,
module checks and diff checks pass. The activity selector React probe sees no
extra renders across 99 revision-only snapshots and a UI preference change,
and one render when the selected running set changes. Reducer tests cover
atomic completion, duplicate/stale frames, reconnect snapshots, epoch restart,
structural sharing, retention, disposal and account isolation.

A fresh baseline build from commit ecf39b9 using the same Vite configuration,
build environment and dependency installation passes and measures 401.2 kB
initial gzip. Final branch measures 413.5 kB (+12.3 kB). Both fail the unchanged
400 kB size budget. These fresh results supersede the earlier existing-artifact
measurement and ENOSPC limitation. Desktop UI checks are unavailable because
auth configuration is server-side; no native recovery behavior is claimed.
