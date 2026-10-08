// Remote access: give a user's computer a stable public address, so their
// phone can reach that computer's Mira without any networking setup.
//
//   POST /enable   (user)  { machine_id, port } → { hostname, token }
//   POST /disable  (user)  { machine_id }       → { ok }
//
// Each (user, computer) gets one Cloudflare Tunnel and a hostname
// https://m-<random>.runmira.dev. The computer's Mira runs Cloudflare's
// connector with the returned token; the connector dials out to Cloudflare,
// so nothing is opened on the user's network. Cloudflare forwards requests
// to that computer's loopback port, where Mira's device pairing decides who
// gets in (crates/mira-server/src/pairing.rs) — a hostname alone grants
// nothing.
//
// The Cloudflare API token (CLOUDFLARE_API_TOKEN secret) never leaves this
// function. "(user)" routes take the caller's Supabase session as a Bearer
// token, as in github-app; the gateway's JWT check is off and each route
// checks the caller itself.

import { createClient } from "npm:@supabase/supabase-js@2";

const SUPABASE_URL = Deno.env.get("SUPABASE_URL")!;
const CF = "https://api.cloudflare.com/client/v4";
const CF_TOKEN = Deno.env.get("CLOUDFLARE_API_TOKEN") ?? "";
// Not secret: they identify the account and the runmira.dev zone.
const ACCOUNT = Deno.env.get("CLOUDFLARE_ACCOUNT_ID") ?? "bb8f0e46ed03b3ea2bcdc4dab563269c";
const ZONE = Deno.env.get("CLOUDFLARE_ZONE_ID") ?? "151e3c6ff614dd06d81155a58f3b01ba";
const DOMAIN = "runmira.dev";
/** Computers one account may expose at once. */
const MAX_MACHINES = 5;

const db = createClient(SUPABASE_URL, Deno.env.get("SUPABASE_SERVICE_ROLE_KEY")!, {
  auth: { persistSession: false },
});

const cors = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Headers": "authorization, x-client-info, apikey, content-type",
  "Access-Control-Allow-Methods": "POST, OPTIONS",
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

async function user(req: Request) {
  const jwt = req.headers.get("Authorization")?.replace(/^Bearer\s+/i, "");
  if (!jwt) throw new HttpError(401, "Sign in to Mira first.");
  const { data, error } = await db.auth.getUser(jwt);
  if (error || !data.user) throw new HttpError(401, "Your Mira session has expired; sign in again.");
  return data.user;
}

/** A Cloudflare API call; throws a readable error on failure. */
async function cf(path: string, init: RequestInit = {}) {
  if (!CF_TOKEN) throw new HttpError(503, "Remote access isn't set up on the server yet.");
  const res = await fetch(`${CF}${path}`, {
    ...init,
    headers: {
      Authorization: `Bearer ${CF_TOKEN}`,
      ...(init.body ? { "Content-Type": "application/json" } : {}),
    },
  });
  const body = await res.json().catch(() => null);
  if (!res.ok || body?.success === false) {
    const msg = body?.errors?.map((e: { message: string }) => e.message).join("; ") || res.status;
    console.error(`cloudflare ${init.method ?? "GET"} ${path}: ${msg}`);
    throw new HttpError(502, `Cloudflare refused the request (${msg}).`);
  }
  return body?.result;
}

function machineId(v: unknown): string {
  const id = String(v ?? "");
  if (!/^[0-9a-f]{32}$/.test(id)) throw new HttpError(400, "machine_id must be 32 hex characters.");
  return id;
}

function port(v: unknown): number {
  const p = Number(v);
  if (!Number.isInteger(p) || p < 1 || p > 65535) throw new HttpError(400, "port must be 1–65535.");
  return p;
}

/** Lowercase letters and digits, unguessable: the hostname isn't a secret
 *  (pairing is), but it shouldn't be enumerable either. */
function randomLabel(len = 12): string {
  const alphabet = "abcdefghijkmnpqrstuvwxyz23456789";
  const bytes = crypto.getRandomValues(new Uint8Array(len));
  return Array.from(bytes, (b) => alphabet[b % alphabet.length]).join("");
}

/** Point the tunnel at the computer's Mira; anything else is a 404. */
async function configure(tunnelId: string, hostname: string, p: number) {
  await cf(`/accounts/${ACCOUNT}/cfd_tunnel/${tunnelId}/configurations`, {
    method: "PUT",
    body: JSON.stringify({
      config: {
        ingress: [
          { hostname, service: `http://127.0.0.1:${p}` },
          { service: "http_status:404" },
        ],
      },
    }),
  });
}

async function connectorToken(tunnelId: string): Promise<string> {
  return await cf(`/accounts/${ACCOUNT}/cfd_tunnel/${tunnelId}/token`);
}

async function enable(req: Request) {
  const u = await user(req);
  const body = await req.json().catch(() => ({}));
  const machine = machineId(body.machine_id);
  const p = port(body.port);

  const { data: existing } = await db
    .from("remote_tunnels")
    .select("tunnel_id, hostname, port")
    .eq("user_id", u.id)
    .eq("machine_id", machine)
    .maybeSingle();

  if (existing) {
    if (existing.port !== p) {
      await configure(existing.tunnel_id, existing.hostname, p);
      await db
        .from("remote_tunnels")
        .update({ port: p, updated_at: new Date().toISOString() })
        .eq("user_id", u.id)
        .eq("machine_id", machine);
    }
    return json({ hostname: existing.hostname, token: await connectorToken(existing.tunnel_id) });
  }

  const { count } = await db
    .from("remote_tunnels")
    .select("machine_id", { count: "exact", head: true })
    .eq("user_id", u.id);
  if ((count ?? 0) >= MAX_MACHINES) {
    throw new HttpError(409, `Remote access is on for ${MAX_MACHINES} computers already. Turn it off on one first.`);
  }

  const label = `m-${randomLabel()}`;
  const hostname = `${label}.${DOMAIN}`;
  const tunnel = await cf(`/accounts/${ACCOUNT}/cfd_tunnel`, {
    method: "POST",
    body: JSON.stringify({ name: `mira-${label}`, config_src: "cloudflare" }),
  });
  try {
    await configure(tunnel.id, hostname, p);
    const record = await cf(`/zones/${ZONE}/dns_records`, {
      method: "POST",
      body: JSON.stringify({
        type: "CNAME",
        name: hostname,
        content: `${tunnel.id}.cfargotunnel.com`,
        proxied: true,
        ttl: 1,
        comment: "Mira remote access",
      }),
    });
    const { error } = await db.from("remote_tunnels").insert({
      user_id: u.id,
      machine_id: machine,
      tunnel_id: tunnel.id,
      hostname,
      dns_record_id: record.id,
      port: p,
    });
    if (error) {
      await cf(`/zones/${ZONE}/dns_records/${record.id}`, { method: "DELETE" }).catch(() => {});
      throw new HttpError(500, `Couldn't save the tunnel: ${error.message}`);
    }
  } catch (e) {
    // Don't leave a half-made tunnel behind.
    await cf(`/accounts/${ACCOUNT}/cfd_tunnel/${tunnel.id}`, { method: "DELETE" }).catch(() => {});
    throw e;
  }
  return json({ hostname, token: await connectorToken(tunnel.id) });
}

async function disable(req: Request) {
  const u = await user(req);
  const body = await req.json().catch(() => ({}));
  const machine = machineId(body.machine_id);
  const { data: row } = await db
    .from("remote_tunnels")
    .select("tunnel_id, dns_record_id")
    .eq("user_id", u.id)
    .eq("machine_id", machine)
    .maybeSingle();
  if (!row) return json({ ok: true });

  if (row.dns_record_id) {
    await cf(`/zones/${ZONE}/dns_records/${row.dns_record_id}`, { method: "DELETE" });
  }
  }
  // A tunnel with live connections can't be deleted; drop them first.
  await cf(`/accounts/${ACCOUNT}/cfd_tunnel/${row.tunnel_id}/connections`, { method: "DELETE" }).catch(() => {});
  await cf(`/accounts/${ACCOUNT}/cfd_tunnel/${row.tunnel_id}`, { method: "DELETE" });
  await db.from("remote_tunnels").delete().eq("user_id", u.id).eq("machine_id", machine);
  return json({ ok: true });
}

Deno.serve(async (req) => {
  if (req.method === "OPTIONS") return new Response("ok", { headers: cors });
  const url = new URL(req.url);
  const route = url.pathname.replace(/^.*?\/remote-access/, "") || "/";
  try {
    switch (`${req.method} ${route}`) {
      case "POST /enable": return await enable(req);
      case "POST /disable": return await disable(req);
      default: return json({ error: "not found" }, 404);
    }
  } catch (e) {
    const status = e instanceof HttpError ? e.status : 500;
    const message = e instanceof Error ? e.message : String(e);
    if (status === 500) console.error(e);
    return json({ error: message }, status);
  }
});
