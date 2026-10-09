# Measuring Mira

Two ways to put a number on how well Mira works:

- **`mira eval`**: quick checks on the small tasks in `evals/tasks/`
  (minutes, a few cents). Good for spotting a prompt or tool change that
  broke something.
- **`mira eval swebench`**: SWE-bench, the standard benchmark for coding
  agents. Each task is a real GitHub issue in a real Python project;
  the fix counts only if the project's hidden tests pass.

## Quick checks

```bash
mira --provider anthropic --model claude-haiku-4-5 eval     # every task
mira eval --task python                                     # names containing "python"
```

With an agent's own CLI instead of Mira's harness, on its subscription
(no API key):

```bash
mira eval --agent claude --agent-model haiku                # Claude Code, `claude -p`
```

Claude Code runs with only the task folder's settings, so your own hooks,
plugins and MCP servers stay out of the results.

Each task in `evals/tasks/` is a YAML file: a `prompt`, an optional
`fixture` folder copied into a fresh temp directory first, and how it's
graded:

- `verify`: a shell command run in that directory afterwards; exit 0 is a
  pass. `EVAL_FIXTURE` points at the untouched fixture, so a check can
  also make sure the agent didn't change what it shouldn't
  (`git diff --no-index --quiet "$EVAL_FIXTURE/legacy.py" legacy.py`).
- `expect_grep`: a regex the final answer must match.

The summary shows tokens, the share of input served from the provider's
prompt cache, and cost (for priced models). Cache share is the number to
watch when changing how Mira builds its prompts: a drop means something
started invalidating the cache.

| Task | Checks |
| --- | --- |
| `01-explain` | A plain answer, no tools |
| `02-write-hello` | Creating a file |
| `03-rust-fix` | Fixing a compile error |
| `04-failing-test` | Fixing code to pass a test |
| `05-rename-symbol` | Renaming a method and every call |
| `06-multi-file` | A rename across several files |
| `07-python-bug` | Fixing two bugs without touching the tests |
| `08-implement-stubs` | Reading and implementing four functions (many read-then-edit rounds) |
| `09-scoped-change` | Changing one function and leaving other code alone |
| `10-config-edit` | Editing JSON precisely |
| `11-find-code` | Finding an answer in code, read-only, in a fixed format |

## SWE-bench

### What you need

- Python 3.9+ with the official harness: `pip install swebench`. It
  also downloads the dataset.
- Docker, running, for scoring. The first scoring run builds images for
  each project, which takes a while and needs disk space: plan on tens
  of GB for a full split.
- A model set up in Mira as usual (`default_model` / `--model`). Every
  task is a full agent run, so it costs real money: a 50-task sample
  on a mid-size model is typically a few dollars to a few tens of
  dollars, depending on the model.

### 1. Run Mira on the tasks

```bash
mira eval swebench --dataset lite --limit 50 --workers 4
```

- `--dataset`: `lite` (300 tasks), `verified` (500, checked by people;
  the one most results quote), `full`, or your own `.jsonl` with
  `instance_id`, `repo`, `base_commit`, `problem_statement`.
- `--limit N` takes the first N; `--instance ID` (repeatable) picks
  specific ones.
- `--workers N` runs N tasks at once. Mind your provider's rate limits.
- `--timeout-secs` (default 1800) and `--max-turns` cap each task.
- `--keep` leaves each task's checkout in `~/.mira/evals/work/` so you
  can look at what Mira did.

For each task, Mira checks out the project at the issue's commit, gets
the issue text, works on it unattended (every tool allowed, inside the
usual command sandbox), and its changes become the task's patch.

It writes two files:

- `predictions.jsonl`: one patch per task, in the harness's format.
- `predictions.log.jsonl`: per task, whether it produced a patch,
  tokens, cost and time.

Stopped halfway? Run the same command again: tasks already in the
predictions file are skipped, and tasks that errored are retried.

Repos are cloned once into `~/.mira/evals/repos/`. The dataset is
downloaded once into `~/.mira/evals/swebench/`.

### 2. Score it

```bash
mira eval swebench score --predictions predictions.jsonl --dataset lite
```

This runs the official harness in Docker and ends with a line like:

```
Resolved 21 of 50 submitted (42.0%) · 300 in the dataset
```

The full report (which tasks passed) is the `.json` file it names.

### Comparing fairly

- **Same tasks.** Compare on the same instance ids. `--limit 50` always
  takes the same first 50.
- **Same model.** The point is Mira's harness, so compare Mira with
  model X against another agent with model X.
- **More than one run.** Models are random; results move by a few
  points between runs. Run twice before believing a small difference.
- **Other agents** can be scored the same way: produce a predictions
  file in the same format (`instance_id`, `model_name_or_path`,
  `model_patch`) and run the `score` command on it.

### Limits of this first version

- Mira works on your machine, where the project's dependencies usually
  aren't installed, so it often can't run the project's tests while
  fixing. It's told this and reads the code instead. The leaderboard's
  top agents run inside each task's Docker image; doing that is the next
  step, and will likely raise the score.
- Hints (`hints_text`) aren't given to Mira, which matches how most
  results are reported.

`MIRA_SWEBENCH_REPO_BASE` points repo cloning at a mirror instead of
`https://github.com`.
