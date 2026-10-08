-- Reserve capacity before calling Cloudflare, including in-flight enables.
create table public.remote_tunnel_slots (
  user_id uuid not null references auth.users(id) on delete cascade,
  machine_id text not null check (machine_id ~ '^[0-9a-f]{32}$'),
  primary key (user_id, machine_id)
);
alter table public.remote_tunnel_slots enable row level security;
revoke all on public.remote_tunnel_slots from anon, authenticated;
insert into public.remote_tunnel_slots (user_id, machine_id)
select user_id, machine_id from public.remote_tunnels;

create function public.claim_remote_tunnel_slot(owner_id uuid, computer_id text)
returns text language plpgsql set search_path = '' as $$
begin
  -- Serialize claims across machines and Edge Function workers.
  perform id from auth.users where id = owner_id for update;
  if not found then raise exception 'Unknown user'; end if;
  if exists (select 1 from public.remote_tunnel_slots
             where user_id = owner_id and machine_id = computer_id) then
    return 'busy';
  end if;
  if (select count(*) from public.remote_tunnel_slots where user_id = owner_id) >= 5 then
    return 'full';
  end if;
  insert into public.remote_tunnel_slots values (owner_id, computer_id);
  return 'claimed';
end;
$$;

-- Delete the resource row and release capacity in one transaction. Failure
-- preserves both rows for a retry after Cloudflare cleanup.
create function public.release_remote_tunnel_slot(owner_id uuid, computer_id text)
returns void language plpgsql set search_path = '' as $$
begin
  perform id from auth.users where id = owner_id for update;
  delete from public.remote_tunnels where user_id = owner_id and machine_id = computer_id;
  delete from public.remote_tunnel_slots where user_id = owner_id and machine_id = computer_id;
end;
$$;
revoke all on function public.claim_remote_tunnel_slot(uuid, text) from public, anon, authenticated;
revoke all on function public.release_remote_tunnel_slot(uuid, text) from public, anon, authenticated;
grant execute on function public.claim_remote_tunnel_slot(uuid, text) to service_role;
grant execute on function public.release_remote_tunnel_slot(uuid, text) to service_role;
