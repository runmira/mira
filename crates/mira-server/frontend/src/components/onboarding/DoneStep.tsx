/**
 * The last step: a short "here's how it works" before the app, then
 * onboarding is recorded as finished.
 */
import { useState } from 'react';
import { ArrowRight, Command, MessageSquareText, Smile } from 'lucide-react';
import { markOnboardingComplete } from './persist';
import type { Profile } from './OnboardingFlow';

const TIPS = [
  {
    Icon: MessageSquareText,
    title: 'Switch engines per chat',
    body: 'The model picker under the composer holds both your providers and your external agents.',
  },
  {
    Icon: Smile,
    title: 'Meet the subagents',
    body: 'Scout, Iris, Atlas and the rest take on focused jobs for Mira. Make them yours in Settings → Subagents.',
  },
  {
    Icon: Command,
    title: 'Everything is a ⌘K away',
    body: 'Chats, settings, the browser, the context window — press ⌘K and type.',
  },
];

export function DoneStep({
  userId,
  imported,
  onSaved,
}: {
  userId: string;
  /** Chats brought over in the previous step. */
  imported: number;
  onSaved: (profile: Profile) => void;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function finish() {
    setPending(true);
    setError(null);
    try {
      onSaved(await markOnboardingComplete(userId));
    } catch (e) {
      setError((e as Error).message);
      setPending(false);
    }
  }

  return (
    <section className="flex flex-col gap-7">
      <header className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">You're all set</h1>
        <p className="text-[13.5px] leading-relaxed text-muted-foreground">
          {imported > 0
            ? `${imported} chat${imported === 1 ? ' is' : 's are'} waiting in your sidebar. A few things worth knowing:`
            : 'A few things worth knowing:'}
        </p>
      </header>
      <ul className="flex flex-col gap-3">
        {TIPS.map(({ Icon, title, body }) => (
          <li key={title} className="flex gap-3 rounded-xl border border-border/60 bg-white/[0.02] p-3.5">
            <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-mira-blue/12 text-mira-blue">
              <Icon className="size-4" />
            </span>
            <span>
              <span className="block text-[13.5px] font-medium text-foreground">{title}</span>
              <span className="mt-0.5 block text-[12.5px] leading-relaxed text-muted-foreground">{body}</span>
            </span>
          </li>
        ))}
      </ul>
      {error && <p className="text-[12.5px] text-destructive">{error}</p>}
      <button
        type="button"
        disabled={pending}
        onClick={() => void finish()}
        className="flex items-center justify-center gap-2 self-start rounded-lg bg-mira-blue px-5 py-2.5 text-[13.5px] font-medium text-white transition-opacity disabled:opacity-50"
      >
        {pending ? 'Opening…' : 'Open Mira'}
        <ArrowRight className="size-4" />
      </button>
    </section>
  );
}
