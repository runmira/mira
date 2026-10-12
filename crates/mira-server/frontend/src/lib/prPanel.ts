/**
 * Bridge for opening a GitHub pull request in the right-side panel.
 *
 * Deliberately a `window` event, like the composer bridges in
 * `attachBridge`: a PR chip can appear anywhere Markdown renders
 * (transcript, subagent results, review comments), and threading a
 * callback through every `Markdown` call site would be prop-drilling
 * for a single caller. The right panel listens and focuses (or opens)
 * a `pr` tool tab.
 */

export type PrTarget = { owner: string; repo: string; number: number };

export const OPEN_PR_EVENT = 'mira:open-pr';

/** Show `target` in the right panel's PR window. */
export function openPrPanel(target: PrTarget): void {
  window.dispatchEvent(new CustomEvent<PrTarget>(OPEN_PR_EVENT, { detail: target }));
}
