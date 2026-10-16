/** Keep the route across a web sign-in. The provider sends the browser back
 *  to the bare origin (`/?code=…`), which the hash router would open as
 *  Chat. Remember the route before leaving and put it back on return —
 *  before the router reads the URL, so this module is imported first. */

const KEY = 'mira.auth.return';

/** Where to send the callback URL, or null to leave it alone: only an OAuth
 *  return (`code` or `error` in the query) that carries no route of its own. */
export function returnUrl(href: string, saved: string | null): string | null {
  if (!saved || !/^#\/./.test(saved)) return null;
  const url = new URL(href);
  if (!url.searchParams.has('code') && !url.searchParams.has('error')) return null;
  if (url.hash && url.hash !== '#' && url.hash !== '#/') return null;
  return url.pathname + url.search + saved;
}

export function rememberReturnRoute() {
  try {
    sessionStorage.setItem(KEY, window.location.hash);
  } catch {
    /* private mode: sign-in still works, it lands on Chat */
  }
}

function restoreReturnRoute() {
  if (typeof window === 'undefined') return;
  try {
    const target = returnUrl(window.location.href, sessionStorage.getItem(KEY));
    sessionStorage.removeItem(KEY);
    if (target) window.history.replaceState(window.history.state, '', target);
  } catch {
    /* storage unavailable */
  }
}

restoreReturnRoute();
