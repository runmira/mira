import { useEffect, useState } from 'react';

/**
 * File/folder icons from the Material Icon Theme set
 * (`@baybreezy/file-extension-icon`).
 *
 * Loaded lazily on purpose. The package ships the whole icon collection in
 * a couple of lookup maps that Rollup can't tree-shake, and pulling it in
 * eagerly cost ~870 kB on the main chunk — for glyphs that only appear in
 * the file explorer, which is itself a right-panel tab the user has to
 * open. Dynamically importing keeps it in its own chunk, fetched the first
 * time a file icon is actually rendered.
 *
 * Every accessor returns `null` until the module lands, so callers fall back
 * to their own glyphs for those first frames rather than flashing nothing.
 */

type IconModule = typeof import('@baybreezy/file-extension-icon');

let mod: IconModule | null = null;
let inflight: Promise<IconModule> | null = null;
let waiters: Array<(m: IconModule) => void> = [];

function load(): Promise<IconModule> {
  if (mod) return Promise.resolve(mod);
  if (!inflight) {
    inflight = import('@baybreezy/file-extension-icon').then((m) => {
      mod = m;
      const pending = waiters;
      waiters = [];
      for (const w of pending) w(m);
      return m;
    });
  }
  return inflight;
}

/** Kick the chunk off early if you know an icon is about to be needed. */
export function preloadFileIcons(): void {
  void load();
}

/**
 * `null` until the icon set is ready, then the accessors.
 *
 * Re-renders once on arrival, which is what upgrades the fallback glyphs to
 * real themed icons.
 */
export function useFileIcons(): {
  ready: boolean;
  fileIcon: (name: string) => string | null;
  folderIcon: (name: string, open?: boolean) => string | null;
} {
  const [ready, setReady] = useState(() => mod !== null);

  useEffect(() => {
    if (mod) {
      setReady(true);
      return;
    }
    let live = true;
    waiters.push(() => {
      if (live) setReady(true);
    });
    void load();
    return () => {
      live = false;
    };
  }, []);

  return {
    ready,
    fileIcon: (name: string) => {
      if (!mod) return null;
      try {
        return mod.getMaterialFileIcon(name) || null;
      } catch {
        return null;
      }
    },
    folderIcon: (name: string, open = false) => {
      if (!mod) return null;
      try {
        return mod.getMaterialFolderIcon(name, open) || null;
      } catch {
        return null;
      }
    },
  };
}
