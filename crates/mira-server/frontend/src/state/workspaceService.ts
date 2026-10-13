import { getBoolPref,PREF_KEYS } from '../lib/prefs';
import { QueueMutations } from '../lib/queueMutations';
import type { AcpAgentStatus } from '../types';
import type { WsClient } from '../ws';
/** Transport, timers and DOM handles have a service lifetime, never a store snapshot. */
export function createWorkspaceService() {
  let disposed = false;
  const timers = new Set<number>(); const frames = new Set<number>();
  const refs = {
    pingPrimedRef: { current: false }, activityTitlesRef: { current: new Map<string, string>() },
    acpAgentsRef: { current: [] as AcpAgentStatus[] }, completedUpdatesRef: { current: new Set<string>() },
    thinkingIdleTimerRef: { current: null as number | null }, drawerTouchX: { current: null as number | null },
    drawerRef: { current: null as HTMLDivElement | null }, paneRef: { current: null as HTMLDivElement | null },
    chatShortcutRequest: { current: 0 }, chatColObserver: { current: null as ResizeObserver | null },
    lastScrollInputRef: { current: 0 }, readingAnchorRef: { current: null as number | null },
    followRef: { current: getBoolPref(PREF_KEYS.transcriptFollow, true) }, chatTitleRef: { current: null as string | null },
    visibleApprovalRef: { current: null as string | null }, browserShownFor: { current: null as string | null },
    browserPingSeen: { current: 0 }, wsRef: { current: null as WsClient | null },
    queueMutationsRef: { current: new QueueMutations() },
  };
  return {
    refs,
    schedule(callback: () => void, delay: number) {
      if (disposed) return 0;
      const timer = window.setTimeout(() => { timers.delete(timer); if (!disposed) callback(); }, delay);
      timers.add(timer); return timer;
    },
    frame(callback: () => void) {
      if (disposed) return 0;
      const frame = requestAnimationFrame(() => { frames.delete(frame); if (!disposed) callback(); });
      frames.add(frame); return frame;
    },
    dispose() {
      disposed = true;
      refs.wsRef.current?.close(); refs.wsRef.current = null;
      refs.queueMutationsRef.current.setConnected(false);
      if (typeof window !== 'undefined') {
        for (const timer of timers) window.clearTimeout(timer);
        if (refs.thinkingIdleTimerRef.current != null) window.clearTimeout(refs.thinkingIdleTimerRef.current);
      }
      for (const frame of frames) cancelAnimationFrame(frame);
      refs.chatColObserver.current?.disconnect(); refs.chatColObserver.current = null;
      timers.clear(); frames.clear(); refs.activityTitlesRef.current.clear(); refs.completedUpdatesRef.current.clear();
      refs.paneRef.current = null; refs.drawerRef.current = null;
    },
  };
}
