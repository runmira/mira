-- Run after all migrations against a disposable database (CI: the
-- `supabase` job in .github/workflows/ci.yml). Everything rolls back.
begin;

insert into auth.users (id) values ('11300000-0000-0000-0000-000000000001');

do $$
declare
  owner uuid := '11300000-0000-0000-0000-000000000001';
  m1 text := lpad('1', 32, '0');
  m6 text := lpad('6', 32, '0');
  r jsonb;
  claim1 uuid;
  claim_new uuid;
begin
  -- The cap, and a second enable for the same computer while one is in flight.
  for i in 1..5 loop
    r := public.claim_remote_tunnel_slot(owner, lpad(i::text, 32, '0'));
    assert r->>'status' = 'claimed', format('claim %s: %s', i, r);
    if i = 1 then claim1 := (r->>'claim_id')::uuid; end if;
  end loop;
  assert public.claim_remote_tunnel_slot(owner, m1)->>'status' = 'busy';
  assert public.claim_remote_tunnel_slot(owner, m6)->>'status' = 'full';

  -- Activating makes the computer's tunnel; a second enable then reuses it.
  assert public.activate_remote_tunnel(owner, m1, claim1,
    '11300000-0000-0000-0000-0000000000a1', 'm-one.runmira.dev', 'dns-1', 8787);
  assert public.claim_remote_tunnel_slot(owner, m1)->>'status' = 'active';

  -- Releasing with the current claim frees capacity and the tunnel row.
  assert public.release_remote_tunnel_slot(owner, m1, claim1);
  assert not exists (select 1 from public.remote_tunnels where machine_id = m1);
  assert public.claim_remote_tunnel_slot(owner, m6)->>'status' = 'claimed';
end;
$$;

-- A failed enable whose cleanup also failed: its resources were recorded,
-- and once stale, the next enable takes the slot over with the leftovers.
do $$
declare
  owner uuid := '11300000-0000-0000-0000-000000000001';
  m2 text := lpad('2', 32, '0');
  claim_old uuid;
  r jsonb;
begin
  select claim_id into claim_old from public.remote_tunnel_slots where machine_id = m2;
  assert public.record_remote_tunnel_resources(owner, m2, claim_old,
    '11300000-0000-0000-0000-0000000000b2', 'dns-2');

  -- Recent: still someone's in-flight enable.
  assert public.claim_remote_tunnel_slot(owner, m2)->>'status' = 'busy';

  -- Stale: taken over, leftovers handed to the new claim to delete.
  r := public.claim_remote_tunnel_slot(owner, m2, 5, interval '0 seconds');
  assert r->>'status' = 'claimed', r::text;
  assert r->>'leftover_tunnel_id' = '11300000-0000-0000-0000-0000000000b2', r::text;
  assert r->>'leftover_dns_record_id' = 'dns-2', r::text;
  assert (r->>'claim_id')::uuid <> claim_old;

  -- The superseded claim can no longer record, activate, or release.
  assert not public.record_remote_tunnel_resources(owner, m2, claim_old, null, null);
  assert not public.activate_remote_tunnel(owner, m2, claim_old,
    '11300000-0000-0000-0000-0000000000b3', 'm-old.runmira.dev', 'dns-3', 8787);
  assert not public.release_remote_tunnel_slot(owner, m2, claim_old);
  assert exists (select 1 from public.remote_tunnel_slots where machine_id = m2);
  assert not exists (select 1 from public.remote_tunnels where machine_id = m2);
end;
$$;

-- A stale disable (read before a re-enable) can't delete the new tunnel.
do $$
declare
  owner uuid := '11300000-0000-0000-0000-000000000001';
  m3 text := lpad('3', 32, '0');
  first_claim uuid;
  second_claim uuid;
begin
  select claim_id into first_claim from public.remote_tunnel_slots where machine_id = m3;
  assert public.activate_remote_tunnel(owner, m3, first_claim,
    '11300000-0000-0000-0000-0000000000c1', 'm-three.runmira.dev', 'dns-c1', 8787);
  -- Disable #1 finishes, then the computer is enabled again.
  assert public.release_remote_tunnel_slot(owner, m3, first_claim);
  second_claim := (public.claim_remote_tunnel_slot(owner, m3)->>'claim_id')::uuid;
  assert public.activate_remote_tunnel(owner, m3, second_claim,
    '11300000-0000-0000-0000-0000000000c2', 'm-three-b.runmira.dev', 'dns-c2', 8787);
  -- Disable #2 arrives late, holding the first claim: a no-op.
  assert not public.release_remote_tunnel_slot(owner, m3, first_claim);
  assert exists (select 1 from public.remote_tunnels
                 where machine_id = m3 and tunnel_id = '11300000-0000-0000-0000-0000000000c2');
end;
$$;

-- Release is all-or-nothing: if deleting the tunnel row fails, the slot stays.
insert into public.remote_tunnels (user_id, machine_id, tunnel_id, hostname, port)
select user_id, machine_id, '11300000-0000-0000-0000-0000000000d4', 'm-four.runmira.dev', 3000
from public.remote_tunnel_slots where machine_id = lpad('4', 32, '0');
create function pg_temp.reject_delete() returns trigger language plpgsql as $$
begin raise exception 'simulated deletion failure'; end;
$$;
create trigger reject_delete before delete on public.remote_tunnels
for each row execute function pg_temp.reject_delete();
do $$
declare
  owner uuid := '11300000-0000-0000-0000-000000000001';
  m4 text := lpad('4', 32, '0');
  c uuid;
begin
  select claim_id into c from public.remote_tunnel_slots where machine_id = m4;
  begin
    perform public.release_remote_tunnel_slot(owner, m4, c);
    raise exception 'expected deletion failure';
  exception when others then
    if sqlerrm <> 'simulated deletion failure' then raise; end if;
  end;
  assert exists (select 1 from public.remote_tunnels where machine_id = m4);
  assert exists (select 1 from public.remote_tunnel_slots where machine_id = m4);
end;
$$;
drop trigger reject_delete on public.remote_tunnels;

select 'remote_tunnel_slots: all assertions passed' as result;
rollback;
