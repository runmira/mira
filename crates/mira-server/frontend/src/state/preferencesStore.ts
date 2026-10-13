import { createStore } from '@tanstack/react-store';
import { PREF_KEYS } from '../lib/preferenceKeys';
import type { UiStorage } from './createAppRuntime';
const STORAGE = 'mira.preferences.v1';
const allowed = new Set<string>(Object.values(PREF_KEYS));
const booleans = new Set(['sidebarOpen', 'terminalRestore', 'composerCmdEnter', 'transcriptFollow', 'transcriptTurnStats',
  'reduceMotion', 'notifyTurnDone', 'browserAutoOpen', 'turnSound', 'diffWrap', 'diffInline', 'diffMarkers', 'diffLineNumbers']
  .map(key => PREF_KEYS[key as keyof typeof PREF_KEYS]));
function valid(key: string, value: unknown): value is string {
  if (!allowed.has(key) || typeof value !== 'string' || value.length > 256) return false;
  if (booleans.has(key as typeof PREF_KEYS[keyof typeof PREF_KEYS])) return value === '0' || value === '1';
  if (key === PREF_KEYS.theme) return ['system', 'light', 'dark'].includes(value);
  if (key === PREF_KEYS.diffLayout) return ['unified', 'split'].includes(value);
  return true;
}
export function createPreferencesStore(storage?: UiStorage) {
  let disposed = false;
  function read() {
    const values: Record<string, string> = {};
    try {
      const record = JSON.parse(storage?.getItem(STORAGE) ?? 'null');
      if (record?.version === 1 && record.values) for (const [key, value] of Object.entries(record.values))
        if (valid(key, value)) values[key] = value;

    } catch { /* malformed or unavailable storage */ }
    for (const key of allowed) {
      try { const value = storage?.getItem(key); if (valid(key, value)) values[key] = value; } catch { /* private mode */ }
    }
    return values;
  }
  const store = createStore(read());
  function sync() {
    if (disposed) return;
    const next = read();
    if (JSON.stringify(next) !== JSON.stringify(store.get())) store.setState(() => next);
  }
  if (typeof window !== 'undefined') { window.addEventListener('storage', sync); window.addEventListener('mira:prefs', sync); }
  return {
    source: { get: () => store.get(), subscribe: store.subscribe.bind(store) },
    set(key: string, value: string) {
      if (disposed || !valid(key, value)) return;
      const next = { ...store.get(), [key]: value }; store.setState(() => next);
      try { storage?.setItem(key, value); storage?.setItem(STORAGE, JSON.stringify({ version: 1, values: next })); } catch { /* private mode */ }
      if (typeof window !== 'undefined') window.dispatchEvent(new Event('mira:prefs'));
    },
    dispose() {
      disposed = true;
      if (typeof window !== 'undefined') { window.removeEventListener('storage', sync); window.removeEventListener('mira:prefs', sync); }
    },
  };
}
