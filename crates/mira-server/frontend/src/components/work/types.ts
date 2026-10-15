import type { CiState, ReviewState } from '../../lib/workItems';

/** Everything the work item page shows beyond the grouped chats. None of
 *  it comes from the server yet: the prototype fakes it. */
export type WorkDetail = {
  /** The issue's body, when the work started from one. */
  goal?: string;
  plan: { text: string; done: boolean }[];
  files: { path: string; added: number; removed: number }[];
  worktree?: string;
  base?: string;
  ahead?: number;
  behind?: number;
  checks: { name: string; state: CiState; duration?: string }[];
  reviewers: { name: string; state: ReviewState }[];
  timeline: { at: number; text: string; kind: TimelineKind }[];
};

export type TimelineKind = 'issue' | 'chat' | 'agent' | 'commit' | 'pr' | 'ci' | 'review' | 'you';

export function diffTotals(detail: WorkDetail | undefined): { added: number; removed: number; files: number } {
  const files = detail?.files ?? [];
  return {
    added: files.reduce((n, f) => n + f.added, 0),
    removed: files.reduce((n, f) => n + f.removed, 0),
    files: files.length,
  };
}
