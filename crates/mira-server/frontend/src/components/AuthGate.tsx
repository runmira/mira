import { useEffect, useState } from 'react';
import type { User } from '@supabase/supabase-js';
import { getSupabase, isSupabaseConfigured } from '../lib/supabase';
import { Login } from './Login';

type Status = 'loading' | 'signed-out' | 'signed-in';

/**
 * Gates the whole app behind Supabase auth. Uses `getUser()` (which
 * revalidates the JWT against the auth server) rather than the cached
 * `getSession()`, so a user deleted in the Supabase dashboard is booted
 * on the next page load instead of getting up to an hour of stale access.
 */
export function AuthGate({ children }: { children: React.ReactNode }) {
  const [status, setStatus] = useState<Status>('loading');
  const [_user, setUser] = useState<User | null>(null);

  useEffect(() => {
    const supabase = getSupabase();
    if (!supabase) {
      setStatus('signed-out');
      return;
    }

    let cancelled = false;

    async function validate() {
      const { data, error } = await supabase!.auth.getUser();
      if (cancelled) return;
      if (error || !data.user) {
        // Any error from getUser() with a cached session means the token
        // isn't valid anymore (deleted user, revoked, expired refresh). Clear
        // the local session so the UI stops showing "you're signed in".
        try {
          await supabase!.auth.signOut({ scope: 'local' });
        } catch {
          /* ignore */
        }
        setUser(null);
        setStatus('signed-out');
        return;
      }
      setUser(data.user);
      setStatus('signed-in');
    }

    void validate();

    // Clean the OAuth `?code=...` off the URL after Supabase has consumed
    // it, so a refresh doesn't try to redeem it again.
    if (typeof window !== 'undefined') {
      const url = new URL(window.location.href);
      if (url.searchParams.has('code') || url.searchParams.has('error')) {
        url.searchParams.delete('code');
        url.searchParams.delete('error');
        url.searchParams.delete('error_description');
        url.searchParams.delete('state');
        window.history.replaceState({}, '', url.pathname + url.search + url.hash);
      }
    }

    const { data: sub } = supabase.auth.onAuthStateChange((_event, session) => {
      // Session change fired locally — re-validate against the server so a
      // stale but present JWT can't leave us stuck in `signed-in`.
      if (!session) {
        setUser(null);
        setStatus('signed-out');
        return;
      }
      void validate();
    });

    return () => {
      cancelled = true;
      sub.subscription.unsubscribe();
    };
  }, []);

  if (!isSupabaseConfigured()) {
    return <UnconfiguredNotice />;
  }

  if (status === 'loading') {
    return (
      <div className="flex h-screen items-center justify-center text-sm text-neutral-500">
        Loading…
      </div>
    );
  }

  if (status === 'signed-out') {
    return <Login />;
  }

  return <>{children}</>;
}

function UnconfiguredNotice() {
  return (
    <div className="mx-auto flex min-h-screen max-w-md flex-col items-center justify-center gap-3 px-6 text-center">
      <h1 className="text-lg font-semibold">Auth not configured</h1>
      <p className="text-sm text-neutral-500">
        Set <code className="rounded bg-neutral-800/50 px-1 py-0.5 text-xs">VITE_SUPABASE_URL</code>{' '}
        and{' '}
        <code className="rounded bg-neutral-800/50 px-1 py-0.5 text-xs">
          VITE_SUPABASE_PUBLISHABLE_KEY
        </code>{' '}
        in <code className="rounded bg-neutral-800/50 px-1 py-0.5 text-xs">frontend/.env.local</code>{' '}
        and rebuild.
      </p>
    </div>
  );
}
