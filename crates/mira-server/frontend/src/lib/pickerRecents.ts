/**
 * Models the user picked recently, per engine (`provider:openrouter`,
 * `agent:codex`). A long catalog (OpenRouter lists hundreds) is unusable
 * without them. Per-browser and best-effort: storage may be unavailable.
 */
const KEY = 'mira.picker.recent.v1';
const LIMIT = 5;

function read(): Record<string, string[]> {
  try {
    const raw = localStorage.getItem(KEY);
    const v = raw ? JSON.parse(raw) : {};
    return v && typeof v === 'object' ? v : {};
  } catch {
    return {};
  }
}

export function recentModels(engineKey: string): string[] {
  const list = read()[engineKey];
  return Array.isArray(list) ? list.filter((m) => typeof m === 'string').slice(0, LIMIT) : [];
}

export function rememberModel(engineKey: string, model: string): void {
  try {
    const all = read();
    all[engineKey] = [model, ...(all[engineKey] ?? []).filter((m) => m !== model)].slice(0, LIMIT);
    localStorage.setItem(KEY, JSON.stringify(all));
  } catch {
    /* storage unavailable: recents are a convenience */
  }
}
