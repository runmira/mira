import { useState } from 'react';
import { PullRequestDetail } from '../pullRequests/PullRequestDetail';
import type { ToolPaneTab } from './toolPanes';

/**
 * A pull request opened from a chat link, living as a tab in the
 * right-side panel. Reuses the main PR view's detail (summary / code,
 * checks, activity, review + merge actions) in a narrow column — one
 * tab per PR, so several can sit side by side.
 */
export function PrPane({
  tab,
  onReviewPr,
}: {
  tab: ToolPaneTab;
  onReviewPr: (owner: string, repo: string, number: number) => void;
}) {
  const [pane, setPane] = useState<'summary' | 'code'>('summary');
  const pr = tab.pr;
  if (!pr) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-center text-[13px] text-muted-foreground">
        No pull request selected.
      </div>
    );
  }
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PullRequestDetail
        key={`${pr.owner}/${pr.repo}#${pr.number}`}
        selection={{
          owner: pr.owner,
          repo: pr.repo,
          number: pr.number,
          project_label: `${pr.owner}/${pr.repo}`,
        }}
        pane={pane}
        onPane={setPane}
        expanded
        onToggleExpand={() => {}}
        hideExpand
        onMutation={() => {}}
        onReviewPr={onReviewPr}
      />
    </div>
  );
}
