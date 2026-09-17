import { useEffect, useState } from 'react';
import { getSupabase } from './supabase';
import { useCurrentUser } from './useCurrentUser';

export type AccountType = 'personal' | 'team';

export type ProfileView = {
  fullName: string | null;
  accountType: AccountType | null;
  teamName: string | null;
};

/**
 * Fetches the current user's profile row + their team's name (for team
 * accounts). Refetches when the auth user changes.
 */
export function useProfile(): { profile: ProfileView | null; loading: boolean } {
  const user = useCurrentUser();
  const [profile, setProfile] = useState<ProfileView | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!user) {
      setProfile(null);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void (async () => {
      const supabase = getSupabase();
      if (!supabase) {
        if (!cancelled) setLoading(false);
        return;
      }

      const { data: prof } = await supabase
        .from('profiles')
        .select('full_name, account_type')
        .eq('id', user.id)
        .maybeSingle();

      let teamName: string | null = null;
      if (prof?.account_type === 'team') {
        const { data: team } = await supabase
          .from('teams')
          .select('name')
          .eq('owner_id', user.id)
          .limit(1)
          .maybeSingle();
        teamName = team?.name ?? null;
      }

      if (cancelled) return;
      setProfile({
        fullName: (prof?.full_name as string | null) ?? null,
        accountType: (prof?.account_type as AccountType | null) ?? null,
        teamName,
      });
      setLoading(false);
    })();
    return () => {
      cancelled = true;
    };
  }, [user?.id]);

  return { profile, loading };
}
