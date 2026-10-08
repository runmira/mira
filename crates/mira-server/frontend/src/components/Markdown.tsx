import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';
import rehypeSanitize, { defaultSchema } from 'rehype-sanitize';
import { useEffect, useMemo, useState } from 'react';
import 'highlight.js/styles/atom-one-dark.css';
import type { DiffPreview } from '../types';
import { GithubRef } from './GithubRef';
import { linkifyGithubRefs, parseGithubRef } from '../lib/githubRefs';
import { classifyInline, parseWorkspaceFileLink } from '../lib/refs';
import { BranchChip, ColorChip, CommitChip, FileChip, KeysChip, LinkRef, SymbolRef } from './RichRefs';
import { Blockquote, DiffBlock, Table, Th, remarkCallouts } from './RichBlocks';
import { resolveTheme } from '../lib/theme';
import { lazyModule, useLazyModule } from '../lib/lazy';

/**
 * Markdown rendering for assistant / thought content.
 *
 * Enhancements (issue #68), all safe-by-default:
 * - ` ```mermaid ` fences render as diagrams via the mermaid package,
 *   dynamically imported so the base bundle never pays for it. Invalid
 *   diagrams fall back to the source code block with an error note, and
 *   a "view source" toggle is always available.
 * - `$…$` / `$$…$$` render with KaTeX. remark-math (tiny) parses the
 *   delimiters up-front; rehype-katex + the KaTeX stylesheet (heavy) are
 *   dynamically imported the first time a message actually has math.
 * - Raw HTML is parsed (rehype-raw) and passed through rehype-sanitize,
 *   so long answers can use `<details>/<summary>` etc. — script and event
 *   handlers are stripped. Sanitize runs BEFORE rehype-highlight so the
 *   syntax-highlight spans survive.
 */

type Props = {
  text: string;
  onOpenFile?: (path: string, diff: DiffPreview | null, line?: number | null) => void;
  /** File previews retain document text rather than chat reference chips. */
  document?: boolean;
};

// How short a block has to be to be treated as a "one-liner" (path, filename,
// short command) instead of a full code block. Models routinely wrap single
// values in ``` fences where inline `` would have been the right call.
const ONE_LINER_MAX = 80;

/** Anything remark-math would turn into math: $$..$$, \(..\), \[…\], or
 *  a reasonably-shaped inline $..$ (no spaces right inside, no digits
 *  immediately after the closing $ so "$5 and $10" stays prose). */
/** ``` / ~~~ fence info strings — the only code rehype-highlight touches
 *  (`detect: false`), so the highlighter loads only when one is present. */
const FENCE_LANG_RE = /^ {0,3}(?:`{3,}|~{3,})[ \t]*([\w+#.-]+)/gm;

function fenceLanguages(text: string): string[] {
  const out = new Set<string>();
  for (const m of text.matchAll(FENCE_LANG_RE)) {
    const lang = m[1].toLowerCase();
    if (lang !== 'mermaid') out.add(lang);
  }
  return [...out].sort();
}

/** Anything that could be a raw HTML tag or comment. */
const HTML_RE = /<[a-z!/]/i;

// Kept out of the main chunk (issue #72): the highlighter (core + common
// grammars) and rehype-raw (parse5) load the first time a message needs
// them, then stay cached for every later message.
const highlighter = lazyModule(() => import('../lib/highlight'));
const rawHtml = lazyModule(() => import('rehype-raw'));

const MATH_RE =
  /\$\$[\s\S]+?\$\$|\\\[[\s\S]+?\\\]|\\\([\s\S]+?\\\)|\$(?!\s)(?:[^$\n\\]|\\.)+?\$(?!\d)/;

// Sanitize schema: the default already allows details/summary; we add the
// `open` attribute so a model can ship a pre-expanded section.
const SANITIZE_SCHEMA = {
  ...defaultSchema,
  attributes: {
    ...defaultSchema.attributes,
    details: [...(defaultSchema.attributes?.details ?? []), 'open'],
    // Set by remarkCallouts for `> [!NOTE]` alerts.
    blockquote: [...(defaultSchema.attributes?.blockquote ?? []), 'dataCallout'],
  },
};

// Strip anything mermaid's strict mode could conceivably let through:
// script tags and on* event handler attributes. Diagram labels are the
// injection surface (they echo model text), so this runs before the SVG
// ever reaches the DOM.
function sanitizeMermaidSvg(svg: string): string {
  // Mermaid puts HTML labels inside <foreignObject> ("<br>", unclosed
  // tags) — legal HTML but NOT well-formed XML. Parsing with
  // 'image/svg+xml' yields a <parsererror> document (the pink "This page
  // contains the following errors" box), so parse as HTML — foreignObject
  // is an HTML integration point, so lenient parsing keeps the SVG
  // structure intact — then re-serialize the svg as well-formed XML.
  const doc = new DOMParser().parseFromString(svg, 'text/html');
  const el = doc.querySelector('svg');
  if (!el) return svg;
  el.querySelectorAll('script').forEach((n) => n.remove());
  el.querySelectorAll('*').forEach((n) => {
    for (const attr of [...n.attributes]) {
      if (/^on/i.test(attr.name)) n.removeAttribute(attr.name);
    }
  });
  return new XMLSerializer().serializeToString(el);
}

/** Load any fence languages outside the common set once the highlighter
 *  is in; returns a counter that bumps as grammars land. */
function useFenceGrammars(hl: typeof import('../lib/highlight') | null, langs: string[]): number {
  const [n, setN] = useState(0);
  const key = langs.join('\0');
  useEffect(() => {
    if (!hl) return;
    const missing = langs.filter((l) => !hl.isRegistered(l));
    if (!missing.length) return;
    let live = true;
    void Promise.all(missing.map(hl.loadLanguage)).then((ok) => {
      if (live && ok.some(Boolean)) setN((v) => v + 1);
    });
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hl, key]);
  return n;
}

export function Markdown({ text, onOpenFile, document = false }: Props) {
  // `owner/repo#123` mentions become links, rendered as PR/issue chips.
  const linked = useMemo(() => document ? text : linkifyGithubRefs(text), [text, document]);
  const hasMath = MATH_RE.test(text);

  // rehype-katex + katex.min.css load on first use (issue #68 acceptance:
  // KaTeX never touches the base bundle). Until then math nodes fall
  // through the code renderer as plain monospaced text.
  const [katex, setKatex] = useState<typeof import('rehype-katex') | null>(null);
  useEffect(() => {
    if (!hasMath || katex) return;
    let cancelled = false;
    Promise.all([import('rehype-katex'), import('katex/dist/katex.min.css')]).then(
      ([k]) => {
        if (!cancelled) setKatex(k);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [hasMath, katex]);

  // Until they land, code shows unhighlighted and raw HTML is skipped.
  const fenceLangs = useMemo(() => fenceLanguages(text), [text]);
  const hl = useLazyModule(highlighter, fenceLangs.length > 0);
  const html = useLazyModule(rawHtml, HTML_RE.test(text));
  const grammars = useFenceGrammars(hl, fenceLangs);

  const rehypePlugins = useMemo(() => {
    const plugins: any[] = [];
    if (html) plugins.push(html.default);
    plugins.push([rehypeSanitize, SANITIZE_SCHEMA]);
    if (katex) plugins.push([katex.default, { strict: false }]);
    if (hl) plugins.push([hl.rehypeHighlight, { detect: false, ignoreMissing: true, languages: hl.markdownLanguages() }]);
    return plugins;
    // `grammars` bumps when an on-demand language lands.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [html, katex, hl, grammars]);

  return (
    <div className="md">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath, remarkCallouts]}
        rehypePlugins={rehypePlugins}
        components={{
          // react-markdown wraps fenced code in `<pre><code>...</code></pre>`.
          // We manage our own container inside the code renderer, so make the
          // outer <pre> transparent — otherwise the browser's default pre
          // margins/monospace forcing fight with our chip layout.
          pre({ children }: any) {
            return <>{children}</>;
          },
          code({ className, children, node, ...rest }: any) {
            // From the parsed node, not `children`: once a block is
            // highlighted its children are spans, and String() of those
            // is "[object Object]".
            const codeText = node ? hastText(node) : String(children ?? '');
            const raw = codeText.replace(/\n$/, '');
            const lang = /language-([\w-]+)/.exec(className ?? '')?.[1];

            // Diagram fences render as diagrams (lazy-loaded), not code.
            if (lang === 'mermaid') {
              return <MermaidBlock code={raw} />;
            }
            // Diffs render as diffs, and can be applied.
            if (!document && (lang === 'diff' || lang === 'patch')) {
              return <DiffBlock raw={raw} />;
            }

            // react-markdown v9 dropped the `inline` prop. Fall back to a
            // heuristic: treat as inline unless there's a real language
            // hint or a newline (i.e. something the user clearly meant as
            // a code block). Short single-line ``` fences without a lang
            // also render inline — models routinely wrap filenames or
            // one-word commands in ``` where `` was the right call.
            const looksInline = document
              ? !codeText.endsWith('\n') && !lang
              : !raw.includes('\n') && (!lang || lang === 'text') && raw.length <= ONE_LINER_MAX;

            if (looksInline) {
              const isFenced = /\blanguage-/.test(className ?? '');
              const cls = isFenced ? 'md-inline md-inline-fenced' : 'md-inline';
              if (document) {
                return <code className={`${cls} ${className ?? ''}`} {...rest}>{children}</code>;
              }
              // A span that names something gets drawn as that thing.
              const ref = classifyInline(raw);
              const open = onOpenFile ? (path: string, line?: number | null) => onOpenFile(path, null, line) : undefined;
              switch (ref?.kind) {
                case 'file':
                  return <FileChip path={ref.path} line={ref.line} onOpen={open} />;
                case 'commit':
                  return <CommitChip sha={ref.sha} />;
                case 'color':
                  return <ColorChip color={ref.color}>{children}</ColorChip>;
                case 'keys':
                  return <KeysChip keys={ref.keys} />;
                case 'branch':
                  return <BranchChip name={ref.name}>{children}</BranchChip>;
                case 'symbol':
                  return <SymbolRef name={ref.name} onOpen={open}>{children}</SymbolRef>;
              }
              return (
                <code className={`${cls} ${className ?? ''}`} {...rest}>
                  {children}
                </code>
              );
            }

            return <CodeBlock lang={lang} raw={raw} className={className}>{children}</CodeBlock>;
          },
          a({ children, node: _node, ...rest }: any) {
            const href: string = rest.href ?? '';
            if (document) {
              const local = href && !href.startsWith('#') && !href.startsWith('//') && !/^\w[\w+.-]*:/.test(href);
              return (
                <a
                  {...rest}
                  target={local || href.startsWith('#') ? undefined : '_blank'}
                  rel="noreferrer"
                  onClick={local && onOpenFile ? (e) => {
                    e.preventDefault();
                    const match = /^(.*?)(?:#L(\d+))?$/.exec(href);
                    onOpenFile(match?.[1] ?? href, null, match?.[2] ? Number(match[2]) : undefined);
                  } : undefined}
                >
                  {children}
                </a>
              );
            }
            const target = parseGithubRef(href);
            if (target) return <GithubRef target={target} href={href}>{children}</GithubRef>;
            const fileLink = parseWorkspaceFileLink(href);
            if (fileLink && onOpenFile) {
              const { path, line: at } = fileLink;
              return (
                <a
                  href={href}
                  className="md-link"
                  onClick={(e) => {
                    e.preventDefault();
                    onOpenFile(path, null, at);
                  }}
                >
                  {children}
                </a>
              );
            }
            if (/^https?:\/\//i.test(href)) return <LinkRef href={href}>{children}</LinkRef>;
            return <a target="_blank" rel="noreferrer" {...rest}>{children}</a>;
          },
          blockquote({ node: _node, ...props }: any) {
            return <Blockquote {...props} />;
          },
          table({ node, children }: any) {
            return <Table node={node}>{children}</Table>;
          },
          th({ node, children, style }: any) {
            return <Th node={node} style={style}>{children}</Th>;
          },
        }}
      >
        {linked}
      </ReactMarkdown>
    </div>
  );
}

/** The text of a parsed (hast) node. */
function hastText(n: { type: string; value?: string; children?: unknown[] }): string {
  if (n.type === 'text') return n.value ?? '';
  return ((n.children ?? []) as { type: string; value?: string; children?: unknown[] }[]).map(hastText).join('');
}

/** Code blocks longer than this start collapsed. */
const CODE_COLLAPSE_LINES = 30;
/** Output-like blocks (logs, traces, terminal output) fold sooner: their
 *  head and tail matter, the middle rarely does. */
const LOG_COLLAPSE_LINES = 12;
const LOG_LANGS = new Set(['log', 'text', 'txt', 'console', 'output', 'stacktrace', 'traceback', 'stderr', 'stdout']);

function CodeBlock({
  lang, raw, className, children,
}: { lang?: string; raw: string; className?: string; children: React.ReactNode }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(raw);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch { /* ignore */ }
  }
  const lineCount = raw.replace(/\n$/, '').split('\n').length;
  const foldAt = LOG_LANGS.has(lang ?? 'text') ? LOG_COLLAPSE_LINES : CODE_COLLAPSE_LINES;
  const long = lineCount > foldAt;
  const [expanded, setExpanded] = useState(false);
  const collapsed = long && !expanded;
  return (
    <div className="md-code">
      <div className="md-code-head">
        <span className="md-code-lang">{lang ?? 'text'}</span>
        <button className="md-code-copy" onClick={copy}>{copied ? 'copied' : 'copy'}</button>
      </div>
      <div
        className={collapsed ? 'md-code-body md-code-collapsed' : 'md-code-body'}
        style={{
          maxHeight: collapsed ? `calc(${foldAt} * 1.6em + 0.9rem)` : `calc(${lineCount} * 1.6em + 0.9rem)`,
          transition: 'max-height 200ms ease',
          overflow: 'hidden',
        }}
      >
        {lineCount > 1 && (
          <div className="md-code-gutter" aria-hidden="true">
            {Array.from({ length: lineCount }, (_, i) => (
              <div key={i}>{i + 1}</div>
            ))}
          </div>
        )}
        <pre><code className={className}>{children}</code></pre>
      </div>
      {long && (
        <button className="md-code-more" onClick={() => setExpanded((v) => !v)}>
          {expanded ? 'Show less' : `Show all ${lineCount} lines`}
        </button>
      )}
    </div>
  );
}

let mermaidReady: Promise<typeof import('mermaid')> | null = null;

/** One shared dynamic import + initialize. `securityLevel: 'strict'`
 *  escapes HTML in labels and disables click interactivity — the
 *  sanitization story for model-authored diagrams.
 *  `suppressErrorRendering` stops failed parses from appending mermaid's
 *  "Syntax error in text" bomb diagrams to document.body — one per
 *  streaming re-render, piling up at the bottom of the screen. */
let mermaidTheme: 'dark' | 'default' | null = null;
function loadMermaid(): Promise<typeof import('mermaid')> {
  if (!mermaidReady) mermaidReady = import('mermaid');
  // Re-initialize when the app theme has changed since the last diagram.
  return mermaidReady.then((m) => {
    const theme = resolveTheme() === 'light' ? 'default' : 'dark';
    if (theme !== mermaidTheme) {
      m.default.initialize({
        startOnLoad: false,
        securityLevel: 'strict',
        theme,
        suppressErrorRendering: true,
      });
      mermaidTheme = theme;
    }
    return m;
  });
}

let mermaidSeq = 0;

function MermaidBlock({ code }: { code: string }) {
  const [svg, setSvg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showSource, setShowSource] = useState(false);

  useEffect(() => {
    let cancelled = false;
    loadMermaid()
      .then((m) => m.default.render(`mmd-${++mermaidSeq}`, code))
      .then(({ svg: rendered }) => {
        if (!cancelled) {
          setSvg(sanitizeMermaidSvg(rendered));
          setError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setSvg(null);
          setError(e instanceof Error ? e.message : String(e));
        }
        // A failed render can still leave mermaid's temp containers stuck
        // directly onto document.body — sweep only direct children so
        // successfully rendered diagrams (nested in the transcript) are
        // never touched.
        document.body
          .querySelectorAll(':scope > [id^="mmd-"], :scope > [id^="dmmd-"]')
          .forEach((n) => n.remove());
      });
    return () => {
      cancelled = true;
    };
  }, [code]);

  if (error) {
    // Invalid diagram (often true mid-stream while the fence is still
    // arriving): fall back to the source with a note — issue #68
    // acceptance. It self-heals once the full diagram parses.
    return (
      <div className="flex flex-col gap-1">
        <div className="rounded-md border border-amber-500/30 bg-amber-500/[0.06] px-3 py-2 text-[11.5px] text-amber-300/90">
          This Mermaid diagram couldn't be rendered — showing the source instead.
        </div>
        <CodeBlock lang="mermaid" raw={code}>{code}</CodeBlock>
      </div>
    );
  }

  return (
    <div className="md-code">
      <div className="md-code-head">
        <span className="md-code-lang">mermaid</span>
        <button className="md-code-copy" onClick={() => setShowSource((v) => !v)}>
          {showSource ? 'diagram' : 'view source'}
        </button>
      </div>
      {showSource || svg === null ? (
        <div className="md-code-body">
          <pre><code>{code}</code></pre>
        </div>
      ) : (
        <div
          className="flex justify-center overflow-x-auto p-4 [&_svg]:h-auto [&_svg]:max-w-full"
          dangerouslySetInnerHTML={{ __html: svg }}
        />
      )}
    </div>
  );
}
