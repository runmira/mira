import { post } from './request';

/** Claude Code's native file rewind, as the server reports it. */
export type AgentFileRewind = {
  dry_run: boolean;
  can_rewind: boolean;
  files_changed: string[];
  insertions: number | null;
  deletions: number | null;
  skipped: number | null;
  /** What Claude's checkpoints cover; shown before every rewind. */
  coverage: string;
  /** Snapshot to put the files back as they were before the rewind. */
  undo: string | null;
  error: string | null;
};
export function agentFileRewind(
  sessionId: string,
  body: { text?: string; occurrence?: number; dry_run?: boolean; undo?: string },
): Promise<AgentFileRewind> {
  return post(`/api/sessions/${encodeURIComponent(sessionId)}/agent-file-rewind`, body, 'Rewind Claude file edits');
}
