---
name: init
description: Initialize this project by writing a starter MIRA.md that captures how the code is structured, how to build/test/run it, and any project-specific conventions worth remembering across sessions.
category: onboarding
icon: folder-open
color: violet
---

Create a `.mira/MIRA.md` at the project root that a future session (or a new contributor) can read in 60 seconds and understand:
1. What this project is.
2. How it's laid out.
3. How to build, test, and run it.
4. The one or two conventions that would surprise someone who assumed defaults.

**1. Refuse to overwrite blindly.**
If `.mira/MIRA.md` already exists and is non-empty, stop and ask the user whether to append, replace, or open the existing file for editing. Never silently clobber.

**2. Read the surface.**
Walk the repo's top level. Read every one of these that exists:
- `README.md`, `CONTRIBUTING.md`, `AGENTS.md`
- Build manifests: `Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, `pom.xml`, `Makefile`
- The top-level source directory's index/entry: `src/main.rs`, `src/index.ts`, `main.py`, etc.
- `.gitignore` (tells you what NOT to expect in the tree)

Also `glob` for `**/README.md` at depth 2 — subdirectory READMEs are usually where conventions live.

**3. Draft the file with this outline:**

```markdown
# <Project Name>

<One-sentence description of what this project is. Copy from README if it says
it well; write your own if the README opens with marketing prose.>

## Layout

<A short bullet list of the top-level directories, each with a one-line
description. If it's a monorepo/workspace, list the crates/packages.>

## Build / Test / Run

- **Build:** <the exact command>
- **Test:** <the exact command>
- **Run:** <the exact command>

<If any of those need environment variables, list them here as `NAME=... —
what it's for`. Do NOT invent values; if you can't tell what to set, say
"see .env.example" or ask the user.>

## Conventions

<One to five bullets. Only include things that would trip up a reasonable
default assumption. Skip generic advice ("write tests"). Good examples:
- "We use `pnpm`, not `npm` — the lockfile is only kept in sync for pnpm."
- "The `admin/` package is tree-shaken out of the public build; imports
  from there in `web/` are rejected by the bundler config."
- "Test files live next to the source (`foo.ts` + `foo.test.ts`), not in
  a separate `tests/` tree.">
```

**4. Verify each Build / Test / Run line before you write it.**
For each command, either (a) find it explicitly named in a script section of the manifest (`package.json` `scripts`, `Makefile` target, Cargo.toml `[[bin]]`), or (b) run it once yourself and confirm it doesn't error out immediately. Do not invent commands that "should work."

**5. Show the file to the user before writing.**
Print the proposed content and ask for confirmation. On approval, `write_file` to `.mira/MIRA.md`, creating the `.mira` directory first if needed.

**Do not:**
- Include anything the code itself already documents (like "the `Session` struct owns conversation state" — a reader can `grep`).
- Copy the entire README in. Distill it.
- Guess at build commands. Verify or ask.
