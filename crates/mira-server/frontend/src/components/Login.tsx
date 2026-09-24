import { useEffect, useState } from 'react';
import { DESKTOP_AUTH_REDIRECT, isDesktop, onAuthCallback, openExternal } from '../lib/desktop';
import { getSupabase } from '../lib/supabase';

type Provider = 'google' | 'github';

export function Login() {
  const [pending, setPending] = useState<Provider | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Desktop app: the provider sends the system browser back to
  // `mira://auth-callback?code=…`, which the app forwards here. The PKCE
  // verifier was stored by this page, so the exchange happens here too;
  // AuthGate picks up the new session.
  useEffect(() => {
    if (!isDesktop()) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    onAuthCallback(async (url) => {
      const supabase = getSupabase();
      if (!supabase) return;
      const failure = url.searchParams.get('error_description') ?? url.searchParams.get('error');
      const code = url.searchParams.get('code');
      if (failure || !code) {
        setPending(null);
        setError(failure ?? 'Sign-in did not return a code.');
        return;
      }
      const { error } = await supabase.auth.exchangeCodeForSession(code);
      if (error) {
        setPending(null);
        setError(error.message);
      }
    }).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  async function signIn(provider: Provider) {
    const supabase = getSupabase();
    if (!supabase) return;

    setPending(provider);
    setError(null);

    if (isDesktop()) {
      // Providers (Google especially) refuse sign-in inside embedded
      // webviews, so it happens in the system browser.
      const { data, error } = await supabase.auth.signInWithOAuth({
        provider,
        options: { redirectTo: DESKTOP_AUTH_REDIRECT, skipBrowserRedirect: true },
      });
      if (error || !data.url) {
        setPending(null);
        setError(error?.message ?? 'Could not start sign-in.');
        return;
      }
      try {
        await openExternal(data.url);
      } catch (e) {
        setPending(null);
        setError(`Could not open the browser: ${String(e)}`);
      }
      return;
    }

    const { error } = await supabase.auth.signInWithOAuth({
      provider,
      options: {
        redirectTo: window.location.origin,
      },
    });

    if (error) {
      setPending(null);
      setError(error.message);
    }
    // On success the browser is redirected to the provider — no cleanup needed.
  }

  const pendingLabel = isDesktop() ? 'Finish in your browser…' : 'Redirecting…';

  return (
    <div className="mx-auto flex min-h-screen max-w-sm flex-col items-center justify-center gap-8 px-6 py-12">
      <div className="flex flex-col items-center gap-2 text-center">
        <h1 className="text-2xl font-semibold tracking-tight">Sign in to Mira</h1>
        <p className="text-sm text-neutral-500">Continue with Google or GitHub.</p>
      </div>
      <div className="flex w-full flex-col gap-3">
        <button
          type="button"
          onClick={() => signIn('google')}
          disabled={pending !== null}
          className="flex w-full items-center justify-center gap-3 rounded-md border border-neutral-300 bg-white px-4 py-2.5 text-sm font-medium text-neutral-900 shadow-sm transition hover:bg-neutral-50 disabled:opacity-60 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:hover:bg-neutral-800"
        >
          <GoogleGlyph />
          {pending === 'google' ? pendingLabel : 'Continue with Google'}
        </button>
        <button
          type="button"
          onClick={() => signIn('github')}
          disabled={pending !== null}
          className="flex w-full items-center justify-center gap-3 rounded-md bg-neutral-900 px-4 py-2.5 text-sm font-medium text-white shadow-sm transition hover:bg-neutral-800 disabled:opacity-60 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-white"
        >
          <GitHubGlyph />
          {pending === 'github' ? pendingLabel : 'Continue with GitHub'}
        </button>
        {error ? <p className="text-sm text-red-500">{error}</p> : null}
      </div>
    </div>
  );
}

function GoogleGlyph() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
      <path
        fill="#4285F4"
        d="M17.64 9.2c0-.637-.057-1.251-.164-1.84H9v3.481h4.844a4.14 4.14 0 0 1-1.796 2.717v2.258h2.908c1.702-1.567 2.684-3.874 2.684-6.615z"
      />
      <path
        fill="#34A853"
        d="M9 18c2.43 0 4.467-.806 5.956-2.184l-2.908-2.258c-.806.54-1.837.86-3.048.86-2.344 0-4.328-1.584-5.036-3.71H.957v2.332A8.997 8.997 0 0 0 9 18z"
      />
      <path
        fill="#FBBC05"
        d="M3.964 10.708A5.41 5.41 0 0 1 3.682 9c0-.593.102-1.17.282-1.708V4.96H.957A8.997 8.997 0 0 0 0 9c0 1.452.348 2.827.957 4.04l3.007-2.332z"
      />
      <path
        fill="#EA4335"
        d="M9 3.58c1.321 0 2.508.454 3.44 1.345l2.582-2.58C13.463.891 11.426 0 9 0A8.997 8.997 0 0 0 .957 4.96l3.007 2.332C4.672 5.163 6.656 3.58 9 3.58z"
      />
    </svg>
  );
}

function GitHubGlyph() {
  return (
    <svg width="18" height="18" viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
      <path
        fillRule="evenodd"
        d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z"
      />
    </svg>
  );
}
