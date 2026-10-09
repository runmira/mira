import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import { restoreCheckpoint } from '../api';
import { ApprovalDialog } from '../components/ApprovalDialog';
import { FilePicker } from '../components/FilePicker';
import { LazyBoundary } from '../components/LazyBoundary';
import { ReviewPanel } from '../components/ReviewPanel';
import { CommandPalette, ContextInspector, FolderPicker, ImportChats } from '../lazyViews';
import { shortNum } from '../lib/usage';
import { cn } from '../lib/utils';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function WorkspaceDialogs(
  context: Pick<
    WorkspaceRuntime,
    | 'pendingAcpMode'
    | 'agentPostures'
    | 'acpDriverName'
    | 'setPendingAcpMode'
    | 'onSetAcpMode'
    | 'panelFilePickerOpen'
    | 'cwd'
    | 'setPanelFilePickerOpen'
    | 'openFileTab'
    | 'importOpen'
    | 'setImportOpen'
    | 'setSidebarRefresh'
    | 'inspectOpenMounted'
    | 'inspectOpen'
    | 'setInspectOpen'
    | 'busy'
    | 'acpDriver'
    | 'compactAcpAgent'
    | 'onCompact'
    | 'setEntries'
    | 'paletteOpenMounted'
    | 'paletteOpen'
    | 'setPaletteOpen'
    | 'paletteActions'
    | 'attachSession'
    | 'setMainView'
    | 'restoreAsk'
    | 'setRestoreAsk'
    | 'doRestore'
    | 'pickerOpenMounted'
    | 'pickerOpen'
    | 'setPickerOpen'
    | 'reviewPanelOpen'
    | 'reviewState'
    | 'setReviewPanelOpen'
  >,
) {
  const {
    pendingAcpMode,
    agentPostures,
    acpDriverName,
    setPendingAcpMode,
    onSetAcpMode,
    panelFilePickerOpen,
    cwd,
    setPanelFilePickerOpen,
    openFileTab,
    importOpen,
    setImportOpen,
    setSidebarRefresh,
    inspectOpenMounted,
    inspectOpen,
    setInspectOpen,
    busy,
    acpDriver,
    compactAcpAgent,
    onCompact,
    setEntries,
    paletteOpenMounted,
    paletteOpen,
    setPaletteOpen,
    paletteActions,
    attachSession,
    setMainView,
    restoreAsk,
    setRestoreAsk,
    doRestore,
    pickerOpenMounted,
    pickerOpen,
    setPickerOpen,
    reviewPanelOpen,
    reviewState,
    setReviewPanelOpen,
  } = context;
  return (
    <>
      {pendingAcpMode &&
        (() => {
          // One confirm dialog for every mode decision: a pick from the picker
          // and a server-refused privilege converge here. The choice was made
          // in the picker, so this states the consequence and asks for the nod
          // — it does not list every posture again.
          const opt = agentPostures.find((o) => o.modeId === pendingAcpMode.modeId);
          const label = opt?.posture.label ?? pendingAcpMode.modeId;
          return (
            <ApprovalDialog
              tone="consequential"
              request={{
                title: `Switch to ${label}?`,
                source: { label: acpDriverName, detail: 'agent mode' },
                body: (
                  <div className="space-y-2">
                    {pendingAcpMode.reason && <p>{pendingAcpMode.reason}</p>}
                    <p>
                      {opt?.modeDescription ??
                        opt?.posture.blurb ??
                        'This changes what the agent may do without asking.'}
                    </p>
                    <p className="text-muted-foreground/70">
                      {acpDriverName} will keep this mode until you change it back. File, terminal
                      and permission requests still route through Mira either way.
                    </p>
                  </div>
                ),
                choices: [
                  { id: '__cancel', label: 'Cancel' },
                  {
                    id: 'confirm',
                    label: `Switch to ${label}`,
                    mode:
                      opt?.posture.key === 'auto'
                        ? 'edit'
                        : opt?.posture.key === 'yolo'
                          ? 'yolo'
                          : undefined,
                    primary: true,
                    destructive: opt?.posture.key === 'yolo',
                  },
                ],
                onDismiss: () => setPendingAcpMode(null),
                onChoose: (id) => {
                  const modeId = pendingAcpMode.modeId;
                  setPendingAcpMode(null);
                  // Confirmed in this dialog, so the acknowledgement rides
                  // along: the server must not ask again for what was just
                  // agreed.
                  if (id !== '__cancel') onSetAcpMode(modeId, true);
                },
              }}
            />
          );
        })()}
      <FilePicker
        open={panelFilePickerOpen}
        startPath={cwd || undefined}
        onClose={() => setPanelFilePickerOpen(false)}
        onPicked={(p) => {
          setPanelFilePickerOpen(false);
          openFileTab(p, null);
        }}
      />
      <Dialog open={importOpen} onOpenChange={setImportOpen}>
        <DialogContent className="max-w-3xl gap-4 p-5">
          <DialogTitle className="text-[16px] font-semibold">
            Bring chats from other agents
          </DialogTitle>
          {importOpen && (
            <LazyBoundary>
              <ImportChats
                onDone={() => {
                  setImportOpen(false);
                  setSidebarRefresh((n) => n + 1);
                }}
              />
            </LazyBoundary>
          )}
        </DialogContent>
      </Dialog>
      {inspectOpenMounted && (
        <LazyBoundary>
          <ContextInspector
            open={inspectOpen}
            onOpenChange={setInspectOpen}
            busy={busy}
            onCompact={(focus) => (acpDriver ? compactAcpAgent(focus) : onCompact(focus))}
            onDropped={(callId, d) => {
              // The tool card shows what the model now sees, and a status line
              // records the drop where it happened.
              setEntries((prev) => [
                ...prev.map((e) =>
                  e.kind === 'tool' && e.call.id === callId && e.result
                    ? {
                        ...e,
                        result: {
                          ...e.result,
                          content: `[Removed from context — ~${shortNum(d.tokens)} tokens]`,
                          images: undefined,
                        },
                      }
                    : e,
                ),
                {
                  kind: 'warning',
                  text: `[context] removed ${d.label || d.tool} from context (~${shortNum(d.tokens)} tokens)`,
                },
              ]);
            }}
          />
        </LazyBoundary>
      )}
      {paletteOpenMounted && (
        <LazyBoundary>
          <CommandPalette
            open={paletteOpen}
            onOpenChange={setPaletteOpen}
            actions={paletteActions}
            onOpenSession={(id) => {
              attachSession(id);
              setMainView('chat');
            }}
          />
        </LazyBoundary>
      )}
      {restoreAsk && (
        <ApprovalDialog
          tone="consequential"
          request={{
            title: `Restore ${restoreAsk.changes.length} file${restoreAsk.changes.length === 1 ? '' : 's'}?`,
            source: { label: 'Checkpoint', detail: 'before this message' },
            body: (
              <div className="space-y-2">
                <p>
                  Every file that changed since this message goes back to how it was — including
                  changes made after it, by anyone. You can undo this.
                </p>
                <ul className="max-h-48 space-y-0.5 overflow-auto rounded-md bg-background/60 px-2.5 py-2 font-mono text-[11.5px]">
                  {restoreAsk.changes.map((c) => (
                    <li key={c.path} className="flex gap-2">
                      <span
                        className={cn(
                          'w-14 shrink-0',
                          c.action === 'remove'
                            ? 'text-red-400/80'
                            : c.action === 'recreate'
                              ? 'text-green-400/80'
                              : 'text-amber-300/80',
                        )}
                      >
                        {c.action === 'remove'
                          ? 'remove'
                          : c.action === 'recreate'
                            ? 'bring back'
                            : 'revert'}
                      </span>
                      <span className="min-w-0 break-all text-foreground/80">{c.path}</span>
                    </li>
                  ))}
                </ul>
              </div>
            ),
            choices: [
              { id: 'cancel', label: 'Cancel' },
              { id: 'restore', label: 'Restore files', primary: true },
            ],
            onDismiss: () => setRestoreAsk(null),
            onChoose: (id) => {
              const ask = restoreAsk;
              setRestoreAsk(null);
              if (id === 'restore') void doRestore(() => restoreCheckpoint(ask.ref), 'Restored');
            },
          }}
        />
      )}
      {pickerOpenMounted && (
        <LazyBoundary>
          <FolderPicker
            open={pickerOpen}
            onClose={() => setPickerOpen(false)}
            onPicked={(_path, id) => {
              // Server built a fresh slot for the new cwd. Attach the WS so
              // the freshly-published Ready lands in our transcript — without
              // this, the socket keeps forwarding the previous slot's frames
              // and the UI silently stays on the old folder.
              if (id) attachSession(id);
            }}
          />
        </LazyBoundary>
      )}
      <ReviewPanel
        open={reviewPanelOpen}
        state={reviewState}
        onClose={() => setReviewPanelOpen(false)}
      />
    </>
  );
}
