import type { FileChangeView } from '../api';

/** GitHub's file API omits file headers. Preserve paths and line coordinates
 * when adapting its hunks for the same Pierre renderer used by review. */
export function pullRequestPatch(file: FileChangeView): string | null {
  if (!file.patch) return null;
  const oldPath = JSON.stringify(`a/${file.previous_filename ?? file.filename}`);
  const newPath = JSON.stringify(`b/${file.filename}`);
  return `diff --git ${oldPath} ${newPath}\n--- ${file.status === 'added' ? '/dev/null' : oldPath}\n+++ ${file.status === 'removed' ? '/dev/null' : newPath}\n${file.patch.replace(/\n$/, '')}\n`;
}
