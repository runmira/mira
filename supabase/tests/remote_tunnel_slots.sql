-- Run after migrations against a disposable database.
begin;
insert into auth.users (id) values ('11300000-0000-0000-0000-000000000001');
do $$
declare owner uuid := '11300000-0000-0000-0000-000000000001';
begin
  for i in 1..5 loop
    assert public.claim_remote_tunnel_slot(owner, lpad(i::text, 32, '0')) = 'claimed';
  end loop;
  assert public.claim_remote_tunnel_slot(owner, lpad('1', 32, '0')) = 'busy';
  assert public.claim_remote_tunnel_slot(owner, lpad('6', 32, '0')) = 'full';
  perform public.release_remote_tunnel_slot(owner, lpad('1', 32, '0'));
  assert public.claim_remote_tunnel_slot(owner, lpad('6', 32, '0')) = 'claimed';
end;
$$;
insert into public.remote_tunnels (user_id, machine_id, tunnel_id, hostname, port)
values ('11300000-0000-0000-0000-000000000001', lpad('2',32,'0'),
        '11300000-0000-0000-0000-000000000002', 'm-test.runmira.dev', 3000);
create function pg_temp.reject_delete() returns trigger language plpgsql as $$
begin raise exception 'simulated deletion failure'; end;
$$;
create trigger reject_delete before delete on public.remote_tunnels
for each row execute function pg_temp.reject_delete();
do $$
begin
  begin
    perform public.release_remote_tunnel_slot('11300000-0000-0000-0000-000000000001', lpad('2',32,'0'));
    raise exception 'expected deletion failure';
  exception when others then
    if sqlerrm <> 'simulated deletion failure' then raise; end if;
  end;
  assert exists (select 1 from public.remote_tunnels where machine_id = lpad('2',32,'0'));
  assert exists (select 1 from public.remote_tunnel_slots where machine_id = lpad('2',32,'0'));
end;
$$;
drop trigger reject_delete on public.remote_tunnels;
select public.release_remote_tunnel_slot('11300000-0000-0000-0000-000000000001', lpad('2',32,'0'));
do $$
begin
  assert not exists (select 1 from public.remote_tunnels where machine_id = lpad('2',32,'0'));
  assert not exists (select 1 from public.remote_tunnel_slots where machine_id = lpad('2',32,'0'));
end;
$$;
rollback;
