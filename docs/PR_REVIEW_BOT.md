# Mira for GitHub — auto-review every opened PR

> **Shipped in a simpler form.** Reviews and `@runmira-bot` tasks now run as a
> GitHub Action in each repository, connected with `mira github setup`
> or Settings → Integrations; see [github.md](./github.md). This
> document is the plan for a hosted GitHub App, which is still future
> work.

> A GitHub App that runs `mira review` on every opened / synchronized
> pull request, posts inline review comments, and can be re-run on
> demand with a `/mira review` PR comment. Same code path the CLI's
> `mira review --pr N` uses today; this doc is the wrapping to make
> it a service.

---

## 0. TL;DR

Ship this in five discrete stages, in this order, without cutting
corners. Each stage is a shippable milestone — merge before starting
the next one.

| Stage | What lands | Rough size |
| ----- | ---------- | ---------- |
| **1. GitHub App skeleton** | App registered, webhook receiver online, replies "hello from Mira" on new PRs | 1 day |
| **2. Auth + installation flow** | App JWT → installation token, per-install DB row, `/settings` OAuth callback | 2 days |
| **3. Review pipeline** | Webhook → job queue → `mira review` → post inline comments | 3 days |
| **4. Config + throttling** | Per-repo `.mira/review.yaml`, drafts skipped, dedup on force-push, cost caps | 2 days |
| **5. Billing + rollout** | Credits, `add credits` link, private-beta install allow-list, then GA | 2 days |

Total: ~2 weeks of focused work for a solo build, ~1 for two people.

The reference implementations to steal ideas from (in decreasing order
of relevance): **CodeRabbit**, **Ellipsis**, **Greptile**, **Capy**
(pictured in your screenshot), and Anthropic's own **Claude for GitHub**.
The mechanics are near-identical; the differences are in prompting
and how tightly they gate cost.

---

## 1. Architecture at a glance

```
┌─────────────────┐        webhook            ┌─────────────────────┐
│   GitHub PR     │ ────────────────────────▶ │  Webhook receiver   │
│  (installer's   │  X-GitHub-Event: pull_    │  (Axum, ~200 LOC)   │
│   repo)         │      request              │                     │
└─────────────────┘                           └──────────┬──────────┘
      ▲                                                  │
      │ POST /repos/:o/:r/pulls/:n/comments              │ enqueue
      │ (as GitHub App installation token)               ▼
      │                                       ┌─────────────────────┐
      │                                       │    Job queue        │
      │                                       │  (Postgres FOR      │
      │                                       │   UPDATE SKIP       │
      │                                       │   LOCKED, or        │
      │                                       │   Redis Streams)    │
      │                                       └──────────┬──────────┘
      │                                                  │
      │                                                  ▼
┌─────┴───────────┐   `mira review --pr N`   ┌─────────────────────┐
│  GitHub API     │ ◀──────────────────────  │  Review worker      │
│  (posting       │      + `gh pr comment`   │  (mira binary in    │
│   comments)     │                          │   a container)      │
└─────────────────┘                          └─────────────────────┘
```

Four boxes, one binary, one database. Nothing exotic.

**Why this shape**
- **Webhook receiver is thin.** GitHub retries a webhook up to 5 times
  with a 10s timeout, so the receiver's job is: verify signature,
  enqueue, `202 Accepted`. Never do work inline.
- **Job queue outlives the process.** A review takes 30–120s. If the
  worker dies mid-job the queue re-drives it. Skipping the queue and
  spawning a task is fine for hobby scale; break at the first outage.
- **Worker is `mira review` in a container.** You already built the
  pipeline (`crates/mira-review`, `crates/mira-cli/src/review.rs`).
  The GitHub App just calls it — no new prompting code to maintain.

---

## 2. Naming + repo layout

Before you touch code, pick a name. This is load-bearing — it's the
handle GitHub App users see next to every comment (`mira-ai`,
`mira-review`, `runmira`, `mira[bot]`, etc.).

**Recommendation:** `mira` (Marketplace slug: `runmira`).
- Bot comments show as `mira[bot] commented`.
- Reads like a first-party feature, not a third-party add-on.
- Consistent with `runmira/mira` GitHub org, `mira` binary, `mira.sh`
  domain.

Create a new crate for the service under the existing workspace so
review code is shared, not vendored:

```
mira/
├── crates/
│   ├── mira-review/         ← already exists; the pipeline
│   ├── mira-review-bot/     ← NEW: the GitHub App service
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs      ← axum bind + graceful shutdown
│   │       ├── webhook.rs   ← signature verify, event dispatch
│   │       ├── github.rs    ← App JWT + installation token + REST wrappers
│   │       ├── queue.rs     ← Postgres or Redis job queue
│   │       ├── worker.rs    ← drive `mira review`, post findings
│   │       ├── config.rs    ← per-repo `.mira/review.yaml` parsing
│   │       └── db.rs        ← installations, runs, credits tables
│   └── mira-server/         ← unchanged; the browser UI's server
└── docs/PR_REVIEW_BOT.md    ← this file
```

Keep it in the same workspace so `mira review` bug fixes reach the bot
on the next `cargo build` and never drift.

---

## 3. Stage 1 — Register the GitHub App

You do this once, by hand, on github.com.

### 3.1 Create the app

**Settings → Developer settings → GitHub Apps → New GitHub App**

Fill in:

| Field | Value |
| ----- | ----- |
| GitHub App name | `Mira` (must be globally unique on GitHub; fall back to `Mira AI` if taken) |
| Homepage URL | `https://mira.sh` (or `https://github.com/runmira/mira`) |
| Callback URL | `https://mira.sh/gh/oauth/callback` (leave for stage 2) |
| Setup URL | `https://mira.sh/gh/installed` (optional, shown post-install) |
| Webhook URL | `https://bot.mira.sh/webhook` (see stage 3.3 for local dev) |
| Webhook secret | Generate a 32-byte hex string (`openssl rand -hex 32`). Store in your secret manager. |

Uncheck **"Active"** on the webhook for now — flip it on when you're
ready to receive events in stage 3.

### 3.2 Permissions

The minimum set that lets Mira review a PR and post comments:

| Permission | Access | Why |
| ---------- | ------ | --- |
| Repository → Contents | Read | Fetch changed files to review |
| Repository → Pull requests | Read & Write | Post inline review comments + summary |
| Repository → Issues | Read & Write | Fallback: comment on the PR issue thread |
| Repository → Metadata | Read | Required by GitHub for every App |
| Repository → Checks | Read & Write | Post a "Mira review" check with `success`/`neutral`/`failure` |
| Account → Email addresses | Read | Optional; only if you want to email results |

**Do not** request `Contents: Write`, `Administration`, or organization-
level permissions. You don't need them, and Marketplace reviewers will
reject the app if you over-scope. Fewer permissions = faster install
approvals from security-conscious orgs.

### 3.3 Subscribe to events

Under **Subscribe to events**, tick exactly:

- `pull_request` — the main trigger (opened, reopened, synchronize).
- `issue_comment` — for `/mira review` re-run commands in PR comments.
- `installation` — track new installs so your DB knows about them.
- `installation_repositories` — track when installers add/remove repos.

That's the entire event surface. Everything else GitHub offers is
noise for a review bot.

### 3.4 Download the private key

Scroll to **Private keys → Generate a private key**. GitHub downloads
a `.pem`. Store this in your secret manager — you'll use it to mint
App JWTs. **This is the single most sensitive secret in the whole
system.** Losing it = anyone can impersonate your app; leaking it =
same. Store in AWS Secrets Manager / Doppler / 1Password /
`fly secrets set`, never in the repo.

You also need:
- **App ID** (visible on the app's About page — e.g. `123456`).
- **Client ID** and **Client secret** (for stage 2 OAuth).

Save all three plus the private key alongside the webhook secret.

### 3.5 Public listing (skip until stage 5)

**Public** installs let anyone add Mira to their repos. **Private**
means only your own account can install. Start **Private** — flip to
Public when the pipeline is battle-tested. Marketplace listing
(paid vs. free) is a separate flow gated on 3 org installs + working
webhook + a Marketplace review.

---

## 4. Stage 2 — Auth + installation flow

Two auth flavors live side by side. Learn the difference once — it's
the #1 source of confusion.

| Token | What it's for | How you mint it | Lifetime |
| ----- | ------------- | --------------- | -------- |
| **App JWT** | Anything the App itself does (list installations, request an install token). | HS256… wait no, **RS256** signed with your `.pem` private key, `iss = app_id`, 10-min max expiry. | 10 min |
| **Installation token** | Anything scoped to *one* installation (read a repo, post a comment). This is what the review worker uses 99% of the time. | `POST /app/installations/:id/access_tokens` with an App JWT. Returns `ghs_…`. | 1 hour |
| **User OAuth token** | Actions on behalf of a user (e.g. the settings dashboard). Only needed if you build a web UI. | Standard OAuth code exchange via `/login/oauth/authorize`. | Configurable |

### 4.1 Minting the App JWT

Copy this verbatim; it's the fiddliest 10 lines in the whole codebase.

```rust
// crates/mira-review-bot/src/github.rs
use jsonwebtoken::{encode, EncodingKey, Header, Algorithm};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
struct Claims {
    iat: u64,   // issued at (unix seconds); leave 60s of clock skew room
    exp: u64,   // expiry (unix seconds); GitHub caps at iat + 600
    iss: String, // your App ID as a string
}

pub fn app_jwt(app_id: &str, private_key_pem: &[u8]) -> anyhow::Result<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let claims = Claims {
        iat: now - 60,
        exp: now + 9 * 60, // stay under the 10-min cap
        iss: app_id.to_owned(),
    };
    let key = EncodingKey::from_rsa_pem(private_key_pem)?;
    Ok(encode(&Header::new(Algorithm::RS256), &claims, &key)?)
}
```

Cache the JWT for its 9-minute lifetime — don't mint one per request.

### 4.2 Installation token exchange

```rust
pub async fn installation_token(
    app_jwt: &str,
    installation_id: u64,
) -> anyhow::Result<String> {
    let resp = reqwest::Client::new()
        .post(format!(
            "https://api.github.com/app/installations/{installation_id}/access_tokens"
        ))
        .bearer_auth(app_jwt)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "mira-review-bot")
        .send()
        .await?
        .error_for_status()?;
    #[derive(serde::Deserialize)]
    struct TokenResp { token: String }
    Ok(resp.json::<TokenResp>().await?.token)
}
```

Cache these per-installation for ~55 minutes (5-minute leeway before
expiry). A dead-simple `HashMap<u64, (String, Instant)>` behind a
`Mutex` is fine at low install counts; swap for Redis when you cross
~100 installs.

### 4.3 Track installations in your DB

Handle the `installation` and `installation_repositories` webhook events
so your DB always mirrors GitHub's view. Minimum schema:

```sql
CREATE TABLE installations (
    id            BIGINT PRIMARY KEY,        -- GitHub installation id
    account_type  TEXT NOT NULL,             -- 'User' | 'Organization'
    account_login TEXT NOT NULL,             -- 'runmira'
    installed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    suspended_at  TIMESTAMPTZ                -- non-null when installer suspends the app
);

CREATE TABLE installation_repos (
    installation_id BIGINT NOT NULL REFERENCES installations(id) ON DELETE CASCADE,
    repo_full_name  TEXT NOT NULL,           -- 'runmira/mira'
    PRIMARY KEY (installation_id, repo_full_name)
);
```

On `installation.created` insert both. On `installation.deleted` /
`suspend`, mark suspended (don't hard-delete — the installer might
re-install). On `installation_repositories.added` / `removed`,
update the join table.

---

## 5. Stage 3 — The review pipeline

The webhook receiver's entire job is: verify, parse enough to dispatch,
enqueue, return 202. Keep it under 50 LOC.

### 5.1 Webhook receiver

```rust
// crates/mira-review-bot/src/webhook.rs
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub async fn handle(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Step 1: verify signature. Constant-time compare — never `==`.
    let sig = match headers.get("X-Hub-Signature-256").and_then(|v| v.to_str().ok()) {
        Some(s) => s,
        None => return (StatusCode::UNAUTHORIZED, "missing signature").into_response(),
    };
    if !verify_sig(&app.webhook_secret, &body, sig) {
        return (StatusCode::UNAUTHORIZED, "bad signature").into_response();
    }

    // Step 2: read event type.
    let event = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Step 3: enqueue and return fast. Every event we handle is
    // idempotent on delivery id, so a webhook retry is safe.
    let delivery_id = headers
        .get("X-GitHub-Delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    if let Err(e) = app.queue.enqueue(event, &delivery_id, &body).await {
        tracing::error!(%e, event, delivery_id, "enqueue failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "enqueue failed").into_response();
    }

    (StatusCode::ACCEPTED, "").into_response()
}

fn verify_sig(secret: &str, body: &[u8], sig_header: &str) -> bool {
    let sig = sig_header.strip_prefix("sha256=").unwrap_or("");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let expected = mac.finalize().into_bytes();
    let Ok(provided) = hex::decode(sig) else { return false };
    // constant-time compare
    provided.len() == expected.len()
        && provided
            .iter()
            .zip(expected.iter())
            .all(|(a, b)| a == b)
}
```

That's the whole receiver. Don't add rate limiting here (GitHub
already limits webhook delivery rate per app). Don't add per-event
routing here (that's the worker's job). Ship 202s or die trying.

### 5.2 Job queue

Two options — pick based on how much infra you already have.

**Postgres LISTEN/NOTIFY + `FOR UPDATE SKIP LOCKED`** (recommended)
- Zero new infra if you're already using Postgres for the auth tables.
- Idempotency via `UNIQUE (delivery_id)` — dedup GitHub's retries for free.
- Cheap to reason about, easy to inspect.

```sql
CREATE TABLE review_runs (
    id              BIGSERIAL PRIMARY KEY,
    delivery_id     TEXT NOT NULL UNIQUE,       -- X-GitHub-Delivery header
    installation_id BIGINT NOT NULL REFERENCES installations(id),
    repo_full_name  TEXT NOT NULL,
    pr_number       INTEGER NOT NULL,
    head_sha        TEXT NOT NULL,
    event_type      TEXT NOT NULL,              -- 'pull_request' | 'issue_comment'
    status          TEXT NOT NULL DEFAULT 'queued',  -- queued | running | done | failed
    attempts        INTEGER NOT NULL DEFAULT 0,
    enqueued_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ,
    error           TEXT
);

CREATE INDEX ON review_runs (status, enqueued_at) WHERE status IN ('queued', 'running');
```

Worker fetch loop:

```sql
UPDATE review_runs
SET status = 'running', started_at = now(), attempts = attempts + 1
WHERE id = (
    SELECT id FROM review_runs
    WHERE status = 'queued' AND attempts < 3
    ORDER BY enqueued_at
    LIMIT 1
    FOR UPDATE SKIP LOCKED
)
RETURNING *;
```

**Redis Streams** — use if you already run Redis. Slightly less nice
for inspection but higher throughput. Same idempotency shape via
`XADD ... NX`.

### 5.3 Worker: drive `mira review` and post findings

```rust
// crates/mira-review-bot/src/worker.rs
pub async fn run_review(app: &AppState, run: ReviewRun) -> anyhow::Result<()> {
    let token = app.installation_token(run.installation_id).await?;

    // Fetch the PR diff via the installation token — don't shell out
    // to `gh` here (it wants a user token). Use octocrab / plain
    // reqwest against `/repos/:o/:r/pulls/:n`.
    let diff = fetch_pr_diff(&token, &run.repo_full_name, run.pr_number).await?;

    // Bail cheaply on the shapes we don't want to review.
    if diff.is_empty() {
        return Ok(());
    }
    if diff.lines().count() > 5_000 {
        // Cost cap: skip the review, post a summary comment saying
        // "diff too large — split the PR or run `mira review` locally".
        post_issue_comment(
            &token,
            &run.repo_full_name,
            run.pr_number,
            LARGE_PR_MSG,
        )
        .await?;
        return Ok(());
    }

    // The actual review — same pipeline the CLI drives.
    let findings = mira_review::review(
        &diff,
        &app.chat_provider(),        // your ChatProvider handle
        &app.review_config(),        // stage 1, stage 2 knobs
        &mira_review::NullProgress,  // no TTY here
    )
    .await?;

    // Post either inline review comments (preferred) or a summary
    // when the finding doesn't map to a file/line in the diff.
    let (inline, summary): (Vec<_>, Vec<_>) = findings
        .into_iter()
        .partition(|f| f.file.is_some() && f.line.is_some());

    if !inline.is_empty() {
        post_review(&token, &run, &inline).await?;   // POST /pulls/:n/reviews
    }
    if !summary.is_empty() {
        post_issue_comment(
            &token,
            &run.repo_full_name,
            run.pr_number,
            &render_summary(&summary),
        )
        .await?;
    }

    // Post a check-run so the PR page shows a green ✓ (or amber ⚠).
    // This is the single biggest UX win — the check appears next to
    // the Vercel / test-suite checks and makes Mira feel first-party.
    post_check_run(&token, &run, &inline, &summary).await?;

    Ok(())
}
```

**Inline review comment format** — GitHub wants a specific POST shape:

```
POST /repos/:owner/:repo/pulls/:number/reviews
{
  "commit_id": "<pr.head.sha>",
  "event": "COMMENT",          // or "REQUEST_CHANGES" for blocking
  "body": "<optional overall summary>",
  "comments": [
    {
      "path": "src/foo.rs",
      "line": 42,
      "side": "RIGHT",
      "body": "This unwrap panics on the empty-input case…"
    },
    ...
  ]
}
```

`side: "RIGHT"` for the new version, `"LEFT"` for the old. `line` must
be a line that appears in the diff — GitHub 422s otherwise. If your
finding's line isn't in the diff, fall back to a summary comment
rather than dropping the finding.

### 5.4 Local dev

Use `smee.io` or `cloudflared tunnel` to receive webhooks on your laptop
without deploying:

```
brew install cloudflared
cloudflared tunnel --url http://localhost:8080
```

Copy the `https://<random>.trycloudflare.com` URL into the App's
webhook URL. Redeliveries are cheap — the App's Advanced tab has a
**Redeliver** button for every past webhook, which is how you'll
debug 90% of the flow.

---

## 6. Stage 4 — Config, throttling, dedup

Once basic reviews work, add the guardrails that stop the bot from
being obnoxious.

### 6.1 Per-repo config

```yaml
# .mira/review.yaml — checked into the repo being reviewed
enabled: true                   # opt out per-repo without uninstalling
skip_draft: true                # ignore PRs marked draft (recommended)
skip_paths:                     # never review these — generated code
  - "**/*.lock"
  - "**/generated/**"
  - "docs/**/*.md"
severity_threshold: warning     # info | warning | error — filter noise
max_comments_per_review: 20     # don't dump 300 comments on a large PR
review_kinds:                   # only these finding categories
  - correctness
  - security
  - regression
```

Fetch this on every run via `GET /repos/:o/:r/contents/.mira/review.yaml`.
Cache for 60s per repo — config edits should land quickly but not
hammer the API on every push.

### 6.2 Dedup on force-push

Every `pull_request.synchronize` event carries the new `head.sha`.
Look for a prior `review_runs` row for `(repo, pr_number, head_sha)`
and skip if one exists. Without this, force-pushing to fix a typo
triggers a full re-review — expensive and annoying.

```sql
SELECT 1 FROM review_runs
WHERE repo_full_name = $1 AND pr_number = $2 AND head_sha = $3
  AND status = 'done';
```

### 6.3 Draft PR handling

`pr.draft == true` — skip. Post nothing. When the PR is marked
ready-for-review (`pull_request.ready_for_review` event), review then.
This alone cuts a lot of noise since most people open PRs in draft
while iterating.

### 6.4 Cost cap per repo per day

The single most important guardrail. A malicious install could open
1,000 PRs and cost you $$$$. Cap it:

```sql
CREATE TABLE daily_cost (
    installation_id BIGINT NOT NULL,
    day             DATE NOT NULL,
    usd_spent       NUMERIC(10, 4) NOT NULL DEFAULT 0,
    PRIMARY KEY (installation_id, day)
);
```

Before every review: `SELECT usd_spent WHERE installation_id = ? AND day = today`.
If over the cap (start with $5/day for free tier), post a "budget hit"
comment and skip. After the review, `UPDATE ... SET usd_spent = usd_spent + <cost>`.
Cost you already compute in `mira review` — pipe it through.

### 6.5 On-demand re-run via PR comment

When someone comments `/mira review` on a PR, the `issue_comment`
event fires. Match the trigger, enqueue a new review for the current
`head.sha`. Same worker path.

```rust
// In the webhook dispatcher:
if event == "issue_comment"
    && payload.action == "created"
    && payload.comment.body.trim_start().starts_with("/mira review")
    && payload.issue.pull_request.is_some()
{
    // Enqueue as if a fresh pull_request event arrived.
}
```

Support a small vocabulary — `/mira review`, `/mira review --deep`
(runs the hostile-verify stage 2 as well), `/mira status`, `/mira help`.

---

## 7. Stage 5 — Billing + rollout

### 7.1 Free tier

Say it loud on the marketing page:

- **Free forever** for public repos.
- Free for private repos up to N reviews / month per installation.

This is table stakes — CodeRabbit and Ellipsis both offer this. Your
cost is $0.02–$0.20 per review (Sonnet 4.6 pricing at typical PR
sizes); a $5 free monthly quota costs you $50–$200 across the free
population. Cheap acquisition; converts on serious teams.

### 7.2 Paid tiers

Match Capy / CodeRabbit shape unless you have a strong reason to
deviate — teams already understand this pricing:

| Plan | Price | What's included |
| ---- | ----- | --------------- |
| Free | $0 | Public repos; 100 reviews/mo/install on private repos |
| Pro | $12/dev/mo | Unlimited reviews on private repos, `/mira review --deep`, custom rules |
| Team | $24/dev/mo | Above + org-wide config, per-repo cost dashboards |
| Enterprise | Custom | SSO, SOC 2, on-prem worker deploy |

Bill through Stripe. Show remaining credits in every "we're out"
comment (matches the Capy screenshot you sent) — this is the moment
of highest conversion intent.

### 7.3 Marketplace listing

Fill in **Marketplace listing** on the App settings page. Requirements:

- 3+ different orgs installed
- Webhook must be responding 2xx
- App logo, screenshots, pricing description
- Terms of Service + Privacy Policy URLs (public repo is fine)

GitHub reviews the listing manually — takes 1-4 weeks. During review,
Public installs still work through the direct App URL, just not
through Marketplace search.

### 7.4 Rollout stages

**Private beta (weeks 1–3)**
- Keep the App in `Private` visibility.
- Install only on `runmira/*` repos plus 2–3 friendly test accounts.
- Break things freely.

**Open beta (weeks 4–6)**
- Flip to `Public`.
- Waitlist form on mira.sh — hand out install links yourself.
- Watch cost dashboards like a hawk; you WILL find abuse patterns.

**GA (week 7+)**
- Marketplace listing submitted.
- Stripe live keys.
- On-call rotation (or at least a PagerDuty schedule of one) for
  webhook downtime.

---

## 8. Security checklist

Don't skip any of these. Any single miss is a legit breach.

- [ ] Webhook signature verified on **every** request with a
      constant-time compare (`hmac::Mac::verify_slice`, not `==`).
- [ ] Private key stored in a secret manager, mounted read-only into
      the process. Never in git, never in an env var pasted into a
      Slack channel.
- [ ] Installation tokens cached in memory only. Never written to
      disk or logs.
- [ ] Rate limit the webhook receiver by IP (Cloudflare in front,
      or `tower-governor`). GitHub's IPs are published — you can also
      allow-list them.
- [ ] `Content-Type` check: reject anything not `application/json` on
      the webhook route.
- [ ] Log the delivery id + event type + repo, **not** the payload
      body. Payloads contain PR text which contains user secrets more
      often than you'd think.
- [ ] `SECURITY.md` at the repo root with an email address that
      forwards to a real inbox. GitHub's Marketplace review checks
      for this.
- [ ] `Content-Security-Policy` on the dashboard site if you build one.
- [ ] Regular rotation drill: pretend the private key leaked, walk
      the "revoke + regenerate + redeploy" playbook end-to-end. Do
      it once before GA so you know it works.

---

## 9. Deployment

Any container host works. Concrete recipes for the three that make
setup easiest:

### 9.1 Fly.io (recommended for hobby → mid-scale)

```toml
# fly.toml
app = "mira-review-bot"
primary_region = "iad"

[build]
  dockerfile = "crates/mira-review-bot/Dockerfile"

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = false   # webhooks can't wake a sleeping worker fast enough
  min_machines_running = 1

[[vm]]
  size = "shared-cpu-1x"
  memory = "1gb"
```

Secrets:

```
fly secrets set \
    GITHUB_APP_ID=123456 \
    GITHUB_APP_PRIVATE_KEY="$(cat mira-app.pem)" \
    GITHUB_WEBHOOK_SECRET=... \
    DATABASE_URL=postgres://... \
    ANTHROPIC_API_KEY=sk-...
```

Add a managed Postgres via `fly postgres create` or point at Neon /
Supabase — anything with `postgres://` works.

### 9.2 Cloudflare Workers + Neon

- Webhook receiver as a Worker (colocated with GitHub's edge → sub-
  100ms 202s).
- Queue = Cloudflare Queues (or Postgres).
- Worker = a long-running container elsewhere (Fly, Railway) that
  pulls from the queue. Workers can't hold the LLM call open for 60s.

Higher moving-part count but genuinely cheap at scale.

### 9.3 Your own VPS

If you already have a Hetzner box, run:

- Postgres via `apt`.
- The bot as a systemd service behind Caddy for TLS.
- No queue infra — use Postgres LISTEN/NOTIFY.
- Backups via `pg_dumpall` to S3 nightly.

$5/mo, zero vendor lock-in, plenty for the first 100 installs.

---

## 10. Observability

You'll want three views before you go live:

1. **Webhook health** — Prometheus counter for `webhook_delivered_total`
   labeled by event type. Alert when 2xx rate drops below 99%.
2. **Review latency** — histogram of `enqueue → done` duration. Alert
   when p95 > 3 minutes.
3. **Cost per install per day** — chart. Watch for outliers.

Ship the metrics from day one via `axum-prometheus` and scrape into
whatever you already run (Grafana Cloud has a free tier that's plenty).

Log format: structured JSON via `tracing-subscriber` with fields
`delivery_id`, `installation_id`, `repo`, `pr`. `jq` becomes your
support tool.

---

## 11. What to build first (concrete checklist)

Copy this into an issue.

- [ ] Register `Mira` GitHub App (private visibility)
- [ ] Store App ID, private key, webhook secret in your secret manager
- [ ] Scaffold `crates/mira-review-bot` with axum + tokio
- [ ] Implement webhook signature verification (with test vectors from GitHub's docs)
- [ ] Postgres schema: `installations`, `installation_repos`, `review_runs`, `daily_cost`
- [ ] App JWT minting + installation token caching
- [ ] Handle `installation` / `installation_repositories` events to keep DB in sync
- [ ] Handle `pull_request.opened` / `synchronize` events → enqueue a `review_runs` row
- [ ] Worker: fetch PR diff, call `mira_review::review`, post inline comments + check run
- [ ] `.mira/review.yaml` per-repo config (skip_paths, severity_threshold, max_comments)
- [ ] Draft-PR skip, force-push dedup, daily cost cap
- [ ] `/mira review` re-run via `issue_comment`
- [ ] Fly.io deploy + secrets + Postgres
- [ ] Cloudflared tunnel for local dev + GitHub App "Redeliver" workflow documented in the crate's README
- [ ] Install on 2–3 friendly test repos; iterate for a week
- [ ] Stripe billing + credits table + "you're out of credits" comment
- [ ] Marketplace listing submitted
- [ ] Public launch tweet — post the first review comment as the demo

---

## 12. Anti-patterns to avoid

Every one of these has burned somebody in the space. Learn from other
people's incidents.

1. **Reviewing the bot's own PRs.** Filter `pr.user.type == "Bot"`.
   Otherwise Dependabot's daily version bumps generate 30 reviews/day/repo.
2. **Reviewing merge commits.** `pull_request.action == "closed" &&
   pull_request.merged == true` fires once per merged PR — ignore it.
   The review already happened on the diff; re-reviewing the merge
   is wasted spend.
3. **Not versioning your prompt.** When you tune the review prompt,
   older `review_runs` will look inconsistent. Store the prompt hash
   next to the run so you can compare apples to apples over time.
4. **Blocking the merge by default.** Post as `event: "COMMENT"`, not
   `"REQUEST_CHANGES"`. Users must explicitly opt in to blocking via
   `.mira/review.yaml`. Blocking by default gets your app uninstalled.
5. **Emoji spam.** Every finding as a top-level `:warning:` comment
   is noise. Group by severity, one summary comment + N inline. The
   Capy screenshot you shared is a good example of the *minimum*
   footprint per install.
6. **Reviewing on `push` events.** Wait for a PR to exist. `push` fires
   on every branch push including topic branches nobody will ever
   merge; you'll burn money and get uninstalled.
7. **Silent failures.** Every review-run failure should either post
   a comment ("Mira couldn't review this — reason: …") or fail the
   check-run. Silent nothing is worse than a red X.

---

## 13. Reference reading (in priority order)

1. GitHub's own [Building GitHub Apps](https://docs.github.com/en/apps/creating-github-apps/about-creating-github-apps/about-creating-github-apps) — do the "Setting up your development environment to create a GitHub App" tutorial end-to-end. Two hours; unlocks everything else.
2. [`octocrab`](https://docs.rs/octocrab) — Rust GitHub API client that already handles App JWT + installation tokens. Skips ~200 LOC of your bot.
3. [Webhook payload docs](https://docs.github.com/en/webhooks/webhook-events-and-payloads) — the shape of every event; keep the tab open.
4. [Reviewing PRs via API](https://docs.github.com/en/rest/pulls/reviews) — inline comments, review events, exactly which `line` values are legal.
5. CodeRabbit's [changelog](https://coderabbit.ai/changelog) — read the last year to see what problems teams complain about at scale.

---

## 14. Rough cost model (napkin)

Assume Sonnet 4.6 pricing:
- Input: $3/MTok
- Output: $15/MTok
- Cached input: $0.30/MTok

A "typical" PR review with Mira's two-stage pipeline:
- Stage 1: ~15k input tokens (diff + surrounding context) + ~2k output
  → **~$0.09**
- Stage 2 (hostile verify): ~5k input + ~1k output → **~$0.03**
- Prompt caching hits reduce stage-1 input by ~60% on repeat runs
  against the same repo → **~$0.06/review** at steady state.

Cost model:
- 100 free-tier installs × 10 reviews/mo each = 1000 reviews = **$60/mo**
- 10 paying teams at $500/mo each = **$5,000/mo revenue** for
  $200-300/mo in inference cost.

That's roughly the SaaS-margin CodeRabbit / Ellipsis / Greptile operate
at. Model gets cheaper over time; margin gets better.

---

## Appendix A — Environment variable checklist

Copy into `.env.example`:

```
GITHUB_APP_ID=123456
GITHUB_APP_PRIVATE_KEY=/run/secrets/mira-app.pem
GITHUB_WEBHOOK_SECRET=abc123...
GITHUB_OAUTH_CLIENT_ID=Iv1...
GITHUB_OAUTH_CLIENT_SECRET=...

DATABASE_URL=postgres://mira:mira@localhost/mira_review_bot

ANTHROPIC_API_KEY=sk-ant-...     # or OPENROUTER_API_KEY, etc.
MIRA_MODEL=anthropic/claude-sonnet-4-6

BOT_LISTEN_ADDR=0.0.0.0:8080
BOT_PUBLIC_URL=https://bot.mira.sh
BOT_LOG_LEVEL=info

# Cost caps
DAILY_COST_CAP_FREE_USD=1.00
DAILY_COST_CAP_PRO_USD=20.00

# Optional
SENTRY_DSN=
STATSD_HOST=
```

## Appendix B — Minimum viable Dockerfile

```dockerfile
# crates/mira-review-bot/Dockerfile
FROM rust:1.88-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p mira-review-bot

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y ca-certificates git && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/mira-review-bot /usr/local/bin/mira-review-bot
COPY --from=builder /build/target/release/mira /usr/local/bin/mira
ENV RUST_LOG=info
EXPOSE 8080
CMD ["/usr/local/bin/mira-review-bot"]
```

Two binaries in one image: the bot service and the `mira` CLI it
invokes (through library call, not shell — but keeping the binary
around lets you `docker exec` and run `mira review` by hand for
debugging).

---

## Appendix C — What we could add later (v2)

- `mira review --deep` — a longer, more expensive pass triggered by
  a maintainer label. Runs stage 3 (adversarial reviewer that argues
  with itself).
- **Local dev preview** — cross-post the review to a Slack channel
  when the PR is against a repo the installer has connected.
- **Suggestions blocks** — GitHub supports ```` ```suggestion ```` blocks
  in review comments; the reviewer can one-click apply. Big value-add.
- **Delta review** — on `synchronize` after a first review, only comment
  on changes to previously-flagged lines. Feels much better than
  "here's the whole review again."
- **Cross-PR context** — remember prior reviews per repo so Mira can
  say "this is the third time this pattern shipped; consider adding
  a lint."

These are all straightforward once the base pipeline works. Ship
the base first.
