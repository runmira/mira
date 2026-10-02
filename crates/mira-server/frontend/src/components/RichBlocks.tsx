/**
 * Block-level formatting for replies: diffs you can apply, callouts, and
 * tables you can sort and copy.
 */
import { Children, cloneElement, createContext, isValidElement, useContext, useEffect, useMemo, useState, type ReactElement, type ReactNode } from 'react';
import { ArrowDown, ArrowUp, ArrowUpDown, Check, CircleAlert, Copy, Info, Lightbulb, MessageSquareWarning, OctagonAlert, Wand2 } from 'lucide-react';
import { cn } from '@/lib/utils';

/* ---------------------------------------------------------------- */
/* Diff                                                              */
/* ---------------------------------------------------------------- */

/** Files a unified diff touches (from its `+++` lines, else `---`). */
function diffFiles(text: string): string[] {
  const out: string[] = [];
  for (const l of text.split('\n')) {
    const m = /^\+\+\+ (?:b\/)?(.+?)\s*$/.exec(l) ?? /^--- (?:a\/)?(.+?)\s*$/.exec(l);
    if (m && m[1] !== '/dev/null' && !out.includes(m[1])) out.push(m[1]);
  }
  return out;
}

type ApplyState = 'checking' | 'ready' | 'cannot' | 'applying' | 'applied' | 'failed';

async function applyPatch(patch: string, check: boolean): Promise<string | null> {
  try {
    const r = await fetch('/api/git/apply', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ patch, check }),
    });
    if (r.ok) return null;
    const j = (await r.json().catch(() => ({}))) as { error?: string };
    return j.error ?? `HTTP ${r.status}`;
  } catch (e) {
    return (e as Error).message;
  }
}

export function DiffBlock({ raw }: { raw: string }) {
  const lines = useMemo(() => raw.replace(/\n$/, '').split('\n'), [raw]);
  const files = useMemo(() => diffFiles(raw), [raw]);
  const added = lines.filter((l) => l.startsWith('+') && !l.startsWith('+++')).length;
  const removed = lines.filter((l) => l.startsWith('-') && !l.startsWith('---')).length;
  // Only a real unified diff (file headers and a hunk) can be applied.
  const applicable = files.length > 0 && lines.some((l) => l.startsWith('@@'));
  const [state, setState] = useState<ApplyState>('checking');
  const [reason, setReason] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!applicable) return;
    let live = true;
    void applyPatch(raw, true).then((err) => {
      if (!live) return;
      setState(err ? 'cannot' : 'ready');
      setReason(err);
    });
    return () => {
      live = false;
    };
  }, [raw, applicable]);

  async function apply() {
    setState('applying');
    const err = await applyPatch(raw, false);
    setState(err ? 'failed' : 'applied');
    setReason(err);
    if (!err) window.dispatchEvent(new Event('mira:repo-changed'));
  }

  return (
    <div className="md-code md-diff">
      <div className="md-code-head">
        <span className="md-code-lang">diff</span>
        {files.length > 0 && (
          <span className="min-w-0 truncate font-mono text-[11px] text-foreground/80 dark:text-[#a9b1d6]" title={files.join('\n')}>
            {files.length === 1 ? files[0] : `${files.length} files`}
          </span>
        )}
        <span className="font-mono text-[11px]">
          <span className="text-emerald-400">+{added}</span> <span className="text-red-400">−{removed}</span>
        </span>
        <span className="flex-1" />
        <button
          className="md-code-copy"
          onClick={() => {
            void navigator.clipboard.writeText(raw).then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1200);
            });
          }}
        >
          {copied ? 'copied' : 'copy'}
        </button>
        {applicable && (
          <button
            className={cn('md-diff-apply', state)}
            disabled={state !== 'ready' && state !== 'failed'}
            onClick={() => void apply()}
            title={
              state === 'cannot'
                ? `Doesn't apply to your files: ${reason ?? ''}`
                : state === 'applied'
                  ? 'Applied to your files'
                  : 'Apply this change to your files'
            }
          >
            {state === 'applied' ? (
              <>
                <Check className="size-3" /> applied
              </>
            ) : state === 'applying' ? (
              'applying…'
            ) : state === 'checking' ? (
              'checking…'
            ) : state === 'cannot' ? (
              'already applied or outdated'
            ) : (
              <>
                <Wand2 className="size-3" /> apply
              </>
            )}
          </button>
        )}
      </div>
      <pre className="md-diff-body">
        {lines.map((l, i) => {
          const kind = l.startsWith('+++') || l.startsWith('---') || l.startsWith('diff ') || l.startsWith('index ')
            ? 'file'
            : l.startsWith('@@')
              ? 'hunk'
              : l.startsWith('+')
                ? 'add'
                : l.startsWith('-')
                  ? 'del'
                  : 'ctx';
          return (
            <div key={i} className={`md-diff-line ${kind}`}>
              {l || ' '}
            </div>
          );
        })}
      </pre>
      {state === 'failed' && reason && <div className="md-diff-error">{reason}</div>}
    </div>
  );
}

/* ---------------------------------------------------------------- */
/* Callouts                                                          */
/* ---------------------------------------------------------------- */

const CALLOUT_KINDS = ['note', 'tip', 'important', 'warning', 'caution'] as const;
type CalloutKind = (typeof CALLOUT_KINDS)[number];

type MdNode = { type: string; value?: string; children?: MdNode[]; data?: { hProperties?: Record<string, unknown> } };

/** remark plugin: GitHub's `> [!NOTE]` alerts. Marks the blockquote and
 *  removes the marker; the blockquote renderer draws the rest. */
export function remarkCallouts() {
  return (tree: MdNode) => {
    const walk = (n: MdNode) => {
      if (n.type === 'blockquote') {
        const p = n.children?.[0];
        const t = p?.type === 'paragraph' ? p.children?.[0] : undefined;
        const m = t?.type === 'text' ? /^\[!(note|tip|important|warning|caution)\][ \t]*\n?/i.exec(t.value ?? '') : null;
        if (m && t) {
          t.value = (t.value ?? '').slice(m[0].length);
          n.data = { ...n.data, hProperties: { ...(n.data?.hProperties ?? {}), dataCallout: m[1].toLowerCase() } };
        }
      }
      n.children?.forEach(walk);
    };
    walk(tree);
  };
}

const CALLOUT: Record<CalloutKind, { label: string; Icon: React.ComponentType<{ className?: string }> }> = {
  note: { label: 'Note', Icon: Info },
  tip: { label: 'Tip', Icon: Lightbulb },
  important: { label: 'Important', Icon: MessageSquareWarning },
  warning: { label: 'Warning', Icon: CircleAlert },
  caution: { label: 'Caution', Icon: OctagonAlert },
};

export function Blockquote({ children, ...rest }: { children?: ReactNode; 'data-callout'?: string }) {
  const kind = rest['data-callout'] as CalloutKind | undefined;
  if (!kind || !(kind in CALLOUT)) return <blockquote>{children}</blockquote>;
  const { label, Icon } = CALLOUT[kind];
  return (
    <div className={`md-callout ${kind}`} role="note">
      <div className="md-callout-title">
        <Icon className="size-3.5" />
        {label}
      </div>
      <div className="md-callout-body">{children}</div>
    </div>
  );
}

/* ---------------------------------------------------------------- */
/* Tables                                                            */
/* ---------------------------------------------------------------- */

type Hast = { type: string; tagName?: string; value?: string; children?: Hast[] };

function text(n: Hast): string {
  if (n.type === 'text') return n.value ?? '';
  return (n.children ?? []).map(text).join('');
}
function elements(n: Hast | undefined, tag: string): Hast[] {
  return (n?.children ?? []).filter((c) => c.type === 'element' && c.tagName === tag);
}

type Sort = { col: number; dir: 1 | -1 } | null;
const TableCtx = createContext<{ sort: Sort; onSort: (col: number) => void; sortable: boolean } | null>(null);

/** Numbers, sizes and percentages compare as numbers; the rest as text. */
function sortKey(s: string): number | string {
  const n = Number(s.replace(/[,$%\s]|(?<=\d)(ms|s|kb|mb|gb|x)$/gi, ''));
  return s.trim() !== '' && Number.isFinite(n) ? n : s.toLowerCase();
}

function csvCell(s: string): string {
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export function Table({ node, children }: { node?: Hast; children?: ReactNode }) {
  const [sort, setSort] = useState<Sort>(null);
  const [copied, setCopied] = useState(false);
  const head = elements(node, 'thead')[0];
  const body = elements(node, 'tbody')[0];
  const headers = elements(elements(head, 'tr')[0], 'th').map(text);
  const rows = elements(body, 'tr').map((tr) => [...elements(tr, 'td'), ...elements(tr, 'th')].map(text));
  const sortable = rows.length > 2;

  // Reorder the rendered rows; their contents (code, links) stay as rendered.
  const kids = Children.toArray(children).map((c) => {
    if (!sort || !isValidElement(c)) return c;
    const el = c as ReactElement<{ children?: ReactNode }>;
    if (el.type !== 'tbody') return c;
    const trs = Children.toArray(el.props.children).filter(isValidElement);
    const order = trs.map((_, i) => i).sort((a, b) => {
      const x = sortKey(rows[a]?.[sort.col] ?? '');
      const y = sortKey(rows[b]?.[sort.col] ?? '');
      const cmp = typeof x === 'number' && typeof y === 'number' ? x - y : String(x).localeCompare(String(y));
      return cmp * sort.dir;
    });
    return cloneElement(el, {}, order.map((i) => trs[i]));
  });

  const onSort = (col: number) =>
    setSort((s) => (s?.col !== col ? { col, dir: 1 } : s.dir === 1 ? { col, dir: -1 } : null));

  return (
    <TableCtx.Provider value={{ sort, onSort, sortable }}>
      <div className="md-table">
        {rows.length > 3 && (
          <div className="md-table-bar">
            <span>
              {rows.length} rows{sort ? ` · sorted by ${headers[sort.col] || 'column'}` : ''}
            </span>
            <button
              type="button"
              onClick={() => {
                const csv = [headers, ...rows].map((r) => r.map(csvCell).join(',')).join('\n');
                void navigator.clipboard.writeText(csv).then(() => {
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1200);
                });
              }}
            >
              {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
              {copied ? 'copied' : 'copy CSV'}
            </button>
          </div>
        )}
        <div className="md-table-scroll">
          <table>{kids}</table>
        </div>
      </div>
    </TableCtx.Provider>
  );
}

/** A header cell; sortable ones toggle ascending → descending → off. */
export function Th({ node, children, ...rest }: { node?: Hast & { position?: { start: { column: number } } }; children?: ReactNode; style?: React.CSSProperties }) {
  const ctx = useContext(TableCtx);
  const [col, setCol] = useState(-1);
  if (!ctx?.sortable) return <th style={rest.style}>{children}</th>;
  const active = ctx.sort?.col === col;
  const Icon = !active ? ArrowUpDown : ctx.sort!.dir === 1 ? ArrowUp : ArrowDown;
  return (
    <th style={rest.style} aria-sort={active ? (ctx.sort!.dir === 1 ? 'ascending' : 'descending') : undefined}>
      <button
        type="button"
        className="md-th-sort"
        ref={(b) => {
          // The column, from the cell's position among its siblings.
          if (b?.parentElement && col < 0) setCol(Array.from(b.parentElement.parentElement?.children ?? []).indexOf(b.parentElement));
        }}
        onClick={() => col >= 0 && ctx.onSort(col)}
      >
        {children}
        <Icon className={cn('size-3 shrink-0', active ? 'opacity-90' : 'opacity-30')} />
      </button>
    </th>
  );
}
