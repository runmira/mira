import React, { Suspense } from 'react';
import { retryFailedLazyLoads } from '../lib/lazy';

/**
 * Suspense for a lazily loaded view, plus an error boundary so a failed
 * chunk download (flaky network, a deploy that replaced the old chunks)
 * shows a retryable error in place of that view instead of unmounting the
 * whole app. Suspense alone only covers the pending state.
 */
export class LazyBoundary extends React.Component<
  {
    children: React.ReactNode;
    /** Shown while the chunk loads. */
    fallback?: React.ReactNode;
    /** Shown instead of the default error card when the load fails. */
    errorFallback?: React.ReactNode;
  },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error) {
    console.error('[lazy] view failed to load', error);
  }

  retry = () => {
    retryFailedLazyLoads();
    this.setState({ error: null });
  };

  render() {
    const { error } = this.state;
    if (error) {
      if (this.props.errorFallback !== undefined) return this.props.errorFallback;
      return (
        <div className="m-4 flex items-center gap-3 rounded-xl border border-destructive/30 bg-destructive/[0.04] px-3 py-2 text-[12.5px] text-muted-foreground" role="alert">
          <span>
            <span className="font-medium text-destructive/80">Couldn't load this.</span>{' '}
            <span className="text-muted-foreground/70">Check the connection to the Mira server.</span>
          </span>
          <button
            type="button"
            onClick={this.retry}
            className="ml-auto rounded-md border border-border px-2 py-1 text-[12px] text-foreground hover:bg-secondary"
          >
            Retry
          </button>
        </div>
      );
    }
    return <Suspense fallback={this.props.fallback ?? null}>{this.props.children}</Suspense>;
  }
}
