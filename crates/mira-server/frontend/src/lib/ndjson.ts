/**
 * POST JSON, read the reply as newline-delimited JSON, one callback per
 * object as it arrives. Used by the panes that stream from the server
 * (Ask aside's answer, a test run). Aborting `signal` closes the request,
 * which the server takes as "stop".
 */
export async function postNdjson<T>(
  url: string,
  body: unknown,
  onEvent: (ev: T) => void,
  signal?: AbortSignal,
): Promise<void> {
  const r = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
    signal,
  });
  if (!r.ok || !r.body) {
    const j = (await r.json().catch(() => ({}))) as { error?: string };
    throw new Error(j.error ?? `HTTP ${r.status}`);
  }
  const reader = r.body.getReader();
  const dec = new TextDecoder();
  let buf = '';
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += dec.decode(value, { stream: true });
    let nl: number;
    while ((nl = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (line) onEvent(JSON.parse(line) as T);
    }
  }
  if (buf.trim()) onEvent(JSON.parse(buf) as T);
}

/** `?session=…` for the session-scoped pane endpoints. */
export function sessionQuery(sessionId: string | null | undefined): string {
  return sessionId ? `?session=${encodeURIComponent(sessionId)}` : '';
}
