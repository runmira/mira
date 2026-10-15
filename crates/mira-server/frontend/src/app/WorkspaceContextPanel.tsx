import { openTurnReview } from '../lib/openTurnReview';
import { AnimatePresence } from 'framer-motion';
import { memo } from 'react';
import { gitCommit,gitPush } from '../api';
import { ContextPanel } from '../components/ContextPanel';
import { useAppRuntime } from '../state/RuntimeProvider';
import { titleFromEntries } from '../transcript/entries';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
type Props = Pick<WorkspaceRuntime, 'turnDiffs' | 'stableOpenFile' | 'sessionTitle' | 'outlineEntries' | 'tasks' | 'backgroundProcesses' | 'sessionId' | 'setSelectedProcess' | 'openToolPane' | 'subagentState' | 'gitStatus' | 'sessionDiff' | 'branchPrError' | 'branchPr' | 'sessionCommitted' | 'ctxOpen' | 'onCtxOpenChange' | 'openAgentTab' | 'setReviewFocus' | 'setReviewOpen' | 'refreshRepo'>;
export const WorkspaceContextPanel = memo(function WorkspaceContextPanel({ context }: { context: Props }) {
  const appRuntime = useAppRuntime();
  const { sessionTitle, outlineEntries, tasks, backgroundProcesses, sessionId, setSelectedProcess, openToolPane, subagentState, gitStatus, sessionDiff, branchPr, sessionCommitted, ctxOpen, onCtxOpenChange, openAgentTab, setReviewFocus, setReviewOpen, refreshRepo } = context;
  return (
                <AnimatePresence>
                  <ContextPanel
                    key="ctx"
                    sessionId={sessionId ?? undefined}
                    turnDiffs={context.turnDiffs}
                    onReviewTurn={summary => openTurnReview(appRuntime, summary)}
                    onOpenFile={path => context.stableOpenFile(path, null)}
                    sessionTitle={sessionTitle ?? titleFromEntries(outlineEntries)}
                    tasks={tasks}
                    processes={backgroundProcesses.processes}
                    stoppingProcesses={
                      new Set(
                        [...backgroundProcesses.stopping]
                          .filter((key) => key.startsWith(`${sessionId}:`))
                          .map((key) => Number(key.split(':').at(-1))),
                      )
                    }
                    onOpenProcess={(id) => {
                      setSelectedProcess({ session: sessionId, id, nonce: Date.now() });
                      openToolPane('processes');
                    }}
                    onStopProcess={backgroundProcesses.stop}
                    subagentState={subagentState}
                    entries={outlineEntries}
                    gitStatus={gitStatus}
                    sessionDiff={sessionDiff}
                    branchPr={branchPr}
                    branchPrError={context.branchPrError}
                    sessionCommitted={sessionCommitted}
                    open={ctxOpen}
                    onOpenChange={onCtxOpenChange}
                    onOpenAgent={openAgentTab}
                    onReview={(path) => {
                      appRuntime.updateWorkspaceUi('reviewTurn', null);
                      setReviewFocus(path ?? null);
                      setReviewOpen(true);
                    }}
                    onPush={async () => {
                      await gitPush(sessionId ?? undefined);
                      refreshRepo();
                    }}
                    onCommit={async (message, includeUnstaged, pushAfter) => {
                      const owner = appRuntime.sessionState.get().active;
                      await gitCommit({
                        message,
                        include_unstaged: includeUnstaged,
                        push_after: pushAfter,
                      }, owner ?? undefined);
                      if (owner) appRuntime.updateSessionDataAt(owner, 'sessionCommitted', () => true);
                      refreshRepo();
                    }}
                  />
                </AnimatePresence>
  );
});
