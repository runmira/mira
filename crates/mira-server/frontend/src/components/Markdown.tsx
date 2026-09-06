import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';
import { useState } from 'react';
import 'highlight.js/styles/tokyo-night-dark.css';

type Props = { text: string };

// How short a block has to be to be treated as a "one-liner" (path, filename,
// short command) instead of a full code block. Models routinely wrap single
// values in ``` fences where inline `` would have been the right call.
const ONE_LINER_MAX = 80;

export function Markdown({ text }: Props) {
  return (
    <div className="md">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeHighlight, { detect: true, ignoreMissing: true }]]}
        components={{
          // react-markdown wraps fenced code in `<pre><code>...</code></pre>`.
          // We manage our own container inside the code renderer, so make the
          // outer <pre> transparent — otherwise the browser's default pre
          // margins/monospace forcing fight with our chip layout.
          pre({ children }: any) {
            return <>{children}</>;
          },
          code({ className, children, ...rest }: any) {
            const raw = String(children ?? '').replace(/\n$/, '');
            const lang = /language-([\w-]+)/.exec(className ?? '')?.[1];

            // react-markdown v9 dropped the `inline` prop. Fall back to a
            // heuristic: treat as inline unless there's a real language
            // hint or a newline (i.e., something the user clearly meant as
            // a code block). Short single-line ``` fences without a lang
            // also render inline — models routinely wrap filenames or
            // one-word commands in ``` where `` was the right call.
            const looksInline =
              !raw.includes('\n') &&
              (!lang || lang === 'text') &&
              raw.length <= ONE_LINER_MAX;

            if (looksInline) {
              // Extra cyan-tinted variant when the source used ``` fences
              // (has a className) so it visually reads as "this is a value",
              // vs mid-sentence `` which stays the default rose tone.
              const isFenced = /\blanguage-/.test(className ?? '');
              const cls = isFenced ? 'md-inline md-inline-fenced' : 'md-inline';
              return (
                <code className={`${cls} ${className ?? ''}`} {...rest}>
                  {children}
                </code>
              );
            }

            return <CodeBlock lang={lang} raw={raw} className={className}>{children}</CodeBlock>;
          },
          a({ children, ...rest }: any) {
            return <a target="_blank" rel="noreferrer" {...rest}>{children}</a>;
          },
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}

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
  return (
    <div className="md-code">
      <div className="md-code-head">
        <span className="md-code-lang">{lang ?? 'text'}</span>
        <button className="md-code-copy" onClick={copy}>{copied ? 'copied' : 'copy'}</button>
      </div>
      <pre><code className={className}>{children}</code></pre>
    </div>
  );
}
