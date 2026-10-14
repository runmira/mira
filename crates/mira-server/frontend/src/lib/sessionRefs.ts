/** Session references in transcript text (`sess_<32 hex>`). */

export type SessionRefTarget = { id: string };

const SESS_ID = 'sess_[0-9a-fA-F]{32}';

const SESSION_HREF = new RegExp(`^#session-(${SESS_ID})$`);

/** The session a `#session-…` link points at, if it's one. */
export function parseSessionRef(href: string | undefined): SessionRefTarget | null {
  const m = href ? SESSION_HREF.exec(href) : null;
  return m ? { id: m[1] } : null;
}

/** Short label for inline display — the full id is 37 chars. */
export function shortSessionId(id: string): string {
  return id.length > 13 ? `${id.slice(0, 13)}…` : id;
}

/** Turn bare `sess_…` ids into links, outside code. Mirrors
 *  `linkifyGithubRefs`: fenced blocks and inline code are prose the
 *  user quoted, not references. Idempotent — already-linked
 *  `[sess_…](#session-sess_…)` text is left alone. */
export function linkifySessionRefs(text: string): string {
  if (!text.includes('sess_')) return text;
  return text
    .split(/(```[\s\S]*?```|`[^`\n]*`)/g)
    .map((part, i) =>
      i % 2 === 1
        ? part
        : part.replace(
            new RegExp(`(^|[\\s(>])(sess_[0-9a-fA-F]{32})\\b(?![\\]\\)])`, 'g'),
            (_m, pre: string, id: string) => `${pre}[${id}](#session-${id})`,
          ),
    )
    .join('');
}
