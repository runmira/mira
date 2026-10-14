/** Follow mode for the transcript pane: when new content should keep
 *  pulling the viewport to the bottom.
 *
 *  Pure and separate from the hook so the thresholds are testable — the
 *  bug this exists to prevent is a timing/threshold bug, which is exactly
 *  the kind that a DOM-shaped test can't cover. */

/** Distance from the bottom at which follow takes over, in px. */
export const FOLLOW_ENGAGE_AT = 80;
/** Distance at which follow gives up. Deliberately much larger: with a
 *  single threshold, scrolling *down* through a long streamed turn crosses
 *  the engage line, follow turns on, and the next token hard-pins the pane
 *  to the bottom — past the section the reader was scrolling toward. The
 *  gap between the two is the whole point. */
export const FOLLOW_RELEASE_AT = 240;

/**
 * Next follow state.
 *
 * @param following current state
 * @param distance  px between the bottom of the content and the viewport
 * @param prefOn    Settings → General → Transcript auto-follow. Consulted
 *                  only when engaging, so an explicit jump-to-latest still
 *                  works while the preference is off.
 */
export function nextFollow(following: boolean, distance: number, prefOn: boolean): boolean {
  if (following) return distance <= FOLLOW_RELEASE_AT;
  return distance < FOLLOW_ENGAGE_AT && prefOn;
}

/** Px between the bottom of the content and the bottom of the viewport. */
export function distanceFromBottom(el: {
  scrollHeight: number;
  scrollTop: number;
  clientHeight: number;
}): number {
  return el.scrollHeight - el.scrollTop - el.clientHeight;
}

/** Explicit gestures own follow intent; layout scroll events cannot release it. */
export function followFromInput(event: {
  nativeEvent: Event; target: EventTarget | null; currentTarget: HTMLElement;
}, prefOn: boolean): boolean | null {
  if ((event.target as Element)?.closest?.('[data-thinking-scroll], [data-live-output]')) return null;
  const native = event.nativeEvent;
  if (native instanceof KeyboardEvent && !['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' '].includes(native.key)) return null;
  const towardEnd = native instanceof WheelEvent ? native.deltaY >= 0 :
    native instanceof KeyboardEvent && ['ArrowDown', 'PageDown', 'End', ' '].includes(native.key) && !native.shiftKey;
  return towardEnd && distanceFromBottom(event.currentTarget) <= 2 && prefOn;
}
