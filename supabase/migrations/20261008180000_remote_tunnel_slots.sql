-- Remote access capacity: a reservation per (user, computer), taken before
-- any Cloudflare resources are made, so concurrent enables can't exceed the
-- per-user cap. Only the remote-access edge function (service role) uses
-- these.
--
-- Each reservation carries a claim_id, and records the Cloudflare resources
-- made under it as they're made. That makes failures recoverable instead of
-- permanent:
-- - An enable that fails, and whose cleanup also fails, leaves its tunnel
--   and DNS ids here. Once the reservation is older than `stale_after`
--   without becoming active, the next enable takes it over and deletes the
--   leftovers first.
-- - Activating and releasing check the caller's claim_id, so a slow request
--   from before a re-enable can't activate over, or delete, the new tunnel.

create table public.remote_tunnel_slots (
  user_id uuid not null references auth.users (id) on delete cascade,
  machine_id text not null check (machine_id ~ '^[0-9a-f]{32}$'),
  claim_id uuid not null default gen_random_uuid(),
  -- Cloudflare resources made under this claim so far.
  tunnel_id uuid,
  dns_record_id text,
  claimed_at timestamptz not null default now(),
  primary key (user_id, machine_id)
);

alter table public.remote_tunnel_slots enable row level security;
revoke all on public.remote_tunnel_slots from anon, authenticated;

-- Tunnels made before reservations existed hold a slot too.
insert into public.remote_tunnel_slots (user_id, machine_id, tunnel_id, dns_record_id, claimed_at)
select user_id, machine_id, tunnel_id, dns_record_id, created_at
from public.remote_tunnels;

-- Reserve a slot. Returns { status } where status is:
--   claimed  { claim_id, leftover_tunnel_id?, leftover_dns_record_id? } —
--            go ahead; delete any leftovers first
--   active   this computer already has a tunnel
--   busy     another enable for this computer is in flight
--   full     the user is at the cap
create function public.claim_remote_tunnel_slot(
  owner_id uuid,
  computer_id text,
  max_slots int default 5,
  stale_after interval default interval '2 minutes'
)
returns jsonb
language plpgsql
set search_path = ''
as $$
declare
  s public.remote_tunnel_slots;
  new_claim uuid := gen_random_uuid();
begin
  -- Serialize claims for one user across machines and function workers.
  perform 1 from auth.users where id = owner_id for update;
  if not found then
    raise exception 'unknown user';
  end if;

  select * into s from public.remote_tunnel_slots
  where user_id = owner_id and machine_id = computer_id;

  if found then
    if exists (select 1 from public.remote_tunnels
               where user_id = owner_id and machine_id = computer_id) then
      return jsonb_build_object('status', 'active');
    end if;
    if s.claimed_at > now() - stale_after then
      return jsonb_build_object('status', 'busy');
    end if;
    -- A failed or abandoned enable: take it over. Its recorded resources
    -- stay recorded until the new claim records its own, so a takeover that
    -- dies before deleting them doesn't lose track of them.
    update public.remote_tunnel_slots
    set claim_id = new_claim, claimed_at = now()
    where user_id = owner_id and machine_id = computer_id;
    return jsonb_build_object(
      'status', 'claimed',
      'claim_id', new_claim,
      'leftover_tunnel_id', s.tunnel_id,
      'leftover_dns_record_id', s.dns_record_id
    );
  end if;

  if (select count(*) from public.remote_tunnel_slots where user_id = owner_id) >= max_slots then
    return jsonb_build_object('status', 'full');
  end if;

  insert into public.remote_tunnel_slots (user_id, machine_id, claim_id)
  values (owner_id, computer_id, new_claim);
  return jsonb_build_object('status', 'claimed', 'claim_id', new_claim);
end;
$$;

-- Note the Cloudflare resources made so far under a claim. False if the
-- claim was taken over or released meanwhile.
create function public.record_remote_tunnel_resources(
  owner_id uuid,
  computer_id text,
  claim uuid,
  tunnel uuid,
  dns_record text
)
returns boolean
language plpgsql
set search_path = ''
as $$
begin
  update public.remote_tunnel_slots
  set tunnel_id = tunnel, dns_record_id = dns_record
  where user_id = owner_id and machine_id = computer_id and claim_id = claim;
  return found;
end;
$$;

-- Finish an enable: the tunnel becomes this computer's, atomically with
-- checking the claim is still ours. False if it isn't (the caller then
-- removes what it made).
create function public.activate_remote_tunnel(
  owner_id uuid,
  computer_id text,
  claim uuid,
  tunnel uuid,
  host text,
  dns_record text,
  local_port int
)
returns boolean
language plpgsql
set search_path = ''
as $$
begin
  perform 1 from auth.users where id = owner_id for update;
  update public.remote_tunnel_slots
  set tunnel_id = tunnel, dns_record_id = dns_record
  where user_id = owner_id and machine_id = computer_id and claim_id = claim;
  if not found then
    return false;
  end if;
  insert into public.remote_tunnels (user_id, machine_id, tunnel_id, hostname, dns_record_id, port)
  values (owner_id, computer_id, tunnel, host, dns_record, local_port);
  return true;
end;
$$;

-- Give a slot back, with its tunnel row, in one transaction — but only if
-- the caller's claim is still current. False (and nothing deleted) for a
-- stale claim, so an old disable can't remove a newer tunnel.
create function public.release_remote_tunnel_slot(owner_id uuid, computer_id text, claim uuid)
returns boolean
language plpgsql
set search_path = ''
as $$
begin
  perform 1 from auth.users where id = owner_id for update;
  delete from public.remote_tunnel_slots
  where user_id = owner_id and machine_id = computer_id and claim_id = claim;
  if not found then
    return false;
  end if;
  delete from public.remote_tunnels where user_id = owner_id and machine_id = computer_id;
  return true;
end;
$$;

revoke all on function public.claim_remote_tunnel_slot(uuid, text, int, interval) from public, anon, authenticated;
revoke all on function public.record_remote_tunnel_resources(uuid, text, uuid, uuid, text) from public, anon, authenticated;
revoke all on function public.activate_remote_tunnel(uuid, text, uuid, uuid, text, text, int) from public, anon, authenticated;
revoke all on function public.release_remote_tunnel_slot(uuid, text, uuid) from public, anon, authenticated;
grant execute on function public.claim_remote_tunnel_slot(uuid, text, int, interval) to service_role;
grant execute on function public.record_remote_tunnel_resources(uuid, text, uuid, uuid, text) to service_role;
grant execute on function public.activate_remote_tunnel(uuid, text, uuid, uuid, text, text, int) to service_role;
grant execute on function public.release_remote_tunnel_slot(uuid, text, uuid) to service_role;
