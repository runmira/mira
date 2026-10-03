/**
 * The address Device preview shows. Set from the Processes pane ("Preview"
 * on a dev server) or typed in the preview itself; remembered across
 * reloads.
 */
import { useSyncExternalStore } from 'react';

const KEY = 'mira.devicePreviewUrl';
const listeners = new Set<() => void>();

function read(): string {
  try {
    return localStorage.getItem(KEY) ?? '';
  } catch {
    return '';
  }
}

let current = read();

export function setPreviewUrl(url: string): void {
  current = url;
  try {
    localStorage.setItem(KEY, url);
  } catch {
    /* private mode: keep it for this run */
  }
  listeners.forEach((l) => l());
}

export function usePreviewUrl(): string {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => current,
  );
}
