/* ---------- MCP servers + plugins ---------- */

/** Throw with the server's `{ error }` message when a request fails. */
export async function jsonOrThrow<T>(r: Response, what: string): Promise<T> {
  if (!r.ok) {
    let msg = `${what} failed (${r.status})`;
    try {
      const j = await r.json();
      if (j.error) msg = j.error;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
  return (await r.json()) as T;
}

export async function post<T>(url: string, body: unknown, what: string): Promise<T> {
  const r = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body ?? {}),
  });
  return jsonOrThrow<T>(r, what);
}
