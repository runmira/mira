import { useQuery } from '@tanstack/react-query';
import { KeyRound, X } from 'lucide-react';
import { cn } from '@/lib/utils';
import { PREF_KEYS, useStringPref } from '../lib/prefs';
import { prQueries } from '../state/pullRequests/queries';
import { useResource } from '../state/resources/hooks';
import { useOptionalRuntime } from '../state/RuntimeProvider';

/** The saved GitHub key, as Settings shows it: what the status query keys on. */
export function useGithubKeyStatus() {
  const runtime = useOptionalRuntime();
  const settings = useResource('settings');
  const saved = settings.data?.keys.find((k) => k.name === 'GITHUB_TOKEN');
  const savedKey = saved ? saved.masked || (saved.from_env ? '$env' : '') : '';
  return useQuery({ ...prQueries.githubKey(runtime, savedKey), enabled: !!settings.data });
}

/** A card in the main pane's corner when GitHub rejected the saved key. If
 *  the `gh` login stood in, it's a tidy-up; if not, GitHub is unreachable.
 *  Dismissing hides it until a different key is rejected. */
export function GithubKeyNotice({ onOpenSettings }: { onOpenSettings: () => void }) {
  const status = useGithubKeyStatus().data;
  const [dismissed, setDismissed] = useStringPref(PREF_KEYS.githubKeyNoticeDismissed, '');
  if (!status?.rejected || !status.fingerprint || status.fingerprint === dismissed) return null;
  const fallback = status.cli_login;
  return (
    <div
      role="status"
      className={cn(
        'absolute right-3 top-14 z-40 w-80 rounded-xl border bg-popover px-3.5 py-3 text-[12.5px] shadow-[0_12px_32px_-12px_rgba(0,0,0,0.5)] animate-fade-in',
        fallback ? 'border-border' : 'border-red-500/40',
      )}
    >
      <div className="flex items-start gap-2.5">
        <KeyRound className={cn('mt-0.5 size-4 shrink-0', fallback ? 'text-amber-500' : 'text-red-500')} />
        <div className="min-w-0 flex-1">
          <div className="font-medium text-foreground">
            {fallback ? 'GitHub rejected your saved key' : 'GitHub sign-in expired'}
          </div>
          <p className="mt-0.5 leading-relaxed text-muted-foreground">
            {fallback
              ? 'Mira is using your gh login instead. Replace or remove the key so requests stop trying it first.'
              : "Pull requests and issues won't load. Replace the key, or run gh auth login on the machine running Mira."}
          </p>
          <button
            type="button"
            onClick={onOpenSettings}
            className="mt-2 rounded-md bg-fg/[0.07] px-2.5 py-1 font-medium text-foreground transition-colors hover:bg-fg/[0.12]"
          >
            Open Search &amp; keys
          </button>
        </div>
        <button
          type="button"
          aria-label="Dismiss"
          onClick={() => setDismissed(status.fingerprint!)}
          className="-mr-1 -mt-0.5 rounded p-1 text-muted-foreground/70 transition-colors hover:bg-fg/[0.06] hover:text-foreground"
        >
          <X className="size-3.5" />
        </button>
      </div>
    </div>
  );
}
