import { useState } from 'react';
import { Globe2 } from 'lucide-react';
import { cn } from '@/lib/utils';
import { useBrowserPage } from '@/lib/browserPage';

/** A round toolbar button, the browser header's basic control. */
export function RoundBtn({ children, title, onClick, disabled, active }: {
  children: React.ReactNode;
  title: string;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'flex size-7 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground disabled:opacity-30 disabled:hover:bg-transparent',
        active && 'bg-fg/[0.06] text-foreground',
      )}
    >
      {children}
    </button>
  );
}

/** A site's icon, or a globe when there's none (or it fails to load). */
export function SiteFavicon({ src, className }: { src: string | null; className?: string }) {
  const [failed, setFailed] = useState<string | null>(null);
  if (!src || failed === src) return <Globe2 className={cn('shrink-0 text-muted-foreground/70', className)} />;
  return (
    <img
      key={src}
      src={src}
      alt=""
      referrerPolicy="no-referrer"
      onError={() => setFailed(src)}
      className={cn('shrink-0 rounded-[3px] object-contain', className)}
    />
  );
}

/** The browser tab's icon in the panel strip: the current site's. */
export function BrowserTabIcon() {
  const { url, icon } = useBrowserPage();
  return <SiteFavicon src={url ? icon : null} className="size-3.5" />;
}

/** `https://www.example.com/a/b?q` → `example.com/a/b`, for reading. */
function pretty(url: string) {
  try {
    const u = new URL(url);
    const path = u.pathname === '/' ? '' : u.pathname;
    return `${u.host.replace(/^www\./, '')}${path}`;
  } catch {
    return url;
  }
}

/**
 * The address capsule: the site's icon, its address (tidy at rest, the
 * full URL while editing), and the live dot when the agent browser is
 * streaming.
 */
export function UrlCapsule({ url, icon, live, onNavigate }: {
  url: string;
  icon: string | null;
  live: boolean;
  onNavigate: (target: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  return (
    <form
      className={cn(
        'flex h-7 min-w-0 flex-1 items-center gap-2 rounded-full border bg-card px-2.5 transition-colors',
        editing ? 'border-mira-blue/50 ring-2 ring-mira-blue/15' : 'border-border/70 hover:border-border',
      )}
      onSubmit={(e) => {
        e.preventDefault();
        const target = draft.trim();
        if (target) onNavigate(target);
        (document.activeElement as HTMLElement | null)?.blur();
      }}
    >
      <SiteFavicon src={url ? icon : null} className="size-3.5" />
      <input
        value={editing ? draft : pretty(url)}
        onFocus={(e) => {
          setDraft(url);
          setEditing(true);
          requestAnimationFrame(() => e.target.select());
        }}
        onBlur={() => setEditing(false)}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') (e.target as HTMLInputElement).blur();
        }}
        placeholder="Search or enter address"
        spellCheck={false}
        className={cn(
          'min-w-0 flex-1 bg-transparent text-[12px] text-foreground outline-none placeholder:text-muted-foreground/60',
          !editing && 'text-center',
        )}
      />
      <span
        title={live ? 'Live: the browser Mira and your agents use' : 'Not connected'}
        className={cn('size-1.5 shrink-0 rounded-full', live ? 'bg-emerald-400' : 'bg-muted-foreground/30')}
      />
    </form>
  );
}
