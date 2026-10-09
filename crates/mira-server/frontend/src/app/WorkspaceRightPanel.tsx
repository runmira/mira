import { LazyBoundary } from '../components/LazyBoundary';
import {
  ActivityPane,
  AsidePane,
  DevicesPane,
  ProcessesPane,
  SubagentPanel,
  TestsPane,
} from '../lazyViews';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function WorkspaceRightPanel(
  context: Pick<
    WorkspaceRuntime,
    | 'panelOpen'
    | 'phone'
    | 'subagentTabs'
    | 'fileTabs'
    | 'toolTabs'
    | 'sendWhiteboardToChat'
    | 'openToolPane'
    | 'setPanelFilePickerOpen'
    | 'sessionId'
    | 'entries'
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
  const {
    panelOpen,
    phone,
    subagentTabs,
    fileTabs,
    toolTabs,
    sendWhiteboardToChat,
    openToolPane,
    setPanelFilePickerOpen,
    sessionId,
    entries,
    busy,
    selectedProcess,
    cwd,
    activityTurns,
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
  return (
    <>
      {panelOpen && (
        <div
          // On a phone the panel is a full-screen sheet over the chat; its
          // close button returns there.
          className={
            phone
              ? 'pt-safe pb-safe px-safe fixed left-0 right-0 top-0 z-40 flex flex-col bg-background'
              : 'min-h-0 min-w-0 py-2 pr-2'
          }
          style={phone ? { height: 'var(--app-height, 100dvh)' } : undefined}
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
                onResizeStart={handlePanelResizeStart}
                onOpenFile={openFileTab}
              />
            </LazyBoundary>
          </div>
        </div>
      )}
    </>
  );
}
