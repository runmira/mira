// Tests for supabase/functions/remote-access against a fake database and a
// fake Cloudflare API. Run from the repository root, after `npm ci` in the
// frontend (for TypeScript):
//
//   node supabase/tests/remote-access.cjs
//
// The SQL side (claims, takeovers, releases) is tested for real in
// remote_tunnel_slots.sql; here the database is a stub that records calls.
const ts = require(process.cwd() + '/crates/mira-server/frontend/node_modules/typescript');
const fs = require('fs');
const vm = require('vm');
const assert = require('assert/strict');

const source = ts.transpileModule(
  fs.readFileSync('supabase/functions/remote-access/index.ts', 'utf8').replace(/import .*createClient.*;\n/, ''),
  { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } },
).outputText;

const MACHINE = 'a'.repeat(32);

/**
 * One function instance. `tables` seeds what selects return; `rpcs` maps an
 * RPC name to its result (a value, or a function of the args); `cfFail`
 * maps "METHOD /path-substring" to an HTTP status to fail with.
 */
function harness({ tables = {}, rpcs = {}, cfFail = {} } = {}) {
  const calls = { rpc: [], cf: [], updates: [] };
  const query = (table) => {
    const q = {
      select() { return q; },
      eq() { return q; },
      update(v) { calls.updates.push([table, v]); return q; },
      async maybeSingle() { return { data: tables[table] ?? null, error: null }; },
      then(resolve) { resolve({ error: null }); },
    };
    return q;
  };
  const db = {
    auth: { getUser: async () => ({ data: { user: { id: 'user-1' } } }) },
    from: query,
    rpc: async (name, args) => {
      calls.rpc.push([name, args]);
      const r = rpcs[name];
      return { data: typeof r === 'function' ? r(args) : r ?? true, error: null };
    },
  };
  let n = 0;
  const fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    const path = url.replace('https://api.cloudflare.com/client/v4', '');
    calls.cf.push(`${method} ${path}`);
    for (const [key, status] of Object.entries(cfFail)) {
      const [m, frag] = key.split(' ');
      if (m === method && path.includes(frag)) {
        return new Response(JSON.stringify({ success: false, errors: [{ message: `boom ${status}` }] }), { status });
      }
    }
    let result = {};
    if (method === 'POST' && path.endsWith('/cfd_tunnel')) result = { id: `tunnel-${++n}` };
    else if (method === 'POST' && path.includes('/dns_records')) result = { id: `dns-${++n}` };
    else if (path.endsWith('/token')) result = 'connector-token';
    return new Response(JSON.stringify({ success: true, result }), { status: 200 });
  };
  let handler;
  vm.runInNewContext(source, {
    createClient: () => db,
    Deno: { env: { get: (k) => (k === 'CLOUDFLARE_API_TOKEN' ? 'cf-token' : 'test') }, serve: (f) => (handler = f) },
    fetch, Response, URL, console: { ...console, error() {} }, crypto,
  });
  const call = async (route, body) => {
    const res = await handler(new Request(`http://localhost/remote-access/${route}`, {
      method: 'POST',
      headers: { Authorization: 'Bearer session' },
      body: JSON.stringify(body),
    }));
    return { status: res.status, body: await res.json() };
  };
  return { call, calls };
}

const rpcNames = (calls) => calls.rpc.map(([name]) => name);

async function main() {
  // A new computer: claim, create, record as it goes, activate.
  {
    const h = harness({ rpcs: { claim_remote_tunnel_slot: { status: 'claimed', claim_id: 'c1' } } });
    const r = await h.call('enable', { machine_id: MACHINE, port: 8787 });
    assert.equal(r.status, 200, JSON.stringify(r.body));
    assert.match(r.body.hostname, /^m-[a-z0-9]{12}\.runmira\.dev$/);
    assert.equal(r.body.token, 'connector-token');
    assert.deepEqual(rpcNames(h.calls), [
      'claim_remote_tunnel_slot',
      'record_remote_tunnel_resources',
      'record_remote_tunnel_resources',
      'activate_remote_tunnel',
    ]);
    const activate = h.calls.rpc.at(-1)[1];
    assert.equal(activate.claim, 'c1');
    assert.equal(activate.local_port, 8787);
    console.log('enable: claims, records resources as made, activates with its claim');
  }

  // An existing tunnel: reused, and re-pointed when Mira's port changed.
  {
    const h = harness({ tables: { remote_tunnels: { tunnel_id: 't9', hostname: 'm-x.runmira.dev', port: 8787 } } });
    const r = await h.call('enable', { machine_id: MACHINE, port: 8790 });
    assert.equal(r.status, 200);
    assert.equal(r.body.hostname, 'm-x.runmira.dev');
    assert(h.calls.cf.includes('PUT /accounts/test/cfd_tunnel/t9/configurations'));
    assert.deepEqual(h.calls.updates[0], ['remote_tunnels', h.calls.updates[0][1]]);
    assert.equal(h.calls.updates[0][1].port, 8790);
    assert.deepEqual(rpcNames(h.calls), []);
    console.log('enable: reuses the tunnel and follows a new local port');
  }

  // Setup fails and so does cleanup: the slot (with recorded ids) is kept,
  // not released — the next enable recovers it once stale.
  {
    const h = harness({
      rpcs: { claim_remote_tunnel_slot: { status: 'claimed', claim_id: 'c1' } },
      cfFail: { 'POST /dns_records': 502, 'DELETE /cfd_tunnel/': 502 },
    });
    const r = await h.call('enable', { machine_id: MACHINE, port: 8787 });
    assert.equal(r.status, 502);
    const recorded = h.calls.rpc.find(([n]) => n === 'record_remote_tunnel_resources')[1];
    assert.equal(recorded.tunnel, 'tunnel-1');
    assert(!rpcNames(h.calls).includes('release_remote_tunnel_slot'));
    console.log('enable: failed setup with failed cleanup keeps the recorded slot');
  }

  // Setup fails, cleanup works: resources removed, slot released.
  {
    const h = harness({
      rpcs: { claim_remote_tunnel_slot: { status: 'claimed', claim_id: 'c1' } },
      cfFail: { 'POST /dns_records': 502 },
    });
    const r = await h.call('enable', { machine_id: MACHINE, port: 8787 });
    assert.equal(r.status, 502);
    assert(h.calls.cf.includes('DELETE /accounts/test/cfd_tunnel/tunnel-1'));
    const release = h.calls.rpc.find(([n]) => n === 'release_remote_tunnel_slot');
    assert.equal(release[1].claim, 'c1');
    console.log('enable: failed setup cleans up and releases its own claim');
  }

  // Taking over a stale slot: leftovers deleted before anything new is made.
  {
    const h = harness({
      rpcs: {
        claim_remote_tunnel_slot: {
          status: 'claimed', claim_id: 'c2', leftover_tunnel_id: 'old-t', leftover_dns_record_id: 'old-d',
        },
      },
    });
    const r = await h.call('enable', { machine_id: MACHINE, port: 8787 });
    assert.equal(r.status, 200);
    const firstCreate = h.calls.cf.findIndex((c) => c === 'POST /accounts/test/cfd_tunnel');
    assert(h.calls.cf.indexOf('DELETE /zones/test/dns_records/old-d') < firstCreate);
    assert(h.calls.cf.indexOf('DELETE /accounts/test/cfd_tunnel/old-t') < firstCreate);
    console.log('enable: takeover removes leftovers first');
  }

  // Leftovers that can't be removed: nothing new is made.
  {
    const h = harness({
      rpcs: { claim_remote_tunnel_slot: { status: 'claimed', claim_id: 'c2', leftover_tunnel_id: 'old-t' } },
      cfFail: { 'DELETE /cfd_tunnel/old-t': 502 },
    });
    const r = await h.call('enable', { machine_id: MACHINE, port: 8787 });
    assert.equal(r.status, 502);
    assert(!h.calls.cf.includes('POST /accounts/test/cfd_tunnel'));
    console.log('enable: unremovable leftovers stop the enable');
  }

  // The claim was taken over mid-enable: undo our resources, report 409.
  {
    const h = harness({
      rpcs: { claim_remote_tunnel_slot: { status: 'claimed', claim_id: 'c1' }, activate_remote_tunnel: false },
    });
    const r = await h.call('enable', { machine_id: MACHINE, port: 8787 });
    assert.equal(r.status, 409);
    assert(h.calls.cf.includes('DELETE /accounts/test/cfd_tunnel/tunnel-1'));
    console.log('enable: a lost claim undoes its own resources');
  }

  // Busy and full.
  for (const [status, re] of [['busy', /already being turned on/], ['full', /5 computers/]]) {
    const h = harness({ rpcs: { claim_remote_tunnel_slot: { status } } });
    const r = await h.call('enable', { machine_id: MACHINE, port: 8787 });
    assert.equal(r.status, 409);
    assert.match(r.body.error, re);
    assert.equal(h.calls.cf.filter((c) => c.startsWith('POST')).length, 0);
  }
  console.log('enable: busy and full make nothing at Cloudflare');

  // Disable: removes the slot's resources, releases with the claim it read.
  {
    const h = harness({ tables: { remote_tunnel_slots: { claim_id: 'c7', tunnel_id: 't7', dns_record_id: 'd7' } } });
    const r = await h.call('disable', { machine_id: MACHINE });
    assert.equal(r.status, 200);
    assert(h.calls.cf.includes('DELETE /zones/test/dns_records/d7'));
    assert(h.calls.cf.includes('DELETE /accounts/test/cfd_tunnel/t7'));
    assert.equal(h.calls.rpc.find(([n]) => n === 'release_remote_tunnel_slot')[1].claim, 'c7');
    console.log('disable: removes resources and releases its own claim');
  }

  // Disable when Cloudflare fails: nothing released, error reported.
  {
    const h = harness({
      tables: { remote_tunnel_slots: { claim_id: 'c7', tunnel_id: 't7', dns_record_id: 'd7' } },
      cfFail: { 'DELETE /dns_records': 502 },
    });
    const r = await h.call('disable', { machine_id: MACHINE });
    assert.equal(r.status, 502);
    assert(!rpcNames(h.calls).includes('release_remote_tunnel_slot'));
    console.log('disable: a Cloudflare failure keeps the slot for a retry');
  }

  // Disable for a computer with nothing: fine, and resources already gone are fine.
  {
    const h = harness();
    assert.equal((await h.call('disable', { machine_id: MACHINE })).status, 200);
    const gone = harness({
      tables: { remote_tunnel_slots: { claim_id: 'c7', tunnel_id: 't7', dns_record_id: 'd7' } },
      cfFail: { 'DELETE /dns_records': 404, 'DELETE /cfd_tunnel/t7': 404 },
    });
    assert.equal((await gone.call('disable', { machine_id: MACHINE })).status, 200);
    console.log('disable: no slot, or already-deleted resources, succeed');
  }

  // Settings -> Devices: the pairing button.
  {
    const ui = fs.readFileSync('crates/mira-server/frontend/src/components/settings/DevicesSection.tsx', 'utf8');
    const start = ui.slice(ui.indexOf('  async function startPairing()'), ui.indexOf('  async function revoke'));
    const js = ts.transpileModule(start, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
    const make = (overrides = {}) => {
      const state = { opens: 0, errors: [], paired: null };
      let resolveBaseline;
      const ctx = {
        pairingStarting: { current: false }, setPairingBusy() {}, setJustPaired() {}, setRemoteBusy() {},
        remote: null, remoteAccessAvailable: () => true, setRemote() {}, getRemote: async () => ({}),
        turnOnRemote: async () => ({ enabled: true }),
        refresh: () => new Promise((r) => (resolveBaseline = r)),
        openPairingCode: async () => { state.opens++; return { code: 'ABCD-EFGH', expires_at: 100 }; },
        setPairing: (v) => (state.paired = v), setError: (e) => e && state.errors.push(e), Set,
        ...overrides,
      };
      vm.runInNewContext(js, ctx);
      return { ctx, state, baseline: (v) => resolveBaseline(v) };
    };

    // Repeated clicks are ignored; the code waits for a fresh device list.
    const t = make();
    const first = t.ctx.startPairing();
    await t.ctx.startPairing();
    assert.equal(t.state.opens, 0);
    t.baseline({ devices: [{ id: 'existing' }] });
    await first;
    assert.equal(t.state.opens, 1);
    assert(t.state.paired.known.has('existing'));
    assert.equal(t.ctx.pairingStarting.current, false);

    // Remote access failing still opens a code, and says why it's local-only.
    const f = make({
      remote: { enabled: false, supported: true },
      turnOnRemote: async () => { throw new Error('Remote access isn\'t set up on the server yet.'); },
    });
    const p = f.ctx.startPairing();
    await new Promise((r) => setTimeout(r, 0));
    f.baseline({ devices: [] });
    await p;
    assert.equal(f.state.opens, 1);
    assert.match(f.state.errors.at(-1), /only works from this network/);
    console.log('pairing: repeated clicks ignored; a remote-access failure still opens a code');
  }

  console.log('remote-access: all checks passed');
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
