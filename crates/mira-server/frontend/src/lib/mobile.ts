// Phone-sized layout. Below `md` (768px) the app becomes one column: the
// sidebar is a slide-out drawer and the right panel a full-screen sheet.
// See App.tsx and the `max-md:` / `touch:` classes in components.

import { useEffect, useSyncExternalStore, type RefObject } from 'react';

const PHONE = '(max-width: 767px)';

function subscribe(query: string) {
  return (onChange: () => void) => {
    const media = window.matchMedia(query);
    media.addEventListener('change', onChange);
    return () => media.removeEventListener('change', onChange);
  };
}

const subscribePhone = subscribe(PHONE);

/** True at phone width. */
export function useIsPhone(): boolean {
  return useSyncExternalStore(subscribePhone, () => window.matchMedia(PHONE).matches, () => false);
}

/**
 * Keep `--app-height` equal to the visible viewport, so the composer stays
 * above the on-screen keyboard. iOS Safari doesn't resize the layout (or
 * `100dvh`) for the keyboard — it pans the page instead, which pushes the
 * header off-screen and leaves the composer under the keys. Sizing the app
 * to the visual viewport and pinning the page at the top avoids both.
 */
export function trackVisibleViewport() {
  const vv = window.visualViewport;
  if (!vv) return;
  const root = document.documentElement;
  const update = () => {
    // Unscaled, so pinch-zooming doesn't shrink the app.
    const height = vv.height * vv.scale;
    root.style.setProperty('--app-height', `${height}px`);
    // A keyboard is the only thing that takes this much off the bottom.
    root.toggleAttribute('data-keyboard', window.innerHeight - height > 120);
    // Undo the pan iOS applies when an input near the bottom is focused.
    if (window.scrollY !== 0 && window.matchMedia(PHONE).matches) window.scrollTo(0, 0);
  };
  vv.addEventListener('resize', update);
  vv.addEventListener('scroll', update);
  update();
}

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"]), [contenteditable="true"]';

/**
 * A modal panel's focus handling, for one that isn't a Radix dialog (the
 * phone sidebar drawer): while `open`, focus moves into `ref`, Tab stays
 * inside it and Escape anywhere calls `onClose`; on close, focus goes back
 * to what had it before.
 */
export function useModalFocus(open: boolean, ref: RefObject<HTMLElement | null>, onClose: () => void) {
  useEffect(() => {
    const panel = ref.current;
    if (!open || !panel) return;
    const previous = document.activeElement as HTMLElement | null;
    const focusables = () => Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE)).filter((el) => el.offsetParent !== null);
    (focusables()[0] ?? panel).focus({ preventScroll: true });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key !== 'Tab') return;
      const items = focusables();
      if (items.length === 0) {
        e.preventDefault();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      if (e.shiftKey && (active === first || !panel.contains(active))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (active === last || !panel.contains(active))) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('keydown', onKey);
      previous?.focus?.({ preventScroll: true });
    };
  }, [open, ref, onClose]);
}
