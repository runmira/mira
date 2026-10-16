import { useCallback, useEffect, useRef } from 'react';
import { useLocation, useNavigate, useNavigationType } from 'react-router';
import type { MainView } from '../components/Sidebar';
import { useAppRuntime } from '../state/RuntimeProvider';
import type { WorkspaceUiState } from '../state/createAppRuntime';

/** `/chat/:id`, the URL for one chat. */
export const chatPath = (id: string) => `/chat/${encodeURIComponent(id)}`;

/** The chat a `/chat/:id` path names, or '' on `/chat` or elsewhere. */
function routeChat(pathname: string): string {
  const [, segment, id] = pathname.split('/');
  return segment === 'chat' && id ? decodeURIComponent(id) : '';
}

/**
 * Keeps the open chat in the URL, so the router's history is the app's
 * back/forward: each chat or page the user moves to is one entry, and
 * nothing else needs to be snapshotted. The chat's state stays mounted in
 * the root route whichever entry is showing.
 *
 * Both directions run here. A chat opened any other way (the sidebar, a new
 * chat, the palette) is recorded as a new entry; landing on an entry with
 * Back, Forward or a reload opens the chat it names.
 */
export function useChatRoute(
  mainView: MainView,
  sessionId: string,
  attachSession: (id: string) => void,
) {
  const { pathname, key } = useLocation();
  const type = useNavigationType();
  const navigate = useNavigate();
  const routeId = routeChat(pathname);
  // The entry whose chat was opened from the URL, so a later chat switch on
  // that same entry is recorded instead of being pulled back to it.
  const opened = useRef<string | null>(null);
  const attach = useRef(attachSession);
  useEffect(() => {
    attach.current = attachSession;
  });

  useEffect(() => {
    if (mainView !== 'chat' || !sessionId || routeId === sessionId) return;
    if (routeId && type === 'POP' && opened.current !== key) {
      opened.current = key;
      attach.current(routeId);
      return;
    }
    // Bare `/chat` (from the sidebar or the palette) becomes this chat's
    // own URL in place, so it isn't an extra step back.
    void navigate(chatPath(sessionId), { replace: !routeId });
  }, [mainView, sessionId, routeId, type, key, navigate]);
}

/** The layout a history entry restores: the right panel, its tabs, the
 *  terminal and the review. Not the sidebar, which sits beside the arrows
 *  and toggles on its own; not dialogs, widths or anything in a chat. */
const LAYOUT_KEYS = [
  'contextOpen',
  'terminalOpen',
  'reviewPanelOpen',
  'reviewOpen',
  'agentTabs',
  'fileTabs',
  'toolTabs',
  'activeAgentTab',
] as const satisfies readonly (keyof WorkspaceUiState)[];
type Layout = { [K in (typeof LAYOUT_KEYS)[number]]: WorkspaceUiState[K] };

function layoutOf(state: WorkspaceUiState): Layout {
  return {
    contextOpen: state.contextOpen,
    terminalOpen: state.terminalOpen,
    reviewPanelOpen: state.reviewPanelOpen,
    reviewOpen: state.reviewOpen,
    agentTabs: state.agentTabs,
    // Paths only: a file's diff reloads when its tab is shown again.
    fileTabs: state.fileTabs.map((tab) => ({ ...tab, diff: null })),
    toolTabs: state.toolTabs,
    activeAgentTab: state.activeAgentTab,
  };
}
const sameLayout = (a: Layout, b: Layout) => JSON.stringify(a) === JSON.stringify(b);

/** A click or key press is being handled right now. Panel changes made
 *  then are the user's own; ones made from a server message or a timer
 *  (a subagent's tab opening, a chat's saved layout coming back) are not. */
function userInitiated(): boolean {
  const event = (window as { event?: Event }).event;
  return typeof UIEvent !== 'undefined' && event instanceof UIEvent;
}

/**
 * Makes opening and closing the right panel, its tabs, the terminal and the
 * review steps in Back/Forward, alongside chats and pages.
 *
 * One watcher on the workspace UI store, rather than every action recording
 * itself: each history entry carries the layout it was left in, as router
 * state. A change the user makes adds an entry; any other change updates the
 * current one, so it still comes back as it was left. Landing on an entry
 * with Back or Forward puts its layout back.
 */
export function useLayoutHistory() {
  const runtime = useAppRuntime();
  const location = useLocation();
  const type = useNavigationType();
  const navigate = useNavigate();
  const here = useRef(location);
  useEffect(() => {
    here.current = location;
  });
  const restoring = useRef(false);

  const entryLayout = (state: unknown) => (state as { layout?: Layout } | null)?.layout;

  // Every entry records its layout, including ones a chat or page added.
  useEffect(() => {
    const layout = entryLayout(location.state);
    if (type === 'POP' && layout) {
      if (sameLayout(layout, layoutOf(runtime.workspaceUi.get()))) return;
      restoring.current = true;
      for (const key of LAYOUT_KEYS) runtime.updateWorkspaceUi(key, layout[key] as never);
      restoring.current = false;
      return;
    }
    if (!layout) {
      const { pathname, search, state } = location;
      void navigate(
        { pathname, search },
        { replace: true, state: { ...(state as object | null), layout: layoutOf(runtime.workspaceUi.get()) } },
      );
    }
  }, [location, type, navigate, runtime]);

  useEffect(() => {
    // Opening another chat restores that chat's own panels in the same
    // click, before this watcher hears the chat changed: the chat's own
    // history entry covers that, so it isn't a step of its own.
    const chatOf = () => {
      const { selected, active } = runtime.sessionState.get();
      return `${selected}|${active}`;
    };
    let chat = chatOf();
    const sessions = runtime.sessionState.subscribe(() => {
      chat = chatOf();
    });
    let last = layoutOf(runtime.workspaceUi.get());
    const subscription = runtime.workspaceUi.subscribe(() => {
      const next = layoutOf(runtime.workspaceUi.get());
      if (sameLayout(next, last)) return;
      last = next;
      if (restoring.current) return;
      const switching = chatOf() !== chat;
      const { pathname, search, state } = here.current;
      void navigate(
        { pathname, search },
        userInitiated() && !switching
          ? { replace: false, state: { layout: next } }
          : { replace: true, state: { ...(state as object | null), layout: next } },
      );
    });
    return () => {
      subscription.unsubscribe();
      sessions.unsubscribe();
    };
  }, [navigate, runtime]);
}

/** Where the hash router's history stands, from the index it stores. */
const historyIndex = () => (window.history.state as { idx?: number } | null)?.idx ?? 0;
let furthest = 0;

/** Back and Forward through the app's own history, and whether either has
 *  anywhere to go. Going back past the first entry would leave the app. */
export function useHistoryArrows() {
  // Subscribes to every navigation, which is when the index moves.
  useLocation();
  const type = useNavigationType();
  const navigate = useNavigate();
  const index = historyIndex();
  // A new entry drops everything after it; going back keeps it for Forward.
  furthest = type === 'PUSH' ? index : Math.max(furthest, index);
  const canGoBack = index > 0;
  const canGoForward = index < furthest;
  const goBack = useCallback(() => {
    if (historyIndex() > 0) void navigate(-1);
  }, [navigate]);
  const goForward = useCallback(() => {
    if (historyIndex() < furthest) void navigate(1);
  }, [navigate]);
  return { canGoBack, canGoForward, goBack, goForward };
}
