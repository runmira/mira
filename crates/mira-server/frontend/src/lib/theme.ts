/**
 * Light, dark, or the system's choice. The preference lives in
 * localStorage (`mira.theme`); the resolved theme goes on <html> as
 * `data-theme` and the `dark` class, which the stylesheet's tokens and
 * Tailwind's `dark:` variant read.
 *
 * index.html runs the same resolution inline before the first paint, so
 * a light-theme user never sees a dark flash. Keep the two in step.
 */
import { useSyncExternalStore } from 'react';
import { getPref, PREF_KEYS, setPref } from './prefs';

export type ThemePref = 'system' | 'light' | 'dark';
export type Theme = 'light' | 'dark';

const media = typeof window !== 'undefined' ? window.matchMedia?.('(prefers-color-scheme: light)') : undefined;

export function themePref(): ThemePref {
  const v = getPref(PREF_KEYS.theme, 'system');
  return v === 'light' || v === 'dark' ? v : 'system';
}

export function resolveTheme(pref: ThemePref = themePref()): Theme {
  if (pref !== 'system') return pref;
  return media?.matches ? 'light' : 'dark';
}

/** Put the resolved theme on the document (and the desktop window). */
export function applyTheme(): void {
  if (typeof document === 'undefined') return;
  const pref = themePref();
  const theme = resolveTheme(pref);
  const root = document.documentElement;
  root.dataset.theme = theme;
  root.classList.toggle('dark', theme === 'dark');
  root.style.colorScheme = theme;
  // The desktop app's window follows too, so macOS draws its vibrancy and
  // traffic lights to match. `null` hands it back to the system.
  void window.__TAURI__?.window?.getCurrentWindow?.()?.setTheme?.(pref === 'system' ? null : theme)?.catch?.(() => {});
  window.dispatchEvent(new Event('mira:theme'));
}

export function setThemePref(pref: ThemePref): void {
  setPref(PREF_KEYS.theme, pref);
  applyTheme();
}

/** Follow the system while the preference is "system". Call once at boot. */
export function watchSystemTheme(): void {
  media?.addEventListener?.('change', () => {
    if (themePref() === 'system') applyTheme();
  });
}

function subscribe(cb: () => void) {
  window.addEventListener('mira:theme', cb);
  return () => window.removeEventListener('mira:theme', cb);
}

/** The theme in effect now, re-rendering when it changes. */
export function useTheme(): Theme {
  return useSyncExternalStore(subscribe, () => resolveTheme(), () => 'dark');
}

/** The user's preference, re-rendering when it changes. */
export function useThemePref(): [ThemePref, (p: ThemePref) => void] {
  const pref = useSyncExternalStore(subscribe, themePref, () => 'system' as ThemePref);
  return [pref, setThemePref];
}
