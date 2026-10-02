import { useEffect, useState } from 'react';
import { getSupabase } from '../../lib/supabase';
import { upsertProfile } from './persist';
import { Mail, Plus } from 'lucide-react';
import type { Profile } from './OnboardingFlow';
import { ErrorText, GhostButton, PrimaryButton, StepFooter, StepHeader, field } from './ui';

const MAX_ROWS = 5;

/**
 * Team accounts only: capture teammate emails as pending invites.
 * Skippable — no invites is a valid outcome.
 */
export function InviteTeamStep({
  userId,
  onSaved,
  onBack,
}: {
  userId: string;
  onSaved: (profile: Profile) => void;
  onBack?: () => void;
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
      onSaved(await upsertProfile(userId, {}));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="flex flex-col gap-7">
      <StepHeader eyebrow="Your team" title="Invite your teammates">
        {teamName ? (
          <>
            Add people to <span className="font-medium text-foreground">{teamName}</span>. You can always invite more
            later.
          </>
        ) : (
          <>Add people to your team. You can always invite more later.</>
        )}
      </StepHeader>

      <div className="flex flex-col gap-2.5">
        {emails.map((email, i) => (
          <div key={i} className="relative">
            <Mail className="pointer-events-none absolute left-3.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground/60" />
            <input
              type="email"
              value={email}
              onChange={(e) => updateAt(i, e.target.value)}
              placeholder="teammate@company.com"
              className={`${field} pl-10`}
            />
          </div>
        ))}
        {emails.length < MAX_ROWS && (
          <GhostButton onClick={addRow} className="-ml-2 h-8 self-start px-2.5 text-[12.5px]">
            <Plus className="size-3.5" />
            Add another
          </GhostButton>
        )}
      </div>

      {error && <ErrorText>{error}</ErrorText>}

      <StepFooter onBack={onBack}>
        <GhostButton onClick={() => void submit(true)} disabled={pending}>
          Skip for now
        </GhostButton>
        <PrimaryButton pending={pending} onClick={() => void submit(false)}>
          {pending ? 'Sending…' : 'Send invites'}
        </PrimaryButton>
      </StepFooter>
    </section>
  );
}
