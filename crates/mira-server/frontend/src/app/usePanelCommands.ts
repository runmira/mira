import { getSessionHistory } from '../api';
import {
  isToolPaneId,
  TOOL_PANE_DEFS,
  toolPaneId,
  type ToolPaneKind,
} from '../components/panes/toolPanes';
import { extractAgentId } from '../lib/agentIdentifiers';
import { attachFilesToComposer, dataUrlToFile } from '../lib/attachBridge';
import { splitFileRef } from '../lib/refs';
import { updateSubagent, type ToolEntry } from '../transcript/entries';
import { historyToEntries } from '../transcript/replay';
import type { DiffPreview } from '../types';

import type React from 'react';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function usePanelCommands(
  context: Pick<
    WorkspaceRuntime,
    | 'setAgentTabs'
    | 'setActiveAgentTab'
    | 'subagentState'
    | 'entries'
    | 'setSubagentState'
    | 'activeAgentTab'
    | 'fileTabs'
    | 'toolTabs'
    | 'setFileTabs'
    | 'setToolTabs'
    | 'agentTabs'
    | 'rightPanelWidth'
    | 'setRightPanelWidth'
    | 'setExpandedTurns'
  >,
) {
  const {
    setAgentTabs,
    setActiveAgentTab,
    subagentState,
    entries,
    setSubagentState,
    activeAgentTab,
    fileTabs,
    toolTabs,
    setFileTabs,
    setToolTabs,
    agentTabs,
    rightPanelWidth,
    setRightPanelWidth,
    setExpandedTurns,
  } = context;

  function openAgentTab(callId: string) {
    setAgentTabs((prev) => (prev.includes(callId) ? prev : [...prev, callId]));
    setActiveAgentTab(callId);
    // Fire-and-forget: if we don't already have a live stream for this
    // agent (fresh page after a reload, or a resumed session), pull the
    // child's persisted history and reconstruct the transcript.
    void hydrateSubagentIfNeeded(callId);
  }

  /** If subagentState has no entries for `callId`, extract the child
   *  session id from the parent's tool_result marker and fetch the
   *  child's history from the backend. Populates subagentState so the
   *  panel body renders the full timeline. No-op when the marker is
   *  missing (older sessions before the marker landed) or when a live
   *  stream already exists. */
  async function hydrateSubagentIfNeeded(callId: string) {
    // Bail if we already have a stream in flight or on record for this call.
    const existing = subagentState.get(callId);
    if (existing && existing.entries.length > 0) return;
    // Find the parent's tool entry to read the tool_result content.
    const entry = entries.find((e): e is ToolEntry => e.kind === 'tool' && e.call.id === callId);
    if (!entry?.result) return;
    const agentId = extractAgentId(entry.result.content);
    if (!agentId) return;
    try {
      const view = await getSessionHistory(agentId);
      const rebuilt = historyToEntries(view.messages, view.previews);
      setSubagentState((prev) =>
        updateSubagent(prev, callId, (s) => ({
          ...s,
          agentId,
          model: view.model,
          entries: rebuilt,
          done: true,
        })),
      );
    } catch (e) {
      // Non-fatal: panel falls back to the summary block. Log so
      // developers see the failure without breaking the user's flow.
      console.warn('subagent hydrate failed', callId, agentId, e);
    }
  }

  function closeAgentTab(callId: string) {
    setAgentTabs((prev) => {
      const next = prev.filter((id) => id !== callId);
      if (activeAgentTab === callId) {
        // Fall through to remaining agent tabs, then file/tool tabs, then null.
        const remaining = [...next, ...fileTabs.map((t) => t.id), ...toolTabs.map((t) => t.id)];
        setActiveAgentTab(remaining.length > 0 ? remaining[remaining.length - 1] : null);
      }
      return next;
    });
  }

  function closeSubagentPanel() {
    setAgentTabs([]);
    setFileTabs([]);
    setToolTabs([]);
    setActiveAgentTab(null);
  }

  function openFileTab(ref: string, diff: DiffPreview | null, atLine?: number | null) {
    // `src/a.rs:42` is a file and a line, not a file named that.
    const { path, line: refLine } = splitFileRef(ref);
    const line = atLine ?? refLine;
    const reveal = Date.now();
    setFileTabs((prev) => {
      // If already open, update the diff (re-opening after a new write).
      if (prev.some((t) => t.id === path)) {
        return prev.map((t) => (t.id === path ? { ...t, diff, line, reveal } : t));
      }
      return [...prev, { id: path, path, diff, line, reveal }];
    });
    setActiveAgentTab(path);
  }

  function closeFileTab(tabId: string) {
    setFileTabs((prev) => {
      const next = prev.filter((t) => t.id !== tabId);
      if (activeAgentTab === tabId) {
        const remaining = [...agentTabs, ...next.map((t) => t.id), ...toolTabs.map((t) => t.id)];
        setActiveAgentTab(remaining.length > 0 ? remaining[remaining.length - 1] : null);
      }
      return next;
    });
  }

  function closeToolTab(tabId: string) {
    setToolTabs((prev) => {
      const next = prev.filter((t) => t.id !== tabId);
      if (activeAgentTab === tabId) {
        const remaining = [...agentTabs, ...fileTabs.map((t) => t.id), ...next.map((t) => t.id)];
        setActiveAgentTab(remaining.length > 0 ? remaining[remaining.length - 1] : null);
      }
      return next;
    });
  }

  /**
   * Open (or focus) a utility pane. Singletons by kind, so asking twice just
   * re-focuses.
   *
   * The `new` launcher is a tab like any other, but it's retired the moment
   * you pick something — otherwise picking a pane leaves a stale "New" tab
   * sitting in the strip forever.
   */
  function openToolPane(kind: ToolPaneKind) {
    const id = toolPaneId(kind);
    setToolTabs((prev) => {
      const withoutLauncher = kind === 'new' ? prev : prev.filter((t) => t.kind !== 'new');
      return withoutLauncher.some((t) => t.id === id)
        ? withoutLauncher
        : [...withoutLauncher, { id, kind, title: TOOL_PANE_DEFS[kind].title }];
    });
    setActiveAgentTab(id);
  }

  function closeAnyTab(id: string) {
    if (agentTabs.includes(id)) closeAgentTab(id);
    else if (isToolPaneId(id)) closeToolTab(id);
    else closeFileTab(id);
  }

  /** Whiteboard "Send": hand the drawing to the composer as an image
   *  attachment so the model can actually see it. */
  async function sendWhiteboardToChat(pngDataUrl: string) {
    const file = await dataUrlToFile(pngDataUrl, 'whiteboard.png');
    attachFilesToComposer(file);
  }

  function handlePanelResizeStart(e: React.MouseEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = rightPanelWidth;
    function onMove(ev: MouseEvent) {
      // Dragging left increases panel width (panel is on the right side).
      const next = Math.max(280, Math.min(800, startWidth + (startX - ev.clientX)));
      setRightPanelWidth(next);
    }
    function onUp() {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    }
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  }

  function toggleTurn(idx: number) {
    setExpandedTurns((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  }
  return {
    openAgentTab,
    hydrateSubagentIfNeeded,
    closeAgentTab,
    closeSubagentPanel,
    openFileTab,
    closeFileTab,
    closeToolTab,
    openToolPane,
    closeAnyTab,
    sendWhiteboardToChat,
    handlePanelResizeStart,
    toggleTurn,
  };
}
