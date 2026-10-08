/**
 * Getting the user's attention when Mira is waiting on them.
 *
 * Two signals, both only while the tab is in the background (a visible tab
 * already shows the card and plays the ping):
 *  - a system notification, if the user turned them on in Settings and the
 *    browser allows it; clicking it brings Mira back to the front;
 *  - a dot in the tab title, cleared as soon as the tab is visible again,
 *    so a glance at the tab strip says "Mira needs you".
 */
import { getBoolPref, PREF_KEYS } from './prefs';

export type AttentionKind = 'done' | 'approval' | 'question' | 'plan';

const BODY: Record<AttentionKind, string> = {
  done: 'Finished',
  approval: 'Needs your approval',
  question: 'Has a question for you',
  plan: 'Has a plan for you to review',
};

const MARK = '● ';
let marked = false;

function clearMark() {
  if (!marked) return;
  marked = false;
  if (document.title.startsWith(MARK)) document.title = document.title.slice(MARK.length);
}

if (typeof document !== 'undefined') {
  document.addEventListener('visibilitychange', () => {
    if (!document.hidden) clearMark();
  });
  window.addEventListener('focus', clearMark);
}

/** Tell a user who's looking elsewhere that a chat wants them.
 *  `title` names the chat, so several running chats stay distinguishable. */
export function callForAttention(kind: AttentionKind, title?: string | null, options?: {sessionId?:string;onOpen?:()=>void;whileVisible?:boolean}) {
  if (typeof document === 'undefined' || (!document.hidden && !options?.whileVisible)) return;
  if (document.hidden && !marked) {
    marked = true;
    document.title = MARK + document.title;
  }
  if (!getBoolPref(PREF_KEYS.notifyTurnDone, false)) return;
  try {
    if (typeof Notification === 'undefined' || Notification.permission !== 'granted') return;
    const n = new Notification(title?.trim() ? title.trim().slice(0, 80) : 'Mira', {
      body: BODY[kind],
      // One notification per chat and kind: a burst of approvals replaces
      // itself instead of stacking up.
      tag: `mira-${kind}-${options?.sessionId ?? title ?? ''}`,
      silent: false,
    });
    n.onclick = () => {
      window.focus();
      options?.onOpen?.();
      n.close();
    };
  } catch {
    /* notifications unavailable */
  }
}
