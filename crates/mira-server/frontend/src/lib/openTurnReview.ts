import { reviewTurnSources } from './reviewSources';
import { selectActiveSession } from '../state/runtimeStore';
import type { AppRuntime } from '../state/createAppRuntime';
import type { TurnDiffSummary } from '../types';

/** Keep historical review ownership separate from current working-tree review. */
export function openTurnReview(runtime: AppRuntime, summary: TurnDiffSummary) {
  const session = runtime.sessionState.get().active ?? runtime.sessionState.get().selected;
  if (!session) return;
  const data = selectActiveSession(runtime.sessionState.get());
  const source = reviewTurnSources(data.entries, data.turnDiffs.length ? data.turnDiffs : [summary]).find(item => item.summary.text === summary.text && item.summary.occurrence === summary.occurrence);
  runtime.updateWorkspaceUi('reviewTurn', { session, summary, turnKey: source?.key });
  runtime.updateWorkspaceUi('reviewFocus', null);
  runtime.updateWorkspaceUi('reviewOpen', true);
}
