# Chat change tracking

Each prompt saves a before snapshot, and turn completion saves an after
snapshot. The transcript's turn card reads those frozen trees, so committing
the changes before replying does not erase the turn's diff. When there are no
uncommitted files, the context panel offers **Latest turn edits** to review
the most recent saved edit. Repeated prompts are matched by occurrence.

Chats opened inside a Git repository keep their existing repository scope.
For a chat opened one folder above repositories, Mira discovers direct child
repositories only (at most 128 directory entries, without following directory
symlinks). It takes before snapshots within the existing shared five-second
budget. Discovery alone does not include a repository in the chat's changes.

A child repository is included after this chat reports an edited file there,
records a write through Mira's file guard, or executes an agent command with
that repository as its working directory. The first such activity uses that
turn's before snapshot as the repository's baseline, excluding older sibling
activity. Paths remain relative to the chat folder, such as `mira/src/main.rs`.
The review and per-file discard endpoints route that path to its repository.
Commit actions remain available only when the chat folder is inside one repo;
a parent workspace does not silently choose a repository to commit.

This is filesystem snapshot attribution, not isolation between processes.
Concurrent writers in the same claimed repository can affect its counts.
Commands with no reported repository working directory or edited paths cannot
claim a child repository. Ignored files, deeper repositories, skipped snapshots,
and turns from before snapshot metadata existed are not reconstructed. Parent
workspace support covers change review; whole-turn restore still requires a
chat scoped to a repository.

# Remote control

`/remote control` and `/remote control on` enable access to this Mira from
another device. `/remote control off` disables it. These commands run locally,
including during an external-agent chat, and are not sent to the agent.

The context panel always offers the same machine-wide remote-access control
used by Devices settings. It reports startup, connection, and error states.
When enabled, its ellipsis menu offers **Copy link** and **Open link**. Enabling
access still requires a supported computer and a signed-in Mira account;
device pairing remains required. Opening the panel does not enable access.
