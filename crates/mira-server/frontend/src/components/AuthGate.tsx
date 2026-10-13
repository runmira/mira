import { RuntimeProvider } from '../state/RuntimeProvider';
import { LoadingScreen } from './LoadingScreen';
import { useEffect, useState } from 'react';
import { LazyBoundary } from './LazyBoundary';
import type { User } from '@supabase/supabase-js';
import { getSupabase, isSupabaseConfigured } from '../lib/supabase';
import { lazyNamed } from '../lib/lazy';

// Signed-out only, so it stays out of every signed-in load.
const Login = lazyNamed(() => import('./Login'), 'Login');

type Status = 'loading' | 'signed-out' | 'signed-in';

/**
 * Gates the whole app behind Supabase auth. Uses `getUser()` (which
 * revalidates the JWT against the auth server) rather than the cached
 * `getSession()`, so a user deleted in the Supabase dashboard is booted
 * on the next page load instead of getting up to an hour of stale access.
 */
export function AuthGate({ children }: { children: React.ReactNode }) {
  const [status, setStatus] = useState<Status>('loading');
  const [user, setUser] = useState<User | null>(null);

  useEffect(() => {
    const supabase = getSupabase();
    if (!supabase) {
      setStatus('signed-out');
      return;
    }

    let cancelled = false;
    let validation = 0;
    let validatedAccount: string | null = null;

    async function validate(expectedAccount?: string) {
      const request = ++validation;
      const { data, error } = await supabase!.auth.getUser();
      if (cancelled || request !== validation) return;
      if (expectedAccount && data.user && data.user.id !== expectedAccount) return;
      if (error || !data.user) {
        // Any error from getUser() with a cached session means the token
        // isn't valid anymore (deleted user, revoked, expired refresh). Clear
        // the local session so the UI stops showing "you're signed in".
        try {
          await supabase!.auth.signOut({ scope: 'local' });
        } catch {
          /* ignore */
        }
        if (cancelled || request !== validation) return;
        validatedAccount = null;
        setUser(null);
        setStatus('signed-out');
        return;
      }
      validatedAccount = data.user.id;
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
        validation++;
        validatedAccount = null;
        setUser(null);
        setStatus('signed-out');
        return;
      }
      if (validatedAccount !== session.user.id) { setUser(null); setStatus('loading'); }
      void validate(session.user.id);
    });

    return () => {
      cancelled = true;
      validation++;
      sub.subscription.unsubscribe();
    };
  }, []);

  if (!isSupabaseConfigured()) {
    return <UnconfiguredNotice />;
  }

  if (status === 'loading') {
    return <LoadingScreen />;
  }

  if (status === 'signed-out') {
    return (
      <LazyBoundary>
        <Login />
      </LazyBoundary>
    );
  }

  return (
    <RuntimeProvider key={user!.id} scope={{ backend: window.location.origin, account: user!.id }}>
      {children}
    </RuntimeProvider>
  );
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
