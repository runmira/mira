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
now_ms() { python3 -c 'import time; print(int(time.time() * 1000))'; }
cleanup() { q -c "delete from auth.users where id = '$owner'"; }

q -c "insert into auth.users (id) values ('$owner')"
trap cleanup EXIT

# Four of five slots taken.
for i in 1 2 3 4; do
  q -c "set role service_role; select public.claim_remote_tunnel_slot('$owner', lpad('$i', 32, 'c'))" >/dev/null
done

# Session A claims the fifth and holds its transaction open.
q -c "begin;
      set local role service_role;
      select public.claim_remote_tunnel_slot('$owner', lpad('5', 32, 'c'));
      select pg_sleep(2);
      commit;" >/dev/null &
a=$!
sleep 0.5

# Session B wants a sixth: it must wait for A, then find the user full.
start=$(now_ms)
b=$(q -c "set role service_role; select public.claim_remote_tunnel_slot('$owner', lpad('6', 32, 'c'))->>'status'")
waited_ms=$(( $(now_ms) - start ))
wait "$a"

slots=$(q -c "select count(*) from public.remote_tunnel_slots where user_id = '$owner'")
echo "second claim: $b after ${waited_ms}ms; slots: $slots"
[ "$b" = full ] || { echo "FAIL: expected the second claim to find the user full"; exit 1; }
[ "$slots" = 5 ] || { echo "FAIL: expected 5 slots"; exit 1; }
[ "$waited_ms" -ge 1000 ] || { echo "FAIL: the second claim didn't wait for the first"; exit 1; }
echo "remote_tunnel_slots concurrency: passed"
