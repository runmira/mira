type User = { id: string; email?: string; user_metadata?: Record<string, unknown> };
export function useCurrentUser(): User | null { return null; }

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
