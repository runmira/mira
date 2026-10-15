# Guided change review

Open **Review changes → Walk me through it**, or use **Walk me through it** in a pull request's code pane. Select the agent's changes, a Git range such as `main...HEAD`, or a GitHub PR URL, then start the walkthrough.

Mira reads the complete diff and proposes a review order: core behavior, dependent code, tests, then incidental edits. Each stop includes an explanation, the diff, and any AI-generated risk markers. Incidental edits are grouped in a collapsed section. Previous/next controls work without opening files manually, including on narrow screens.

Ask a question at any stop for an answer grounded in that file's diff. Answers and risk markers are suggestions to verify against the code, rather than verified findings.

Approve individual hunks to track review progress. Approvals are local to this chat, source and exact diff snapshot; they do not submit a GitHub review or stage changes. Storage failures leave the current review usable but prevent persistence.

Reverting a text hunk requires confirmation. Mira re-reads the source diff, rejects stale patches, checks reverse application, and updates the local working file. It preserves the Git index. For session changes, it refreshes the walkthrough and retains approvals for unchanged files. For committed ranges and PRs, the source diff remains unchanged and the reverted hunk is marked locally. PR reverts require a matching local repository and compatible working files. Binary, rename, mode, new-file and deleted-file changes can be reviewed here; partial revert is unavailable for these structural changes. Use the existing file-level revert in the session review drawer when appropriate.

Plans require a configured Mira model and include every changed file exactly once. Changes above 150 files or 400,000 diff bytes need a smaller range. Provider failures, incomplete plans and conflicts produce an error with a reload action instead of silently omitting files.

Development fixture: `npm run dev`, then `/walkthrough-preview.html`. This uses the real diff viewer with simulated model responses and revert results; it does not modify files or call a model.

## Turn and pull-request views

A turn card and the context panel's **Latest turn edits** section show changed files directly. File rows open the current file in the right-side panel; **View changes** opens that turn's frozen before/after snapshots in the review drawer, including repeated prompts. Snapshot review does not expose working-tree revert actions. Binary or expired snapshots are listed explicitly rather than hiding the rest of a turn.

Thinking disclosures render Markdown while streaming and after completion, keeping the existing scroll and follow behavior. Both PR surfaces use the shared Pierre diff renderer and the same wrapping action bar. The context panel discovers the current branch's PR through the main panel's GitHub authentication path, refreshes on focus and every 30 seconds, and does not require an agent action report. Remote control remains a compact switch, with connection/link chrome removed.

Development fixtures: `/review-ui-preview.html` and `/walkthrough-preview.html`.


### Diff tab and resizable panels

View changes opens the singleton **Diff** tab in the right panel. Turn entry points retain the selected immutable snapshot; the source selector labels the latest edited turn from transcript order (including repeated prompts). Choose Changes to return to uncommitted session changes. File clicks continue to open separate file tabs. Switching tabs keeps the review mounted, preserving draft comments and position; closing the Diff tab disposes it.

The panel toolbar offers unified/split layout, wrapping and line numbers using the existing diff preferences. Below 600px the renderer uses unified layout without overwriting the saved split preference; split becomes available again when widened. The file tree appears at a panel width of 760px. Header actions wrap independently of the application viewport. Expand/restore is a panel window action, with space retained for native macOS chrome.
