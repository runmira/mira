/**
 * What an inline `code` span in a reply refers to, so it can render as the
 * right thing: a file (with a line), a commit, a color, a key combo, or a
 * name defined in the code. Pure and conservative — anything ambiguous
 * stays plain code.
 */

export type InlineRef =
  | { kind: 'file'; path: string; line: number | null }
  | { kind: 'commit'; sha: string }
  | { kind: 'color'; color: string }
  | { kind: 'keys'; keys: string[] }
  | { kind: 'symbol'; name: string }
  /** Might be a branch; only the repo can say, so it's checked before it
   *  becomes a chip. */
  | { kind: 'branch'; name: string };

/** `src/a.rs:42` / `src/a.rs:42:7` → path and line. */
export function splitFileRef(ref: string): { path: string; line: number | null } {
  const m = /^(.*?):(\d+)(?::\d+)?$/.exec(ref.trim());
  if (m && m[1] && Number(m[2]) > 0) return { path: m[1], line: Number(m[2]) };
  return { path: ref.trim(), line: null };
}

function isFilePath(path: string): boolean {
  if (/^[a-z]+:\/\//i.test(path) || /\s/.test(path)) return false;
  if (!path.includes('/')) return false;
  return /\.[\w]{1,8}$/.test(path) || /\/(Makefile|Dockerfile|LICENSE|README)$/.test(path);
}

const MODIFIERS: Record<string, string> = {
  cmd: '⌘', command: '⌘', '⌘': '⌘', meta: '⌘', super: '⌘', win: '⊞',
  ctrl: 'Ctrl', control: 'Ctrl', '⌃': '⌃',
  alt: 'Alt', option: '⌥', opt: '⌥', '⌥': '⌥',
  shift: '⇧', '⇧': '⇧',
};
const NAMED_KEYS: Record<string, string> = {
  enter: 'Enter', return: '↵', esc: 'Esc', escape: 'Esc', tab: 'Tab', space: 'Space',
  backspace: '⌫', delete: 'Del', del: 'Del', up: '↑', down: '↓', left: '←', right: '→',
  home: 'Home', end: 'End', pageup: 'PgUp', pagedown: 'PgDn',
  f1: 'F1', f2: 'F2', f3: 'F3', f4: 'F4', f5: 'F5', f6: 'F6', f7: 'F7', f8: 'F8', f9: 'F9', f10: 'F10', f11: 'F11', f12: 'F12',
};

/** `⌘K`, `Cmd+Shift+P`, `Ctrl-C`, `Esc` → the caps to draw. */
function parseKeys(raw: string): string[] | null {
  const s = raw.trim();
  if (!s || s.length > 30) return null;
  // Symbol form: ⌘⇧P, ⌘K
  const sym = /^([⌘⇧⌥⌃]+)(.+)$/.exec(s);
  if (sym) {
    const key = sym[2];
    const k = NAMED_KEYS[key.toLowerCase()] ?? (key.length === 1 ? key.toUpperCase() : null);
    return k ? [...sym[1], k] : null;
  }
  const parts = s.split(/\s*[+-]\s*/).filter(Boolean);
  if (parts.length === 1) {
    const named = NAMED_KEYS[parts[0].toLowerCase()];
    return named && parts[0][0] === parts[0][0].toUpperCase() ? [named] : null;
  }
  const mods = parts.slice(0, -1).map((p) => MODIFIERS[p.toLowerCase()]);
  if (mods.some((m) => !m)) return null;
  const last = parts[parts.length - 1];
  const key = NAMED_KEYS[last.toLowerCase()] ?? (last.length === 1 ? last.toUpperCase() : null);
  return key ? [...(mods as string[]), key] : null;
}

function isColor(s: string): boolean {
  if (/^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(s)) return true;
  return /^(?:rgba?|hsla?|oklch|oklab)\([^()]{3,60}\)$/i.test(s);
}

/** A name worth looking up: snake_case, camelCase, PascalCase with an inner
 *  capital, or `call()`. Plain words stay plain. */
function isSymbol(s: string): boolean {
  const name = s.replace(/\(\)$/, '');
  if (!/^[A-Za-z_][A-Za-z0-9_]{2,79}$/.test(name)) return false;
  return s.endsWith('()') || /[a-z0-9]_[a-z0-9]/i.test(name) || /[a-z][A-Z]/.test(name);
}

const BASE_BRANCHES = new Set(['main', 'master', 'develop', 'dev', 'trunk', 'staging']);

/** `feat/cli-oauth`, `origin/main`, `main`: shaped like a branch (and not a
 *  file — those are caught first). */
function isBranchLike(s: string): boolean {
  if (BASE_BRANCHES.has(s)) return true;
  // kebab-case (`rich-formatting`): a common branch shape.
  if (/^[a-z0-9]+(?:-[a-z0-9]+)+$/.test(s) && s.length <= 60) return true;
  if (s.length > 100 || !/^[A-Za-z0-9][\w.-]*(\/[\w.-]+)+$/.test(s)) return false;
  return !s.includes('..') && !s.endsWith('.lock');
}

export function classifyInline(raw: string): InlineRef | null {
  const s = raw.trim();
  if (!s) return null;
  const { path, line } = splitFileRef(s);
  if (isFilePath(path)) return { kind: 'file', path, line };
  // A hash with both digits and letters, so `deadbee`-style words and plain
  // numbers aren't taken for commits.
  if (/^[0-9a-f]{7,40}$/.test(s) && /[0-9]/.test(s) && /[a-f]/.test(s)) return { kind: 'commit', sha: s };
  if (isColor(s)) return { kind: 'color', color: s };
  const keys = parseKeys(s);
  if (keys) return { kind: 'keys', keys };
  if (isBranchLike(s)) return { kind: 'branch', name: s };
  if (isSymbol(s)) return { kind: 'symbol', name: s.replace(/\(\)$/, '') };
  return null;
}

/** What a bare URL is shown as: GitHub URLs by what they point at, others
 *  as host and path, decoded and shortened in the middle. `kind` picks
 *  the glyph shown after the site icon. */
export type UrlLabel = { label: string; kind: 'branch' | 'file' | 'commit' | 'list' | 'repo' | 'page' };

function middle(s: string, max: number): string {
  return s.length <= max ? s : `${s.slice(0, Math.ceil(max * 0.6))}…${s.slice(s.length - Math.floor(max * 0.4) + 1)}`;
}

export function prettyUrl(href: string): UrlLabel | null {
  let u: URL;
  try {
    u = new URL(href);
  } catch {
    return null;
  }
  const host = u.hostname.replace(/^www\./, '');
  const parts = u.pathname.split('/').filter(Boolean).map((p) => {
    try {
      return decodeURIComponent(p);
    } catch {
      return p;
    }
  });
  if (host === 'github.com' && parts.length >= 2) {
    const repo = `${parts[0]}/${parts[1]}`;
    const [, , kind, ...rest] = parts;
    if (!kind) return { label: repo, kind: 'repo' };
    if (kind === 'tree' && rest.length) return { label: `${repo} · ${rest.join('/')}`, kind: 'branch' };
    if (kind === 'blob' && rest.length > 1) {
      const line = /^#L(\d+)/.exec(u.hash)?.[1];
      return { label: `${repo} · ${middle(rest.slice(1).join('/'), 40)}${line ? `:${line}` : ''}`, kind: 'file' };
    }
    if (kind === 'commit' && rest[0]) return { label: `${repo}@${rest[0].slice(0, 7)}`, kind: 'commit' };
    if (kind === 'pulls' || kind === 'issues') {
      const q = u.searchParams.get('q');
      const branch = q && /head:([^\s]+)|branch:([^\s]+)/.exec(q);
      const what = kind === 'pulls' ? 'pull requests' : 'issues';
      return { label: `${repo} · ${what}${branch ? ` for ${branch[1] ?? branch[2]}` : ''}`, kind: 'list' };
    }
    if (kind === 'actions' && rest[0] === 'runs' && rest[1]) return { label: `${repo} · run ${rest[1]}`, kind: 'page' };
    return { label: `${repo} · ${middle(rest.length ? `${kind}/${rest.join('/')}` : kind, 40)}`, kind: 'page' };
  }
  const path = parts.join('/');
  return { label: middle(path ? `${host}/${path}` : host, 56), kind: 'page' };
}
