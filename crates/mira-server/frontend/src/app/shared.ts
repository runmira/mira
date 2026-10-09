import { useCallback, useRef } from 'react';
import { type MainView } from '../components/Sidebar';
export const IS_MAC =
  typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
export function useStableCallback<A extends unknown[], R>(
  fn: (...args: A) => R,
): (...args: A) => R {
  const ref = useRef(fn);
  ref.current = fn;
  return useCallback((...args: A) => ref.current(...args), []);
}
export const MIN_STREAM_WIDTH = 560;
export const PAGE_TITLES: Record<MainView, string> = {
  chat: 'Chat',
  plugins: 'Plugins',
  'pull-request': 'Pull request',
  scheduled: 'Scheduled',
  settings: 'Settings',
};
