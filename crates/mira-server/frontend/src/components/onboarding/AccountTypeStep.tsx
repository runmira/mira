import { useState } from 'react';
import { getSupabase } from '../../lib/supabase';
import { upsertProfile } from './persist';
import type { AccountType, Profile } from './OnboardingFlow';
import { cn } from '@/lib/utils';

export function AccountTypeStep({
  userId,
  initialProfile,
  onSaved,
}: {
  userId: string;
  initialProfile: Profile | null;
  onSaved: (profile: Profile) => void;
}) {
  const [selected, setSelected] = useState<AccountType | null>(
    initialProfile?.account_type ?? null,
  );
  const [teamName, setTeamName] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    if (!selected) {
      setError('Pick one to continue.');
      return;
    }
    if (selected === 'team' && teamName.trim().length === 0) {
      setError('Give your team a name.');
      return;
    }
    setError(null);
    setPending(true);
    try {
      const profile = await upsertProfile(userId, { account_type: selected });

      if (selected === 'team') {
        const supabase = getSupabase();
        if (!supabase) throw new Error('Supabase not configured');
        const { data: existing } = await supabase
          .from('teams')
          .select('id')
          .eq('owner_id', userId)
          .limit(1)
          .maybeSingle();

        if (!existing) {
          const { data: team, error: teamErr } = await supabase
            .from('teams')
            .insert({ name: teamName.trim(), owner_id: userId })
            .select('id')
            .single();
          if (teamErr) throw new Error(teamErr.message);
          const { error: memberErr } = await supabase
            .from('team_members')
            .insert({ team_id: team.id, user_id: userId, role: 'owner' });
          if (memberErr) throw new Error(memberErr.message);
        } else {
          await supabase.from('teams').update({ name: teamName.trim() }).eq('id', existing.id);
        }
      }

      onSaved(profile);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="flex flex-col gap-8">
      <header className="flex flex-col gap-2">
        <p className="text-[11px] uppercase tracking-wider text-muted-foreground">Step 1</p>
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">
          Who are you setting up Mira for?
        </h1>
      </header>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Option
          title="Personal"
          description="Just me. I'm exploring Mira for my own projects."
          selected={selected === 'personal'}
          onSelect={() => setSelected('personal')}
        />
        <Option
          title="Team"
          description="For my company, group, or a shared codebase."
          selected={selected === 'team'}
          onSelect={() => setSelected('team')}
        />
      </div>

      {selected === 'team' && (
        <label className="flex flex-col gap-2">
          <span className="text-[13px] font-medium text-foreground/90">Team name</span>
          <input
            value={teamName}
            onChange={(e) => setTeamName(e.target.value)}
            placeholder="Acme, Inc."
            className="rounded-md border border-border bg-card px-3 py-2 text-[14px] text-foreground outline-none focus:ring-2 focus:ring-mira-blue"
            autoFocus
          />
        </label>
      )}

      {error && <p className="text-[13px] text-destructive">{error}</p>}

      <div className="flex justify-end">
        <button
          type="button"
          onClick={() => void submit()}
          disabled={pending}
          className="rounded-md bg-mira-blue px-5 py-2.5 text-[13.5px] font-medium text-white shadow-sm transition hover:opacity-90 disabled:opacity-60"
        >
          {pending ? 'Saving…' : 'Continue'}
        </button>
      </div>
    </section>
  );
}

function Option({
  title,
  description,
  selected,
  onSelect,
}: {
  title: string;
  description: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={cn(
        'flex flex-col items-start gap-1 rounded-lg border p-4 text-left transition',
        selected
          ? 'border-mira-blue bg-accent/60 shadow-sm'
          : 'border-border hover:border-foreground/40',
      )}
    >
      <span className="text-[14px] font-semibold text-foreground">{title}</span>
      <span className="text-[12.5px] text-muted-foreground">{description}</span>
    </button>
  );
}
