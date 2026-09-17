import { useEffect, useState } from 'react';
import type { User } from '@supabase/supabase-js';
import { getSupabase } from './supabase';

/**
 * Subscribes to the current Supabase user. Prefers `getUser()` for the
 * initial read (server-validated) and swaps in the local session on later
 * `onAuthStateChange` events for snappy sign-in / sign-out updates.
 */
export function useCurrentUser(): User | null {
  const [user, setUser] = useState<User | null>(null);

  useEffect(() => {
    const supabase = getSupabase();
    if (!supabase) return;

    let cancelled = false;

    supabase.auth.getUser().then(({ data }) => {
      if (cancelled) return;
      setUser(data.user ?? null);
    });

    const { data: sub } = supabase.auth.onAuthStateChange((_event, session) => {
      setUser(session?.user ?? null);
    });

    return () => {
      cancelled = true;
      sub.subscription.unsubscribe();
    };
  }, []);

  return user;
}

/** Best-effort display name from OAuth metadata, falling back to email local-part. */
export function displayName(user: User | null): string {
  if (!user) return '';
  const meta = user.user_metadata ?? {};
  const name =
    (meta.full_name as string | undefined) ||
    (meta.name as string | undefined) ||
    (meta.user_name as string | undefined);
  if (name && name.trim().length > 0) return name.trim();
  if (user.email) return user.email.split('@')[0];
  return 'Signed in';
}

/** OAuth avatar URL if the provider gave us one. */
export function avatarUrl(user: User | null): string | null {
  if (!user) return null;
  const meta = user.user_metadata ?? {};
  return (meta.avatar_url as string | undefined) || (meta.picture as string | undefined) || null;
}

/** Two-letter monogram from the display name — falls back to `??`. */
export function initials(name: string): string {
  const parts = name.split(/\s+/).filter(Boolean);
  if (parts.length === 0) return '??';
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}
