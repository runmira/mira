/**
 * `#` issue mentions in the composer: typing `#` (or `#205`, `#handoff`)
 * opens a picker of the chat repo's issues and PRs; a pick becomes an
 * atomic chip that serializes to `owner/repo#205` — the reference form
 * the transcript already renders as a GitHub chip with a hover card.
 */
import { sessionFetch } from '../api/sessionScope';

export type IssueState = 'open' | 'closed' | 'merged' | 'draft';

export type IssueOption = {
  kind: 'issue' | 'pr';
  number: number;
  title: string;
  state: IssueState;
  author: string | null;
  labels: { name: string; color: string }[];
  updated_at: string | null;
};

export type IssueSearch = {
  owner: string | null;
  repo: string | null;
  items: IssueOption[];
  error: string | null;
};

/** Narrow cached rows synchronously, before a debounced network lookup. */
export function filterIssueOptions(items: IssueOption[], query: string, cached = true): IssueOption[] {
  if (!query) return items;
  if (/^\d+$/.test(query)) {
    const number = query.replace(/^0+(?=\d)/, '');
    const exact = items.filter(item => String(item.number) === number);
    return exact.length ? exact : items.filter(item => String(item.number).startsWith(number));
  }
  return cached ? items.filter(item => item.title.toLowerCase().includes(query.toLowerCase())) : items;
}

/** The `#query` at the end of the text, when the user is typing one.
 *  Like the `/` and `@` palettes, only the tail counts — no caret
 *  tracking. A `#` glued to a word (`C#`, `foo#1`) isn't a trigger. */
export function issueTrigger(text: string): { query: string; triggerStart: number } | null {
  const m = /(?:^|\s)#([A-Za-z0-9_.-]{0,40})$/.exec(text);
  if (!m) return null;
  return { query: m[1], triggerStart: m.index + m[0].length - m[1].length - 1 };
}

export async function searchIssues(query: string, sessionId?: string, signal?: AbortSignal): Promise<IssueSearch> {
  const r = await sessionFetch(`/api/issues?q=${encodeURIComponent(query)}`, { signal }, sessionId);
  if (!r.ok) throw new Error(`Couldn't load issues (HTTP ${r.status}).`);
  return (await r.json()) as IssueSearch;
}

/** `owner/repo#123` in composer text; the shape `linkifyGithubRefs` reads. */
export const ISSUE_TOKEN = /(^|[\s(])([A-Za-z0-9][\w-]*\/[\w.-]+)#(\d+)\b/g;

export function issueToken(owner: string, repo: string, number: number): string {
  return `${owner}/${repo}#${number}`;
}

/** What the picker knew about each picked reference, so a chip rebuilt
 *  from text (a draft restore, an edit) keeps its title and state. */
const known = new Map<string, Pick<IssueOption, 'kind' | 'title' | 'state'>>();

export function rememberIssue(token: string, option: IssueOption) {
  known.set(token, { kind: option.kind, title: option.title, state: option.state });
}

export function knownIssue(token: string) {
  return known.get(token);
}

/** Split plain text into prose and `owner/repo#N` references. */
export function splitIssueTokens(text: string): ({ kind: 'text'; text: string } | { kind: 'issue'; token: string; number: number })[] {
  const out: ({ kind: 'text'; text: string } | { kind: 'issue'; token: string; number: number })[] = [];
  let last = 0;
  for (const m of text.matchAll(ISSUE_TOKEN)) {
    const start = (m.index ?? 0) + m[1].length;
    if (start > last) out.push({ kind: 'text', text: text.slice(last, start) });
    const token = `${m[2]}#${m[3]}`;
    out.push({ kind: 'issue', token, number: Number(m[3]) });
    last = start + token.length;
  }
  if (last < text.length) out.push({ kind: 'text', text: text.slice(last) });
  return out;
}

/** Tailwind tone for a state icon, matching `GithubRef`. */
export function issueTone(state: IssueState | undefined): string {
  switch (state) {
    case 'open':
      return 'text-emerald-400';
    case 'merged':
      return 'text-violet-400';
    case 'closed':
      return 'text-red-400';
    default:
      return 'text-muted-foreground';
  }
}

/** Lucide's CircleDot / GitPullRequest / GitMerge, as DOM for the
 *  contenteditable chip (React can't render inside it). */
function iconSvg(kind: 'issue' | 'pr', state: IssueState | undefined): SVGSVGElement {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '2');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('class', `issue-chip-icon ${issueTone(state)}`);
  const add = (tag: string, attrs: Record<string, string>) => {
    const el = document.createElementNS(ns, tag);
    for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
    svg.appendChild(el);
  };
  if (kind === 'issue') {
    add('circle', { cx: '12', cy: '12', r: '10' });
    add('circle', { cx: '12', cy: '12', r: '1' });
  } else if (state === 'merged') {
    add('circle', { cx: '18', cy: '18', r: '3' });
    add('circle', { cx: '6', cy: '6', r: '3' });
    add('path', { d: 'M6 21V9a9 9 0 0 0 9 9' });
  } else {
    add('circle', { cx: '18', cy: '18', r: '3' });
    add('circle', { cx: '6', cy: '6', r: '3' });
    add('path', { d: 'M13 6h3a2 2 0 0 1 2 2v7' });
    add('line', { x1: '6', x2: '6', y1: '9', y2: '21' });
  }
  return svg;
}

/** The composer chip for `owner/repo#N`: state icon, number, short title. */
export function buildIssueChip(token: string, number: number): HTMLSpanElement {
  const meta = knownIssue(token);
  const el = document.createElement('span');
  el.dataset.issue = token;
  el.contentEditable = 'false';
  el.className = 'issue-chip';
  el.title = meta ? `${token} · ${meta.title}` : token;
  el.appendChild(iconSvg(meta?.kind ?? 'issue', meta?.state));
  const num = document.createElement('span');
  num.className = 'issue-chip-number';
  num.textContent = `#${number}`;
  el.appendChild(num);
  if (meta?.title) {
    const title = document.createElement('span');
    title.className = 'issue-chip-title';
    title.textContent = meta.title;
    el.appendChild(title);
  }
  return el;
}
