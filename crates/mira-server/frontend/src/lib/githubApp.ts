/**
 * The Runmira GitHub App, through the `github-app` Supabase edge function
 * (see `supabase/functions/github-app`). The function holds the app's
 * private key; Mira only ever gets short-lived tokens for one repository.
 */
import { getSupabase } from './supabase';

export type AppStatus = { configured: boolean; slug: string | null };

export type AppRepo = {
  full_name: string;
  private: boolean;
  enabled: boolean;
  /** You're an admin there, which setting it up needs. */
  can_manage: boolean;
};

export type AppInstallation = {
  id: number;
  account: string;
  account_type: string | null;
  repos: AppRepo[];
};

export type AppInstallations = { installations: AppInstallation[]; manage_url: string };

function base(): string | null {
  const url = import.meta.env.VITE_SUPABASE_URL as string | undefined;
  return url ? `${url.replace(/\/+$/, '')}/functions/v1/github-app` : null;
}

export function githubAppAvailable(): boolean {
  return base() !== null && getSupabase() !== null;
}

async function call<T>(path: string, init: RequestInit = {}, auth = true): Promise<T> {
  const root = base();
  if (!root) throw new Error('Supabase isn’t configured for this build.');
  const headers: Record<string, string> = { ...(init.headers as Record<string, string> ?? {}) };
  if (init.body) headers['content-type'] = 'application/json';
  if (auth) {
    const { data } = await getSupabase()!.auth.getSession();
    const token = data.session?.access_token;
    if (!token) throw new Error('Sign in to Mira first.');
    headers.authorization = `Bearer ${token}`;
  }
  const r = await fetch(`${root}${path}`, { ...init, headers });
  const body = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(body?.error ?? `GitHub app ${path}: ${r.status}`);
  return body as T;
}

export const appStatus = () => call<AppStatus>('/status', {}, false);

/** Start installing: returns the GitHub URL to open. GitHub sends the
 *  user back to `returnTo` with `?github=connected` (or `github_error`). */
export const startInstall = (returnTo: string) =>
  call<{ url: string }>('/start', { method: 'POST', body: JSON.stringify({ return_to: returnTo }) });

export const listInstallations = () => call<AppInstallations>('/installations');

export const repoToken = (installationId: number, repo: string) =>
  call<{ token: string; expires_at: string }>('/token', {
    method: 'POST',
    body: JSON.stringify({ installation_id: installationId, repo }),
  });

/** Owner-only, once: create the app on GitHub from its manifest. */
export async function createApp(key: string, org: string, name: string): Promise<void> {
  const q = new URLSearchParams({ key, name });
  if (org.trim()) q.set('org', org.trim());
  const { action, manifest } = await call<{ action: string; manifest: unknown }>(
    `/manifest?${q}`,
    {},
    false,
  );
  // GitHub takes the manifest as a form post from the browser.
  const form = document.createElement('form');
  form.method = 'post';
  form.action = action;
  const input = document.createElement('input');
  input.type = 'hidden';
  input.name = 'manifest';
  input.value = JSON.stringify(manifest);
  form.appendChild(input);
  document.body.appendChild(form);
  form.submit();
}
