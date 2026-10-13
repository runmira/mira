import { useOptionalRuntime } from '../state/RuntimeProvider';
import { PREF_KEYS } from './preferenceKeys';
/**
 * Local app preferences (General tab). Browser-local localStorage settings
 * for shell behavior — no server round-trip, unlike the model settings in
 * mira.yaml. Same-tab updates broadcast so every hook re-renders.
 */
import { useSyncExternalStore } from 'react';

export type DiffLayout = 'unified' | 'split';

const listeners = new Set<() => void>();

function broadcast() {
  listeners.forEach((l) => l());
}

if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key?.startsWith('mira.')) broadcast();
  });
  window.addEventListener('mira:prefs', () => broadcast());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getPref(key: string, def: string): string {
  try {
    return localStorage.getItem(key) ?? def;
  } catch {
    return def;
  }
}

export function getBoolPref(key: string, def: boolean): boolean {
  try {
    const raw = localStorage.getItem(key);
    return raw == null ? def : raw === '1';
  } catch {
    return def;
  }
}

export function setPref(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* private mode */
  }
  broadcast();
}

export function setBoolPref(key: string, value: boolean): void {
  setPref(key, value ? '1' : '0');
}

/** Reactive boolean pref for render-gating. */
export function useBoolPref(key: string, def: boolean): [boolean, (value: boolean) => void] {
  const runtime = useOptionalRuntime();
  const value = useSyncExternalStore(runtime ? listener => {
    const sub = runtime.preferences.source.subscribe(listener); return () => sub.unsubscribe();
  } : subscribe, () => runtime ? (runtime.preferences.source.get()[key] == null ? def : runtime.preferences.source.get()[key] === '1') : getBoolPref(key, def), () => def);
  return [value, next => runtime ? runtime.preferences.set(key, next ? '1' : '0') : setBoolPref(key, next)];
}
export function useStringPref(key: string, def: string): [string, (value: string) => void] {
  const runtime = useOptionalRuntime();
  const value = useSyncExternalStore(runtime ? listener => {
    const sub = runtime.preferences.source.subscribe(listener); return () => sub.unsubscribe();
  } : subscribe, () => runtime ? runtime.preferences.source.get()[key] ?? def : getPref(key, def), () => def);
  return [value, next => runtime ? runtime.preferences.set(key, next) : setPref(key, next)];
}

export { PREF_KEYS } from './preferenceKeys';

/** Apply the reduce-motion class to <html>. Call on boot and on toggle. */
export function applyReduceMotion(): void {
  try {
    document.documentElement.classList.toggle(
      'reduce-motion',
      getBoolPref(PREF_KEYS.reduceMotion, false),
    );
  } catch {
    /* non-DOM */
  }
}
