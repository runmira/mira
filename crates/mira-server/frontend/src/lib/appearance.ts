/**
 * Accent color and interface size (Settings → Appearance). Both live in
 * localStorage and apply to <html>; index.html applies them before first
 * paint too, so keep the two in step.
 *
 * - Accent: `data-accent` on <html> swaps `--mira-blue` (and the text
 *   that sits on it), which every accented control reads.
 * - Size: the desktop app zooms its webview (native, so the browser pane's
 *   native child view can be placed in the same units — see
 *   childWebview.ts); a browser gets CSS zoom on <html>.
 */
import { useSyncExternalStore } from 'react';
import { getPref, PREF_KEYS, setPref } from './prefs';
import { isDesktop } from './desktop';

export const ACCENTS = ['blue', 'violet', 'teal', 'green', 'orange', 'pink'] as const;
export type Accent = (typeof ACCENTS)[number];

/** Swatch colors for the picker, [dark, light] — the same values as the
 *  `[data-accent]` blocks in styles.css. */
export const ACCENT_SWATCH: Record<Accent, [string, string]> = {
  blue: ['#7aa2f7', '#3b5bdb'],
  violet: ['#a78bfa', '#7c3aed'],
  teal: ['#2dd4bf', '#0d9488'],
  green: ['#4ade80', '#16a34a'],
  orange: ['#fb923c', '#ea580c'],
  pink: ['#f472b6', '#db2777'],
};

export const UI_SCALES = [0.9, 1, 1.1, 1.2] as const;
export type UiScale = (typeof UI_SCALES)[number];

export function accentPref(): Accent {
  const v = getPref(PREF_KEYS.accent, 'blue');
  return (ACCENTS as readonly string[]).includes(v) ? (v as Accent) : 'blue';
}

export function uiScale(): UiScale {
  const v = Number(getPref(PREF_KEYS.uiScale, '1'));
  return (UI_SCALES as readonly number[]).includes(v) ? (v as UiScale) : 1;
}

/** The page zoom the desktop webview applies, for converting the page's
 *  CSS pixels into the window's logical pixels (1 in a browser). */
export function desktopZoom(): number {
  return isDesktop() ? uiScale() : 1;
}

export function applyAppearance(): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  const accent = accentPref();
  if (accent === 'blue') delete root.dataset.accent;
  else root.dataset.accent = accent;

  const scale = uiScale();
  if (isDesktop()) {
    root.style.removeProperty('zoom');
    void window.__TAURI__?.webview?.getCurrentWebview?.()?.setZoom?.(scale)?.catch?.(() => {});
  } else if (scale === 1) {
    root.style.removeProperty('zoom');
  } else {
    root.style.setProperty('zoom', String(scale));
  }
  window.dispatchEvent(new Event('mira:appearance'));
}

export function setAccent(accent: Accent): void {
  setPref(PREF_KEYS.accent, accent);
  applyAppearance();
}

export function setUiScale(scale: UiScale): void {
  setPref(PREF_KEYS.uiScale, String(scale));
  applyAppearance();
}

function subscribe(cb: () => void) {
  window.addEventListener('mira:appearance', cb);
  return () => window.removeEventListener('mira:appearance', cb);
}

export function useAccent(): Accent {
  return useSyncExternalStore(subscribe, accentPref, () => 'blue' as Accent);
}

export function useUiScale(): UiScale {
  return useSyncExternalStore(subscribe, uiScale, () => 1 as UiScale);
}
