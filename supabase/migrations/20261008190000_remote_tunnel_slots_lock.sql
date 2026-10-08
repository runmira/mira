-- The slot functions serialized per user with `select … from auth.users
-- for update`, which the service role isn't allowed to do on Supabase
-- ("permission denied for table users"). A transaction-scoped advisory lock
-- keyed on the user gives the same serialization without touching auth.
-- Bodies are otherwise unchanged from 20261008180000_remote_tunnel_slots;
-- the grants carry over with `create or replace`.

create or replace function public.claim_remote_tunnel_slot(
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
  perform pg_advisory_xact_lock(hashtextextended('remote_tunnel_slots:' || owner_id::text, 0));

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

  -- An unknown user fails here, on the foreign key.
  insert into public.remote_tunnel_slots (user_id, machine_id, claim_id)
  values (owner_id, computer_id, new_claim);
  return jsonb_build_object('status', 'claimed', 'claim_id', new_claim);
end;
$$;

create or replace function public.activate_remote_tunnel(
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
  perform pg_advisory_xact_lock(hashtextextended('remote_tunnel_slots:' || owner_id::text, 0));
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

create or replace function public.release_remote_tunnel_slot(owner_id uuid, computer_id text, claim uuid)
returns boolean
language plpgsql
set search_path = ''
as $$
begin
  perform pg_advisory_xact_lock(hashtextextended('remote_tunnel_slots:' || owner_id::text, 0));
  delete from public.remote_tunnel_slots
  where user_id = owner_id and machine_id = computer_id and claim_id = claim;
  if not found then
    return false;
  end if;
  delete from public.remote_tunnels where user_id = owner_id and machine_id = computer_id;
  return true;
end;
$$;
