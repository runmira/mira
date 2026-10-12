/** How an external agent's turn ended, and what to say about it.
 *
 *  These are pure string helpers with no transcript dependencies, kept out
 *  of `entries.ts` so they can be reasoned about (and tested) on their own —
 *  the distinction they encode is a product decision, not a rendering one:
 *  a turn the *user* stopped is a deliberate act, not a fault. */

/**
 * Whether a stop reason means the turn actually produced an answer.
 *
 * Deliberately a small allowlist rather than a blocklist: an unfamiliar
 * reason is far more likely to be a failure or a limit than a clean finish,
 * and saying so beats claiming success.
 */
export function isSuccessfulAcpStop(stop: string): boolean {
  return ['end_turn', 'success', 'completed', 'stop_sequence'].includes(stop);
}

/**
 * Did *the user* end this turn (the Stop button, `session/cancel`)? A
 * manual stop is a deliberate act, not a fault: there is nothing to retry
 * and nothing to fix, so it must not be dressed up as an error the user
 * has to dismiss. Kept separate from `isSuccessfulAcpStop` because a
 * stopped turn produced no answer — it needs a different kind of note, not
 * silence.
 */
export function isUserCancelledStop(stop: string): boolean {
  return stop === 'cancelled' || stop === 'canceled';
}

/** A human explanation of why a turn stopped, for the transcript. */
export function describeAcpStop(stop: string): string {
  if (stop === 'rate_limited') {
    return 'The agent hit a usage limit, so this turn failed without an answer.';
  }
  if (isUserCancelledStop(stop)) {
    return 'Turn ended manually by user.';
  }
  if (stop === 'max_tokens' || stop === 'max_turns') {
    return `The agent stopped early (${stop}).`;
  }
  return `The agent stopped: ${stop}.`;
}