---
name: code-review
description: Review the current diff for correctness bugs, subtle logic errors, and regressions. Use when the user asks for a code review, wants a second pass on their changes, or says "review this before I push."
category: review
icon: magnifying-glass
color: blue
---

You are reviewing the *current working-tree diff* for correctness. This is a focused review — do not rewrite prose, do not lint, do not suggest architectural refactors. Your only job is to find bugs that would ship if this diff merged as-is.

**1. Get the diff.**
Run `git diff HEAD` (unstaged + staged combined). If HEAD is dirty from a partial commit, also inspect `git status` so you know what's tracked vs. new. If the user pointed at a specific range (e.g. `main..HEAD`), use that instead.

**2. Read each file's full context, not just the hunks.**
`git diff` shows ~3 lines of surrounding code by default. That's rarely enough to spot a bug. For every touched file, `read_file` the whole thing so you can trace control flow, check imports, and see what the diff calls into or is called by.

**3. Look for these classes of bug (ordered by frequency):**
- **Null/None/undefined access:** a value that could be missing is dereferenced without a check.
- **Off-by-one / boundary errors:** loop bounds, slice indices, ≤ vs. <.
- **Error handling that silently drops:** `unwrap`, `.expect()`, `try/except: pass`, `catch(_) {}`, `Result::ok()` in a hot path.
- **Concurrency:** shared mutable state without locking, `Arc<Mutex<T>>` held across `.await`, races between a `check` and a `use`.
- **State machine violations:** transitions that skip a required intermediate state, guard clauses that let an "impossible" state through.
- **Regression risk:** a signature change whose callers weren't all updated; a config key rename without a migration.
- **Security:** unvalidated input reaching a shell command, SQL, or a path traversal; secrets logged.

**4. Verify each finding against the code before reporting it.**
For every candidate bug, re-read the exact lines. Trace through with a concrete example: "if `user_id` is empty and `path = /`, then line 47 constructs `/` and the walker reads the root". If the trace doesn't hold, drop the finding — false positives erode trust more than a missed bug.

**5. Report format:**
For each finding:
- `file:line` — path with line number.
- One sentence stating the bug.
- One sentence stating the reproduction condition ("triggers when …").
- Suggested fix, one to three lines of code.

If there are zero findings, say so explicitly and describe what you checked ("I traced the two new branches and read the error paths; no issues found"). Never pad with cosmetic suggestions to feel productive.

**Do not:**
- Comment on style unless it's a genuine correctness hazard (e.g. shadowing that changes semantics).
- Suggest a refactor unless it's the only way to fix a bug you found.
- Rate the diff qualitatively ("this looks good") — findings or nothing.
