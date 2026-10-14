/** Login stays in the agent CLI: no provider secret is copied into the page. */
export function openCodeLoginCommand(binaryPath?: string | null): string {
  const binary = binaryPath?.trim() || 'opencode';
  const quoted = /^[a-zA-Z0-9_./-]+$/.test(binary) ? binary : `'${binary.replace(/'/g, "'\\''")}'`;
  return `${quoted} auth login`;
}
