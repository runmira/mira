import { listEngines, setSessionBackgroundMode } from '../api';
import { InfoNoticeHost, noticeId } from '../components/InfoNoticeHost';
import { Sidebar } from '../components/Sidebar';
import { cn } from '../lib/utils';
import { WorkspaceDialogs } from './WorkspaceDialogs';
import { WorkspaceMain } from './WorkspaceMain';
import { WorkspaceRightPanel } from './WorkspaceRightPanel';

import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function WorkspaceLayout(context: WorkspaceRuntime) {
  const {
    status,
    cwd,
    sessionId,
    busy,
    sessionActivity,
    completedSessions,
    sidebarRefresh,
    mainView,
    closeDrawer,
    setMainView,
    onNewChat,
    openSettings,
    openProjectPicker,
    attachSession,
    setSidebarRefresh,
    branchPr,
    settingsSection,
    setSettingsSection,
    exitSettings,
    phone,
    hiddenTitleBar,
    sidebarCol,
    rightCol,
    infoNotices,
    dismissInfoNotice,
    completedUpdatesRef,
    requestAcpStatus,
    setEngines,
    pushInfoNotice,
    drawerOpen,
    drawerRef,
    drawerTouchX,
  } = context;

  const sidebar = (
    <Sidebar
      status={status}
      cwd={cwd}
      activeSessionId={sessionId}
      activeBusy={busy}
      runningSessions={sessionActivity?.running ?? null}
      completedSessions={completedSessions}
      refreshKey={sidebarRefresh}
      activeView={mainView}
      onNavigate={(view) => {
        closeDrawer();
        setMainView(view);
      }}
      onNewChat={async () => {
        closeDrawer();
        setMainView('chat');
        await onNewChat();
      }}
      onOpenSettings={() => {
        closeDrawer();
        openSettings();
      }}
      onOpenPicker={() => {
        closeDrawer();
        void openProjectPicker();
      }}
      onSessionLoaded={() => {
        // Ready broadcast refreshes + jumps to chat.
        closeDrawer();
      }}
      onAttachSession={(id) => {
        closeDrawer();
        attachSession(id);
      }}
      onSetBackgroundMode={async (id, mode) => {
        await setSessionBackgroundMode(id, mode);
        setSidebarRefresh((n) => n + 1);
      }}
      activePr={branchPr}
      settingsSection={settingsSection}
      onSettingsSectionChange={(section) => {
        closeDrawer();
        setSettingsSection(section);
      }}
      onExitSettings={() => {
        closeDrawer();
        exitSettings();
      }}
    />
  );

  return (
    <div
      // Window wash ( `--color-sidebar`): the sidebar
      // sits full-bleed on it; the main + right columns float as inset
      // rounded cards (`--color-panel`).
      // With the native title bar hidden, the window is transparent and the
      // OS paints vibrancy behind it. The grid's own wash has to be
      // transparent too or it would cover that; the sidebar supplies a
      // translucent tint over the vibrancy, and the main and right columns
      // stay opaque panels so body text never sits on wallpaper.
      className={cn(
        'grid grid-rows-1',
        phone ? 'bg-background' : 'h-screen transition-[grid-template-columns] duration-150',
        !phone && (hiddenTitleBar ? 'bg-transparent' : 'bg-panel'),
      )}
      style={
        phone
          ? // The visible viewport, so the composer sits above the keyboard.
            { gridTemplateColumns: 'minmax(0,1fr)', height: 'var(--app-height, 100dvh)' }
          : { gridTemplateColumns: `${sidebarCol} minmax(0,1fr) ${rightCol}` }
      }
    >
      <InfoNoticeHost
        notices={infoNotices.filter((notice) => notice.kind === 'short')}
        onDismiss={dismissInfoNotice}
        onAgentUpdated={(kind) => {
          completedUpdatesRef.current.add(kind);
          requestAcpStatus();
          void listEngines()
            .then((view) => setEngines(view.engines))
            .catch(() => {});
          pushInfoNotice({
            id: noticeId('updated'),
            kind: 'short',
            tone: 'success',
            title: 'Agent update complete',
          });
        }}
      />
      {phone ? (
        <>
          <div
            aria-hidden
            onClick={closeDrawer}
            className={cn(
              'fixed inset-0 z-40 bg-black/50 transition-opacity duration-200',
              drawerOpen ? 'opacity-100' : 'pointer-events-none opacity-0',
            )}
          />
          <div
            ref={drawerRef}
            // A dialog only while open: the app's shortcuts stand down
            // whenever a dialog is on screen.
            role={drawerOpen ? 'dialog' : undefined}
            aria-modal={drawerOpen ? 'true' : undefined}
            aria-label="Sidebar"
            tabIndex={-1}
            inert={!drawerOpen}
            // Swipe left to close, like any drawer.
            onTouchStart={(e) => {
              drawerTouchX.current = e.touches[0].clientX;
            }}
            onTouchEnd={(e) => {
              const start = drawerTouchX.current;
              drawerTouchX.current = null;
              if (start != null && start - e.changedTouches[0].clientX > 60) closeDrawer();
            }}
            className={cn(
              'pt-safe pb-safe fixed left-0 top-0 z-50 flex w-[min(86vw,320px)] flex-col overflow-hidden bg-panel shadow-2xl transition-transform duration-200 ease-out',
              drawerOpen ? 'translate-x-0' : '-translate-x-full',
            )}
            style={{ height: 'var(--app-height, 100dvh)' }}
          >
            {sidebar}
          </div>
        </>
      ) : (
        // overflow-hidden clips sidebar content when the grid column animates to 0
        <div className="overflow-hidden">{sidebar}</div>
      )}

      {/* Gutters are asymmetric: 2px on the sidebar edge vs 8px elsewhere,
          so the chat column reads as pulled toward the sidebar without
          touching it. The right panel keeps the full 8px on its outer edge. */}
      <WorkspaceMain {...context} />

      {/* A privileged agent mode is a standing grant of more access than Mira
          would allow, so it gets an explicit confirmation that says what it
          does. The server refuses without this acknowledgement; the client
          setting the flag is not consent on its own. */}
      {/* Changing the agent's mode from the composer. The mode picker used to
          live inside the model dialog as a silent dropdown; a permission-mode
          change is a grant of standing authority, not a preference, so it is
          confirmed here — one dialog for every decision. */}
      <WorkspaceDialogs {...context} />
      <WorkspaceRightPanel {...context} />
    </div>
  );
}
