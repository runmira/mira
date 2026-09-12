---
name: commit
description: Compose a git commit for the current working-tree changes. Use when the user asks you to commit, wants help writing a commit message, or says "commit this."
category: git
icon: git-commit
color: amber
---

Write and create ONE git commit for the current uncommitted changes.

**1. Confirm the user wants you to commit.**
Do not commit without explicit approval unless the user's message clearly asked you to (e.g. "commit this", "make a commit", "create a commit for these changes"). If they only asked for a commit *message*, stop after drafting the message and let them run `git commit` themselves.

**2. Survey the working tree.**
Run these in parallel:
- `git status` (no `-uall` flag on large repos — it can be slow)
- `git diff` (staged + unstaged, so nothing is missed)
- `git log -20 --oneline` (learn the repo's commit-message style before you write one)

**3. Decide what belongs in this commit.**
- If everything is one logical change, commit all of it.
- If the working tree mixes concerns (e.g. a bug fix + an unrelated refactor), ask the user how they want to split it before staging anything.
- Never commit files that look like secrets or local state (`.env`, `credentials.json`, `.DS_Store`, editor swap files). Ask before staging if you see one.

**4. Draft the message.**
Match the repo's existing style — that's what `git log` was for.
- **Subject line:** imperative mood, under 72 chars, no trailing period. Start with a scope prefix if the repo uses them (`feat:`, `fix:`, `docs:`, or a subsystem name like `harness:`).
- **Body (optional):** explain *why*, not *what*. The diff shows what changed. A body is worth writing when the change has a non-obvious motivation, works around a subtle issue, or affects behavior beyond the touched files.
- **Do NOT add a `Co-Authored-By:` trailer** for the agent unless the user explicitly asks for it — most users find it noise.

**5. Stage explicitly.**
Prefer `git add <path>` per file over `git add -A` / `git add .` — those two grab everything, including files you might not have inspected. Only use them if the user asked for "everything."

**6. Create the commit.**
Use a heredoc so multi-line messages format correctly:

```bash
git commit -m "$(cat <<'EOF'
Subject line here

Optional body explaining why.
EOF
)"
```

**7. Verify.**
Run `git status` after committing to confirm success and show the user the clean tree.

**Do not:**
- Amend an existing commit unless the user explicitly asked for `--amend`.
- Push. Never push without a separate explicit request.
- Skip hooks (`--no-verify`) unless the user asked for it. If a hook fails, fix the underlying issue.
