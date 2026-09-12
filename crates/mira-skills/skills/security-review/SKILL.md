---
name: security-review
description: Security-focused review of the current diff — look for injection, unsafe deserialization, path traversal, secret leaks, and other OWASP-shaped vulnerabilities that a general code review might miss.
category: review
icon: shield
color: red
---

Review the current diff for *security* bugs specifically. This is a narrower scope than a general code review — you are hunting for issues that create attack surface, not for correctness bugs.

**1. Get the diff and understand the trust boundary it touches.**
Run `git diff HEAD`. For each changed file, ask: does anything in this file cross a trust boundary?
- Does it receive user input (HTTP request, WS frame, CLI arg, environment variable, file the user chose)?
- Does it construct a command, SQL query, path, URL, or piece of HTML from that input?
- Does it authenticate, authorize, or gate access to something?
- Does it handle a secret (API key, token, password, cookie)?

If a file crosses none of these, you can skip deep review on it.

**2. Look for these classes of bug (ordered by frequency in real diffs):**
- **Command injection.** Unquoted user-controlled string reaches `Command::new`, `subprocess.run`, `child_process.exec`, `system()`, backticks. Even seemingly "safe" chars (space, `;`, `$`, `\``) matter.
- **Path traversal.** User-controlled string concatenated into a filesystem path without canonicalization + a prefix check. `../` and absolute-path escape are the two shapes.
- **SQL injection.** String-formatted SQL. `f"SELECT ... {user_input}"`. If you see a raw query, verify parameterization.
- **Insecure deserialization.** `pickle.loads`, `yaml.load` without `SafeLoader`, `serde_json::from_str::<Value>` on untrusted input followed by dispatch on internal type tags.
- **SSRF / open redirect.** User-supplied URL passed to `fetch` / `reqwest::get` without a scheme + host allowlist; user-supplied path in a `Location:` header.
- **Secret leaks.** Tokens or keys printed to logs, included in error messages, or sent to third-party services. Also: secrets committed to the repo (check `.env`, `credentials.*`, `.pem` files in the diff).
- **Auth bypass / IDOR.** A route or resource that identifies its object by user-controlled ID without checking the caller is authorized for that specific object.
- **Timing attacks.** String equality (`==`) on secrets, tokens, HMACs — use `constant_time_eq` / `crypto.timingSafeEqual`.
- **CSRF / cross-origin.** A state-changing endpoint that accepts a cookie-authed request without a same-site or CSRF-token check.
- **XSS.** User content rendered into HTML without escaping. In React, `dangerouslySetInnerHTML`. In server-side templates, `{{ … | safe }}` and equivalent.

**3. Verify each finding by tracing the taint.**
For every candidate bug, follow the untrusted value from where it enters the system to where the exploitable sink is. If there's a sanitizer, validator, or type constraint that closes the gap, drop the finding. False positives on security are especially costly — they train the reader to ignore future reports.

**4. Report format:**
For each finding:
- `file:line` — path with line number.
- **Class:** one of the categories above (or "other").
- **Attack:** one sentence, what an attacker can do.
- **Proof:** the exact taint path (`user input → line 47 → line 91 → sink`).
- **Fix:** one to three lines showing the mitigation (sanitizer, parameterization, allowlist).

If there are zero findings, say so and describe what you audited. Never pad.

**Do not:**
- Report generic best-practices ("consider using HTTPS") unless the diff introduces the specific bug.
- Suggest defense-in-depth mitigations for issues you didn't verify.
- Flag `unsafe` blocks without a specific memory-safety concern (Rust `unsafe` is not a security bug by itself).
