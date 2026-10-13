/** Capture an HTTP action's owner when it starts, not when it resolves. */
export function sessionFetch(url: string, options?: RequestInit, session?: string): Promise<Response> {
  const headers = new Headers(options?.headers);
  if (session) headers.set('X-Mira-Session', session);
  return fetch(url, { ...options, headers });
}
