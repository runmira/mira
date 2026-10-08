// Phone-sized layout. Below `md` (768px) the app becomes one column: the
// sidebar is a slide-out drawer and the right panel a full-screen sheet.
// See App.tsx and the `max-md:` / `touch:` classes in components.

import { useSyncExternalStore } from 'react';

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
