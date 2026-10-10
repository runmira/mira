import { AnimatePresence } from 'framer-motion';
import { ArrowDown, PanelLeft, SquareTerminal } from 'lucide-react';
import { Outlet } from 'react-router';
import { gitCommit, gitPush } from '../api';
import { AssistantSelectionToolbar } from '../components/AssistantSelectionToolbar';
import { ChatRelationships } from '../components/ChatRelationships';
import { CONTEXT_PANEL_RESERVE, ContextPanel } from '../components/ContextPanel';
import { EditorPicker } from '../components/EditorPicker';
import { ImageLightbox } from '../components/ImageLightbox';
import { LazyBoundary } from '../components/LazyBoundary';
import { toolPaneKindOf } from '../components/panes/toolPanes';
import { RightPanelButton } from '../components/RightPanelButton';
import { SecondOpinion } from '../components/SecondOpinion';
import { SettingsNavigationContext } from '../components/settings/SettingsContext';
import { SourceCitationNavigator } from '../components/SourceCitationNavigator';
import { TaskListPanel } from '../components/TaskListPanel';
import { Thinking } from '../components/Thinking';
import { TimelineMinimap } from '../components/TimelineMinimap';
import { ComingSoon, EmptyState } from '../components/transcript/EmptyState';
import { WorkspaceSetupCard } from '../components/WorkspaceSetupCard';
import {
  GetStarted,
  GoalPanel,
  PluginsPanel,
  PullRequestPanel,
  ReviewChanges,
  SubagentPanel,
  TerminalPanel,
} from '../lazyViews';
import { composeQuote, setAsidePassage } from '../lib/attachBridge';
import { shortcutLabelForCommand } from '../lib/keybindings';
import { lazyNamed, preloadOnIntent } from '../lib/lazy';
import { cn } from '../lib/utils';
import { goalActivity, titleFromEntries } from '../transcript/entries';
import { IS_MAC, PAGE_TITLES } from './shared';
import { WorkspaceComposer } from './WorkspaceComposer';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
const WorkspaceTranscript = lazyNamed(() => import('./WorkspaceTranscript'), 'WorkspaceTranscript');
export function WorkspaceMain(
  context: Pick<
    WorkspaceRuntime,
    | 'phone'
    | 'mainView'
    | 'toggleSidebar'
    | 'sidebarOpen'
    | 'sessionTitle'
    | 'entries'
    | 'sessionDiff'
    | 'setReviewOpen'
    | 'panelOpen'
    | 'activeAgentTab'
    | 'openToolPane'
    | 'cwd'
    | 'setSettingsSection'
    | 'openSettings'
    | 'setTerminal'
    | 'terminalOpen'
    | 'keybindings'
    | 'chatColRef'
    | 'ctxReserve'
    | 'ctxPill'
    | 'paneRef'
    | 'onPaneScroll'
    | 'noteScrollInput'
    | 'configured'
    | 'acpDriver'
    | 'isEmpty'
    | 'acpAgents'
    | 'startAcpAgent'
    | 'openAgentSettings'
    | 'sessionId'
    | 'sidebarRefresh'
    | 'attachSession'
    | 'onSend'
    | 'switchCwd'
    | 'openProjectPicker'
    | 'goal'
    | 'busy'
    | 'onClearGoal'
    | 'onSetGoal'
    | 'tasks'
    | 'historyCursor'
    | 'historyLoading'
    | 'loadOlderHistory'
    | 'historyError'
    | 'setMainView'
    | 'messageActions'
    | 'turns'
    | 'historyOffsetRef'
    | 'turnDiffs'
    | 'turnTimings'
    | 'turnUsage'
    | 'turnModels'
    | 'model'
    | 'expandedTurns'
    | 'stableToggleTurn'
    | 'stableDecide'
    | 'stablePlanReply'
    | 'stableAskUserReply'
    | 'stableOpenAgent'
    | 'stableOpenFile'
    | 'skills'
    | 'mode'
    | 'stableSetMode'
    | 'recoveryBySession'
    | 'stableFailureSettings'
    | 'status'
    | 'changeRecovery'
    | 'showSecondOpinion'
    | 'acpDriverName'
    | 'retireSecondOpinion'
    | 'runReview'
    | 'thinking'
    | 'minimapItems'
    | 'jumpToMinimapTurn'
    | 'lightbox'
    | 'setLightbox'
    | 'reviewMounted'
    | 'reviewOpen'
    | 'setReviewFocus'
    | 'refreshRepo'
    | 'reviewFocus'
    | 'showJump'
    | 'jumpToLatest'
    | 'backgroundProcesses'
    | 'setSelectedProcess'
    | 'subagentState'
    | 'gitStatus'
    | 'branchPr'
    | 'sessionCommitted'
    | 'ctxOpen'
    | 'onCtxOpenChange'
    | 'openAgentTab'
    | 'setSessionCommitted'
    | 'restoreNote'
    | 'setRestoreNote'
    | 'doRestore'
    | 'providerName'
    | 'usage'
    | 'rateLimit'
    | 'queueMessage'
    | 'queuedFor'
    | 'removeQueuedMessage'
    | 'editQueuedMessage'
    | 'reorderQueuedMessage'
    | 'steerQueuedMessage'
    | 'onSetMode'
    | 'onSetModel'
    | 'engine'
    | 'engines'
    | 'acpStatusPending'
    | 'requestAcpStatus'
    | 'acpConfig'
    | 'acpDescriptors'
    | 'onSetModelOption'
    | 'wsRef'
    | 'compactAcpAgent'
    | 'forkAcpAgent'
    | 'acpModes'
    | 'agentPostures'
    | 'setPendingAcpMode'
    | 'environment'
    | 'environments'
    | 'envSwitching'
    | 'setEnvSwitching'
    | 'setBusy'
    | 'busyRef'
    | 'setThinking'
    | 'onNewChat'
    | 'onCompact'
    | 'usageRing'
    | 'pendingApprovals'
    | 'decideApproval'
    | 'visibleApprovalRef'
    | 'ruleEditorCallId'
    | 'setRuleEditorCallId'
    | 'pendingPlans'
    | 'pendingQuestions'
    | 'secretRequests'
    | 'replyToSecret'
    | 'infoNotices'
    | 'dismissInfoNotice'
    | 'completedUpdatesRef'
    | 'setEngines'
    | 'replyToPlan'
    | 'replyToAskUser'
    | 'acpCommands'
    | 'commands'
    | 'extensionsVersion'
    | 'runPrReview'
    | 'settingsSection'
    | 'settingsHandler'
    | 'exitSettings'
    | 'skillsVersion'
    | 'githubReturn'
    | 'acpError'
  >,
) {
  const {
    phone,
    mainView,
    toggleSidebar,
    sidebarOpen,
    sessionTitle,
    entries,
    sessionDiff,
    setReviewOpen,
    panelOpen,
    activeAgentTab,
    openToolPane,
    cwd,
    setSettingsSection,
    openSettings,
    setTerminal,
    terminalOpen,
    keybindings,
    chatColRef,
    ctxReserve,
    ctxPill,
    paneRef,
    onPaneScroll,
    noteScrollInput,
    configured,
    acpDriver,
    isEmpty,
    acpAgents,
    startAcpAgent,
    openAgentSettings,
    sessionId,
    sidebarRefresh,
    attachSession,
    onSend,
    switchCwd,
    openProjectPicker,
    goal,
    busy,
    onClearGoal,
    onSetGoal,
    tasks,
    historyCursor,
    historyLoading,
    loadOlderHistory,
    historyError,
    setMainView,
    showSecondOpinion,
    acpDriverName,
    retireSecondOpinion,
    runReview,
    thinking,
    minimapItems,
    jumpToMinimapTurn,
    lightbox,
    setLightbox,
    reviewMounted,
    reviewOpen,
    setReviewFocus,
    refreshRepo,
    reviewFocus,
    showJump,
    jumpToLatest,
    backgroundProcesses,
    setSelectedProcess,
    subagentState,
    gitStatus,
    branchPr,
    sessionCommitted,
    ctxOpen,
    onCtxOpenChange,
    openAgentTab,
    setSessionCommitted,
    acpStatusPending,
    requestAcpStatus,
    extensionsVersion,
    runPrReview,
    settingsSection,
    settingsHandler,
    exitSettings,
    skillsVersion,
    githubReturn,
    acpError,
  } = context;
  return (
    <>
      <main
        className={cn(
          'flex min-h-0 min-w-0 flex-col',
          phone ? 'pt-safe px-safe' : 'py-2 pl-0.5 pr-2',
        )}
      >
        {/* The main chat surface is the app's base surface, not a card: it
            keeps the flat theme background (pure black in dark) so the
            composer and cards inside it are what read as elevated. */}
        <div
          className={cn(
            'flex min-h-0 flex-1 flex-col overflow-hidden bg-background',
            !phone && 'rounded-xl border border-border',
          )}
        >
          {/* The chat view has its own header with the drawer button; the
            other pages need one too, or a phone has no way back. */}
          {phone && mainView !== 'chat' && (
            <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border/60 px-2">
              <button
                type="button"
                onClick={toggleSidebar}
                aria-label="Open sidebar"
                className="shrink-0 rounded p-2.5 text-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
              >
                <PanelLeft className="size-4" />
              </button>
              <span className="min-w-0 flex-1 truncate text-[13.5px] text-foreground">
                {PAGE_TITLES[mainView]}
              </span>
            </div>
          )}
          {mainView === 'chat' && (
            <>
              {/* Drag region: with the native title bar hidden this row is
                the natural place to move the window, and `Overlay` needs at
                least one or the window can't be dragged at all. */}
              <div
                data-tauri-drag-region
                className="flex h-11 shrink-0 items-center gap-3 border-b border-border/60 px-4 max-md:gap-2 max-md:px-2"
              >
                {/* `data-tauri-drag-region` matches ancestors, so anything
                  interactive inside the header would drag the window on
                  mousedown. Opting these out keeps the clicks landing. */}
                <button
                  data-tauri-drag-region="false"
                  type="button"
                  onClick={toggleSidebar}
                  title={
                    phone ? 'Open sidebar' : sidebarOpen ? 'Collapse sidebar' : 'Expand sidebar'
                  }
                  aria-label={
                    phone ? 'Open sidebar' : sidebarOpen ? 'Collapse sidebar' : 'Expand sidebar'
                  }
                  className={cn(
                    'shrink-0 rounded p-1.5 transition-colors touch:p-2.5',
                    sidebarOpen || phone
                      ? 'text-foreground/70 hover:bg-accent hover:text-foreground'
                      : 'text-muted-foreground/50 hover:bg-accent hover:text-foreground',
                  )}
                >
                  <PanelLeft className="size-4" />
                </button>
                <span className="min-w-0 flex-1 truncate text-[13.5px] text-foreground">
                  {sessionTitle ?? titleFromEntries(entries)}
                </span>
                {(sessionDiff.uncommitted ?? 0) > 0 && (
                  <button
                    type="button"
                    data-tauri-drag-region="false"
                    onClick={() => setReviewOpen(true)}
                    {...preloadOnIntent(ReviewChanges.preload)}
                    title="Review this session's changes"
                    // h-8 and rounded-lg to match the icon controls beside it.
                    // It used to be a `rounded-full` py-1 pill, which read as a
                    // different kind of control because it was both shorter
                    // than its neighbours and the only fully-pill shape in the
                    // row.
                    className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-border/60 elev-card dark:bg-secondary/40 px-2 text-[12.5px] text-muted-foreground transition-colors hover:border-border dark:hover:bg-secondary hover:text-foreground"
                  >
                    <span className="font-mono text-green-400/80">+{sessionDiff.added}</span>
                    <span className="font-mono text-red-400/80">−{sessionDiff.removed}</span>
                    <span className="hidden lg:inline">Review</span>
                  </button>
                )}
                <div className="flex items-center gap-0.5" data-tauri-drag-region="false">
                  <div className="contents" {...preloadOnIntent(SubagentPanel.preload)}>
                    <RightPanelButton
                      open={panelOpen}
                      activeKind={toolPaneKindOf(activeAgentTab ?? '')}
                      onOpen={() => openToolPane('new')}
                      onOpenPane={openToolPane}
                    />
                  </div>
                  {/* Opening a local editor or a terminal means nothing from a
                    phone; the right panel's panes still work there. */}
                  {!phone && (
                    <>
                      <EditorPicker
                        cwd={cwd}
                        onOpenSettings={() => {
                          setSettingsSection('general');
                          openSettings();
                        }}
                      />
                      <div className="group relative">
                        <button
                          type="button"
                          onClick={() => setTerminal(!terminalOpen)}
                          aria-label="Toggle terminal"
                          aria-pressed={terminalOpen}
                          className={cn(
                            'flex h-8 shrink-0 items-center rounded-lg border border-transparent px-1.5 transition-colors',
                            terminalOpen
                              ? 'border-mira-blue/50 bg-mira-blue/10 text-foreground'
                              : 'elev-card dark:border-border/60 dark:bg-secondary/40 text-muted-foreground hover:bg-secondary hover:text-foreground',
                          )}
                        >
                          <span className="inline-flex size-6 items-center justify-center overflow-hidden rounded-md bg-fg/[0.04] ring-1 ring-fg/10">
                            <SquareTerminal className="size-3.5" strokeWidth={1.75} />
                          </span>
                        </button>
                        <span className="tooltip pointer-events-none absolute right-0 top-full z-30 mt-1.5 opacity-0 transition-opacity delay-300 group-hover:opacity-100">
                          Toggle terminal{' '}
                          <span className="tooltip-tag">
                            {shortcutLabelForCommand(keybindings, 'terminal.toggle') ??
                              (IS_MAC ? '⌘J' : 'Ctrl+J')}
                          </span>
                        </span>
                      </div>
                    </>
                  )}
                </div>
              </div>

              {/* Transcript — full width, panel floats above it */}
              <div className="relative flex-1 min-h-0" ref={chatColRef}>
                <div
                  className="absolute inset-0 overflow-y-auto overscroll-contain px-5 pb-5 transition-[padding] duration-200 max-md:px-3"
                  style={{
                    paddingRight: ctxReserve ? CONTEXT_PANEL_RESERVE + 12 : phone ? 12 : 20,
                    paddingTop: ctxPill ? 52 : 16,
                  }}
                  ref={paneRef}
                  onScroll={onPaneScroll}
                  onWheel={noteScrollInput}
                  onTouchMove={noteScrollInput}
                  onKeyDown={noteScrollInput}
                  onPointerDown={noteScrollInput}
                  onPointerMove={(e) => {
                    if (e.buttons) noteScrollInput();
                  }}
                >
                  {/* Agents bring their own login, so a chat on one needs no provider. */}
                  {configured === false && !acpDriver && isEmpty && (
                    <LazyBoundary>
                      <GetStarted
                        agents={acpAgents}
                        onUseAgent={(kind) => startAcpAgent(kind, null)}
                        onAddProvider={() => {
                          openSettings();
                          setSettingsSection('provider');
                        }}
                        onSetUpAgents={openAgentSettings}
                      />
                    </LazyBoundary>
                  )}
                  {configured === false && !acpDriver && !isEmpty && (
                    <div className="mx-auto mb-4 max-w-3xl rounded-lg border border-amber-500/30 bg-amber-500/[0.08] px-3 py-2 text-[13px] text-amber-200">
                      No provider configured —{' '}
                      <button
                        className="underline underline-offset-2 hover:text-amber-100"
                        onClick={() => openSettings()}
                      >
                        open Settings
                      </button>{' '}
                      to add one.
                    </div>
                  )}

                  <div className="mx-auto max-w-3xl">
                    <WorkspaceSetupCard />
                    <ChatRelationships
                      key={sessionId}
                      sessionId={sessionId}
                      refreshKey={sidebarRefresh}
                      onOpen={attachSession}
                    />
                  </div>
                  {isEmpty ? (
                    configured === false && !acpDriver ? null : (
                      <EmptyState
                        cwd={cwd}
                        onPrompt={(text) => onSend(text)}
                        onOpenSession={(id) => attachSession(id)}
                        onSwitchProject={(path) => void switchCwd(path)}
                        onNewProject={() => void openProjectPicker()}
                      />
                    )
                  ) : (
                    <div data-transcript-column className="mx-auto flex max-w-3xl flex-col gap-2">
                      {goal && (
                        <LazyBoundary>
                          <GoalPanel
                            goal={goal}
                            busy={busy}
                            activity={goalActivity(entries)}
                            onClear={onClearGoal}
                            onRestart={(condition, maxIter) => onSetGoal(condition, maxIter)}
                          />
                        </LazyBoundary>
                      )}
                      {tasks.length > 0 && <TaskListPanel tasks={tasks} />}
                      <SourceCitationNavigator
                        pane={paneRef}
                        sessionId={sessionId}
                        hasOlder={!!historyCursor}
                        loading={historyLoading}
                        loadOlder={loadOlderHistory}
                        historyError={historyError}
                        onOpenSession={attachSession}
                      />
                      <AssistantSelectionToolbar
                        pane={paneRef}
                        sessionId={sessionId}
                        onQuote={(text, turn, href) => composeQuote({ text, turn, href })}
                        onAskAside={(text) => {
                          setAsidePassage(text);
                          setMainView('chat');
                          openToolPane('aside');
                        }}
                      />
                      {historyCursor && (
                        <button
                          className="mx-auto py-3 text-xs text-muted-foreground"
                          disabled={historyLoading}
                          onClick={loadOlderHistory}
                        >
                          {historyLoading ? 'Loading older messages…' : 'Load older messages'}
                        </button>
                      )}
                      {historyError && (
                        <p role="alert" className="text-xs text-destructive">
                          {historyError}
                        </p>
                      )}
                      <LazyBoundary
                        fallback={
                          <div role="status" className="py-4 text-xs text-muted-foreground">
                            Loading messages…
                          </div>
                        }
                      >
                        <WorkspaceTranscript {...context} />
                      </LazyBoundary>
                      {showSecondOpinion && (
                        <SecondOpinion
                          agentName={acpDriverName}
                          files={sessionDiff.files.length}
                          canReview={configured !== false}
                          onReview={() => {
                            retireSecondOpinion();
                            void runReview('');
                          }}
                          onAddProvider={() => {
                            openSettings();
                            setSettingsSection('provider');
                          }}
                          onDismiss={retireSecondOpinion}
                        />
                      )}
                      {thinking && (
                        <div className="flex justify-start">
                          <Thinking />
                        </div>
                      )}
                    </div>
                  )}
                </div>

                <TimelineMinimap
                  items={minimapItems}
                  paneRef={paneRef}
                  onSelect={jumpToMinimapTurn}
                />
                <ImageLightbox src={lightbox} onClose={() => setLightbox(null)} />
                {reviewMounted && (
                  <LazyBoundary>
                    <ReviewChanges
                      open={reviewOpen}
                      onClose={() => {
                        setReviewOpen(false);
                        setReviewFocus(null);
                      }}
                      onSendComments={(text) => onSend(text)}
                      onChanged={refreshRepo}
                      focusPath={reviewFocus}
                    />
                  </LazyBoundary>
                )}

                {showJump && (
                  <button
                    type="button"
                    onClick={jumpToLatest}
                    className="absolute bottom-3 left-1/2 z-10 inline-flex -translate-x-1/2 animate-fade-in items-center gap-1.5 rounded-full border border-border bg-secondary/95 px-3 py-1.5 text-[12px] text-muted-foreground shadow-lg backdrop-blur transition-colors hover:text-foreground"
                  >
                    <ArrowDown strokeWidth={2.5} className="size-3" />
                    Jump to latest
                  </button>
                )}

                {/* Floating context panel — absolutely anchored to top-right */}
                <AnimatePresence>
                  <ContextPanel
                    key="ctx"
                    sessionTitle={sessionTitle ?? titleFromEntries(entries)}
                    tasks={tasks}
                    processes={backgroundProcesses.processes}
                    stoppingProcesses={
                      new Set(
                        [...backgroundProcesses.stopping]
                          .filter((key) => key.startsWith(`${sessionId}:`))
                          .map((key) => Number(key.split(':').at(-1))),
                      )
                    }
                    onOpenProcess={(id) => {
                      setSelectedProcess({ session: sessionId, id, nonce: Date.now() });
                      openToolPane('processes');
                    }}
                    onStopProcess={backgroundProcesses.stop}
                    subagentState={subagentState}
                    entries={entries}
                    gitStatus={gitStatus}
                    sessionDiff={sessionDiff}
                    branchPr={branchPr}
                    sessionCommitted={sessionCommitted}
                    open={ctxOpen}
                    onOpenChange={onCtxOpenChange}
                    onOpenAgent={openAgentTab}
                    onReview={(path) => {
                      setReviewFocus(path ?? null);
                      setReviewOpen(true);
                    }}
                    onPush={async () => {
                      await gitPush();
                      refreshRepo();
                    }}
                    onCommit={async (message, includeUnstaged, pushAfter) => {
                      await gitCommit({
                        message,
                        include_unstaged: includeUnstaged,
                        push_after: pushAfter,
                      });
                      setSessionCommitted(true);
                      refreshRepo();
                    }}
                  />
                </AnimatePresence>
              </div>

              <WorkspaceComposer {...context} />
              {terminalOpen && !phone && (
                <LazyBoundary>
                  <TerminalPanel onClose={() => setTerminal(false)} />
                </LazyBoundary>
              )}
            </>
          )}

          {mainView === 'plugins' && (
            <div className="flex-1 min-h-0 overflow-y-auto">
              <LazyBoundary>
                <PluginsPanel version={extensionsVersion} />
              </LazyBoundary>
            </div>
          )}

          {mainView === 'pull-request' && (
            <LazyBoundary>
              <PullRequestPanel onOpenSettings={() => openSettings()} onReviewPr={runPrReview} />
            </LazyBoundary>
          )}
          {mainView === 'scheduled' && <ComingSoon label="Scheduled" />}
          {mainView === 'settings' && (
            <LazyBoundary>
              <SettingsNavigationContext.Provider
                value={{
                  section: settingsSection,
                  onSectionChange: setSettingsSection,
                  onSaved: settingsHandler,
                  onExit: exitSettings,
                  skillsVersion: skillsVersion,
                  githubReturn: githubReturn,
                  acpAgents: acpAgents,
                  acpRefreshing: acpStatusPending,
                  acpDriver: acpDriver,
                  acpError: acpError,
                  onAcpRefresh: requestAcpStatus,
                  onAcpStart: startAcpAgent,
                }}
              >
                <Outlet />
              </SettingsNavigationContext.Provider>
            </LazyBoundary>
          )}
        </div>
      </main>
    </>
  );
}
