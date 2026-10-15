/** Login stays in the agent CLI: no provider secret is copied into the page. */
export function openCodeLoginCommand(binaryPath?: string | null): string {
  const binary = binaryPath?.trim() || 'opencode';
  const quoted = /^[a-zA-Z0-9_./-]+$/.test(binary) ? binary : `'${binary.replace(/'/g, "'\\''")}'`;
  return `${quoted} auth login`;
}

/** The one-line status for a terminal sign-in. */
export function terminalSignInStatus(
  view: { phase: string; models: number | null; ready: boolean | null; exit_code: number | null } | null,
  enabled: boolean,
): string {
  if (!enabled) return 'Enable this agent to sign in.';
  switch (view?.phase) {
    case 'running': return 'Finish signing in in the terminal below.';
    case 'refreshing': return 'Signed in. Checking models…';
    case 'succeeded':
      if (!view.ready) return 'Signed in, but the agent is not ready.';
      return view.models ? `Signed in · ${view.models} model${view.models === 1 ? '' : 's'} available` : 'Signed in.';
    case 'failed': return view.exit_code != null ? `Sign-in failed (exit ${view.exit_code}).` : 'Sign-in failed.';
    case 'cancelled': return 'Sign-in cancelled.';
    case 'unsupported': return 'This OpenCode version has no terminal sign-in. Run its login command yourself.';
    default: return 'Connect a provider with OpenCode’s own login.';
  }
}
