import React from 'react';

/**
 * Catches a render crash in one transcript turn and shows an inline error
 * card instead of unmounting the whole chat (a blank window).
 *
 * Transcript bodies render untrusted model output (markdown, tool results),
 * so any single message can carry input no renderer anticipated. Without
 * this, one throwing message whitescreens every chat that contains it.
 *
 * The parent keys each instance by session + turn index so opening another
 * chat resets a tripped boundary instead of replaying its stale error.
 */
export class EntryBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error) {
    console.error('[transcript] turn render failed', error);
  }

  render() {
    const { error } = this.state;
    if (error) {
      return (
        <div className="flex justify-start" role="alert">
          <div className="max-w-[90%] rounded-xl border border-destructive/30 bg-destructive/[0.04] px-3 py-2 text-[12.5px] text-muted-foreground">
            <span className="font-medium text-destructive/80">Couldn't show this turn.</span>{' '}
            <span className="text-muted-foreground/70">{error.message}</span>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}
