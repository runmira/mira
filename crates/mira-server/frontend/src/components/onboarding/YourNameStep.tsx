import { useState } from 'react';
import { upsertProfile } from './persist';
import type { Profile } from './OnboardingFlow';

/**
 * Display name. Team accounts go on to invite teammates; everyone then
 * sets up how Mira runs and can bring their chats over.
 */
export function YourNameStep({
  userId,
  initialProfile,
  fallbackName,
  onSaved,
}: {
  userId: string;
  initialProfile: Profile | null;
  fallbackName: string;
  onSaved: (profile: Profile) => void;
}) {
  const [name, setName] = useState<string>(initialProfile?.full_name || fallbackName);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    const trimmed = name.trim();
    if (trimmed.length === 0) {
      setError('Give us something to call you.');
      return;
    }
    setError(null);
    setPending(true);
    try {
      // Not the last step any more: setting up how Mira runs and
      // bringing chats over come next, for every account.
      onSaved(await upsertProfile(userId, { full_name: trimmed }));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="flex flex-col gap-8">
      <header className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">
          What should we call you?
        </h1>
        <p className="text-[13px] text-muted-foreground">
          This is how your name appears in Mira. You can change it later.
        </p>
      </header>

      <label className="flex flex-col gap-2">
        <span className="text-[13px] font-medium text-foreground/90">Your name</span>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Ada Lovelace"
          className="rounded-md border border-border bg-card px-3 py-2 text-[14px] text-foreground outline-none focus:ring-2 focus:ring-mira-blue"
          autoFocus
          onKeyDown={(e) => {
            if (e.key === 'Enter') void submit();
          }}
        />
      </label>

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
