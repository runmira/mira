export type WalkthroughSource = { kind: 'session' } | { kind: 'branch'; range: string } |
  { kind: 'pr'; owner: string; repo: string; number: number };
export type WalkthroughStop = {
  file_index: number;
  category: 'core' | 'dependent' | 'tests' | 'incidental';
  summary: string;
  risks: string[];
};
export type WalkthroughPlan = { files: string[]; stops: WalkthroughStop[] };

/** Preserve headers and exact hunk text, including no-newline markers. */
export function splitWalkthroughHunks(patch: string): string[] {
  const first = patch.search(/^@@ /m);
  if (first < 0) return [];
  const header = patch.slice(0, first);
  return patch.slice(first).split(/(?=^@@ )/m).filter(Boolean).map(hunk => header + hunk);
}
export function canRevertWalkthroughHunk(patch: string): boolean {
  return !/^(?:rename |old mode |new mode |new file mode |deleted file mode |GIT binary patch)/m.test(patch) && splitWalkthroughHunks(patch).length > 0;
}
export function parseWalkthroughPr(input: string): Extract<WalkthroughSource, { kind: 'pr' }> {
  const match = input.trim().match(/^(?:https:\/\/github\.com\/)?([\w.-]+)\/([\w.-]+)\/(?:pull\/)?([1-9]\d*)\/?$/);
  if (!match) throw new Error('Enter a GitHub PR URL or owner/repo/number.');
  return { kind: 'pr', owner: match[1], repo: match[2], number: Number(match[3]) };
}
/** Exact patches are retained alongside approvals to guard against hash collisions. */
export function reviewStorageKey(owner: string, source: WalkthroughSource): string {
  return `mira:walkthrough:${owner}:${JSON.stringify(source)}`;
}
export function restoreApprovals(raw: string | null, files: string[]): string[] {
  try {
    const saved = JSON.parse(raw ?? 'null');
    if (!saved || JSON.stringify(saved.files) !== JSON.stringify(files) || !Array.isArray(saved.approved)) return [];
    return saved.approved.filter((key: unknown): key is string => {
      if (typeof key !== 'string' || !/^\d+:\d+$/.test(key)) return false;
      const [file, hunk] = key.split(':').map(Number);
      return file < files.length && hunk < Math.max(1, splitWalkthroughHunks(files[file]).length);
    });
  } catch { return []; }
}
