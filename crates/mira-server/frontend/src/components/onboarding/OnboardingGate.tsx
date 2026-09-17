import { useEffect, useState } from 'react';
import { getSupabase } from '../../lib/supabase';
import { useCurrentUser } from '../../lib/useCurrentUser';
import { OnboardingFlow, type Profile } from './OnboardingFlow';

type Status = 'loading' | 'onboarding' | 'done' | 'error';

/**
 * Sits between AuthGate and App. Reads `profiles` for the current user;
 * renders the onboarding flow if it isn't complete, otherwise passes
 * through to the app. Uses the same table apps/web writes to, so finishing
 * onboarding in either surface unlocks both.
 */
export function OnboardingGate({ children }: { children: React.ReactNode }) {
  const user = useCurrentUser();
  const [status, setStatus] = useState<Status>('loading');
  const [profile, setProfile] = useState<Profile | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!user) return;
    let cancelled = false;
    void loadProfile(user.id).then((res) => {
      if (cancelled) return;
      if ('error' in res) {
        setError(res.error);
        setStatus('error');
        return;
      }
      setProfile(res.profile);
      setStatus(res.profile?.onboarding_completed_at ? 'done' : 'onboarding');
    });
    return () => {
      cancelled = true;
    };
  }, [user?.id]);

  if (!user || status === 'loading') {
    return (
      <div className="flex h-screen items-center justify-center text-sm text-muted-foreground">
        Loading…
      </div>
    );
  }

  if (status === 'error') {
    return (
      <div className="mx-auto flex min-h-screen max-w-md flex-col items-center justify-center gap-3 px-6 text-center">
        <h1 className="text-lg font-semibold">Couldn't load your profile</h1>
        <p className="text-sm text-muted-foreground">{error}</p>
        <p className="text-xs text-muted-foreground">
          Make sure both SQL migrations ran in Supabase (profiles + team_invites).
        </p>
      </div>
    );
  }

  if (status === 'onboarding') {
    return (
      <OnboardingFlow
        userId={user.id}
        initialProfile={profile}
        userMetadata={user.user_metadata ?? {}}
        onDone={() => setStatus('done')}
      />
    );
  }

  return <>{children}</>;
}

async function loadProfile(
  userId: string,
): Promise<{ profile: Profile | null } | { error: string }> {
  const supabase = getSupabase();
  if (!supabase) return { error: 'Supabase not configured' };
  const { data, error } = await supabase
    .from('profiles')
    .select('id, account_type, full_name, onboarding_completed_at')
    .eq('id', userId)
    .maybeSingle();
  if (error) return { error: error.message };
  return { profile: (data ?? null) as Profile | null };
}
