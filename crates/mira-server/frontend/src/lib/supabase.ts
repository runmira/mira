import type { SupabaseClient } from '@supabase/supabase-js';

let cached: SupabaseClient | null = null;
let missing = false;
let loading: Promise<void> | null = null;

/**
 * Lazy singleton so we don't error out on module import when the env vars
 * are missing — the auth gate surfaces a friendly message instead.
 */
export function getSupabase(): SupabaseClient | null {
  if (cached) return cached;
  if (missing) return null;
  const url = import.meta.env.VITE_SUPABASE_URL;
  const key = import.meta.env.VITE_SUPABASE_PUBLISHABLE_KEY;
  if (!url || !key) {
    missing = true;
    return null;
  }
  return cached;
}

export function isSupabaseConfigured(): boolean {
  return getSupabase() !== null;
}


/** Load the auth SDK only for builds with a configured account backend. */
export function initializeSupabase(): Promise<void> {
  if (loading) return loading;
  const url = import.meta.env.VITE_SUPABASE_URL;
  const key = import.meta.env.VITE_SUPABASE_PUBLISHABLE_KEY;
  if (!url || !key) { missing = true; return Promise.resolve(); }
  loading = import('@supabase/supabase-js').then(({ createClient }) => {
  cached = createClient(url, key, {
    auth: {
      persistSession: true,
      autoRefreshToken: true,
      detectSessionInUrl: true,
      flowType: 'pkce',
    },
  });
  });
  return loading;
}
