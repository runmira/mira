# Measuring Mira

Two ways to put a number on how well Mira works:

- **`mira eval`**: quick checks on the small tasks in `evals/tasks/`
  (minutes, a few cents). Good for spotting a prompt or tool change that
  broke something.
- **`mira eval swebench`**: SWE-bench, the standard benchmark for coding
  agents. Each task is a real GitHub issue in a real Python project;
  the fix counts only if the project's hidden tests pass.

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
