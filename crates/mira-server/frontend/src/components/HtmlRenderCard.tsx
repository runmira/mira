import { useState } from 'react';
import { Maximize2, Minimize2 } from 'lucide-react';

/**
 * A self-contained page an agent published to the chat. Rendered in a
 * sandboxed frame with scripts allowed but no same-origin access, so the
 * page can draw charts but can't reach Mira, its storage or its APIs.
 */
export function HtmlRenderCard({ title, html }: { title: string; html: string }) {
  const [tall, setTall] = useState(false);
  return (
    <figure className="overflow-hidden rounded-xl border border-border/70 bg-background">
      <figcaption className="flex items-center gap-2 border-b border-border/60 px-3 py-1.5 text-[12px] text-muted-foreground">
        <span className="min-w-0 flex-1 truncate font-medium text-foreground/80">{title}</span>
        <button
          type="button"
          className="rounded p-1 hover:bg-muted"
          aria-label={tall ? 'Shrink page' : 'Expand page'}
          onClick={() => setTall((t) => !t)}
        >
          {tall ? <Minimize2 className="size-3.5" /> : <Maximize2 className="size-3.5" />}
        </button>
      </figcaption>
      <iframe
        title={title}
        srcDoc={html}
        sandbox="allow-scripts"
        referrerPolicy="no-referrer"
        className="block w-full bg-white"
        style={{ height: tall ? '80vh' : 420 }}
      />
    </figure>
  );
}
