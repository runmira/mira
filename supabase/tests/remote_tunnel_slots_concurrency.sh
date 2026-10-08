#!/usr/bin/env bash
# Two sessions racing for a user's last remote-access slot. The SQL tests
# run in one transaction, so they can't tell whether claims are serialized;
# this one can. Without the per-user lock in claim_remote_tunnel_slot, the
# second claim doesn't see the first's uncommitted slot and goes over the
# cap.
#
# Needs a database with the stubs and migrations applied (CI: the
# `supabase` job), and the usual PG* environment variables.
set -euo pipefail

owner=11300000-0000-0000-0000-0000000000c0
q() { psql -v ON_ERROR_STOP=1 -qAt "$@"; }
a= b=
cleanup() {
  # End the racing sessions first: an open transaction would hold up the
  # delete below.
  exec 3>&- 2>/dev/null || true
  for pid in $a $b; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  q -c "delete from auth.users where id = '$owner'" >/dev/null || true
  rm -rf "$tmp"
}
tmp=$(mktemp -d)
trap cleanup EXIT

# Wait (up to 10s) until a query about the sessions returns true. The test
# steps on what Postgres reports, not on sleeps, so a slow runner can't
# reorder it.
until_true() {
  for _ in $(seq 100); do
    [ "$(q -c "$2")" = t ] && return 0
    sleep 0.1
  done
  echo "FAIL: timed out waiting until $1"
  exit 1
}
# Whether session $1 holds (granted=true) or waits for (false) an advisory lock.
advisory() {
  echo "select exists (select 1 from pg_locks l join pg_stat_activity a using (pid)
        where a.application_name = '$1' and l.locktype = 'advisory' and l.granted = $2)"
}

q -c "insert into auth.users (id) values ('$owner')"

# Four of five slots taken.
for i in 1 2 3 4; do
  q -c "set role service_role; select public.claim_remote_tunnel_slot('$owner', lpad('$i', 32, 'c'))" >/dev/null
done

# Session A claims the fifth, and keeps its transaction open until told.
mkfifo "$tmp/a"
PGAPPNAME=slot-a psql -v ON_ERROR_STOP=1 -qAt <"$tmp/a" >/dev/null &
a=$!
exec 3>"$tmp/a"
echo "begin;
      set local role service_role;
      select public.claim_remote_tunnel_slot('$owner', lpad('5', 32, 'c'));" >&3
until_true "session A holds the user's lock" "$(advisory slot-a true)"

# Session B wants a sixth. It must block on A's lock...
PGAPPNAME=slot-b q -c "set role service_role; select public.claim_remote_tunnel_slot('$owner', lpad('6', 32, 'c'))->>'status'" >"$tmp/b" &
b=$!
until_true "session B waits for that lock" "$(advisory slot-b false)"

# ...and once A commits, find the user full.
echo "commit;" >&3
exec 3>&-
wait "$a"
wait "$b"

result=$(cat "$tmp/b")
slots=$(q -c "select count(*) from public.remote_tunnel_slots where user_id = '$owner'")
echo "second claim: $result; slots: $slots"
[ "$result" = full ] || { echo "FAIL: expected the second claim to find the user full"; exit 1; }
[ "$slots" = 5 ] || { echo "FAIL: expected 5 slots"; exit 1; }
echo "remote_tunnel_slots concurrency: passed"
