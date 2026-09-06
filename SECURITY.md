# Security Policy

## Supported versions

Mira is pre-1.0. Only the `main` branch is supported. Security fixes land on
`main` and are cut into the next release.

## Reporting a vulnerability

**Please do not open a public issue for vulnerabilities.**

Email **flutterdami@gmail.com** with:

- A description of the issue and its impact.
- Steps to reproduce (a minimal input that triggers it is ideal).
- The commit / version you tested against.
- Whether you'd like credit in the fix commit.

> **Note:** Please encrypt any sensitive information in your report if possible.

You should get an acknowledgement within 72 hours. If it's confirmed, we'll
work on a fix in a private branch and coordinate disclosure with you.

## Scope

In scope:

- Sandbox escapes (a tool call that reads or writes outside the allowed root).
- Permission bypasses (a rule that should deny but doesn't).
- Credential leaks (API keys, tokens surfaced in logs or tool output).
- Prompt-injection paths that let untrusted content invoke privileged tools
  without user approval.

Out of scope:

- Model misbehavior that stays within granted permissions (e.g. `yolo` mode
  producing destructive commands — that's what `yolo` means).
- Denial of service via user-supplied prompts (rate limits are a provider
  concern).
- Issues in third-party providers or models Mira connects to.

Thank you for helping to keep Mira secure!
