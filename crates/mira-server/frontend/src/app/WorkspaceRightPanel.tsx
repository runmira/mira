import { TRAFFIC_LIGHT_INSET, hasHiddenTitleBar } from '../lib/desktop';
import { ReviewChanges } from '../lazyViews';
import { useWorkspaceUiField } from '../state/RuntimeProvider';
import { useSelector } from '@tanstack/react-store';
import { useEffect,useMemo,useState } from 'react';
import { LazyBoundary } from '../components/LazyBoundary';
import {
ActivityPane,
AsidePane,
DevicesPane,
PrPane,
ProcessesPane,
SubagentPanel,
TestsPane,
} from '../lazyViews';
import { OPEN_PR_EVENT,type PrTarget } from '../lib/prPanel';
import { useAppRuntime,useLiveEntries } from '../state/RuntimeProvider';
import { selectActiveSession } from '../state/runtimeStore';
import { liveSubagentTabs,liveTranscriptView } from './liveTranscriptView';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function WorkspaceRightPanel(
  context: Pick<
    WorkspaceRuntime,
    | 'onSend'
    | 'refreshRepo'
    | 'panelOpen'
    | 'phone'
    | 'subagentTabs'
    | 'fileTabs'
    | 'toolTabs'
    | 'sendWhiteboardToChat'
    | 'openToolPane'
    | 'openPrPane'
    | 'runPrReview'
    | 'setPanelFilePickerOpen'
    | 'sessionId'
    | 'outlineEntries'
    | 'historyOffsetRef'
    | 'agentTabs'
    | 'busy'
    | 'selectedProcess'
    | 'cwd'
    | 'activityTurns'
    | 'setMainView'
    | 'jumpToMinimapTurn'
    | 'askRestore'
    | 'openFileTab'
    | 'activeAgentTab'
    | 'setActiveAgentTab'
    | 'closeAnyTab'
    | 'closeSubagentPanel'
    | 'replyToSubagentReview'
    | 'handlePanelResizeStart'
  >,
) {
  const runtime = useAppRuntime();
  const [expanded, setExpanded] = useState(false);
  useEffect(() => { if (!context.panelOpen) setExpanded(false); }, [context.panelOpen]);
  const [reviewOpen, setReviewOpen] = useWorkspaceUiField('reviewOpen');
  const [reviewFocus] = useWorkspaceUiField('reviewFocus');
  useEffect(() => {
    if (!reviewOpen) return;
    context.openToolPane('diff');
    setReviewOpen(false);
  }, [reviewOpen, context.openToolPane, setReviewOpen]);
  const entries = useLiveEntries();
  const streams = useSelector(runtime.sessionState, state => selectActiveSession(state).subagentState);
  const { activityTurns } = useMemo(() => liveTranscriptView(entries, context.historyOffsetRef.current), [entries]);
  const subagentTabs = useMemo(() => liveSubagentTabs(context.agentTabs, entries, streams), [context.agentTabs, entries, streams]);
  const {
    panelOpen,
    phone,
    fileTabs,
    toolTabs,
    sendWhiteboardToChat,
    openToolPane,
    openPrPane,
    runPrReview,
    setPanelFilePickerOpen,
    sessionId,
    busy,
    selectedProcess,
    cwd,
    setMainView,
    jumpToMinimapTurn,
    askRestore,
    openFileTab,
    activeAgentTab,
    setActiveAgentTab,
    closeAnyTab,
    closeSubagentPanel,
    replyToSubagentReview,
    handlePanelResizeStart,
  } = context;
  // PR chips in the transcript open here: a plain click dispatches the
  // event (see `lib/prPanel`), and the panel focuses (or opens) that
  // PR's tab. Modifier-clicks bypass the panel for a real GitHub tab.
  useEffect(() => {
    const onOpenPr = (e: Event) => {
      const t = (e as CustomEvent<PrTarget>).detail;
      if (t) openPrPane(t.owner, t.repo, t.number);
    };
    window.addEventListener(OPEN_PR_EVENT, onOpenPr);
    return () => window.removeEventListener(OPEN_PR_EVENT, onOpenPr);
  }, [openPrPane]);
  // The active PR tab, for the pane body below. `renderPane` only gets
  // the kind, so the tab is resolved here where the full list is in
  // scope.
  const activePrTab =
    toolTabs.find((t) => t.id === activeAgentTab && t.kind === 'pr') ??
    [...toolTabs].reverse().find((t) => t.kind === 'pr') ??
    null;
  return (
    <>
      {panelOpen && (
        <div
          // On a phone the panel is a full-screen sheet over the chat; its
          // close button returns there.
          className={
            expanded && !phone
              ? 'fixed inset-2 z-40 min-h-0 min-w-0 bg-background shadow-2xl'
              : phone
              ? 'pt-safe pb-safe px-safe fixed left-0 right-0 top-0 z-40 flex flex-col bg-background'
              : 'min-h-0 min-w-0 py-2 pr-2'
          }
          style={phone ? { height: 'var(--app-height, 100dvh)' } : expanded ? { top: hasHiddenTitleBar() ? TRAFFIC_LIGHT_INSET.centerY * 2 : 8 } : undefined}
        >
          <div
            className={
              phone
                ? 'min-h-0 flex-1 overflow-hidden'
                : 'h-full overflow-hidden rounded-xl border border-border bg-background'
            }
          >
            <LazyBoundary>
              <SubagentPanel
                expanded={expanded}
                onToggleExpand={phone ? undefined : () => setExpanded(value => !value)}
                diffPane={<LazyBoundary><ReviewChanges embedded open onClose={() => context.closeAnyTab('tool:diff')} onSendComments={context.onSend} onChanged={context.refreshRepo} focusPath={reviewFocus} /></LazyBoundary>}
                tabs={subagentTabs}
                fileTabs={fileTabs}
                toolTabs={toolTabs}
                onWhiteboardSend={(png) => void sendWhiteboardToChat(png)}
                onOpenPane={openToolPane}
                onBrowseFile={() => setPanelFilePickerOpen(true)}
                renderPane={(kind) => {
                  // Panes are lazy chunks; suspend just the pane, not the panel.
                  const pane = (() => {
                    switch (kind) {
                      case 'aside':
                        return (
                          <AsidePane sessionId={sessionId} entries={entries} agentBusy={busy} />
                        );
                      case 'processes':
                        return (
                          <ProcessesPane
                            key={sessionId}
                            sessionId={sessionId}
                            selectedProcess={
                              selectedProcess?.session === sessionId ? selectedProcess : undefined
                            }
                            onPreview={() => openToolPane('devices')}
                          />
                        );
                      case 'tests':
                        return <TestsPane sessionId={sessionId} cwd={cwd ?? ''} />;
                      case 'activity':
                        return (
                          <ActivityPane
                            turns={activityTurns}
                            busy={busy}
                            onJump={(id) => {
                              setMainView('chat');
                              jumpToMinimapTurn(id);
                            }}
                            onRestore={(idx) => void askRestore(idx)}
                            onOpenFile={(path, preview) => openFileTab(path, preview)}
                          />
                        );
                      case 'devices':
                        return <DevicesPane />;
                      case 'pr':
// Keyed by tab: each PR keeps its own
                        // summary/code selection when switching tabs.
                        return activePrTab ? (
                          <PrPane
                            key={activePrTab.id}
                            tab={activePrTab}
                            onReviewPr={runPrReview}
                          />
                        ) : null;
                      default:
                        return null;
                    }
                  })();
                  return pane && <LazyBoundary>{pane}</LazyBoundary>;
                }}
                activeCallId={activeAgentTab}
                cwd={cwd ?? ''}
                onSelectTab={setActiveAgentTab}
                onCloseTab={closeAnyTab}
                onClose={closeSubagentPanel}
                onReview={replyToSubagentReview}
                onResizeStart={expanded ? () => {} : handlePanelResizeStart}
                onOpenFile={openFileTab}
              />
            </LazyBoundary>
          </div>
        </div>
      )}
    </>
  );
}
