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

export function githubAppAvailable(): boolean { return false; }
async function call<T>(_path: string, _init: RequestInit = {}, _auth = true): Promise<T> { throw new Error('Hosted GitHub App integration is unavailable. Use a GitHub token.'); }

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
