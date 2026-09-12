---
name: verify
description: Verify that a code change actually does what it's supposed to by running the app and observing behavior. Use when the user asks you to verify a PR, confirm a fix works, test a change manually, or validate local changes before pushing.
category: qa
icon: shield-check
color: emerald
---

When invoked, run the app and confirm the change *behaves* correctly — code that compiles and type-checks is not the same as code that works.

**1. Identify what to verify.**
Re-read the diff or the change description. Write down the one-sentence claim the change is making (e.g. "the login button submits the form"). If you can't state it in one sentence, ask the user for the acceptance criterion before going further.

**2. Detect how to launch this project.**
Look for the entry point in this order — first match wins:
- `Cargo.toml` → `cargo run`
- `package.json` with a `dev` / `start` script → `npm run <script>` (use `pnpm`, `yarn`, or `bun` if a lockfile signals it)
- `pyproject.toml` / `setup.py` → look for `python -m <package>` or a documented entry
- `Makefile` with a `run` / `dev` target → `make <target>`
- `docker-compose.yml` → `docker compose up`
- Otherwise, ask the user how to run it.

**3. Boot the app in the background.**
Use `bash` with `run_in_background: true` so you can keep working while the process runs. Capture stdout/stderr and watch for the app's ready-signal (a "listening on…" line, an HTTP 200 on the health path, an exit code 0 for CLIs).

**4. Exercise the change.**
- **HTTP servers:** hit the affected endpoint with `curl` and check status + body. Include any headers or auth the change relies on.
- **CLI:** invoke it with the exact arguments the diff touched. Capture the exit code and the last 30 lines of output.
- **UI:** if there's a way to drive it headlessly (Playwright, a `--test` mode), use that. Otherwise say so explicitly — don't fake a "looks good" verdict.
- **Library/pure code:** run the relevant test file. `cargo test --test <name>`, `pytest path/`, `npm test -- <pattern>`.

**5. Report what you saw, not what you expected.**
Quote the exact output. If the observed behavior matches the claim from step 1, say so and stop. If it doesn't match — or you couldn't drive the code path (e.g. no visual capability for a UI change) — say that explicitly and describe what you *were* able to verify.

**Do not:**
- Report success based on "the build passed" alone. Compilation is necessary, not sufficient.
- Skip step 5 and just say "looks good." Every verification ends with an observation.
- Run destructive commands (migrations, deletions, external API calls with real credentials) without confirming first.

When you're done, always end with a one-sentence summary: what you verified, how you verified it, and what (if anything) you couldn't cover.
