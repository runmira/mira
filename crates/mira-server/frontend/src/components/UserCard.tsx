import { useState } from 'react';
import { CaretUp, SignOut, SlidersHorizontal } from '@phosphor-icons/react';
import { getSupabase } from '../lib/supabase';
import { avatarUrl, displayName as oauthDisplayName, initials, useCurrentUser } from '../lib/useCurrentUser';
import { useProfile } from '../lib/useProfile';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import type { WsStatus } from '../ws';

/**
 * Bottom-of-sidebar user card. Shows avatar + name; click opens a menu
 * with Settings and Log out. The tiny connection dot lives on the avatar
 * so we don't need a separate status strip.
 */
export function UserCard({
  status,
  onOpenSettings,
}: {
  status: WsStatus;
  onOpenSettings: () => void;
}) {
  const user = useCurrentUser();
  const { profile } = useProfile();
  const [open, setOpen] = useState(false);
  const [signingOut, setSigningOut] = useState(false);

  // Prefer the name the user chose in onboarding; fall back to OAuth-provided
  // name (Google / GitHub) if the profile hasn't been read yet or is empty.
  const oauthName = oauthDisplayName(user);
  const name = profile?.fullName?.trim() || oauthName;
  const workspace = workspaceLabel(profile, name);
  const avatar = avatarUrl(user);
  const mono = initials(name);

  async function signOut() {
    setSigningOut(true);
    try {
      const supabase = getSupabase();
      if (supabase) await supabase.auth.signOut();
      // AuthGate's onAuthStateChange listener flips us back to the login
      // screen — no explicit navigation needed.
    } finally {
      setSigningOut(false);
      setOpen(false);
    }
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="flex w-full items-center gap-2.5 border-t border-border px-3 py-2.5 text-left transition-colors hover:bg-accent/60"
        >
          <span className="relative shrink-0">
            <Avatar src={avatar} mono={mono} />
            <span
              className={cn(
                'absolute -bottom-0.5 -right-0.5 size-2 rounded-full ring-2 ring-card',
                dotColor(status),
              )}
              aria-label={status}
            />
          </span>
          <span className="flex min-w-0 flex-1 flex-col text-left">
            <span className="truncate text-[13.5px] font-medium text-foreground">{name}</span>
            {workspace && (
              <span className="truncate text-[11.5px] text-muted-foreground/85">{workspace}</span>
            )}
          </span>
          <CaretUp
            className="size-3.5 shrink-0 text-muted-foreground/70"
            weight="bold"
            aria-hidden
          />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" side="top" sideOffset={6} className="w-56 p-1">
        <div className="flex flex-col">
          <MenuItem
            icon={<SlidersHorizontal className="size-3.5" />}
            onClick={() => {
              setOpen(false);
              onOpenSettings();
            }}
          >
            Settings
          </MenuItem>
          <MenuItem
            icon={<SignOut className="size-3.5" />}
            onClick={() => void signOut()}
            disabled={signingOut}
            danger
          >
            {signingOut ? 'Signing out…' : 'Log out'}
          </MenuItem>
        </div>
      </PopoverContent>
    </Popover>
  );
}

/** Workspace label: `<name>'s Workspace` for personal, real team name for
 *  team accounts. Returns empty string until we've resolved the profile so
 *  we don't briefly show the wrong label. */
function workspaceLabel(
  profile: { accountType: 'personal' | 'team' | null; teamName: string | null } | null,
  name: string,
): string {
  if (!profile) return '';
  if (profile.accountType === 'team') {
    const t = (profile.teamName ?? '').trim();
    return t ? `${t}'s Workspace` : 'Team Workspace';
  }
  if (profile.accountType === 'personal') {
    const trimmed = name.trim();
    if (!trimmed) return 'Personal Workspace';
    return `${trimmed}'s Workspace`;
  }
  return '';
}

function Avatar({ src, mono }: { src: string | null; mono: string }) {
  if (src) {
    return (
      <img
        src={src}
        alt=""
        className="size-8 rounded-full object-cover"
        referrerPolicy="no-referrer"
        draggable={false}
      />
    );
  }
  return (
    <span className="flex size-8 items-center justify-center rounded-full bg-rose-500 text-[11px] font-semibold text-white">
      {mono}
    </span>
  );
}

function MenuItem({
  icon,
  onClick,
  disabled,
  danger,
  children,
}: {
  icon: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors disabled:opacity-60',
        danger ? 'text-destructive hover:bg-destructive/10' : 'text-foreground hover:bg-accent/60',
      )}
    >
      <span className={cn('shrink-0', danger ? '' : 'text-muted-foreground')}>{icon}</span>
      <span>{children}</span>
    </button>
  );
}

function dotColor(s: WsStatus): string {
  switch (s) {
    case 'open':
      return 'bg-emerald-500';
    case 'connecting':
      return 'bg-amber-500';
    case 'closed':
      return 'bg-rose-500';
  }
}
