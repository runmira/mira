import { useState } from 'react';
import { cn } from '@/lib/utils';

/** Email addresses — the identifying part of an agent's "signed in as". */
const EMAIL = /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g;

/**
 * Text with any email address frosted over until the user asks to see it.
 *
 * Agent status lines carry the account they are signed in with
 * ("Claude subscription · you@example.com"), and that line sits on screens
 * people share and record. The address stays in the layout — so the line
 * keeps its shape — under a liquid-glass pill: the text blurred beneath a
 * translucent, lightly lit layer. A click reveals it; another hides it.
 */
export function PrivateText({ text, className }: { text: string; className?: string }) {
  const pieces: (string | { email: string })[] = [];
  let last = 0;
  for (const m of text.matchAll(EMAIL)) {
    const at = m.index ?? 0;
    if (at > last) pieces.push(text.slice(last, at));
    pieces.push({ email: m[0] });
    last = at + m[0].length;
  }
  if (last < text.length) pieces.push(text.slice(last));
  return (
    <span className={className}>
      {pieces.map((p, i) => (typeof p === 'string' ? <span key={i}>{p}</span> : <Frosted key={i} value={p.email} />))}
    </span>
  );
}

/** Text with every email address replaced, for `title` tooltips — a
 *  native tooltip cannot be blurred, so it must not carry the address. */
export function redactEmails(text: string): string {
  return text.replace(EMAIL, '••••@••••');
}

function Frosted({ value }: { value: string }) {
  const [shown, setShown] = useState(false);
  return (
    <span
      role="button"
      tabIndex={0}
      aria-label={shown ? 'Hide account' : 'Show account'}
      title={shown ? 'Hide' : 'Click to show'}
      onClick={(e) => {
        e.stopPropagation();
        setShown((v) => !v);
      }}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          e.stopPropagation();
          setShown((v) => !v);
        }
      }}
      className="relative inline-block cursor-pointer rounded-[5px] align-baseline outline-none focus-visible:ring-1 focus-visible:ring-mira-blue/60"
    >
      <span
        className={cn(
          'transition-[filter,opacity] duration-300 ease-out',
          shown ? 'blur-0 opacity-100' : 'select-none opacity-70 blur-[4px]',
        )}
        aria-hidden={!shown}
      >
        {value}
      </span>
      {/* The glass: translucent fill, a backdrop blur, a hairline rim and a
          soft top highlight — the light catching the edge of the pane. */}
      <span
        aria-hidden
        className={cn(
          'pointer-events-none absolute -inset-x-1 -inset-y-[1px] rounded-[6px] transition-opacity duration-300 ease-out',
          'border border-white/15 bg-white/[0.06] backdrop-blur-[6px] backdrop-saturate-150',
          'shadow-[inset_0_1px_0_rgba(255,255,255,0.18),inset_0_-1px_0_rgba(0,0,0,0.25),0_1px_6px_rgba(0,0,0,0.25)]',
          'bg-gradient-to-b from-white/[0.12] to-white/[0.02]',
          shown ? 'opacity-0' : 'opacity-100',
        )}
      />
    </span>
  );
}
