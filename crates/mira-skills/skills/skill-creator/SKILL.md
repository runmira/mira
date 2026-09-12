---
name: skill-creator
description: Create a new skill — the durable playbook for how mira should do a specific task in this project. Use when the user asks you to "make a skill", "teach mira to do X", "save this workflow", or when you notice a repeating pattern that would benefit from being captured as a skill.
category: meta
icon: sparkle
color: pink
---

Author a new skill file that captures a procedural workflow so future turns (and future sessions) execute it consistently instead of improvising each time. A skill is a bundled instruction file — one directory containing a `SKILL.md` + any supporting scripts or templates.

**1. Confirm scope with the user.**
Before writing anything, get answers to these:
- **Name** — kebab-case, short, mnemonic. `deploy-preview`, `write-tests`, `bump-dep`.
- **Description** — one sentence, first-person present tense, ending with when to invoke it. This is what the model reads to decide "should I use this skill?", so it MUST be discriminating. Bad: "helps with deployments." Good: "Deploy a preview environment for the current branch to Vercel. Use when the user asks for a preview URL, staging deploy, or 'let me see this live'."
- **Scope** — user-global (`~/.mira/skills/`), project-only (`<cwd>/.mira/skills/`), or should this become a PR to add it as a mira builtin? Default to project-only unless the user says otherwise.

**2. Decide the shape (flat vs. directory).**
- **Flat** (`~/.mira/skills/<name>.md`) — the skill is pure instructions, no supporting files. Simpler; use for policy/style skills like "always use conventional commits."
- **Directory** (`~/.mira/skills/<name>/SKILL.md`) — the skill references or ships helper scripts, prompt templates, config snippets, reference material. Use this whenever the skill body says "run this script" or "use this template."

Default to directory shape if you're not sure — it's easy to add attachments later, hard to migrate to it.

**3. Write the SKILL.md body.**
Use this outline. Every heading is optional; drop any that don't apply.

```markdown
---
name: <kebab-case>
description: <one sentence, ends with when to invoke>
category: <optional loose grouping: review, deploy, git, meta, …>
---

<A one-paragraph intro explaining what this skill accomplishes and its
non-goals — things you might expect it to do but explicitly doesn't.>

**1. <First step, imperative mood.>**
<One or two sentences. Concrete commands in `code fences`. If a step has
substeps, use a nested numbered list.>

**2. <Second step.>**
...

**Do not:**
- <Things this skill explicitly avoids. This section is load-bearing —
  it stops the model from over-reaching.>
```

Rules for a good skill body:
- **Imperative mood.** "Read the diff." not "The skill should read the diff."
- **Concrete commands** in code fences. `git diff HEAD`, not "check the diff".
- **Failure modes named.** If a step often goes wrong ("don't invent a build command; verify or ask"), say so.
- **Bounded scope.** A skill does ONE thing. Split into two skills before writing a 400-line SKILL.md.
- **End with a "Do not"** — three to five items. Prevents the skill from mission-creeping.

**4. Ship attached resources (directory shape only).**
Put supporting files as siblings of `SKILL.md`:
- `run.sh` / `check.sh` — small executable helpers the body invokes.
- `template.md` / `prompt.txt` — text the skill body reads and edits.
- `references/*.md` — extended documentation the body cites but doesn't need loaded on every invocation.

The runtime tool automatically enumerates these files at invocation time and passes their paths to the model, so the body can just say "read `run.sh` and follow it" without listing them manually.

**5. Write the file(s) to disk.**
For project scope: `mkdir -p .mira/skills/<name>` then `write_file` `.mira/skills/<name>/SKILL.md`. For user scope: `mkdir -p ~/.mira/skills/<name>` then write the same.

Show the user the final content and confirm before writing. Never silently overwrite an existing SKILL.md — if one exists, either `git diff` show what would change, or ask.

**6. Verify the skill loads.**
After writing, tell the user to restart mira (or trigger a reload if we support that yet) and try invoking the skill by asking for the task it covers. The description is what determines whether the model picks it up — if the model doesn't invoke the skill on a matching request, the description isn't discriminating enough. Rewrite it and retry.

**Do not:**
- Create a skill for a one-off task. Skills are for repeated workflows. If the user says "help me fix this bug," don't make a `fix-this-bug` skill.
- Copy-paste generic advice into the body. If a step is `write good code`, delete it. Skills are for project-specific procedure, not general engineering hygiene.
- Nest skills more than one category deep (`~/.mira/skills/review/security/audit.md` — no; either `security-audit.md` or `security/audit/SKILL.md`).
- Include implementation details of `mira` itself (the tool registry, harness internals). Skills are for what to do, not how mira works.
