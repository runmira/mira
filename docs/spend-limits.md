# Spend limits

Set dollar caps in Settings → General → Spend limits, or in the global
`~/.mira/mira.yaml`:

```yaml
spend_limits:
  per_session: 5
  per_day: 20
  per_month: 100
  projects:
    /absolute/path/to/project:
      per_session: 2
      per_day: 10
```

Omit a cap, or leave its Settings field blank, for no limit. Amounts must be
finite and positive. Project paths match the session's working directory.
Project caps apply alongside global caps. Repository `.mira/config.yaml` files
cannot raise or replace the user's spend policy.

Mira warns at 80% and stops further model requests at 100%. The transcript and
pending tool results are saved before the turn ends. The composer offers a
one-click session increase of 25% (at least $1); this persists across restarts
for that session and its children. It does not bypass daily or monthly caps.
Daily and monthly windows reset at midnight UTC.

Native sessions, nested subagents, goal loops, background compaction, memory
extraction, titles, and scheduled runs through the harness share the ledger.
External agents are charged from their reported running totals, deduplicated
across reconnects. Subscription usage is excluded. Native models without a
known price are refused when caps apply; unpriced external usage stops the
session with an explanation rather than silently treating it as free.

Enforcement uses reported usage. A request already in flight can finish above
a cap; an external agent can only be stopped once it reports its spend. Concurrent
requests already admitted before the cap was reached can also finish. The
ledger is local to this Mira installation, not an account-wide provider cap.

Accounting begins when this version runs; existing Usage-page history is not
retroactively imported. Spend, ownership, warning markers and session overrides
live in `~/.mira/spend-limits.json`. Updates use a cross-process file lock and an
atomic rename. A damaged or unwritable ledger stops requests instead of
resetting the counters. Changing a model or rewinding a transcript never refunds
money already spent.
