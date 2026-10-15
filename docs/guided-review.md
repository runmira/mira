# Guided change review

Open **Review changes → Walk me through it**, or use **Walk me through it** in a pull request's code pane. Select the agent's changes, a Git range such as `main...HEAD`, or a GitHub PR URL, then start the walkthrough.

Mira reads the complete diff and proposes a review order: core behavior, dependent code, tests, then incidental edits. Each stop includes an explanation, the diff, and any AI-generated risk markers. Incidental edits are grouped in a collapsed section. Previous/next controls work without opening files manually, including on narrow screens.

Ask a question at any stop for an answer grounded in that file's diff. Answers and risk markers are suggestions to verify against the code, rather than verified findings.

Approve individual hunks to track review progress. Approvals are local to this chat, source and exact diff snapshot; they do not submit a GitHub review or stage changes. Storage failures leave the current review usable but prevent persistence.

Reverting a text hunk requires confirmation. Mira re-reads the source diff, rejects stale patches, checks reverse application, and updates the local working file. It preserves the Git index. For session changes, it refreshes the walkthrough and retains approvals for unchanged files. For committed ranges and PRs, the source diff remains unchanged and the reverted hunk is marked locally. PR reverts require a matching local repository and compatible working files. Binary, rename, mode, new-file and deleted-file changes can be reviewed here; partial revert is unavailable for these structural changes. Use the existing file-level revert in the session review drawer when appropriate.

Plans require a configured Mira model and include every changed file exactly once. Changes above 150 files or 400,000 diff bytes need a smaller range. Provider failures, incomplete plans and conflicts produce an error with a reload action instead of silently omitting files.

Development fixture: `npm run dev`, then `/walkthrough-preview.html`. This uses the real diff viewer with simulated model responses and revert results; it does not modify files or call a model.
