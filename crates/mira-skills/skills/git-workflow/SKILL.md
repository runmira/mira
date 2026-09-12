---
name: git-workflow
description: Follow this project's branch → change → commit → push workflow safely. Use when the user asks you to make a change and land it (create branch, commit, push), or when they say "put this on a branch" or "let's ship this."
category: git
icon: git-branch
color: indigo
---

Take a change from working tree to a pushed branch without any of the failure modes that eat time later: unrelated files sneaking in, force-pushes to shared branches, `.env` in a public PR, hooks skipped.

**1. Read the repo before you touch it.**
Run these in parallel:
- `git status` (see what's already dirty)
- `git branch --show-current` (see what branch you're on)
- `git log -1 --format='%h %s'` (see if HEAD is a WIP commit you might amend)
- `git remote -v` (know which remote you'll push to)

If `main` / `master` / `trunk` is the current branch and you have uncommitted work, **stop and create a feature branch first** before staging anything. Never commit directly to the default branch unless the user explicitly asked.

**2. Choose the branch name.**
Follow the repo's convention (check recent branches with `git branch -a --sort=-committerdate | head -20`). Common shapes:
- `<type>/<slug>` — `feat/user-avatar`, `fix/login-race`, `chore/bump-node-20`.
- `<initials>/<slug>` — `dm/user-avatar`.
- If the repo has no clear convention, use `<type>/<short-slug>` and mention it to the user.

Create with `git switch -c <name>`. If already on a feature branch (not main/master), use it.

**3. Stage exactly what you want.**
Prefer `git add <path>` per file over `git add -A` / `.`. Bulk staging swallows:
- Editor swap files (`*.swp`, `.DS_Store`)
- Local secrets (`.env`, `credentials.json`, `*.pem`)
- Debug artifacts you left lying around (`tmp/`, `*.log`)

Show `git status` after staging so the user sees what will be committed.

**4. Commit.**
Use the `commit` skill for the message — same style as the repo's history. Do NOT append a `Co-Authored-By: Claude/Mira` trailer unless the user explicitly requested it.

**5. Push.**
- **First push of a branch:** `git push -u origin <branch>`. The `-u` sets the upstream so subsequent pushes are just `git push`.
- **Follow-up push:** plain `git push`.
- **NEVER `--force`** without explicit user confirmation. If the remote rejected a non-fast-forward push, stop and explain what happened; don't try to force through.
- **NEVER `--no-verify`.** Hooks exist for a reason; if a hook fails, fix the underlying issue.

**6. Report.**
Print the branch name, commit SHA, and (if the remote is a known git host) construct the compare/PR URL from `git remote get-url origin` so the user has a one-click path to open a PR.

**Do not:**
- Push to `main` / `master` / `trunk` / `develop` directly.
- Commit with `-a` — it grabs modified files you haven't inspected.
- Use `git rebase` on a branch that's already been pushed unless the user explicitly asks. Rewriting shared history breaks anyone else's local clone.
- Silently drop files that `git status` shows. If you're skipping something (a `.env`, a debug artifact), say so explicitly.
- Configure git identity (`git config user.name/email`). If commits are failing on identity, tell the user; don't guess.
