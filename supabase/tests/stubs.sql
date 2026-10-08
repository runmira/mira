-- The pieces of a Supabase database the migrations rely on, for running
-- them on plain Postgres in CI (and locally). Not a migration.
create schema if not exists auth;
create table if not exists auth.users (id uuid primary key);
create or replace function auth.uid() returns uuid language sql stable as 'select null::uuid';
do $$
begin
  if not exists (select 1 from pg_roles where rolname = 'anon') then create role anon; end if;
  if not exists (select 1 from pg_roles where rolname = 'authenticated') then create role authenticated; end if;
  if not exists (select 1 from pg_roles where rolname = 'service_role') then create role service_role bypassrls; end if;
end;
$$;
-- Supabase's grants: API roles may use public tables (RLS decides), and
-- get nothing on auth.users. Tests run as service_role to match the edge
-- functions, so a migration that needs more fails here, not in production.
grant usage on schema public to anon, authenticated, service_role;
alter default privileges in schema public grant all on tables to anon, authenticated, service_role;
alter default privileges in schema public grant all on sequences to anon, authenticated, service_role;
