export const ARG_PREVIEW_REDUNDANT: Record<string, true> = {
  glob: true,
  grep: true,
  read_file: true,
  rustfmt: true,
  find_symbol: true,
  find_references: true,
  find_callers: true,
  web_fetch: true,
  web_search: true,
  memory_read: true,
  memory_search: true,
  task_get: true,
  task_list: true,
  git_status: true,
  git_diff: true,
  git_log: true,
};

export type ToolStatus = 'pending' | 'running' | 'denied' | 'complete';
