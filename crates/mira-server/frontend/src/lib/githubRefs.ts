/** GitHub PR and issue references in transcript text. */

export type GithubRefTarget = { owner: string; repo: string; number: number; kind: 'pull' | 'issues' };

const GH_URL = /^https?:\/\/github\.com\/([\w.-]+)\/([\w.-]+)\/(pull|issues)\/(\d+)(?:\/(?:files|commits|checks))?\/?(?:[?#].*)?$/;

/** The PR or issue a GitHub URL points at, if it's one. */
export function parseGithubRef(href: string | undefined): GithubRefTarget | null {
  const m = href ? GH_URL.exec(href) : null;
  return m ? { owner: m[1], repo: m[2], kind: m[3] as 'pull' | 'issues', number: Number(m[4]) } : null;
}

/** Turn bare `owner/repo#123` into links, outside code. GitHub resolves a
 *  `/pull/` link to the issue when the number is one. */
export function linkifyGithubRefs(text: string): string {
  if (!text.includes('#')) return text;
  // Split on fenced blocks and inline code; only even parts are prose.
  return text
    .split(/(```[\s\S]*?```|`[^`\n]*`)/g)
    .map((part, i) =>
      i % 2 === 1
        ? part
        : part.replace(
            /(^|[\s(])([A-Za-z0-9][\w-]*\/[\w.-]+)#(\d+)\b(?!\])/g,
            (_m, pre: string, repo: string, n: string) =>
              `${pre}[${repo}#${n}](https://github.com/${repo}/pull/${n})`,
          ),
    )
    .join('');
}
