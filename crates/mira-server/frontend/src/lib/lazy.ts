import { createElement, lazy, useEffect, useState, type ComponentType } from 'react';

/** Resets for lazy components whose chunk failed to load. */
const failed = new Set<() => void>();

/**
 * Let lazy components whose download failed try again on their next
 * render. `React.lazy` caches a rejected load for good, so each one swaps
 * in a fresh `lazy()`. Called by `LazyBoundary`'s Retry.
 */
export function retryFailedLazyLoads(): void {
  for (const reset of failed) reset();
  failed.clear();
}

/**
 * `React.lazy` for a named export, so heavy views can live in their own
 * chunk without a default export (issue #72). `preload()` starts fetching
 * the chunk early — call it on hover/focus of whatever opens the view, so
 * the click usually finds it ready. A failed download is retryable (see
 * `retryFailedLazyLoads`); render inside a `LazyBoundary`.
 */
export function lazyNamed<M, K extends keyof M>(
  load: () => Promise<M>,
  name: K,
): (M[K] extends ComponentType<any> ? M[K] : never) & { preload: () => void } {
  // One fetch shared by preload() and render; cleared on failure so the
  // next attempt fetches again.
  let pending: Promise<M> | null = null;
  const once = () => (pending ??= load().catch((e) => {
    pending = null;
    throw e;
  }));
  const make = () =>
    lazy(() =>
      once().then(
        (m) => ({ default: m[name] as ComponentType<any> }),
        (e) => {
          failed.add(() => (current = make()));
          throw e;
        },
      ),
    );
  let current = make();
  const Lazy = (props: any) => createElement(current, props);
  Lazy.displayName = `Lazy(${String(name)})`;
  return Object.assign(Lazy, { preload: () => void once().catch(() => {}) }) as any;
}

/** Hover/focus handlers that preload a lazy view. Both bubble, so they
 *  can go on a wrapper around the control. */
export function preloadOnIntent(preload: () => void) {
  return { onPointerOver: preload, onFocus: preload };
}

/** `true` from the first render where `on` is true, onwards. For lazy views
 *  that own state (drafts, comments) which must survive closing them. */
export function useLatch(on: boolean): boolean {
  const [latched, setLatched] = useState(on);
  if (on && !latched) setLatched(true);
  return latched || on;
}

/** A module fetched on first need and cached for the session, so every
 *  later caller gets it synchronously (no per-component flash). */
export type LazyModule<M> = { get: () => M | null; load: () => Promise<M> };

export function lazyModule<M>(loader: () => Promise<M>): LazyModule<M> {
  let mod: M | null = null;
  let inflight: Promise<M> | null = null;
  return {
    get: () => mod,
    load: () =>
      (inflight ??= loader().then(
        (m) => (mod = m),
        (e) => {
          inflight = null;
          throw e;
        },
      )),
  };
}

/** The module once loaded, kicking off the load when `needed`. */
export function useLazyModule<M>(handle: LazyModule<M>, needed: boolean): M | null {
  const [mod, setMod] = useState(handle.get);
  useEffect(() => {
    if (!needed || mod) return;
    let live = true;
    handle.load().then(
      (m) => live && setMod(() => m),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [handle, needed, mod]);
  return mod;
}
