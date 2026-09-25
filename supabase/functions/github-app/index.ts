// The Runmira GitHub App's backend. Holds the app's private key (which
// can't ship in open-source Mira) and hands out short-lived tokens:
//
//   GET  /status                   is the app set up? (its slug)
//   GET  /manifest?key=…           the app manifest, for the owner to create it once
//   GET  /manifest/callback        GitHub sends the new app's credentials here
//   POST /start          (user)    begin an install; returns the URL to open
//   GET  /install?state=…          remember who is installing, go to GitHub
//   GET  /callback                 GitHub sends the installer back; we send
//                                  them back to their Mira
//   GET  /installations  (user)    the user's installations and repositories
//   POST /token          (user)    a token for one repo, to set it up
//   POST /actions-token  (OIDC)    a token for the repo a workflow runs in
//
// Supabase serves function HTML as plain text, so there are no pages
// here: the browser only passes through on redirects.
//
// "(user)" routes take the caller's Supabase session as a Bearer token;
// /actions-token takes a GitHub Actions OIDC token instead. JWT
// verification at the gateway is off because of those two kinds of
// caller; every route checks what it needs itself.

import { createClient } from "npm:@supabase/supabase-js@2";
import { createRemoteJWKSet, jwtVerify } from "npm:jose@5";
import { createSign, randomBytes } from "node:crypto";
import { Buffer } from "node:buffer";

const SUPABASE_URL = Deno.env.get("SUPABASE_URL")!;
const FN = `${SUPABASE_URL}/functions/v1/github-app`;
const GH = "https://api.github.com";
/** Audience a workflow asks for when it requests its OIDC token. */
const OIDC_AUDIENCE = "runmira";
const OIDC_ISSUER = "https://token.actions.githubusercontent.com";
const jwks = createRemoteJWKSet(new URL(`${OIDC_ISSUER}/.well-known/jwks`));

const db = createClient(SUPABASE_URL, Deno.env.get("SUPABASE_SERVICE_ROLE_KEY")!, {
  auth: { persistSession: false },
});

const cors = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Headers": "authorization, x-client-info, apikey, content-type",
  "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
};

class HttpError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { ...cors, "Content-Type": "application/json" },
  });

const redirect = (to: string, extra: HeadersInit = {}) =>
  new Response(null, { status: 302, headers: { Location: to, ...extra } });

const text = (body: string, status = 200) =>
  new Response(body, { status, headers: { "Content-Type": "text/plain; charset=utf-8" } });

/** Only a Mira running on this machine may be sent back to. */
function safeReturn(to: unknown): string | null {
  try {
    const u = new URL(String(to));
    const local = ["localhost", "127.0.0.1", "[::1]", "tauri.localhost"].includes(u.hostname);
    return local && ["http:", "https:", "tauri:"].includes(u.protocol) ? u.origin + u.pathname : null;
  } catch {
    return null;
  }
}

function back(to: string, params: Record<string, string>) {
  const u = new URL(to);
  for (const [k, v] of Object.entries(params)) u.searchParams.set(k, v);
  return u.toString();
}

/* ---------- the app ---------- */

type App = { app_id: number; slug: string; client_id: string; client_secret: string; private_key: string };

async function app(): Promise<App> {
  const { data } = await db.from("github_app").select("*").eq("id", 1).maybeSingle();
  if (!data) throw new HttpError(503, "The GitHub App hasn't been created yet.");
  return data as App;
}

/** A 9-minute JWT that authenticates as the app itself. */
function appJwt(a: App): string {
  const now = Math.floor(Date.now() / 1000);
  const enc = (o: unknown) => Buffer.from(JSON.stringify(o)).toString("base64url");
  const unsigned = `${enc({ alg: "RS256", typ: "JWT" })}.${enc({ iat: now - 60, exp: now + 540, iss: String(a.app_id) })}`;
  const sig = createSign("RSA-SHA256").update(unsigned).sign(a.private_key).toString("base64url");
  return `${unsigned}.${sig}`;
}

async function gh(path: string, token: string, init: RequestInit = {}) {
  const res = await fetch(path.startsWith("http") ? path : `${GH}${path}`, {
    ...init,
    headers: {
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      "User-Agent": "runmira",
      Authorization: `Bearer ${token}`,
      ...(init.body ? { "Content-Type": "application/json" } : {}),
      ...(init.headers ?? {}),
    },
  });
  const text = await res.text();
  const body = text ? JSON.parse(text) : null;
  if (!res.ok) throw new HttpError(res.status === 404 ? 404 : 502, `GitHub ${path}: ${body?.message ?? res.status}`);
  return body;
}

/** An installation token, optionally limited to repos and permissions. */
async function installationToken(
  a: App,
  installationId: number,
  opts: { repositories?: string[]; permissions?: Record<string, string> } = {},
) {
  return await gh(`/app/installations/${installationId}/access_tokens`, appJwt(a), {
    method: "POST",
    body: JSON.stringify(opts),
  });
}

/* ---------- callers ---------- */

async function user(req: Request) {
  const jwt = req.headers.get("Authorization")?.replace(/^Bearer\s+/i, "");
  if (!jwt) throw new HttpError(401, "Sign in to Mira first.");
  const { data, error } = await db.auth.getUser(jwt);
  if (error || !data.user) throw new HttpError(401, "Your Mira session has expired; sign in again.");
  return data.user;
}

/** The user's link to an installation, with their GitHub login. */
async function linked(userId: string, installationId: number): Promise<string> {
  const { data } = await db
    .from("github_installations")
    .select("github_login")
    .eq("user_id", userId)
    .eq("installation_id", installationId)
    .maybeSingle();
  if (!data) throw new HttpError(403, "That GitHub installation isn't connected to your Mira account.");
  return data.github_login;
}

/** Setting a repository up writes Actions secrets, which GitHub only lets
 *  admins do; hold people to the same rule. */
async function isAdmin(token: string, fullName: string, login: string): Promise<boolean> {
  if (!login) return false;
  try {
    const p = await gh(`/repos/${fullName}/collaborators/${encodeURIComponent(login)}/permission`, token);
    return p?.permission === "admin";
  } catch {
    return false;
  }
}

const randomKey = () => randomBytes(24).toString("base64url");

/* ---------- routes ---------- */

async function status() {
  const { data } = await db.from("github_app").select("slug").eq("id", 1).maybeSingle();
  return json({ configured: !!data, slug: data?.slug ?? null });
}

/** Owner-only: the manifest Mira posts to GitHub to create the app. */
async function manifest(url: URL) {
  const key = url.searchParams.get("key") ?? "";
  const { data } = await db.from("github_app_setup").select("key").eq("key", key).maybeSingle();
  if (!data) throw new HttpError(403, "This setup key is invalid or already used.");
  const org = url.searchParams.get("org");
  const name = url.searchParams.get("name") || "Runmira";
  const manifest = {
    name,
    url: "https://github.com/runmira/mira",
    description: "Mira, the open-source coding agent: reviews pull requests and works on @mira requests.",
    hook_attributes: { url: `${FN}/webhook`, active: false },
    redirect_url: `${FN}/manifest/callback`,
    callback_urls: [`${FN}/callback`],
    setup_url: `${FN}/callback`,
    setup_on_update: true,
    request_oauth_on_install: true,
    public: true,
    default_permissions: {
      contents: "write",
      pull_requests: "write",
      issues: "write",
      workflows: "write",
      secrets: "write",
      actions_variables: "write",
      metadata: "read",
    },
    default_events: [],
  };
  const base = org
    ? `https://github.com/organizations/${encodeURIComponent(org)}/settings/apps/new`
    : "https://github.com/settings/apps/new";
  return json({ action: `${base}?state=${encodeURIComponent(key)}`, manifest });
}

/** GitHub created the app: store its credentials, show the app. */
async function manifestCallback(url: URL) {
  const key = url.searchParams.get("state") ?? "";
  const code = url.searchParams.get("code") ?? "";
  const { data } = await db.from("github_app_setup").select("key").eq("key", key).maybeSingle();
  if (!data) return text("This setup key is invalid or already used.", 403);
  const res = await fetch(`${GH}/app-manifests/${encodeURIComponent(code)}/conversions`, {
    method: "POST",
    headers: { Accept: "application/vnd.github+json", "User-Agent": "runmira" },
  });
  const created = await res.json();
  if (!res.ok) return text(`GitHub refused: ${created?.message ?? res.status}`, 502);
  const { error } = await db.from("github_app").upsert({
    id: 1,
    app_id: created.id,
    slug: created.slug,
    client_id: created.client_id,
    client_secret: created.client_secret,
    private_key: created.pem,
    webhook_secret: created.webhook_secret,
  });
  if (error) return text(`Couldn't save the app: ${error.message}`, 500);
  await db.from("github_app_setup").delete().eq("key", key);
  return redirect(created.html_url);
}

async function start(req: Request) {
  const u = await user(req);
  await app();
  const { return_to } = await req.json().catch(() => ({}));
  const to = safeReturn(return_to);
  if (!to) throw new HttpError(400, "return_to must be the Mira app on this machine.");
  const state = randomKey();
  await db.from("github_install_states").insert({ state, user_id: u.id, return_to: to });
  return json({ url: `${FN}/install?state=${state}` });
}

/** Remember the installer in a cookie, then go to GitHub. */
async function install(url: URL) {
  const a = await app();
  const state = url.searchParams.get("state") ?? "";
  return redirect(`https://github.com/apps/${a.slug}/installations/new?state=${encodeURIComponent(state)}`, {
    "Set-Cookie": `mira_gh_state=${state}; Path=/functions/v1/github-app; HttpOnly; Secure; SameSite=Lax; Max-Age=1800`,
  });
}

/** Back from GitHub: link the installer's installations to their Mira
 *  account, then send them back to their Mira. */
async function callback(req: Request, url: URL) {
  const a = await app();
  const cookie = req.headers.get("Cookie")?.match(/(?:^|;\s*)mira_gh_state=([^;]+)/)?.[1];
  const state = url.searchParams.get("state") || cookie;
  const code = url.searchParams.get("code");
  if (!state) return text("Installed. Go back to Mira and click Connect GitHub to finish.");
  const { data: s } = await db
    .from("github_install_states")
    .select("user_id, return_to, created_at")
    .eq("state", state)
    .maybeSingle();
  if (!s) return text("This link has expired. Go back to Mira and click Connect GitHub again.", 400);
  await db.from("github_install_states").delete().eq("state", state);
  const clear = { "Set-Cookie": "mira_gh_state=; Path=/functions/v1/github-app; Max-Age=0" };
  const done = (params: Record<string, string>) => redirect(back(s.return_to, params), clear);
  if (Date.now() - Date.parse(s.created_at) > 30 * 60_000) return done({ github_error: "The link expired; try again." });
  if (url.searchParams.get("setup_action") === "request") return done({ github: "requested" });
  if (!code) return done({ github_error: "GitHub didn't send a sign-in code." });

  // Who is this on GitHub? Exchange the code for a user token.
  const tokenRes = await fetch("https://github.com/login/oauth/access_token", {
    method: "POST",
    headers: { Accept: "application/json", "Content-Type": "application/json" },
    body: JSON.stringify({ client_id: a.client_id, client_secret: a.client_secret, code }),
  });
  const tok = await tokenRes.json();
  if (!tok.access_token) return done({ github_error: tok.error_description ?? "GitHub didn't accept the sign-in." });
  const me = await gh("/user", tok.access_token);
  // Installations of this app the GitHub user can see: their own, and
  // organizations they belong to. What they may change is checked per
  // repository later, against their permission there.
  const list = await gh("/user/installations?per_page=100", tok.access_token);
  const rows = (list.installations ?? [])
    .filter((i: any) => i.app_slug === a.slug)
    .map((i: any) => ({
      installation_id: i.id,
      user_id: s.user_id,
      account_login: i.account?.login ?? "",
      account_type: i.account?.type ?? null,
      github_login: me.login,
    }));
  if (rows.length) await db.from("github_installations").upsert(rows);
  return done({ github: "connected" });
}

async function installations(req: Request) {
  const u = await user(req);
  const a = await app();
  const { data: rows } = await db
    .from("github_installations")
    .select("installation_id, account_login, account_type, github_login")
    .eq("user_id", u.id);
  const out = await Promise.all(
    (rows ?? []).map(async (r: any) => {
      try {
        const t = await installationToken(a, r.installation_id);
        const list = await gh("/installation/repositories?per_page=100", t.token);
        const repos = await Promise.all(
          (list.repositories ?? []).map(async (repo: any) => {
            const enabled = await fetch(
              `${GH}/repos/${repo.full_name}/contents/.github/workflows/mira.yml`,
              { method: "HEAD", headers: { Authorization: `Bearer ${t.token}`, "User-Agent": "runmira" } },
            ).then((x) => x.ok).catch(() => false);
            const can_manage = await isAdmin(t.token, repo.full_name, r.github_login);
            return { full_name: repo.full_name, private: repo.private, enabled, can_manage };
          }),
        );
        return { id: r.installation_id, account: r.account_login, account_type: r.account_type, repos };
      } catch (e) {
        // Uninstalled on GitHub since: forget it.
        if (e instanceof HttpError && e.status === 404) {
          await db.from("github_installations").delete().eq("installation_id", r.installation_id);
          return null;
        }
        throw e;
      }
    }),
  );
  return json({
    installations: out.filter(Boolean),
    manage_url: `https://github.com/apps/${a.slug}/installations/new`,
  });
}

/** A one-hour token for setting up one repository. */
async function token(req: Request) {
  const u = await user(req);
  const { installation_id, repo } = await req.json();
  const id = Number(installation_id);
  const name = String(repo ?? "").split("/").pop();
  if (!id || !name) throw new HttpError(400, "installation_id and repo are required.");
  const login = await linked(u.id, id);
  const a = await app();
  // A read-only token limited to that repository tells us its real full
  // name (never trust the owner the caller sent), then only admins get a
  // setup token.
  const probe = await installationToken(a, id, { repositories: [name], permissions: { metadata: "read" } });
  const listed = await gh("/installation/repositories?per_page=1", probe.token);
  const full: string | undefined = listed.repositories?.[0]?.full_name;
  if (!full || !(await isAdmin(probe.token, full, login))) {
    throw new HttpError(403, "You need admin access to this repository to turn Mira on.");
  }
  const t = await installationToken(a, id, {
    repositories: [name],
    permissions: {
      contents: "write",
      pull_requests: "write",
      workflows: "write",
      secrets: "write",
      actions_variables: "write",
      metadata: "read",
    },
  });
  return json({ token: t.token, expires_at: t.expires_at });
}

/** A workflow proves which repository it runs in with its OIDC token. */
async function actionsToken(req: Request) {
  const oidc = req.headers.get("Authorization")?.replace(/^Bearer\s+/i, "");
  if (!oidc) throw new HttpError(401, "Missing the workflow's OIDC token.");
  const { payload } = await jwtVerify(oidc, jwks, { issuer: OIDC_ISSUER, audience: OIDC_AUDIENCE }).catch(() => {
    throw new HttpError(401, "Invalid OIDC token.");
  });
  const full = String(payload.repository ?? "");
  const [owner, name] = full.split("/");
  if (!owner || !name) throw new HttpError(400, "The OIDC token names no repository.");
  const a = await app();
  const inst = await gh(`/repos/${owner}/${name}/installation`, appJwt(a)).catch(() => {
    throw new HttpError(404, `The ${a.slug} app isn't installed on ${full}.`);
  });
  const t = await installationToken(a, inst.id, {
    repositories: [name],
    permissions: { contents: "write", pull_requests: "write", issues: "write", metadata: "read" },
  });
  return json({ token: t.token, expires_at: t.expires_at });
}

Deno.serve(async (req) => {
  if (req.method === "OPTIONS") return new Response("ok", { headers: cors });
  const url = new URL(req.url);
  const route = url.pathname.replace(/^.*?\/github-app/, "") || "/";
  try {
    switch (`${req.method} ${route}`) {
      case "GET /status": return await status();
      case "GET /manifest": return await manifest(url);
      case "GET /manifest/callback": return await manifestCallback(url);
      case "POST /start": return await start(req);
      case "GET /install": return await install(url);
      case "GET /callback": return await callback(req, url);
      case "GET /installations": return await installations(req);
      case "POST /token": return await token(req);
      case "POST /actions-token": return await actionsToken(req);
      case "POST /webhook": return new Response("ignored", { status: 202 });
      default: return json({ error: "not found" }, 404);
    }
  } catch (e) {
    const status = e instanceof HttpError ? e.status : 500;
    const message = e instanceof Error ? e.message : String(e);
    if (status === 500) console.error(e);
    return json({ error: message }, status);
  }
});
