/**
 * Read a test run's output: how many passed / failed, and each failure
 * with the lines that explain it. Covers cargo, Vitest, Jest, pytest and
 * Go; anything else still gets counts from the exit code and the raw log.
 *
 * Plain text in, no framework flag: the formats don't collide, so every
 * pattern runs over every line.
 */

export type TestFailure = {
  name: string;
  /** Source file, when the output names one. */
  file: string | null;
  detail: string[];
};

export type TestSummary = {
  passed: number;
  failed: number;
  skipped: number;
  failures: TestFailure[];
};

/** A line that starts a failure's explanation; group 1 is its name. */
const FAIL_HEADERS: RegExp[] = [
  /^---- (\S+) stdout ----$/, // cargo
  /^\s*● (.+)$/, // jest: "● Suite › does a thing"
  /^\s*FAIL\s+(\S+\.\w+\s+>\s+.+)$/, // vitest: "FAIL  src/a.test.ts > suite > test"
  /^_{3,} (.+?) _{3,}$/, // pytest: "____ test_name ____"
  /^\s*--- FAIL: (\S+)/, // go
];

/** Lines that end a failure's explanation without starting another. */
const DETAIL_END: RegExp[] = [
  /^failures:$/, // cargo's list after the stdout sections
  /^test result:/,
  /^=+ /, // pytest section rules
  /^ ?⎯{3,}/, // vitest separators
  /^\s*(PASS|FAIL)\s+\S+\.\w+\s*$/, // jest file lines
  /^(ok|FAIL)\s+\S+\s+[\d.]+s$/, // go package result
  /^Test Suites:/,
  /^(FAIL|PASS)$/, // go's run verdict
];

/** One-line failures (no detail section of their own). */
const FAIL_LINES: RegExp[] = [
  /^test (\S+) \.\.\. FAILED$/, // cargo
  /^FAILED (\S+?)(?: - (.*))?$/, // pytest short summary
  /^\s*[×✕] (.+?)(?: \(?\d+ ?ms\)?)?$/, // vitest / jest
];

const PASS_LINES: RegExp[] = [/^test \S+ \.\.\. ok$/, /^\s*--- PASS: /, /^\s*[✓√] /];
const LIVE_FAIL_LINES: RegExp[] = [/^test \S+ \.\.\. FAILED$/, /^\s*--- FAIL: /, /^\s*[×✕] /];

const MAX_DETAIL = 60;

export function parseTestOutput(lines: string[]): TestSummary {
  let failures: TestFailure[] = [];
  const byName = new Map<string, TestFailure>();
  /** Failures that got a detail section (vs. a one-line mention). */
  const detailed = new Set<TestFailure>();
  let current: TestFailure | null = null;

  const add = (name: string, first?: string) => {
    const key = name.trim();
    let f = byName.get(key);
    if (!f) {
      f = { name: key, file: fileOf(key), detail: [] };
      byName.set(key, f);
      failures.push(f);
    }
    if (first && !f.detail.includes(first)) f.detail.push(first);
    return f;
  };

  for (const raw of lines) {
    const line = raw.replace(/\s+$/, '');
    const header = match(FAIL_HEADERS, line);
    if (header) {
      current = add(header);
      detailed.add(current);
      continue;
    }
    if (current) {
      if (DETAIL_END.some((r) => r.test(line))) {
        current = null;
      } else {
        if (current.detail.length < MAX_DETAIL) current.detail.push(line);
        if (!current.file) current.file = fileOf(line);
        continue;
      }
    }
    for (const r of FAIL_LINES) {
      const m = r.exec(line);
      if (m) {
        add(m[1], m[2]);
        break;
      }
    }
  }

  // One test often shows up twice — a one-line `× adds` / `FAILED
  // tests/a.py::test_x` and a detailed section under a longer or shorter
  // name. Fold the one-liner into the section that ends with its name.
  const leaf = (n: string) => n.split(/\s*(?:›|>|::)\s*/).pop()!.trim();
  failures = failures.filter((f) => {
    if (detailed.has(f)) return true;
    const into = failures.find((h) => detailed.has(h) && leaf(h.name) === leaf(f.name));
    if (!into) return true;
    if (f.name.length > into.name.length) into.name = f.name;
    into.file ??= f.file;
    return false;
  });

  for (const f of failures) {
    // Trim the blank lines framing each explanation.
    while (f.detail.length && !f.detail[0].trim()) f.detail.shift();
    while (f.detail.length && !f.detail[f.detail.length - 1].trim()) f.detail.pop();
  }

  const counts = summaryCounts(lines) ?? liveCounts(lines);
  return {
    ...counts,
    failed: Math.max(counts.failed, failures.length),
    failures,
  };
}

/** Counts as the run goes, from per-test lines (summaries come last). */
export function liveCounts(lines: string[]): { passed: number; failed: number; skipped: number } {
  let passed = 0;
  let failed = 0;
  for (const l of lines) {
    if (PASS_LINES.some((r) => r.test(l))) passed++;
    else if (LIVE_FAIL_LINES.some((r) => r.test(l))) failed++;
  }
  return { passed, failed, skipped: 0 };
}

/** The runner's own totals, when it printed them. */
function summaryCounts(lines: string[]): { passed: number; failed: number; skipped: number } | null {
  let found = false;
  let passed = 0;
  let failed = 0;
  let skipped = 0;
  const num = (s: string, word: RegExp) => {
    const m = new RegExp(`(\\d+) ${word.source}`).exec(s);
    return m ? Number(m[1]) : 0;
  };
  for (const l of lines) {
    // cargo prints one per test binary: add them up.
    const cargo = /^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored/.exec(l);
    if (cargo) {
      found = true;
      passed += Number(cargo[1]);
      failed += Number(cargo[2]);
      skipped += Number(cargo[3]);
      continue;
    }
    // jest "Tests:  1 failed, 3 passed, 4 total" · vitest "Tests  1 failed | 3 passed (4)"
    const js = /^\s*Tests:?\s+(.*\d+ (?:passed|failed).*)$/.exec(l);
    // pytest "==== 1 failed, 3 passed in 0.12s ===="
    const py = /^=+ (.*\d+ (?:passed|failed).*) in [\d.]+s/.exec(l);
    const s = js?.[1] ?? py?.[1];
    if (s) {
      found = true;
      passed = num(s, /passed/);
      failed = num(s, /failed/);
      skipped = num(s, /(?:skipped|todo|pending)/);
    }
  }
  return found ? { passed, failed, skipped } : null;
}

function match(rs: RegExp[], line: string): string | null {
  for (const r of rs) {
    const m = r.exec(line);
    if (m) return m[1];
  }
  return null;
}

/** A source path in a line: `src/a.test.ts`, `tests/test_x.py:12`, `foo_test.go:8`. */
function fileOf(s: string): string | null {
  const m = /([\w./-]+\.(?:rs|tsx?|jsx?|mjs|py|go|rb|ex|exs|swift))(?::\d+)?/.exec(s);
  return m ? m[1] : null;
}
