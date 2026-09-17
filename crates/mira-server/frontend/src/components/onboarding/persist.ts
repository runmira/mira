import { getSupabase } from '../../lib/supabase';
import type { Profile } from './OnboardingFlow';

/**
 * Upsert a partial profile and return the row after the write. Using
 * upsert (not update) means we recover gracefully if the auth-user-created
 * trigger somehow didn't fire — the client owns the safety net.
 */
export async function upsertProfile(
  userId: string,
  patch: Partial<Omit<Profile, 'id'>>,
): Promise<Profile> {
  const supabase = getSupabase();
  if (!supabase) throw new Error('Supabase not configured');
  const { data, error } = await supabase
    .from('profiles')
    .upsert({ id: userId, ...patch }, { onConflict: 'id' })
    .select('id, account_type, full_name, onboarding_completed_at')
    .single();
  if (error) throw new Error(error.message);
  return data as Profile;
}

export async function markOnboardingComplete(userId: string): Promise<Profile> {
  return upsertProfile(userId, { onboarding_completed_at: new Date().toISOString() });
}
