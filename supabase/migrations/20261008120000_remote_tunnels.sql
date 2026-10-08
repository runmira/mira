-- Remote access: one Cloudflare tunnel per (Mira user, computer), so a
-- phone can reach that computer's Mira at a stable https://m-….runmira.dev
-- without the user setting up any networking. Only the remote-access edge
-- function (service role) touches this table; see
-- supabase/functions/remote-access.

create table public.remote_tunnels (
  user_id uuid not null references auth.users (id) on delete cascade,
  -- Random id a Mira install generates for itself (~/.mira/machine-id).
  machine_id text not null check (machine_id ~ '^[0-9a-f]{32}$'),
  tunnel_id uuid not null unique,
  hostname text not null unique,
  dns_record_id text,
  -- The local port Mira listens on; the tunnel forwards there.
  port int not null check (port between 1 and 65535),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  primary key (user_id, machine_id)
);

alter table public.remote_tunnels enable row level security;
revoke all on public.remote_tunnels from anon, authenticated;
