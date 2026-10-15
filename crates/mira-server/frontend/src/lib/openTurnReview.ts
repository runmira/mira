import type { AppRuntime } from '../state/createAppRuntime';
import type { TurnDiffSummary } from '../types';

/** Keep historical review ownership separate from current working-tree review. */
export function openTurnReview(runtime: AppRuntime, summary: TurnDiffSummary) {
  const session = runtime.sessionState.get().active ?? runtime.sessionState.get().selected;
  if (!session) return;
  runtime.updateWorkspaceUi('reviewTurn', { session, summary });
  runtime.updateWorkspaceUi('reviewFocus', null);
  runtime.updateWorkspaceUi('reviewOpen', true);
}
