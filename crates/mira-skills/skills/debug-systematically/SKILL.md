---
name: debug-systematically
description: Debug a bug by isolating the failing behavior, forming a hypothesis, and testing it — instead of shotgunning fixes. Use when the user reports something broken, when a test is failing, when a change didn't produce the expected result, or any time the reflex is "let me try something."
category: debug
icon: bug
color: teal
---

Bugs get fixed faster by narrowing the search space than by pattern-matching a fix from memory. The reflex "let me try changing this" is where hours get lost. Follow this loop instead: **reproduce → isolate → hypothesise → test → fix → verify**.

**1. Reproduce it exactly.**
Before touching anything, get a concrete, deterministic reproduction. Ask the user for:
- The exact command / URL / interaction that triggers the bug.
- The exact observed behavior (error message, wrong output, crash).
- The expected behavior.

If any of these are vague ("it's slow", "sometimes it breaks"), pin them down before proceeding. "Slow" → "the request takes 8 seconds; I expected < 1 second." A hypothesis you can't check with the reproduction is a hypothesis you can't test.

**2. Isolate the failing surface.**
Run the reproduction yourself. If it involves multiple steps, halve them:
- **Recent-change bisect:** `git log --oneline -20`. Was this working an hour ago? Yesterday? If yes, `git bisect` between then and now. This is often instant.
- **Layer bisect:** For a "the button doesn't work" bug — does the API call fire? Does the server receive it? Does it return the expected response? Does the handler run? Each `no` narrows the search 10×.
- **Input bisect:** For a "the parser dies on file X" bug — trim file X in half. Still fails? Trim again.

Stop when you have the smallest reproduction the code can fail on.

**3. Form a specific hypothesis before making changes.**
Not "maybe there's a race condition." Yes: "line 47 reads `session_id` before line 91 writes it, and if the response arrives before line 91 executes, `session_id` is empty — that's why the second request 404s." A specific hypothesis names:
- The file:line that's wrong.
- The condition that triggers it.
- Why the fix will work.

If you can't state all three, you don't have a hypothesis — you have a guess.

**4. Test the hypothesis without fixing yet.**
- **Read the code path.** Trace the value through every function it touches. Note where it could be `None`, empty, wrong-typed, or unmutex'd.
- **Add a probe.** `println!`, `console.log`, `eprintln!`, or a breakpoint. Confirm the condition you predicted is actually what's happening. Real-world bugs are often *almost* what you predicted but not exactly — and the "almost" is the whole fix.
- **If wrong**, go back to step 3 with the new information. Don't fix based on a hypothesis that failed the test.

**5. Fix the root cause, not the symptom.**
The symptom is "response is empty." The root cause might be "the `session_id` write is racing the read." A patch that adds `if response.is_empty() { retry() }` fixes the symptom and leaves the race in place to bite you elsewhere.

Ask: if I removed my fix, would the same bug reappear? If yes, you fixed the symptom.

**6. Verify the fix.**
- Rerun the exact reproduction from step 1. Confirm the fix.
- Run adjacent tests / adjacent code paths. A race condition fix in one place often means the same race exists three lines down.
- Remove any probes you added in step 4 before committing.

**Do not:**
- Change more than one thing at once. If a fix has two edits and it works, you don't know which one mattered.
- Add `try/except: pass` or `.unwrap_or_default()` to make an error "go away." That's not a fix; that's hiding the bug.
- Delete tests that fail. If a test is wrong, fix the test with an explicit commit; if the code is wrong, fix the code. Never both in the same commit.
- Say "I fixed it" without step 6. "I made a change and the error is gone" is not verification — assertions about causation require you to have run the reproduction.
