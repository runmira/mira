-- The Runmira GitHub App: its credentials, and which Mira users can use
-- which installations. Only the github-app edge function (service role)
-- touches these, except that users can read their own installations.

-- One row: the app, filled in by the manifest flow.
create table public.github_app (
  id int primary key default 1 check (id = 1),
  app_id bigint not null,
  slug text not null,
  client_id text not null,
  client_secret text not null,
  private_key text not null,
  webhook_secret text,
  created_at timestamptz not null default now()
);

-- One-time keys that allow creating the app (so only the owner can).
create table public.github_app_setup (
  key text primary key,
  created_at timestamptz not null default now()
);

-- A Mira user starting an install; matched when GitHub sends them back.
create table public.github_install_states (
  state text primary key,
  user_id uuid not null references auth.users (id) on delete cascade,
  created_at timestamptz not null default now()
);

-- Installations a Mira user can manage.
create table public.github_installations (
  installation_id bigint not null,
  user_id uuid not null references auth.users (id) on delete cascade,
  account_login text not null,
  account_type text,
  created_at timestamptz not null default now(),
  primary key (installation_id, user_id)
);

alter table public.github_app enable row level security;
alter table public.github_app_setup enable row level security;
alter table public.github_install_states enable row level security;
alter table public.github_installations enable row level security;

revoke all on public.github_app from anon, authenticated;
revoke all on public.github_app_setup from anon, authenticated;
revoke all on public.github_install_states from anon, authenticated;
revoke insert, update, delete on public.github_installations from anon, authenticated;

create policy "Users read their own installations"
  on public.github_installations for select
  to authenticated
  using (user_id = (select auth.uid()));
