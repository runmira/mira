# Cloud tasks

Start a task, close your laptop, and come back to a pull request.

```sh
mira cloud run "Fix the flaky login test and add a regression test"
```

The whole Mira session (agent loop, model calls, git) runs in an E2B
cloud sandbox, so nothing depends on your machine once the command
returns, which takes seconds.

1. The sandbox clones your repo from GitHub and creates a
   `mira/<task>-<id>` branch off your current branch (or `--base`).
2. It opens a **draft PR** right away, so you can follow along from
   anywhere.
3. Mira works in `/goal` mode: it makes changes, verifies them, and an
   evaluator judges whether the task is done. If not, it keeps going,
   within the limits.
4. After every iteration it commits and pushes, and the PR description
   shows progress.
5. At the end it posts a summary comment (what changed, how it was
   verified, assumptions). The PR is marked **ready for review** if the
   task was judged done, and left as a draft with the reason otherwise.
   Then the sandbox deletes itself.

GitHub emails you when the PR is ready, like any other PR.

## Commands

| Command | What it does |
|---|---|
| `mira cloud run "<task>"` | Start a task. `--base <branch>`, `--env <name>`, `--max-minutes N`, `--max-iterations N`, `--budget-usd X`. |
| `mira cloud list` | Your tasks, with their PR and state (running, ready for review, needs a look, merged…). |
| `mira cloud logs <id>` | Recent log lines while the task runs. |
| `mira cloud stop <id>` | Stop a task and delete its sandbox. Work pushed so far stays on the branch. |

## Setup

You need:

- **An E2B API key** in `E2B_API_KEY` (or under `keys:` in
  `~/.mira/mira.yaml`).
- **A GitHub token** in `GITHUB_TOKEN` or `GH_TOKEN`, or be signed in with
  `gh auth login`. Use a
  [fine-grained token](https://github.com/settings/personal-access-tokens)
  limited to the repo, with **Contents** and **Pull requests** set to
  read and write.
- A model provider set up as usual (`mira init`). The task uses the same
  provider and model as your local sessions.

Optional settings in `~/.mira/mira.yaml` (the whole block is global-only;
a repo's config can't change it):

```yaml
cloud:
  environment: cloud        # a compute environment (below); default: the built-in `e2b`
  max_runtime_secs: 3600    # match your E2B plan's sandbox time limit
  max_iterations: 20
  budget_usd: 5             # when the model's pricing is known
  evaluator_model: claude-haiku-4-5   # cheaper judge for the goal loop
  install: auto             # auto | preinstalled | upload | release
  binary: ~/code/mira/target/x86_64-unknown-linux-gnu/release/mira   # Linux build to upload
  auto_shutdown: true       # sandbox deletes itself when done (needs the E2B key inside)

compute:
  environments:
    cloud:
      backend: e2b
      template: mira-rust   # a template with Mira + your toolchain (below)
      env: { RUST_LOG: info }
      setup: |              # runs in the repo after cloning
        cargo fetch
```

**Time limits.** The task stops itself before `max_runtime_secs`, leaving
time to commit, push and report. Pick a value within your E2B plan's
sandbox limit.

## Getting Mira into the sandbox

E2B sandboxes are Linux x86_64, so they need a Linux `mira` that includes
the cloud worker. `install: auto` (the default) picks, in order:

1. **`cloud.binary`** (or `MIRA_CLOUD_BINARY`): a Linux x86_64 build you
   point at, which is uploaded.
2. **The running `mira`**, when you launch from Linux x86_64. It's
   uploaded as is.
3. **The matching release**, installed in the sandbox with `install.sh`.
   Only works once a release that includes the cloud worker is published.

**From macOS**, build a Linux binary once and point at it:

```sh
brew install zig
scripts/build-linux-mira.sh             # → target/x86_64-unknown-linux-gnu/release/mira
export MIRA_CLOUD_BINARY=$PWD/target/x86_64-unknown-linux-gnu/release/mira
```

**Fastest option:** bake Mira and your toolchain into an E2B template
and set `install: preinstalled`:

```dockerfile
# e2b.Dockerfile — start from E2B's base image (see their template docs
# for the current name), then add Mira and what your project needs.
FROM e2bdev/base
RUN curl -fsSL https://raw.githubusercontent.com/runmira/mira/main/install.sh | bash
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y   # example: a Rust toolchain
```

Build it with E2B's CLI (`e2b template build`), then reference it with
`template:` in a compute environment.

## Security

- **What leaves your machine:** the task text, your model API key, the
  GitHub token, and (with `auto_shutdown`) your E2B key. The code itself
  is cloned from GitHub, not uploaded.
- **How secrets get in:** in a file that the worker reads and deletes
  before the agent starts. They're never put in the sandbox's
  environment, and the agent's shell has these variables stripped.
  The agent still runs in the same sandbox, though, so scope the GitHub
  token to the one repo.
- **Commits** are authored with your local `git config user.name` /
  `user.email`.
- **No approvals:** the agent runs without asking. That's the point of an
  unattended task, and why it runs in an isolated VM. `plan` is
  auto-approved; questions get the recommended option, and the choice is
  recorded in the summary.

## Status

Tested end to end against local fakes of E2B, GitHub and the model API.
The test uploads the real `mira` binary, which clones, works, pushes and
marks the PR ready. It has not yet been run against the live E2B and
GitHub services.

Not yet: a "Cloud tasks" panel in the web UI, attaching to a running
task live, and continuing a task after its time limit (checkpoint and
resume).
