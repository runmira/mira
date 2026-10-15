import { createStore } from '@tanstack/react-store';
import type { MessageRef,RestoreChange } from '../api';
import type { InfoNotice } from '../components/InfoNoticeHost';
import type { FilePanelTab } from '../components/SubagentPanel';
import type { ToolPaneTab } from '../components/panes/toolPanes';
import type { UiStorage } from './createAppRuntime';
const STORAGE = 'mira.workspace-ui.v1';
function read(storage: UiStorage | undefined, key: string) { try { return storage?.getItem(key); } catch { return null; } }
export function createWorkspaceUiStore(storage?: UiStorage) {
  let disposed = false;
  let saved: Partial<{ contextOpen: boolean; terminalOpen: boolean }> = {};
  try {
    const record = JSON.parse(read(storage, STORAGE) ?? 'null');
    if (record?.version === 1 && record.values && typeof record.values === 'object') {
      if (typeof record.values.contextOpen === 'boolean') saved.contextOpen = record.values.contextOpen;
      if (typeof record.values.terminalOpen === 'boolean') saved.terminalOpen = record.values.terminalOpen;
    }
  } catch { /* malformed preferences fall back to legacy defaults */ }
  const store = createStore({
    contextOpen: saved.contextOpen ?? read(storage, 'mira.context.open') === '1',
    terminalOpen: read(storage, 'mira.terminal.restore') !== '0' && (saved.terminalOpen ?? read(storage, 'mira.terminal.open') === '1'),
    sidebarOpen: read(storage, 'mira.sidebar.open') !== '0',
    pickerOpen: (false) as boolean,
    paletteOpen: (false) as boolean,
    inspectOpen: (false) as boolean,
    importOpen: (false) as boolean,
    reportOpen: (false) as boolean,
    panelFilePickerOpen: (false) as boolean,
    drawerOpen: (false) as boolean,
    agentTabs: ([]) as string[],
    fileTabs: ([]) as FilePanelTab[],
    toolTabs: ([]) as ToolPaneTab[],
    activeAgentTab: (null) as string | null,
    rightPanelWidth: (390) as number,
    reviewPanelOpen: (false) as boolean,
    reviewOpen: (false) as boolean,
    reviewFocus: (null) as string | null,
    lightbox: (null) as string | null,
    selectedProcess: (null) as {
    session: string;
    id: number;
    nonce: number;
  } | null,
    infoNotices: ([]) as InfoNotice[],
    sidebarRefresh: (0) as number,
    chatColWidth: (0) as number,
    extensionsVersion: (0) as number,
    skillsVersion: (0) as number,
    showJump: (false) as boolean,
    restoreAsk: (null) as {
    ref: MessageRef;
    changes: RestoreChange[];
    /** Set when Claude's own checkpoints restore the files, not Mira's. */
    native?: { coverage: string; insertions: number | null; deletions: number | null; skipped: number | null };
  } | null,
    restoreNote: (null) as {
    text: string;
    undo?: string;
    /** Undo for a Claude-native rewind (a snapshot taken just before it). */
    undoNative?: string;
    error?: boolean;
  } | null,
    secondOpinionSeen: (new Set<string>()) as Set<string>,
    browserPing: (0) as number,
  });
  type State = ReturnType<typeof store.get>;
  function update<K extends keyof State>(key: K, apply: State[K] | ((previous: State[K]) => State[K])) {
    if (disposed) return;
    const previous = store.get();
    const next = typeof apply === 'function' ? (apply as (value: State[K]) => State[K])(previous[key]) : apply;
    if (Object.is(previous[key], next)) return;
    store.setState(() => ({ ...previous, [key]: next }));
    if (key === 'contextOpen' || key === 'terminalOpen') {
      const state = store.get();
      try {
        storage?.setItem(key === 'contextOpen' ? 'mira.context.open' : 'mira.terminal.open', next ? '1' : '0');
        storage?.setItem(STORAGE, JSON.stringify({ version: 1, values: { contextOpen: state.contextOpen, terminalOpen: state.terminalOpen } }));
      } catch { /* private mode */ }
    }
  }
  return { source: { get: () => store.get(), subscribe: store.subscribe.bind(store) }, update,
    dispose() { disposed = true; } };
}
export type WorkspaceUiState = ReturnType<ReturnType<typeof createWorkspaceUiStore>['source']['get']>;
