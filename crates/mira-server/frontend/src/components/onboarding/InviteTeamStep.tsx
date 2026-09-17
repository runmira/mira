import { useEffect, useState } from 'react';
import { getSupabase } from '../../lib/supabase';
import { markOnboardingComplete } from './persist';
import type { Profile } from './OnboardingFlow';

const MAX_ROWS = 5;

/**
 * Step 3 (team accounts only): capture teammate emails as pending invites,
 * then mark onboarding complete. Skippable — no invites is a valid outcome.
 */
export function InviteTeamStep({
  userId,
  onSaved,
}: {
  userId: string;
  onSaved: (profile: Profile) => void;
}) {
  const [emails, setEmails] = useState<string[]>(['', '', '']);
  const [teamName, setTeamName] = useState<string | null>(null);
  const [teamId, setTeamId] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const supabase = getSupabase();
    if (!supabase) return;
    supabase
      .from('teams')
      .select('id, name')
      .eq('owner_id', userId)
      .limit(1)
      .maybeSingle()
      .then(({ data }) => {
        if (cancelled) return;
        setTeamId(data?.id ?? null);
        setTeamName(data?.name ?? null);
      });
    return () => {
      cancelled = true;
    };
  }, [userId]);

  function updateAt(i: number, value: string) {
    setEmails((prev) => prev.map((e, j) => (j === i ? value : e)));
  }

  function addRow() {
    setEmails((prev) => (prev.length >= MAX_ROWS ? prev : [...prev, '']));
  }

  async function submit(skip: boolean) {
    setError(null);
    setPending(true);
    try {
      if (!skip) {
        const cleaned = emails.map((e) => e.trim().toLowerCase()).filter((e) => e.length > 0);
        if (cleaned.length === 0) {
          setError('Add at least one email — or skip.');
          setPending(false);
          return;
        }
        const bad = cleaned.filter((e) => !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(e));
        if (bad.length > 0) {
          setError(`Not a valid email: ${bad[0]}`);
          setPending(false);
          return;
        }
        if (!teamId) throw new Error('No team found for your account.');
        const supabase = getSupabase();
        if (!supabase) throw new Error('Supabase not configured');
        const rows = cleaned.map((email) => ({
          team_id: teamId,
          email,
          invited_by: userId,
          role: 'member',
        }));
        const { error: insertErr } = await supabase
          .from('team_invites')
          .upsert(rows, { onConflict: 'team_id,email' });
        if (insertErr) throw new Error(insertErr.message);
      }
      const done = await markOnboardingComplete(userId);
      onSaved(done);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="flex flex-col gap-8">
      <header className="flex flex-col gap-2">
        <p className="text-[11px] uppercase tracking-wider text-muted-foreground">Step 3</p>
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">
          Invite your teammates
        </h1>
        <p className="text-[13px] text-muted-foreground">
          {teamName ? (
            <>
              Add people to <span className="font-medium text-foreground">{teamName}</span>. You can
              always invite more later.
            </>
          ) : (
            <>Add people to your team. You can always invite more later.</>
          )}
        </p>
      </header>

      <div className="flex flex-col gap-3">
        {emails.map((email, i) => (
          <input
            key={i}
            type="email"
            value={email}
            onChange={(e) => updateAt(i, e.target.value)}
            placeholder="teammate@company.com"
            className="rounded-md border border-border bg-card px-3 py-2 text-[14px] text-foreground outline-none focus:ring-2 focus:ring-mira-blue"
          />
        ))}
        {emails.length < MAX_ROWS && (
          <button
            type="button"
            onClick={addRow}
            className="self-start text-[13px] text-muted-foreground transition-colors hover:text-foreground"
          >
            + Add another
          </button>
        )}
      </div>

      {error && <p className="text-[13px] text-destructive">{error}</p>}

      <div className="flex items-center justify-between">
        <button
          type="button"
          onClick={() => void submit(true)}
          disabled={pending}
          className="text-[13px] text-muted-foreground transition-colors hover:text-foreground disabled:opacity-60"
        >
          Skip for now
        </button>
        <button
          type="button"
          onClick={() => void submit(false)}
          disabled={pending}
          className="rounded-md bg-mira-blue px-5 py-2.5 text-[13.5px] font-medium text-white shadow-sm transition hover:opacity-90 disabled:opacity-60"
        >
          {pending ? 'Sending…' : 'Send invites'}
        </button>
      </div>
    </section>
  );
}
